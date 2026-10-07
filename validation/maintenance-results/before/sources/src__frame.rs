//! Checksummed, length-prefixed record framing for append-only logs.
//!
//! Implements the format specified in `docs/design/record-framing.md`:
//! each record is a 8-byte little-endian header (payload length, CRC-32
//! of payload) followed by the raw payload. The reader classifies every
//! end-of-stream outcome explicitly so recovery code never has to guess.

use std::fmt;
use std::io;

use crate::crc32;

pub const HEADER_LEN: usize = 8;
pub const MAX_RECORD_LEN: u32 = 1 << 30;

#[derive(Debug)]
pub enum ReadRecordError {
    Io(io::Error),
    TornTail,
    LengthInvalid(u32),
    ChecksumMismatch { expected: u32, actual: u32 },
}

impl fmt::Display for ReadRecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadRecordError::Io(e) => write!(f, "i/o error reading framed record: {e}"),
            ReadRecordError::TornTail => {
                write!(f, "framed stream ends mid-record; the tail is torn")
            }
            ReadRecordError::LengthInvalid(len) => {
                write!(f, "framed record declares invalid length {len}")
            }
            ReadRecordError::ChecksumMismatch { expected, actual } => write!(
                f,
                "framed record checksum mismatch (expected {expected:#010x}, got {actual:#010x})"
            ),
        }
    }
}

impl std::error::Error for ReadRecordError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReadRecordError::Io(e) => Some(e),
            _ => None,
        }
    }
}

pub struct FrameWriter<W: io::Write> {
    inner: W,
}

impl<W: io::Write> FrameWriter<W> {
    pub fn new(inner: W) -> Self {
        FrameWriter { inner }
    }

    pub fn write_record(&mut self, payload: &[u8]) -> io::Result<()> {
        let len = u32::try_from(payload.len()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "record exceeds u32 length")
        })?;
        if len > MAX_RECORD_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "record exceeds MAX_RECORD_LEN",
            ));
        }
        let mut header = [0u8; HEADER_LEN];
        header[..4].copy_from_slice(&len.to_le_bytes());
        header[4..].copy_from_slice(&crc32::crc32(payload).to_le_bytes());
        self.inner.write_all(&header)?;
        self.inner.write_all(payload)?;
        Ok(())
    }

    pub fn into_inner(self) -> W {
        self.inner
    }

    pub fn get_ref(&self) -> &W {
        &self.inner
    }
}

pub struct FrameReader<R: io::Read> {
    inner: R,
}

impl<R: io::Read> FrameReader<R> {
    pub fn new(inner: R) -> Self {
        FrameReader { inner }
    }

    /// Reads the next record. `Ok(None)` means clean end of stream.
    pub fn read_record(&mut self) -> Result<Option<Vec<u8>>, ReadRecordError> {
        let mut header = [0u8; HEADER_LEN];
        let mut filled = 0usize;
        while filled < HEADER_LEN {
            match self
                .inner
                .read(&mut header[filled..])
                .map_err(ReadRecordError::Io)?
            {
                0 => {
                    return if filled == 0 {
                        Ok(None)
                    } else {
                        Err(ReadRecordError::TornTail)
                    };
                }
                n => filled += n,
            }
        }

        let len = u32::from_le_bytes(header[..4].try_into().unwrap());
        if len > MAX_RECORD_LEN {
            return Err(ReadRecordError::LengthInvalid(len));
        }
        let expected_crc = u32::from_le_bytes(header[4..].try_into().unwrap());

        let mut payload = vec![0u8; len as usize];
        let mut filled = 0usize;
        while filled < payload.len() {
            match self
                .inner
                .read(&mut payload[filled..])
                .map_err(ReadRecordError::Io)?
            {
                0 => return Err(ReadRecordError::TornTail),
                n => filled += n,
            }
        }

        let actual_crc = crc32::crc32(&payload);
        if actual_crc != expected_crc {
            return Err(ReadRecordError::ChecksumMismatch {
                expected: expected_crc,
                actual: actual_crc,
            });
        }
        Ok(Some(payload))
    }

    pub fn into_inner(self) -> R {
        self.inner
    }

    pub fn get_ref(&self) -> &R {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn frame_all(records: &[&[u8]]) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut w = FrameWriter::new(&mut buf);
        for r in records {
            w.write_record(r).unwrap();
        }
        buf
    }

    #[test]
    fn roundtrip_multiple_records() {
        let buf = frame_all(&[
            b"first",
            b"",
            b"third record with bytes \xf0\x9f\x8c\x8a",
            &[0u8; 4096],
        ]);
        let mut r = FrameReader::new(Cursor::new(buf));
        assert_eq!(r.read_record().unwrap().unwrap(), b"first");
        assert_eq!(r.read_record().unwrap().unwrap(), b"");
        assert_eq!(
            r.read_record().unwrap().unwrap(),
            b"third record with bytes \xf0\x9f\x8c\x8a"
        );
        assert_eq!(r.read_record().unwrap().unwrap(), vec![0u8; 4096]);
        assert!(r.read_record().unwrap().is_none());
    }

    #[test]
    fn empty_stream_reads_clean_eof() {
        let mut r = FrameReader::new(Cursor::new(Vec::new()));
        assert!(r.read_record().unwrap().is_none());
    }

    #[test]
    fn truncated_header_is_torn_tail() {
        let buf = &frame_all(&[b"complete"])[..6];
        let mut r = FrameReader::new(Cursor::new(buf.to_vec()));
        assert!(matches!(r.read_record(), Err(ReadRecordError::TornTail)));
    }

    #[test]
    fn truncated_payload_is_torn_tail() {
        let full = frame_all(&[b"complete", b"victim"]);
        let cut_to = full.len() - 2;
        let mut r = FrameReader::new(Cursor::new(full[..cut_to].to_vec()));
        assert_eq!(r.read_record().unwrap().unwrap(), b"complete");
        assert!(matches!(r.read_record(), Err(ReadRecordError::TornTail)));
    }

    #[test]
    fn corrupted_payload_byte_is_checksum_mismatch() {
        let mut full = frame_all(&[b"payload to corrupt"]);
        *full.last_mut().unwrap() ^= 0x01;
        let mut r = FrameReader::new(Cursor::new(full));
        assert!(matches!(
            r.read_record(),
            Err(ReadRecordError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn corrupted_length_field_is_length_invalid() {
        let mut full = frame_all(&[b"x"]);
        full[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut r = FrameReader::new(Cursor::new(full));
        assert!(matches!(
            r.read_record(),
            Err(ReadRecordError::LengthInvalid(u32::MAX))
        ));
    }

    #[test]
    fn garbage_after_valid_record_is_detected_not_skipped() {
        let mut buf = frame_all(&[b"real record"]);
        buf.extend_from_slice(&[0xAB; 20]);
        let mut r = FrameReader::new(Cursor::new(buf));
        assert_eq!(r.read_record().unwrap().unwrap(), b"real record");
        let second = r.read_record();
        assert!(
            matches!(
                second,
                Err(ReadRecordError::TornTail | ReadRecordError::LengthInvalid(_))
            ),
            "garbage tail must not read as a valid record or silent EOF, got {second:?}"
        );
    }

    #[test]
    fn writer_rejects_over_max_record_len() {
        let mut buf = Vec::new();
        let mut w = FrameWriter::new(&mut buf);
        let err = w.write_record(&vec![0u8; (MAX_RECORD_LEN + 1) as usize]);
        assert!(err.is_err());
    }
}
