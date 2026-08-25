//! Kiban is a storage engine, currently in phase 2: the memtable and
//! its public semantics.
//!
//! Implemented so far: atomic, durable publication of file contents
//! (`atomic`), CRC-32 (`crc32`), checksummed record framing (`frame`),
//! and the in-memory ordered memtable (`memtable`). See `docs/design/`
//! for the decisions behind each component.

pub mod atomic;
pub mod crc32;
pub mod frame;
pub mod memtable;
