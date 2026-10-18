//! Kiban is a single-node, embedded, durable, ordered byte-key to
//! byte-value storage engine.
//!
//! It includes a write-ahead log, memtables, SSTables, MANIFEST-backed
//! recovery, background maintenance, snapshots, and shared handles.
//!
//! Start with [`db::Kiban`] for a single owner or [`db::SharedKiban`] for
//! concurrent access. Accepted writes require a successful sync for crash
//! durability. This early release targets Linux; independent database owners
//! opening one directory concurrently are rejected by an OS lock. Low-level modules
//! are exposed for tooling and do not promise stable pre-1.0 APIs.

// Diagnostic clocks and sampling disappear entirely from ordinary builds.
macro_rules! measured {
    ($phase:ident, $operation:expr) => {{
        #[cfg(feature = "diagnostics")]
        {
            crate::diagnostics::measure(crate::diagnostics::Phase::$phase, || $operation)
        }
        #[cfg(not(feature = "diagnostics"))]
        {
            $operation
        }
    }};
}

pub mod atomic;
pub mod background;
pub mod bloom;
pub mod cache;
mod cache_budget;
mod counter;
pub mod crc32;
pub mod db;
#[cfg(feature = "diagnostics")]
pub mod diagnostics;
pub(crate) mod engine_lock;
pub mod file_cache;
pub mod frame;
pub mod governor;
pub mod limits;
pub mod manifest;
pub mod memtable;
mod ownership;
#[cfg(test)]
mod parser_tests;
pub mod range_cache;
pub mod sstable;
pub mod sys;
#[cfg(test)]
pub mod testutil;
pub mod wal;

mod remix;
