//! The site's batch, range and snapshot tabs run in the normal test suite.
use std::path::PathBuf;

struct Directory(PathBuf);

impl Directory {
    fn new(label: &str) -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "kiban-site-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn site_batch_example() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = Directory::new("batch")?;
    let directory = &temporary.0;
    // site:begin batch
    use kiban::db::{Kiban, WriteBatch};

    let mut db = Kiban::open(directory)?;
    let mut batch = WriteBatch::new();
    batch.put(b"hello".to_vec(), b"world".to_vec());
    batch.put(b"goodbye".to_vec(), b"moon".to_vec());
    batch.delete(b"obsolete".to_vec());
    db.write(batch)?;
    db.sync()?;
    assert_eq!(db.get(b"hello")?, Some(b"world".to_vec()));
    assert_eq!(db.get(b"goodbye")?, Some(b"moon".to_vec()));
    assert_eq!(db.get(b"obsolete")?, None);
    // site:end
    Ok(())
}

#[test]
fn site_range_example() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = Directory::new("range")?;
    let directory = &temporary.0;
    // site:begin range
    use kiban::db::Kiban;

    let mut db = Kiban::open(directory)?;
    db.put(b"a", b"1")?;
    db.put(b"b", b"2")?;
    db.put(b"c", b"3")?;
    // Byte-wise ascending order, start included and end excluded.
    let entries = db.range(b"a", b"c").collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        entries,
        vec![
            (b"a".to_vec(), b"1".to_vec()),
            (b"b".to_vec(), b"2".to_vec())
        ]
    );
    // site:end
    Ok(())
}

#[test]
fn site_snapshot_example() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = Directory::new("snapshot")?;
    let directory = &temporary.0;
    // site:begin snapshot
    use kiban::db::Kiban;

    let mut db = Kiban::open(directory)?;
    db.put(b"hello", b"before")?;
    let snapshot = db.snapshot();
    db.put(b"hello", b"after")?;
    assert_eq!(db.get_at(&snapshot, b"hello")?, Some(b"before".to_vec()));
    assert_eq!(db.get(b"hello")?, Some(b"after".to_vec()));
    // site:end
    Ok(())
}
