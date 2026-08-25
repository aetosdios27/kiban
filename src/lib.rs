//! Kiban is a single-node, embedded, durable, ordered byte-key to
//! byte-value storage engine, currently in phase 5: the database layer.
//!
//! Implemented so far: durable file primitives (`atomic`, `crc32`,
//! `frame`), the memtable (`memtable`), the write-ahead log (`wal`),
//! sstables (`sstable`), the MANIFEST (`manifest`), and the assembled
//! engine handle (`db`). See `docs/design/` for the decisions behind
//! each component.

pub mod atomic;
pub mod crc32;
pub mod frame;
pub mod manifest;
pub mod memtable;
pub mod sstable;
#[cfg(test)]
pub mod testutil;
pub mod wal;
