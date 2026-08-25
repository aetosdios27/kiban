//! Kiban is a storage engine, currently in phase 3: the write-ahead log.
//!
//! Implemented so far: atomic, durable publication of file contents
//! (`atomic`), CRC-32 (`crc32`), checksummed record framing (`frame`),
//! the in-memory ordered memtable (`memtable`), and the write-ahead log
//! with replay-and-truncate recovery (`wal`). See `docs/design/` for the
//! decisions behind each component.

pub mod atomic;
pub mod crc32;
pub mod frame;
pub mod memtable;
pub mod wal;
