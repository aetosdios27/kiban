//! The write-ahead log: one framed record per mutation, explicit
//! durability, replay-and-truncate recovery.
//!
//! Implements `docs/design/wal.md`. `append`-family methods do not make
//! data durable; only a successful `sync()` does.

use std::fmt;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::atomic;
use crate::frame::{FrameReader, FrameWriter, ReadRecordError};
use crate::memtable::Memtable;
use crate::sys;

const OP_PUT: u8 = 0x01;
const OP_DELETE: u8 = 0x02;
const PAYLOAD_FIXED_LEN: usize = 9;

#[derive(Debug)]
pub enum WalError {
    Io(io::Error),
    Corrupt { offset: u64, reason: String },
}

impl fmt::Display for WalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WalError::Io(e) => write!(f, "wal i/o error: {e}"),
            WalError::Corrupt { offset, reason } => {
                write!(f, "wal corrupt at offset {offset}: {reason}")
            }
        }
    }
}

impl std::error::Error for WalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WalError::Io(e) => Some(e),
            WalError::Corrupt { .. } => None,
        }
    }
}

#[derive(Debug)]
pub struct RecoveryReport {
    pub records_replayed: usize,
    pub torn_tail_truncated: bool,
    pub bytes_truncated: u64,
}

fn encode_put(key: &[u8], value: &[u8], payload: &mut Vec<u8>) {
    payload.clear();
    payload.push(OP_PUT);
    payload.extend_from_slice(&(key.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(value.len() as u32).to_le_bytes());
    payload.extend_from_slice(key);
    payload.extend_from_slice(value);
}

fn encode_delete(key: &[u8], payload: &mut Vec<u8>) {
    payload.clear();
    payload.push(OP_DELETE);
    payload.extend_from_slice(&(key.len() as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(key);
}

fn decode_into_memtable(
    payload: &[u8],
    offset: u64,
    memtable: &mut Memtable,
) -> Result<(), WalError> {
    let bad = |reason: String| WalError::Corrupt { offset, reason };
    if payload.len() < PAYLOAD_FIXED_LEN {
        return Err(bad(format!(
            "payload is {} bytes, shorter than the {}-byte header",
            payload.len(),
            PAYLOAD_FIXED_LEN
        )));
    }
    let op = payload[0];
    let klen = u32::from_le_bytes(payload[1..5].try_into().unwrap()) as usize;
    let vlen = u32::from_le_bytes(payload[5..9].try_into().unwrap()) as usize;
    let key_end = PAYLOAD_FIXED_LEN
        .checked_add(klen)
        .ok_or_else(|| bad("key length overflows payload".to_string()))?;
    let value_end = key_end
        .checked_add(vlen)
        .ok_or_else(|| bad("value length overflows payload".to_string()))?;
    if payload.len() != value_end {
        return Err(bad(format!(
            "payload is {} bytes but header declares {}",
            payload.len(),
            value_end
        )));
    }
    let key = &payload[PAYLOAD_FIXED_LEN..key_end];
    match op {
        OP_PUT => memtable.put(key, &payload[key_end..value_end]),
        OP_DELETE => memtable.delete(key),
        other => return Err(bad(format!("unknown op byte {other:#04x}"))),
    }
    Ok(())
}

struct Counting<R> {
    inner: R,
    pos: u64,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

pub struct Wal {
    writer: BufWriter<sys::File>,
    path: PathBuf,
    payload_scratch: Vec<u8>,
}

impl Wal {
    /// Opens (or durably creates) the WAL at `path`, replaying any valid
    /// prefix of an existing log into `memtable`. A torn tail left by a
    /// crash is truncated and reported; corruption short of the tail is
    /// returned as an error and left untouched.
    pub fn open(
        path: impl AsRef<Path>,
        memtable: &mut Memtable,
    ) -> Result<(Wal, RecoveryReport), WalError> {
        let path = path.as_ref().to_path_buf();

        if !sys::exists(&path) {
            atomic::create_durably(&path).map_err(WalError::Io)?;
        }

        let file = sys::File::open_rw(&path).map_err(WalError::Io)?;

        let report = Self::recover(&file, memtable)?;

        let mut writer = BufWriter::new(file);
        writer.seek(SeekFrom::End(0)).map_err(WalError::Io)?;

        Ok((
            Wal {
                writer,
                path,
                payload_scratch: Vec::new(),
            },
            report,
        ))
    }

    fn recover(file: &sys::File, memtable: &mut Memtable) -> Result<RecoveryReport, WalError> {
        let mut reader = FrameReader::new(Counting {
            inner: BufReader::new(file.try_clone().map_err(WalError::Io)?),
            pos: 0,
        });

        let mut records_replayed = 0usize;
        let mut last_good_offset = 0u64;
        let mut outcome = Result::<Option<u64>, ReadRecordError>::Ok(None);

        loop {
            let offset = reader.get_ref().pos;
            match reader.read_record() {
                Ok(Some(payload)) => {
                    decode_into_memtable(&payload, offset, memtable)?;
                    records_replayed += 1;
                    last_good_offset = reader.get_ref().pos;
                }
                Ok(None) => break,
                Err(e @ ReadRecordError::TornTail) => {
                    outcome = Err(e);
                    break;
                }
                Err(ReadRecordError::Io(e)) => return Err(WalError::Io(e)),
                Err(
                    e @ (ReadRecordError::ChecksumMismatch { .. }
                    | ReadRecordError::LengthInvalid(_)),
                ) => {
                    return Err(WalError::Corrupt {
                        offset,
                        reason: e.to_string(),
                    });
                }
            }
        }

        let end = file.len().map_err(WalError::Io)?;
        let bytes_truncated = end.saturating_sub(last_good_offset);
        let torn_tail_truncated = outcome.is_err();

        if bytes_truncated > 0 {
            file.set_len(last_good_offset).map_err(WalError::Io)?;
            file.sync_all().map_err(WalError::Io)?;
        }

        Ok(RecoveryReport {
            records_replayed,
            torn_tail_truncated,
            bytes_truncated,
        })
    }

    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> io::Result<()> {
        encode_put(key.as_ref(), value.as_ref(), &mut self.payload_scratch);
        self.append_scratch()
    }

    pub fn delete(&mut self, key: impl AsRef<[u8]>) -> io::Result<()> {
        encode_delete(key.as_ref(), &mut self.payload_scratch);
        self.append_scratch()
    }

    fn append_scratch(&mut self) -> io::Result<()> {
        let mut frame_writer = FrameWriter::new(&mut self.writer);
        frame_writer.write_record(&self.payload_scratch)?;
        Ok(())
    }

    /// Makes all appended records durable: flushes userspace buffers,
    /// then fdatasyncs. Only after this returns success may the write be
    /// acknowledged; per docs/design/wal.md D6, a failure here must not
    /// be retried into oblivion — treat it as fatal upstream.
    pub fn sync(&mut self) -> io::Result<()> {
        self.writer.flush()?;
        self.writer.get_ref().sync_data()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(test)]
    pub(crate) fn writer_flush_for_test(&mut self) {
        self.writer.flush().unwrap();
    }

    #[cfg(test)]
    pub(crate) fn writer_get_mut_for_test(&mut self) -> &mut sys::File {
        self.writer.get_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memtable::Entry;
    use std::env;
    use std::fs;

    static DIR_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let base = env::temp_dir();
            let n = DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = base.join(format!("kiban-wal-{label}-{}-{}", std::process::id(), n));
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

    fn snapshot(m: &Memtable) -> Vec<(Vec<u8>, Entry)> {
        m.iter().map(|(k, e)| (k.to_vec(), e.clone())).collect()
    }

    #[test]
    fn fresh_wal_is_created_empty_and_replays_nothing() {
        let td = TempDir::new("fresh");
        let wal_path = td.path().join("WAL");
        let mut m = Memtable::new();
        let (mut wal, report) = Wal::open(&wal_path, &mut m).unwrap();
        assert_eq!(report.records_replayed, 0);
        assert!(!report.torn_tail_truncated);
        assert_eq!(snapshot(&m), vec![]);
        wal.put(b"a", b"1").unwrap();
        wal.sync().unwrap();
        drop(wal);

        let mut m2 = Memtable::new();
        let (_, report2) = Wal::open(&wal_path, &mut m2).unwrap();
        assert_eq!(report2.records_replayed, 1);
        assert_eq!(m2.get("a"), Some(b"1".to_vec()));
    }

    #[test]
    fn put_and_delete_roundtrip_through_recovery() {
        let td = TempDir::new("roundtrip");
        let wal_path = td.path().join("WAL");
        {
            let mut m = Memtable::new();
            let (mut wal, _) = Wal::open(&wal_path, &mut m).unwrap();
            wal.put(b"alpha", b"value-one").unwrap();
            wal.put(b"beta", b"value-two").unwrap();
            wal.delete(b"alpha").unwrap();
            wal.put(b"", b"empty-key").unwrap();
            wal.sync().unwrap();
        }
        let mut m = Memtable::new();
        let (_, report) = Wal::open(&wal_path, &mut m).unwrap();
        assert_eq!(report.records_replayed, 4);
        assert_eq!(
            snapshot(&m),
            vec![
                (b"".to_vec(), Entry::Value(b"empty-key".to_vec())),
                (b"alpha".to_vec(), Entry::Tombstone),
                (b"beta".to_vec(), Entry::Value(b"value-two".to_vec())),
            ]
        );
    }

    #[test]
    fn unsynced_appends_survive_process_crash_via_flushed_file() {
        // append() without sync() must still have reached the kernel page
        // cache via the BufWriter once flushed; here we simulate a clean
        // handoff by flushing explicitly, proving the two-step contract:
        // flush gets it to the OS, sync gets it to the device.
        let td = TempDir::new("flush-vs-sync");
        let wal_path = td.path().join("WAL");
        let mut m = Memtable::new();
        let (mut wal, _) = Wal::open(&wal_path, &mut m).unwrap();
        wal.put(b"k", b"v").unwrap();
        wal.writer.flush().unwrap();
        drop(wal);

        let mut m2 = Memtable::new();
        let (_, _) = Wal::open(&wal_path, &mut m2).unwrap();
        assert_eq!(m2.get("k"), Some(b"v".to_vec()));
    }

    #[test]
    fn torn_tail_is_truncated_to_last_valid_record() {
        let td = TempDir::new("torn-tail");
        let wal_path = td.path().join("WAL");
        {
            let mut m = Memtable::new();
            let (mut wal, _) = Wal::open(&wal_path, &mut m).unwrap();
            wal.put(b"good1", b"v1").unwrap();
            wal.put(b"good2", b"v2").unwrap();
            wal.sync().unwrap();
            // simulate a crash mid-append: partial record reaches the OS
            wal.writer.flush().unwrap();
            wal.writer.get_mut().write_all(&[0x00, 0x00, 0x00]).unwrap();
        }

        let mut m = Memtable::new();
        let (mut wal, report) = Wal::open(&wal_path, &mut m).unwrap();
        assert_eq!(report.records_replayed, 2);
        assert!(report.torn_tail_truncated);
        assert!(report.bytes_truncated > 0);
        assert_eq!(m.get("good2"), Some(b"v2".to_vec()));

        // the truncated WAL accepts appends and reopens cleanly
        wal.put(b"after", b"recovery").unwrap();
        wal.sync().unwrap();
        drop(wal);

        let mut m2 = Memtable::new();
        let (_, report2) = Wal::open(&wal_path, &mut m2).unwrap();
        assert_eq!(report2.records_replayed, 3);
        assert!(!report2.torn_tail_truncated);
        assert_eq!(m2.get("after"), Some(b"recovery".to_vec()));
    }

    #[test]
    fn corrupted_record_is_reported_not_truncated() {
        let td = TempDir::new("corrupt");
        let wal_path = td.path().join("WAL");
        {
            let mut m = Memtable::new();
            let (mut wal, _) = Wal::open(&wal_path, &mut m).unwrap();
            wal.put(b"first", b"record").unwrap();
            wal.put(b"second", b"record").unwrap();
            wal.sync().unwrap();
        }
        // flip one payload bit inside the first record's frame region
        let mut raw = fs::read(&wal_path).unwrap();
        let header_len = 8usize;
        raw[header_len + 9 + 1] ^= 0x08;
        fs::write(&wal_path, &raw).unwrap();

        let original_len = raw.len() as u64;
        let mut m = Memtable::new();
        let err = match Wal::open(&wal_path, &mut m) {
            Err(e) => e,
            Ok(_) => panic!("expected corruption error"),
        };
        assert!(matches!(err, WalError::Corrupt { .. }), "got {err:?}");
        // untouched: no truncation happened behind the caller's back
        assert_eq!(fs::metadata(&wal_path).unwrap().len(), original_len);
    }

    #[test]
    fn unknown_op_byte_is_corruption() {
        let mut m = Memtable::new();
        let payload = [0x7Fu8, 1, 0, 0, 0, 0, 0, 0, 0];
        let err = decode_into_memtable(&payload, 42, &mut m).unwrap_err();
        assert!(matches!(err, WalError::Corrupt { offset: 42, .. }));
    }

    #[test]
    fn truncated_payload_header_is_corruption() {
        let mut m = Memtable::new();
        let err = decode_into_memtable(&[OP_PUT, 4, 0], 0, &mut m).unwrap_err();
        assert!(matches!(err, WalError::Corrupt { .. }));
    }

    #[test]
    fn declared_lengths_must_match_payload_exactly() {
        let mut m = Memtable::new();
        let mut payload = vec![OP_PUT];
        payload.extend_from_slice(&16u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(b"short");
        let err = decode_into_memtable(&payload, 0, &mut m).unwrap_err();
        assert!(matches!(err, WalError::Corrupt { .. }));
    }

    #[test]
    fn reopening_after_clean_close_appends_continue() {
        let td = TempDir::new("reopen");
        let wal_path = td.path().join("WAL");
        for round in 0..3 {
            let mut m = Memtable::new();
            let (mut wal, report) = Wal::open(&wal_path, &mut m).unwrap();
            assert_eq!(report.records_replayed, round);
            assert!(!report.torn_tail_truncated);
            wal.put(format!("key-{round}"), "val").unwrap();
            wal.sync().unwrap();
        }
        let mut m = Memtable::new();
        let (_, report) = Wal::open(&wal_path, &mut m).unwrap();
        assert_eq!(report.records_replayed, 3);
        assert_eq!(m.get("key-2"), Some(b"val".to_vec()));
    }
}
