// The landing page's basic example, verbatim. CI runs it from a scratch
// directory because it opens a relative path.
// site:begin basic
use kiban::db::Kiban;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut db = Kiban::open("example-db")?;
    db.put(b"hello", b"world")?;
    db.sync()?; // put alone is not crash-durable
    assert_eq!(db.get(b"hello")?, Some(b"world".to_vec()));
    drop(db);

    let reopened = Kiban::open("example-db")?;
    assert_eq!(reopened.get(b"hello")?, Some(b"world".to_vec()));
    Ok(())
}
// site:end
