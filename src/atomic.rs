//! Atomic, durable publication of file contents.
//!
//! Implements the policy decided in `docs/design/atomic-commit.md`:
//! temp-file write, fsync, rename, directory fsync.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub const TEMP_MARKER: &str = ".kiban-tmp.";

#[derive(Debug)]
pub enum CommitError {
    Failed(io::Error),
    RenamedNotDurable(io::Error),
}

impl fmt::Display for CommitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommitError::Failed(e) => {
                write!(
                    f,
                    "atomic commit failed before rename; target unmodified: {e}"
                )
            }
            CommitError::RenamedNotDurable(e) => {
                write!(
                    f,
                    "atomic commit renamed the target but the rename is not known to be durable: {e}"
                )
            }
        }
    }
}

impl std::error::Error for CommitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CommitError::Failed(e) | CommitError::RenamedNotDurable(e) => Some(e),
        }
    }
}

fn temp_path(dir: &Path, target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("target");
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".{name}{TEMP_MARKER}{}.{}", std::process::id(), n))
}

fn remove_temp(temp: &Path) {
    let _ = fs::remove_file(temp);
}

pub fn commit_file(target: &Path, contents: &[u8]) -> Result<(), CommitError> {
    let dir = match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };

    let pre_rename = || -> io::Result<()> {
        let temp = temp_path(&dir, target);
        let cleanup_guard = CleanupOnDrop {
            path: &temp,
            armed: true,
        };
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        cleanup_guard.disarm();
        fs::rename(&temp, target)?;
        Ok(())
    };

    match pre_rename() {
        Ok(()) => {}
        Err(e) => return Err(CommitError::Failed(e)),
    }

    let dir_handle = File::open(&dir).and_then(|d| d.sync_all());
    match dir_handle {
        Ok(()) => Ok(()),
        Err(e) => Err(CommitError::RenamedNotDurable(e)),
    }
}

pub fn create_durably(path: &Path) -> io::Result<()> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.sync_all()?;
    drop(file);
    File::open(&dir)?.sync_all()
}

struct CleanupOnDrop<'a> {
    path: &'a Path,
    armed: bool,
}

impl CleanupOnDrop<'_> {
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for CleanupOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            remove_temp(self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    static DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let base = env::temp_dir();
            let n = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("kiban-test-{label}-{}-{}", std::process::id(), n));
            fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn leftover_temps(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.contains(".kiban-tmp."))
            })
            .collect()
    }

    #[test]
    fn commit_creates_file_with_contents() {
        let td = TempDir::new("create");
        let target = td.path().join("MANIFEST-000001");
        commit_file(&target, b"hello kiban").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"hello kiban");
        assert!(leftover_temps(td.path()).is_empty());
    }

    #[test]
    fn commit_overwrites_existing_atomically() {
        let td = TempDir::new("overwrite");
        let target = td.path().join("MANIFEST-000001");
        commit_file(&target, b"first").unwrap();
        commit_file(&target, b"second").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"second");
        assert!(leftover_temps(td.path()).is_empty());
    }

    #[test]
    fn empty_contents_are_committable() {
        let td = TempDir::new("empty");
        let target = td.path().join("EMPTY");
        commit_file(&target, b"").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"");
    }

    #[test]
    fn missing_directory_fails_without_creating_target() {
        let td = TempDir::new("missing-dir");
        let target = td.path().join("nope").join("MANIFEST-000001");
        let err = commit_file(&target, b"data").unwrap_err();
        assert!(matches!(err, CommitError::Failed(_)));
        assert!(!td.path().join("nope").exists());
    }

    #[test]
    fn relative_target_without_parent_commits_to_cwd() {
        let td = TempDir::new("cwd");
        let prev = env::current_dir().unwrap();
        env::set_current_dir(td.path()).unwrap();
        let result = commit_file(Path::new("FILE"), b"cwd data");
        env::set_current_dir(prev).unwrap();
        result.unwrap();
        assert_eq!(fs::read(td.path().join("FILE")).unwrap(), b"cwd data");
    }

    #[test]
    fn durable_creation_creates_exclusively_and_is_empty() {
        let td = TempDir::new("durable-create");
        let path = td.path().join("WAL");
        create_durably(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"");
        let err = create_durably(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn durable_creation_fails_cleanly_for_missing_directory() {
        let td = TempDir::new("durable-create-missing");
        let path = td.path().join("nope").join("WAL");
        assert!(create_durably(&path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn error_states_are_distinguishable_in_display() {
        let failed = CommitError::Failed(io::Error::other("boom"));
        let ambiguous = CommitError::RenamedNotDurable(io::Error::other("boom"));
        assert!(failed.to_string().contains("target unmodified"));
        assert!(ambiguous.to_string().contains("not known to be durable"));
    }
}
