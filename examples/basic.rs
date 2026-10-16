//! Run with `cargo run --example basic -- /path/to/example-db`.
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("kiban-example.db"));
    // site:begin basic
    use kiban::db::Kiban;

    let mut db = Kiban::open(&directory)?;
    db.put(b"hello", b"world")?;
    db.sync()?; // Successful put alone does not promise crash durability.
    assert_eq!(db.get(b"hello")?, Some(b"world".to_vec()));
    // site:end
    drop(db);

    let reopened = Kiban::open(&directory)?;
    assert_eq!(reopened.get(b"hello")?, Some(b"world".to_vec()));
    println!("Persisted hello=world in {}", directory.display());
    Ok(())
}
