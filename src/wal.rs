//! The write-ahead log: one framed record per mutation, explicit
//! durability, replay-and-truncate recovery.
//!
//! Implements `ARCHITECTURE.md`. `append`-family methods do not make
//! data durable; only a successful `sync()` does.

use std::fmt;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::atomic;
use crate::frame::{FrameReader, FrameWriter, HEADER_LEN, ReadRecordError};
use crate::memtable::Memtable;
use crate::sstable::Kind;
use crate::sys;

const OP_PUT: u8 = 0x01;
const OP_DELETE: u8 = 0x02;
const OP_PAD: u8 = 0x04;
const OP_SYNC_MARK: u8 = 0x05;
const OP_WAL_HEADER: u8 = 0x06;
const FORMAT_VERSION: u8 = 1;
const WAL_HEADER_LEN: usize = 10;
const SYNC_MARK_LEN: usize = 17;
const SYNC_ALIGNMENT: u64 = 4096;
/// A batch record: one frame, many mutations, one contiguous sequence
/// interval. Atomicity comes from framing — a torn or failed write
/// discards the whole batch at recovery, never a prefix.
pub const OP_BATCH: u8 = 0x03;
const PAYLOAD_FIXED_LEN: usize = 17;

/// Fixed per-op header inside a batch payload:
/// [op u8][klen u32][vlen u32].
const BATCH_OP_FIXED_LEN: usize = 9;

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

/// Which phase of [`Wal::sync`] failed.
#[derive(Debug)]
pub enum SyncPhase {
    /// Bytes never left process buffering; nothing reached the kernel
    /// on their behalf (append-phase failure).
    Flush(io::Error),
    /// Bytes reached the kernel; the fdatasync itself failed and
    /// durability is ambiguous (engine-poisoning.md D2).
    Fdatasync(io::Error),
}

pub struct RecoveryReport {
    pub records_replayed: usize,
    pub torn_tail_truncated: bool,
    pub bytes_truncated: u64,
    /// Highest sequence number seen in the replayed log.
    pub max_sequence: u64,
}

