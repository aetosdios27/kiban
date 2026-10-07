use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn acknowledged_versions_and_deletes_survive_sigkill() {
    for engine in ["kiban", "rocksdb"] {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "kiban-compare-crash-{}-{engine}-{nonce}",
            std::process::id()
        ));
        let mut child = KillOnDrop(
            Command::new(env!("CARGO_BIN_EXE_kiban-compare"))
                .args(["--crash-probe", engine])
                .arg(&dir)
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let stdout = child.0.stdout.take().unwrap();
        let (send, recv) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line);
            send.send((result, line)).unwrap();
        });
        let (result, line) = recv
            .recv_timeout(Duration::from_secs(60))
            .expect("durable-write child failed to acknowledge within 60s");
        result.unwrap();
        assert_eq!(line, "READY\n");
        reader.join().unwrap();
        child.0.kill().unwrap();
        assert!(!child.0.wait().unwrap().success());
        if engine == "kiban" {
            let db = kiban::db::Kiban::open(&dir).unwrap();
            for key in 0u64..96 {
                let expected = if key < 8 {
                    None
                } else if key < 32 {
                    Some(b"durable".to_vec())
                } else {
                    Some(b"new-durable".to_vec())
                };
                assert_eq!(
                    db.get(key.to_be_bytes()).unwrap(),
                    expected,
                    "{engine} key {key}"
                );
            }
            assert_eq!(db.iter().collect::<Result<Vec<_>, _>>().unwrap().len(), 88);
        } else {
            let db = rocksdb::DB::open_default(&dir).unwrap();
            for key in 0u64..96 {
                let expected = if key < 8 {
                    None
                } else if key < 32 {
                    Some(b"durable".to_vec())
                } else {
                    Some(b"new-durable".to_vec())
                };
                assert_eq!(
                    db.get(key.to_be_bytes()).unwrap(),
                    expected,
                    "{engine} key {key}"
                );
            }
            assert_eq!(
                db.iterator(rocksdb::IteratorMode::Start)
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
                    .len(),
                88
            );
        }
    }
}
