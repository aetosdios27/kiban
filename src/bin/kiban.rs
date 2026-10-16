//! Offline inspection and logical backup/restore. Opening performs recovery.
use kiban::db::Kiban;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, path] if command == "check" || command == "inspect" => {
            if !std::path::Path::new(path).join("MANIFEST").is_file() {
                return Err("database MANIFEST does not exist".into());
            }
            let db = Kiban::open(path)?;
            let report = db.check_integrity()?;
            println!(
                "verified {} tables, {} live keys, {} logical bytes",
                report.tables, report.live_keys, report.logical_bytes
            );
        }
        [command, source, destination] if command == "backup" || command == "restore" => {
            if !std::path::Path::new(source).join("MANIFEST").is_file() {
                return Err("source MANIFEST does not exist".into());
            }
            Kiban::open(source)?.backup(destination)?;
            println!("created logical database at {destination}");
        }
        _ => {
            return Err(
                "usage: kiban check|inspect DATABASE; kiban backup|restore SOURCE NEW_DESTINATION"
                    .into(),
            );
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
