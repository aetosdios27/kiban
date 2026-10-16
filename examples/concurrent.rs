//! Concurrent writes, a stable snapshot, streaming scans and explicit close.
use kiban::db::SharedKiban;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "concurrent-db".into());
    let db = SharedKiban::open(directory)?;
    db.put(b"greeting", b"before")?;
    let snapshot = db.snapshot()?;
    let writer = db.clone();
    std::thread::spawn(move || writer.put(b"greeting", b"after"))
        .join()
        .expect("writer panicked")?;
    assert_eq!(snapshot.get(b"greeting")?, Some(b"before".to_vec()));
    assert_eq!(db.get(b"greeting")?, Some(b"after".to_vec()));
    for entry in snapshot.iter() {
        let (key, value) = entry?;
        println!(
            "{} = {}",
            String::from_utf8_lossy(&key),
            String::from_utf8_lossy(&value)
        );
    }
    drop(snapshot);
    db.close()?;
    Ok(())
}
