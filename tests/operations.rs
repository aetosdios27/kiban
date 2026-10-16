use kiban::db::{DbError, Kiban};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "kiban-ops-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn offline_cli_rejects_another_process_owner_and_restores_backups() {
    let dir = Directory::new();
    let source = dir.0.join("source");
    let mut db = Kiban::open(&source).unwrap();
    db.put(b"key", b"value").unwrap();
    db.checkpoint().unwrap();
    let cli = env!("CARGO_BIN_EXE_kiban");
    let denied = std::process::Command::new(cli)
        .arg("check")
        .arg(&source)
        .output()
        .unwrap();
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("already owned"));
    drop(db);
    let backup = dir.0.join("backup");
    let restore = dir.0.join("restore");
    for (command, from, to) in [("backup", &source, &backup), ("restore", &backup, &restore)] {
        assert!(
            std::process::Command::new(cli)
                .arg(command)
                .arg(from)
                .arg(to)
                .status()
                .unwrap()
                .success()
        );
    }
    assert!(
        std::process::Command::new(cli)
            .arg("check")
            .arg(&restore)
            .status()
            .unwrap()
            .success()
    );
    let mut restored = Kiban::open(&restore).unwrap();
    assert_eq!(restored.get(b"key").unwrap(), Some(b"value".to_vec()));
    assert!(matches!(
        restored.delete(vec![0; kiban::limits::MAX_KEY_BYTES + 1]),
        Err(DbError::InvalidArgument(_))
    ));
    restored.close().unwrap();
}

#[test]
fn nonexistent_cli_database_is_not_created() {
    let dir = Directory::new();
    let absent = dir.0.join("absent");
    assert!(
        !std::process::Command::new(env!("CARGO_BIN_EXE_kiban"))
            .arg("check")
            .arg(&absent)
            .status()
            .unwrap()
            .success()
    );
    assert!(!absent.exists());
}

#[test]
fn ownership_child() {
    let Some(path) = std::env::var_os("KIBAN_OWNERSHIP_CHILD") else {
        return;
    };
    let path = PathBuf::from(path);
    let mut db = Kiban::open(&path).unwrap();
    db.put(b"acknowledged", b"value").unwrap();
    db.sync().unwrap();
    std::fs::write(path.join("owner-ready"), b"ready").unwrap();
    std::thread::sleep(std::time::Duration::from_secs(60));
}

#[test]
fn killed_process_releases_directory_lock_and_preserves_synced_data() {
    let dir = Directory::new();
    let source = dir.0.join("source");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "ownership_child", "--nocapture"])
        .env("KIBAN_OWNERSHIP_CHILD", &source)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    while !source.join("owner-ready").exists()
        && started.elapsed() < std::time::Duration::from_secs(5)
    {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let ready = source.join("owner-ready").exists();
    let locked = if ready {
        matches!(Kiban::open(&source), Err(DbError::Locked))
    } else {
        false
    };
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(ready && locked);
    let db = Kiban::open(&source).unwrap();
    assert_eq!(db.get(b"acknowledged").unwrap(), Some(b"value".to_vec()));
}