fn encode_put(seq: u64, key: &[u8], value: &[u8], payload: &mut Vec<u8>) {
    payload.clear();
    payload.push(OP_PUT);
    payload.extend_from_slice(&seq.to_le_bytes());
    payload.extend_from_slice(&(key.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(value.len() as u32).to_le_bytes());
    payload.extend_from_slice(key);
    payload.extend_from_slice(value);
}

fn encode_delete(seq: u64, key: &[u8], payload: &mut Vec<u8>) {
    payload.clear();
    payload.push(OP_DELETE);
    payload.extend_from_slice(&seq.to_le_bytes());
    payload.extend_from_slice(&(key.len() as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(key);
}

/// Encodes a whole batch as one WAL payload. Sequence numbers are
/// implicit: the i-th operation receives first_seq + i.
///
/// Layout:
/// ```text
/// [OP_BATCH u8][count u32][first_seq u64]
/// ([op u8][klen u32][vlen u32][key][value]) * count
/// ```
pub(crate) fn encode_batch(
    ops: &[(Kind, Vec<u8>, Vec<u8>)],
    first_seq: u64,
    payload: &mut Vec<u8>,
) {
    payload.clear();
    payload.push(OP_BATCH);
    payload.extend_from_slice(&(ops.len() as u32).to_le_bytes());
    payload.extend_from_slice(&first_seq.to_le_bytes());
    for (kind, key, value) in ops {
        let vlen = if *kind == Kind::Put { value.len() } else { 0 };
        payload.push(if *kind == Kind::Put {
            OP_PUT
        } else {
            OP_DELETE
        });
        payload.extend_from_slice(&(key.len() as u32).to_le_bytes());
        payload.extend_from_slice(&(vlen as u32).to_le_bytes());
        payload.extend_from_slice(key);
        if *kind == Kind::Put {
            payload.extend_from_slice(value);
        }
    }
}

/// Decodes a batch payload, applying every operation with its derived
/// sequence number. Strict validation: count and per-op headers must
/// exactly consume the payload, and sequence numbers must be contiguous
/// from `first_seq`. A violation is corruption, never a partial
/// application.
pub(crate) fn decode_batch_into_memtable(
    payload: &[u8],
    offset: u64,
    memtable: &mut Memtable,
    max_seq: &mut u64,
) -> Result<(), WalError> {
    let bad = |reason: String| WalError::Corrupt { offset, reason };
    // payload[0] is OP_BATCH; the fixed header follows it
    if payload.len() < 13 {
        return Err(bad(format!(
            "batch payload is {} bytes, shorter than the fixed header",
            payload.len()
        )));
    }
    let count = u32::from_le_bytes(payload[1..5].try_into().unwrap()) as usize;
    let first_seq = u64::from_le_bytes(payload[5..13].try_into().unwrap());
    if count == 0
        || count > crate::limits::MAX_BATCH_OPERATIONS
        || count > (payload.len() - 13) / BATCH_OP_FIXED_LEN
    {
        return Err(bad("invalid batch count".to_string()));
    }
    if first_seq == 0 || first_seq.checked_add(count as u64 - 1).is_none() {
        return Err(bad("invalid batch sequence interval".to_string()));
    }
    let mut pos = 13usize;
    let mut decoded = Vec::new();
    decoded
        .try_reserve(count)
        .map_err(|e| bad(format!("batch allocation: {e}")))?;
    let mut latest = std::collections::HashMap::new();
    for i in 0..count {
        let expected_seq = first_seq + i as u64;
        let header_end = pos
            .checked_add(BATCH_OP_FIXED_LEN)
            .filter(|end| *end <= payload.len())
            .ok_or_else(|| bad("batch op header runs past payload".to_string()))?;
        let kind = match payload[pos] {
            OP_PUT => Kind::Put,
            OP_DELETE => Kind::Tombstone,
            other => return Err(bad(format!("unknown batch op byte {other:#04x}"))),
        };
        let klen = u32::from_le_bytes(payload[pos + 1..pos + 5].try_into().unwrap()) as usize;
        let vlen = u32::from_le_bytes(payload[pos + 5..pos + 9].try_into().unwrap()) as usize;
        let key_end = header_end
            .checked_add(klen)
            .ok_or_else(|| bad("batch key length overflow".to_string()))?;
        let end = key_end
            .checked_add(vlen)
            .filter(|end| *end <= payload.len())
            .ok_or_else(|| bad("batch key/value runs past payload".to_string()))?;
        let key = &payload[header_end..key_end];
        let value = &payload[key_end..end];
        crate::limits::mutation(key, value).map_err(|e| bad(e.to_string()))?;
        if kind == Kind::Tombstone && !value.is_empty() {
            return Err(bad("delete carries a value".to_string()));
        }
        let previous = latest
            .get(key)
            .copied()
            .or_else(|| memtable.entry(key).map(|e| e.seq()));
        if previous.is_some_and(|seq| seq >= expected_seq) {
            return Err(bad("batch sequence is not increasing for key".to_string()));
        }
        latest.insert(key, expected_seq);
        decoded.push((kind, key, value, expected_seq));
        pos = end;
    }
    if pos != payload.len() {
        return Err(bad("batch has trailing bytes".to_string()));
    }
    // No mutation until all records, sequence ranges and exact consumption
    // have passed validation. A malformed suffix cannot apply a prefix.
    for (kind, key, value, seq) in decoded {
        match kind {
            Kind::Put => memtable.put(key, value, seq),
            Kind::Tombstone => memtable.delete(key, seq),
        }
        *max_seq = (*max_seq).max(seq);
    }
    Ok(())
}

fn decode_into_memtable(
    payload: &[u8],
    offset: u64,
    memtable: &mut Memtable,
    max_seq: &mut u64,
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
    let seq = u64::from_le_bytes(payload[1..9].try_into().unwrap());
    let klen = u32::from_le_bytes(payload[9..13].try_into().unwrap()) as usize;
    let vlen = u32::from_le_bytes(payload[13..17].try_into().unwrap()) as usize;
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
    crate::limits::mutation(key, &payload[key_end..value_end]).map_err(|e| bad(e.to_string()))?;
    if seq == 0 || memtable.entry(key).is_some_and(|e| e.seq() >= seq) {
        return Err(bad("sequence is not increasing for key".to_string()));
    }
    if op == OP_DELETE && vlen != 0 {
        return Err(bad("delete carries a value".to_string()));
    }
    match op {
        OP_PUT => memtable.put(key, &payload[key_end..value_end], seq),
        OP_DELETE => memtable.delete(key, seq),
        other => return Err(bad(format!("unknown op byte {other:#04x}"))),
    }
    *max_seq = (*max_seq).max(seq);
    Ok(())
}

fn sync_marker(payload: &[u8], offset: u64, nonce: u64) -> Option<u64> {
    if !offset.is_multiple_of(SYNC_ALIGNMENT)
        || payload.len() != SYNC_MARK_LEN
        || payload[0] != OP_SYNC_MARK
        || u64::from_le_bytes(payload[9..17].try_into().unwrap()) != nonce
    {
        return None;
    }
    let synced_end = u64::from_le_bytes(payload[1..9].try_into().unwrap());
    (synced_end <= offset).then_some(synced_end)
}

fn later_sync_marker(file: &sys::File, offset: u64, nonce: Option<u64>) -> io::Result<bool> {
    let Some(nonce) = nonce else {
        return Ok(false);
    };
    let mut probe = file.try_clone()?;
    let end = file.len()?;
    let mut boundary = offset - offset % SYNC_ALIGNMENT;
    let mut bytes = [0u8; HEADER_LEN + SYNC_MARK_LEN];
    while let Some(next) = boundary.checked_add(SYNC_ALIGNMENT) {
        boundary = next;
        if end.saturating_sub(boundary) < bytes.len() as u64 {
            return Ok(false);
        }
        probe.seek(SeekFrom::Start(boundary))?;
        probe.read_exact(&mut bytes)?;
        if let Ok(Some(payload)) = FrameReader::new(bytes.as_slice()).read_record()
            && sync_marker(&payload, boundary, nonce).is_some_and(|synced_end| synced_end > offset)
        {
            return Ok(true);
        }
    }
    Ok(false)
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
    position: u64,
    pending_marker: Option<u64>,
    nonce: Option<u64>,
}

impl Wal {
    /// Persist the header in the existing durable-create file sync, before
    /// syncing the directory entry. This adds a write, but no extra fsync.
    pub(crate) fn create_durably(path: &Path) -> io::Result<()> {
        let mut payload = [0u8; WAL_HEADER_LEN];
        payload[0] = OP_WAL_HEADER;
        payload[1] = FORMAT_VERSION;
        payload[2..].copy_from_slice(&sys::wal_nonce().to_le_bytes());
        let mut bytes = Vec::with_capacity(HEADER_LEN + WAL_HEADER_LEN);
        FrameWriter::new(&mut bytes).write_record(&payload)?;
        atomic::create_durably_with_contents(path, &bytes)
    }

    /// Opens (or durably creates) the WAL at `path`, replaying any valid
    /// prefix of an existing log into `memtable`. A torn tail left by a
    /// crash is truncated and reported. A surviving later sync marker
    /// makes damage in its synced prefix an error, leaving the log untouched.
    pub fn open(
        path: impl AsRef<Path>,
        memtable: &mut Memtable,
    ) -> Result<(Wal, RecoveryReport), WalError> {
        let path = path.as_ref().to_path_buf();

        if !sys::exists(&path) {
            Self::create_durably(&path).map_err(WalError::Io)?;
        }

        let mut file = sys::File::open_rw(&path).map_err(WalError::Io)?;

        let (report, mut nonce) = Self::recover(&file, memtable)?;

        // A damaged first frame leaves no header to authenticate future markers.
        // Replace it with a fresh identity before acknowledging any new writes.
        if report.bytes_truncated > 0 && file.len().map_err(WalError::Io)? == 0 {
            let fresh_nonce = sys::wal_nonce();
            let mut payload = [0u8; WAL_HEADER_LEN];
            payload[0] = OP_WAL_HEADER;
            payload[1] = FORMAT_VERSION;
            payload[2..].copy_from_slice(&fresh_nonce.to_le_bytes());
            file.seek(SeekFrom::Start(0)).map_err(WalError::Io)?;
            FrameWriter::new(&mut file)
                .write_record(&payload)
                .map_err(WalError::Io)?;
            nonce = Some(fresh_nonce);
        }
        file.sync_all().map_err(WalError::Io)?;
        let mut writer = BufWriter::new(file);
        let end = writer.seek(SeekFrom::End(0)).map_err(WalError::Io)?;

        Ok((
            Wal {
                writer,
                path,
                payload_scratch: Vec::new(),
                position: end,
                pending_marker: nonce
                    .filter(|_| end > (HEADER_LEN + WAL_HEADER_LEN) as u64)
                    .map(|_| end),
                nonce,
            },
            report,
        ))
    }

    fn recover(
        file: &sys::File,
        memtable: &mut Memtable,
    ) -> Result<(RecoveryReport, Option<u64>), WalError> {
        let mut reader = FrameReader::new(Counting {
            inner: BufReader::new(file.try_clone().map_err(WalError::Io)?),
            pos: 0,
        });

        let mut nonce = None;
        let mut records_replayed = 0usize;
        let mut last_good_offset = 0u64;
        let mut max_sequence = 0u64;
        let mut torn_tail_truncated = false;

        loop {
            let offset = reader.get_ref().pos;
            let failure = match reader.read_record() {
                Ok(Some(payload)) => {
                    let metadata = matches!(
                        payload.first(),
                        Some(&OP_PAD | &OP_SYNC_MARK | &OP_WAL_HEADER)
                    );
                    let applied = match payload.first() {
                        Some(&OP_WAL_HEADER)
                            if offset == 0
                                && payload.len() == WAL_HEADER_LEN
                                && payload[1] == FORMAT_VERSION =>
                        {
                            nonce = Some(u64::from_le_bytes(payload[2..].try_into().unwrap()));
                            Ok(())
                        }
                        Some(&OP_PAD) => Ok(()),
                        // Headerless logs have no authenticated markers. Accept
                        // legacy metadata for replay, but never use it as proof.
                        Some(&OP_SYNC_MARK)
                            if nonce.is_none()
                                || nonce.is_some_and(|n| {
                                    sync_marker(&payload, offset, n).is_some()
                                }) =>
                        {
                            Ok(())
                        }
                        Some(&OP_BATCH) => decode_batch_into_memtable(
                            &payload,
                            offset,
                            memtable,
                            &mut max_sequence,
                        ),
                        _ => decode_into_memtable(&payload, offset, memtable, &mut max_sequence),
                    };
                    match applied {
                        Ok(()) => {
                            records_replayed += usize::from(!metadata);
                            last_good_offset = reader.get_ref().pos;
                            continue;
                        }
                        Err(e) => e,
                    }
                }
                Ok(None) => break,
                Err(ReadRecordError::Io(e)) => return Err(WalError::Io(e)),
                Err(
                    e @ (ReadRecordError::TornTail
                    | ReadRecordError::ChecksumMismatch { .. }
                    | ReadRecordError::LengthInvalid(_)),
                ) => WalError::Corrupt {
                    offset,
                    reason: e.to_string(),
                },
            };
            // A writer marker exists only after the sync covering [0, synced_end)
            // succeeded. Only an aligned, valid frame carrying this file's private
            // header nonce counts: values can mimic a marker only by guessing the
            // 64-bit secret (accidental match ~2^-64 per candidate). Deliberate
            // forgery is outside our buggy-not-malicious threat model. Without a
            // readable header, no marker proves durability and damage truncates.
            if later_sync_marker(file, offset, nonce).map_err(WalError::Io)? {
                return Err(failure);
            }
            torn_tail_truncated = true;
            break;
        }

        let end = file.len().map_err(WalError::Io)?;
        let bytes_truncated = end.saturating_sub(last_good_offset);

        if bytes_truncated > 0 {
            file.set_len(last_good_offset).map_err(WalError::Io)?;
        }

        Ok((
            RecoveryReport {
                records_replayed,
                torn_tail_truncated,
                bytes_truncated,
                max_sequence,
            },
            nonce,
        ))
    }

    pub fn put(
        &mut self,
        seq: u64,
        key: impl AsRef<[u8]>,
        value: impl AsRef<[u8]>,
    ) -> io::Result<()> {
        if seq == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "sequence zero is reserved",
            ));
        }
        crate::limits::mutation(key.as_ref(), value.as_ref())?;
        encode_put(seq, key.as_ref(), value.as_ref(), &mut self.payload_scratch);
        self.append_scratch()
    }

    pub fn delete(&mut self, seq: u64, key: impl AsRef<[u8]>) -> io::Result<()> {
        if seq == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "sequence zero is reserved",
            ));
        }
        crate::limits::mutation(key.as_ref(), b"")?;
        encode_delete(seq, key.as_ref(), &mut self.payload_scratch);
        self.append_scratch()
    }

    /// Appends a whole batch as ONE framed record so recovery either
    /// applies every mutation or none (frame atomicity). The i-th op
    /// receives sequence number first_seq + i.
    pub fn append_batch(
        &mut self,
        first_seq: u64,
        ops: &[(Kind, Vec<u8>, Vec<u8>)],
    ) -> io::Result<()> {
        if ops.is_empty() {
            return Ok(());
        }
        crate::limits::batch(ops)?;
        if first_seq == 0 || first_seq.checked_add(ops.len() as u64 - 1).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid batch sequence interval",
            ));
        }
        encode_batch(ops, first_seq, &mut self.payload_scratch);
        self.append_scratch()
    }

    fn pad_to_alignment(&mut self) -> io::Result<()> {
        let remainder = self.position % SYNC_ALIGNMENT;
        if remainder == 0 {
            return Ok(());
        }
        let mut gap = SYNC_ALIGNMENT - remainder;
        if gap < (HEADER_LEN + 1) as u64 {
            gap += SYNC_ALIGNMENT;
        }
        let mut pad = [0u8; SYNC_ALIGNMENT as usize];
        pad[0] = OP_PAD;
        FrameWriter::new(&mut self.writer).write_record(&pad[..gap as usize - HEADER_LEN])?;
        self.position += gap;
        Ok(())
    }

    fn append_scratch(&mut self) -> io::Result<()> {
        if let Some(synced_end) = self.pending_marker {
            self.pad_to_alignment()?;
            let mut marker = [OP_SYNC_MARK; SYNC_MARK_LEN];
            marker[1..9].copy_from_slice(&synced_end.to_le_bytes());
            marker[9..].copy_from_slice(
                &self
                    .nonce
                    .expect("marker requires a WAL header")
                    .to_le_bytes(),
            );
            FrameWriter::new(&mut self.writer).write_record(&marker)?;
            self.position += (HEADER_LEN + marker.len()) as u64;
            self.pending_marker = None;
        }
        let mut frame_writer = FrameWriter::new(&mut self.writer);
        frame_writer.write_record(&self.payload_scratch)?;
        self.position += (HEADER_LEN + self.payload_scratch.len()) as u64;
        Ok(())
    }

    /// Makes all appended records durable: flushes userspace buffers,
    /// then fdatasyncs. Only after this returns success may the write be
    /// acknowledged; per ARCHITECTURE.md D6, a failure here must not
    /// be retried into oblivion — treat it as fatal upstream.
    pub fn sync(&mut self) -> Result<(), SyncPhase> {
        self.pad_to_alignment().map_err(SyncPhase::Flush)?;
        self.writer.flush().map_err(SyncPhase::Flush)?;
        self.writer
            .get_ref()
            .sync_data()
            .map_err(SyncPhase::Fdatasync)?;
        self.pending_marker = self.nonce.map(|_| self.position);
        Ok(())
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
    /// Zero-filled tails truncate unless a later marker proves durability.
    #[test]
    fn zero_filled_tails_truncate_unless_a_later_marker_proves_sync() {
        use super::*;
        let dir = crate::testutil::TempDir::new("wal-zero-tail");
        let path = dir.path().join("1.wal");
        {
            let mut memtable = Memtable::new();
            let (mut wal, _) = Wal::open(&path, &mut memtable).unwrap();
            wal.put(1, b"a", b"1").unwrap();
            wal.put(2, b"b", b"2").unwrap();
            wal.sync().unwrap();
        }
        let valid = std::fs::metadata(&path).unwrap().len();
        for zeros in [1usize, 8, 25, 4096] {
            let mut bytes = std::fs::read(&path).unwrap();
            bytes.truncate(valid as usize);
            bytes.extend(std::iter::repeat_n(0u8, zeros));
            std::fs::write(&path, &bytes).unwrap();
            let mut memtable = Memtable::new();
            let (_, report) = Wal::open(&path, &mut memtable).unwrap();
            assert_eq!(report.records_replayed, 2);
            assert!(report.torn_tail_truncated);
            assert_eq!(report.bytes_truncated, zeros as u64);
            assert_eq!(std::fs::metadata(&path).unwrap().len(), valid);
        }
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend([0u8; 32]);
        bytes.push(1);
        bytes.resize((valid + SYNC_ALIGNMENT) as usize, 0);
        let mut marker = vec![OP_SYNC_MARK];
        marker.extend_from_slice(&(valid + SYNC_ALIGNMENT).to_le_bytes());
        marker.extend_from_slice(&frames(&path)[0].1[2..]);
        FrameWriter::new(&mut bytes).write_record(&marker).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let mut memtable = Memtable::new();
        assert!(matches!(
            Wal::open(&path, &mut memtable),
            Err(WalError::Corrupt { .. })
        ));
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "corrupt log untouched"
        );
    }

    fn frames(path: &Path) -> Vec<(u64, Vec<u8>)> {
        let bytes = fs::read(path).unwrap();
        let mut reader = FrameReader::new(std::io::Cursor::new(bytes));
        let mut frames = Vec::new();
        loop {
            let offset = reader.get_ref().position();
            let Some(payload) = reader.read_record().unwrap() else {
                return frames;
            };
            frames.push((offset, payload));
        }
    }

    #[test]
    fn sync_padding_ends_on_alignment_even_with_a_short_gap() {
        for gap in [0, 1, 8, 9, 10, 4095] {
            let td = TempDir::new("pad");
            let path = td.path().join("WAL");
            let (mut wal, _) = Wal::open(&path, &mut Memtable::new()).unwrap();
            let record_end = 2 * SYNC_ALIGNMENT - gap;
            let overhead =
                (HEADER_LEN + WAL_HEADER_LEN + HEADER_LEN + PAYLOAD_FIXED_LEN + 1) as u64;
            wal.put(1, b"k", vec![1; (record_end - overhead) as usize])
                .unwrap();
            assert_eq!(wal.position, record_end);
            wal.sync().unwrap();
            let expected = if gap == 0 {
                record_end
            } else if gap < 9 {
                3 * SYNC_ALIGNMENT
            } else {
                2 * SYNC_ALIGNMENT
            };
            assert_eq!(wal.position, expected);
            assert_eq!(fs::metadata(&path).unwrap().len(), expected);
            let all = frames(&path);
            let (offset, payload) = all.last().unwrap();
            if gap != 0 {
                assert_eq!(payload[0], OP_PAD);
                assert_eq!(offset + (HEADER_LEN + payload.len()) as u64, expected);
            } else {
                assert_eq!(payload[0], OP_PUT);
            }
        }
    }

    #[test]
    fn markers_are_lazy_and_cover_only_successful_syncs() {
        let td = TempDir::new("lazy-marker");
        let path = td.path().join("WAL");
        let (mut wal, _) = Wal::open(&path, &mut Memtable::new()).unwrap();
        assert_eq!(frames(&path)[0].1[0], OP_WAL_HEADER);
        wal.put(1, b"a", b"1").unwrap();
        wal.delete(2, b"a").unwrap();
        wal.writer.flush().unwrap();
        assert_eq!(
            frames(&path)
                .iter()
                .filter(|(_, p)| p[0] == OP_SYNC_MARK)
                .count(),
            0
        );
        wal.sync().unwrap();
        assert_eq!(
            frames(&path)
                .iter()
                .filter(|(_, p)| p[0] == OP_SYNC_MARK)
                .count(),
            0
        );
        let synced_end = wal.position;
        wal.append_batch(3, &[(Kind::Put, b"b".to_vec(), b"2".to_vec())])
            .unwrap();
        wal.writer.flush().unwrap();
        assert_eq!(frames(&path)[3].1[0], OP_PAD);
        assert_eq!(
            sync_marker(&frames(&path)[4].1, synced_end, wal.nonce.unwrap()),
            Some(synced_end)
        );
        assert_eq!(frames(&path)[4].0, synced_end);

        for fault in [0, 1] {
            crate::sys::install_fault(fault);
            let result = wal.sync();
            crate::sys::clear_fault();
            assert!(matches!(
                (fault, result),
                (0, Err(SyncPhase::Flush(_))) | (1, Err(SyncPhase::Fdatasync(_)))
            ));
            assert!(wal.pending_marker.is_none());
        }
        wal.writer.flush().unwrap();
        assert_eq!(
            frames(&path)
                .iter()
                .filter(|(_, p)| p[0] == OP_SYNC_MARK)
                .count(),
            1
        );
    }

    #[test]
    fn damaged_records_need_a_later_sync_marker_to_be_corruption() {
        for synced in [false, true] {
            for marked in [false, true] {
                if marked && !synced {
                    continue;
                }
                for damage in 0..4 {
                    let td = TempDir::new("damaged-tail");
                    let path = td.path().join("WAL");
                    let (mut wal, _) = Wal::open(&path, &mut Memtable::new()).unwrap();
                    wal.put(1, b"first", [7; 10_000]).unwrap();
                    if synced {
                        wal.sync().unwrap();
                    }
                    if marked {
                        wal.put(2, b"later", b"marker").unwrap();
                    }
                    drop(wal);
                    let mut bytes = fs::read(&path).unwrap();
                    let offset = frames(&path)
                        .into_iter()
                        .find(|(_, p)| p[0] == OP_PUT)
                        .unwrap()
                        .0;
                    let start = offset as usize;
                    match damage {
                        0 => bytes[start + HEADER_LEN + 512..start + HEADER_LEN + 1024].fill(0),
                        1 => bytes[start..start + 4].copy_from_slice(&u32::MAX.to_le_bytes()),
                        2 => {
                            let len =
                                u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap())
                                    as usize;
                            bytes[start + HEADER_LEN] = 0x7f;
                            let crc = crate::crc32::crc32(
                                &bytes[start + HEADER_LEN..start + HEADER_LEN + len],
                            );
                            bytes[start + 4..start + HEADER_LEN]
                                .copy_from_slice(&crc.to_le_bytes());
                        }
                        3 => bytes[start..start + 4]
                            .copy_from_slice(&crate::frame::MAX_RECORD_LEN.to_le_bytes()),
                        _ => unreachable!(),
                    }
                    fs::write(&path, &bytes).unwrap();
                    let mut mem = Memtable::new();
                    let result = Wal::open(&path, &mut mem);
                    if marked {
                        assert!(
                            matches!(result, Err(WalError::Corrupt { offset: x, .. }) if x == offset)
                        );
                        assert_eq!(fs::read(&path).unwrap(), bytes);
                    } else {
                        let (_, report) = result.unwrap();
                        assert!(report.torn_tail_truncated);
                        assert_eq!(report.bytes_truncated, bytes.len() as u64 - offset);
                        assert_eq!(report.records_replayed, 0);
                        assert!(mem.is_empty());
                        assert_eq!(fs::metadata(&path).unwrap().len(), offset);
                    }
                }
            }
        }
    }

    #[test]
    fn legacy_headerless_logs_replay_and_ignore_markers() {
        let td = TempDir::new("legacy");
        let path = td.path().join("WAL");
        let mut payload = Vec::new();
        encode_put(1, b"old", b"version", &mut payload);
        let mut bytes = Vec::new();
        FrameWriter::new(&mut bytes).write_record(&payload).unwrap();
        let mut pad = vec![0; SYNC_ALIGNMENT as usize - bytes.len() - HEADER_LEN];
        pad[0] = OP_PAD;
        FrameWriter::new(&mut bytes).write_record(&pad).unwrap();
        // Both round-3 markers and nonce-bearing markers are ignored as proof.
        for with_nonce in [false, true] {
            let mut legacy = bytes.clone();
            let mut marker = vec![OP_SYNC_MARK];
            marker.extend_from_slice(&SYNC_ALIGNMENT.to_le_bytes());
            if with_nonce {
                marker.extend_from_slice(&42u64.to_le_bytes());
            }
            FrameWriter::new(&mut legacy).write_record(&marker).unwrap();
            fs::write(&path, &legacy).unwrap();
            let mut mem = Memtable::new();
            let (mut wal, report) = Wal::open(&path, &mut mem).unwrap();
            assert_eq!(report.records_replayed, 1);
            assert_eq!(mem.get(b"old"), Some(b"version".to_vec()));
            assert!(wal.nonce.is_none());
            wal.delete(2, b"old").unwrap();
            wal.sync().unwrap();
            drop(wal);
            let (_, report) = Wal::open(&path, &mut Memtable::new()).unwrap();
            assert_eq!(report.records_replayed, 2);
            legacy[HEADER_LEN + PAYLOAD_FIXED_LEN] ^= 1;
            fs::write(&path, &legacy).unwrap();
            let (_, report) = Wal::open(&path, &mut Memtable::new()).unwrap();
            assert!(report.torn_tail_truncated);
            assert_eq!(report.bytes_truncated, legacy.len() as u64);
        }
    }

    #[test]
    fn aligned_records_and_invalid_markers_do_not_prove_durability() {
        for candidate in 0..6 {
            let td = TempDir::new("invalid-marker");
            let path = td.path().join("WAL");
            let (wal, _) = Wal::open(&path, &mut Memtable::new()).unwrap();
            let nonce = wal.nonce.unwrap();
            drop(wal);
            let mut bytes = fs::read(&path).unwrap();
            let header_end = bytes.len();
            bytes.resize(SYNC_ALIGNMENT as usize, 0);
            let mut payload = vec![OP_SYNC_MARK];
            payload.extend_from_slice(&SYNC_ALIGNMENT.to_le_bytes());
            payload.extend_from_slice(&nonce.to_le_bytes());
            match candidate {
                0 => encode_put(1, b"ordinary", b"aligned", &mut payload),
                1 => payload.push(0),
                2 => payload[1..9].copy_from_slice(&(SYNC_ALIGNMENT + 1).to_le_bytes()),
                3 => bytes.push(0),
                5 => payload[9..].copy_from_slice(&(nonce ^ 1).to_le_bytes()),
                _ => {}
            }
            FrameWriter::new(&mut bytes).write_record(&payload).unwrap();
            if candidate == 4 {
                *bytes.last_mut().unwrap() ^= 1;
            }
            fs::write(&path, &bytes).unwrap();
            let (_, report) = Wal::open(&path, &mut Memtable::new()).unwrap();
            assert!(report.torn_tail_truncated);
            assert_eq!(report.bytes_truncated, (bytes.len() - header_end) as u64);
        }
    }

    #[test]
    fn embedded_marker_with_different_nonce_in_torn_unsynced_value_truncates() {
        let td = TempDir::new("embedded-marker");
        let path = td.path().join("WAL");
        let (mut wal, _) = Wal::open(&path, &mut Memtable::new()).unwrap();
        let mut fake = Vec::new();
        let mut payload = vec![OP_SYNC_MARK];
        payload.extend_from_slice(&SYNC_ALIGNMENT.to_le_bytes());
        payload.extend_from_slice(&(wal.nonce.unwrap() ^ 1).to_le_bytes());
        FrameWriter::new(&mut fake).write_record(&payload).unwrap();
        let mut value = vec![7; 10_000];
        let value_start = HEADER_LEN + WAL_HEADER_LEN + HEADER_LEN + PAYLOAD_FIXED_LEN + 1;
        let embedded = SYNC_ALIGNMENT as usize - value_start;
        value[embedded..embedded + fake.len()].copy_from_slice(&fake);
        wal.put(1, b"k", &value).unwrap();
        drop(wal); // Flush, but no sync of this put.
        let mut bytes = fs::read(&path).unwrap();
        assert_eq!(&bytes[4096..4096 + fake.len()], fake);
        bytes[512..1024].fill(0);
        fs::write(&path, &bytes).unwrap();
        let mut mem = Memtable::new();
        let (_, report) = Wal::open(&path, &mut mem).unwrap();
        assert!(report.torn_tail_truncated);
        assert_eq!(report.records_replayed, 0);
        assert!(mem.is_empty());
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            (HEADER_LEN + WAL_HEADER_LEN) as u64
        );
    }

    #[test]
    fn unreadable_header_disables_matching_marker_proof() {
        let td = TempDir::new("damaged-header");
        let path = td.path().join("WAL");
        let (mut wal, _) = Wal::open(&path, &mut Memtable::new()).unwrap();
        wal.put(1, b"k", b"synced").unwrap();
        wal.sync().unwrap();
        wal.put(2, b"later", b"marker").unwrap();
        drop(wal);
        let mut bytes = fs::read(&path).unwrap();
        bytes[HEADER_LEN + 2] ^= 1;
        fs::write(&path, &bytes).unwrap();
        let (_, report) = Wal::open(&path, &mut Memtable::new()).unwrap();
        assert!(report.torn_tail_truncated);
        assert_eq!(report.bytes_truncated, bytes.len() as u64);
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            (HEADER_LEN + WAL_HEADER_LEN) as u64
        );
    }

    #[test]
    fn truncated_header_is_replaced_durably_and_authenticates_new_markers() {
        let td = TempDir::new("rewrite-header");
        let path = td.path().join("WAL");
        sys::enable_device_sim();
        let (wal, _) = Wal::open(&path, &mut Memtable::new()).unwrap();
        let old_nonce = wal.nonce.unwrap();
        drop(wal);
        let mut bytes = sys::read(&path).unwrap();
        bytes[HEADER_LEN + 2] ^= 1;
        {
            let mut file = sys::File::open_rw(&path).unwrap();
            file.set_len(0).unwrap();
            file.write_all(&bytes).unwrap();
            file.sync_all().unwrap();
        }
        let (wal, report) = Wal::open(&path, &mut Memtable::new()).unwrap();
        assert!(report.torn_tail_truncated);
        assert_eq!(report.bytes_truncated, bytes.len() as u64);
        let new_nonce = wal.nonce.unwrap();
        assert_ne!(old_nonce, new_nonce);
        drop(wal);
        let rewritten = sys::read(&path).unwrap();
        sys::power_loss();
        assert_eq!(sys::read(&path).unwrap(), rewritten);
        let (mut wal, _) = Wal::open(&path, &mut Memtable::new()).unwrap();
        assert_eq!(wal.nonce, Some(new_nonce));
        wal.put(1, b"synced", b"value").unwrap();
        wal.sync().unwrap();
        wal.put(2, b"later", b"marker").unwrap();
        wal.writer_flush_for_test();
        drop(wal);
        let mut bytes = sys::read(&path).unwrap();
        bytes[HEADER_LEN + WAL_HEADER_LEN + HEADER_LEN] ^= 1;
        {
            let mut file = sys::File::open_rw(&path).unwrap();
            file.set_len(0).unwrap();
            file.write_all(&bytes).unwrap();
            file.sync_all().unwrap();
        }
        assert!(matches!(
            Wal::open(&path, &mut Memtable::new()),
            Err(WalError::Corrupt { .. })
        ));
        assert_eq!(sys::read(&path).unwrap(), bytes);
        sys::disable_device_sim();
    }

    #[test]
    fn header_is_synced_during_creation_without_an_extra_fsync() {
        struct DeviceGuard;
        impl Drop for DeviceGuard {
            fn drop(&mut self) {
                sys::clear_fault();
                sys::disable_device_sim();
            }
        }
        let td = TempDir::new("durable-header");
        let path = td.path().join("WAL");
        sys::enable_device_sim();
        let _guard = DeviceGuard;
        sys::install_fault(usize::MAX);
        Wal::create_durably(&path).unwrap();
        // create, header write, file fsync, directory fsync.
        assert_eq!(sys::op_count(), 4);
        sys::clear_fault();
        let before = sys::read(&path).unwrap();
        sys::power_loss();
        assert_eq!(sys::read(&path).unwrap(), before);
        let payload = FrameReader::new(before.as_slice())
            .read_record()
            .unwrap()
            .unwrap();
        assert_eq!(payload.len(), WAL_HEADER_LEN);
        assert_eq!(&payload[..2], &[OP_WAL_HEADER, FORMAT_VERSION]);
        let (wal, report) = Wal::open(&path, &mut Memtable::new()).unwrap();
        assert_eq!(
            wal.nonce,
            Some(u64::from_le_bytes(payload[2..].try_into().unwrap()))
        );
        assert_eq!(report.records_replayed, 0);
        drop(wal);
        // New simulated devices repeat their identities; successive WALs differ.
        let next = td.path().join("NEXT");
        Wal::create_durably(&next).unwrap();
        assert_ne!(sys::read(&next).unwrap(), before);
        sys::enable_device_sim();
        Wal::create_durably(&path).unwrap();
        assert_eq!(sys::read(&path).unwrap(), before);
    }

    #[test]
    fn malformed_batch_never_applies_a_prefix() {
        use super::*;
        let ops = vec![
            (Kind::Put, b"a".to_vec(), b"1".to_vec()),
            (Kind::Put, b"b".to_vec(), b"2".to_vec()),
        ];
        let mut payload = Vec::new();
        encode_batch(&ops, 1, &mut payload);
        let mut trailing = payload.clone();
        trailing.push(0);
        let mut invalid_delete = payload.clone();
        invalid_delete[13] = OP_DELETE;
        let mut overflow = payload.clone();
        overflow[5..13].copy_from_slice(&u64::MAX.to_le_bytes());
        for bad in [
            payload[..payload.len() - 1].to_vec(),
            trailing,
            invalid_delete,
            overflow,
        ] {
            let mut mem = Memtable::new();
            let mut max = 0;
            assert!(decode_batch_into_memtable(&bad, 0, &mut mem, &mut max).is_err());
            assert!(mem.is_empty());
            assert_eq!(max, 0);
        }
    }
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
    fn fresh_wal_has_a_header_and_replays_nothing() {
        let td = TempDir::new("fresh");
        let wal_path = td.path().join("WAL");
        let mut m = Memtable::new();
        let (mut wal, report) = Wal::open(&wal_path, &mut m).unwrap();
        assert_eq!(report.records_replayed, 0);
        assert!(!report.torn_tail_truncated);
        assert_eq!(snapshot(&m), vec![]);
        wal.put(1, b"a", b"1").unwrap();
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
            wal.put(2, b"alpha", b"value-one").unwrap();
            wal.put(3, b"beta", b"value-two").unwrap();
            wal.delete(99, b"alpha").unwrap();
            wal.put(4, b"", b"empty-key").unwrap();
            wal.sync().unwrap();
        }
        let mut m = Memtable::new();
        let (_, report) = Wal::open(&wal_path, &mut m).unwrap();
        assert_eq!(report.records_replayed, 4);
        assert_eq!(
            snapshot(&m),
            vec![
                (
                    b"".to_vec(),
                    Entry::Value {
                        value: b"empty-key".to_vec(),
                        seq: 4
                    }
                ),
                (b"alpha".to_vec(), Entry::Tombstone { seq: 99 }),
                (
                    b"beta".to_vec(),
                    Entry::Value {
                        value: b"value-two".to_vec(),
                        seq: 3
                    }
                ),
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
        wal.put(5, b"k", b"v").unwrap();
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
            wal.put(6, b"good1", b"v1").unwrap();
            wal.put(7, b"good2", b"v2").unwrap();
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
        wal.put(8, b"after", b"recovery").unwrap();
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
            wal.put(9, b"first", b"record").unwrap();
            wal.put(10, b"second", b"record").unwrap();
            wal.sync().unwrap();
            wal.put(11, b"after-sync", b"marker").unwrap();
        }
        // flip one payload bit inside the first record's frame region
        let mut raw = fs::read(&wal_path).unwrap();
        let first_record = frames(&wal_path)
            .into_iter()
            .find(|(_, p)| p[0] == OP_PUT)
            .unwrap()
            .0 as usize;
        raw[first_record + HEADER_LEN + 9 + 1] ^= 0x08;
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
        let err = decode_into_memtable(&payload, 42, &mut m, &mut 0).unwrap_err();
        assert!(matches!(err, WalError::Corrupt { offset: 42, .. }));
    }

    #[test]
    fn truncated_payload_header_is_corruption() {
        let mut m = Memtable::new();
        let err = decode_into_memtable(&[OP_PUT, 4, 0], 0, &mut m, &mut 0).unwrap_err();
        assert!(matches!(err, WalError::Corrupt { .. }));
    }

    #[test]
    fn declared_lengths_must_match_payload_exactly() {
        let mut m = Memtable::new();
        let mut payload = vec![OP_PUT];
        payload.extend_from_slice(&16u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(b"short");
        let err = decode_into_memtable(&payload, 0, &mut m, &mut 0).unwrap_err();
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
            wal.put(round as u64 + 1, format!("key-{round}"), "val")
                .unwrap();
            wal.sync().unwrap();
        }
        let mut m = Memtable::new();
        let (_, report) = Wal::open(&wal_path, &mut m).unwrap();
        assert_eq!(report.records_replayed, 3);
        assert_eq!(m.get("key-2"), Some(b"val".to_vec()));
    }
}
