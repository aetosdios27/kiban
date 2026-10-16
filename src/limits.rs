//! Limits checked before mutation and before allocating decoded records.
use std::io;
pub const MAX_KEY_BYTES: usize = 1024 * 1024;
pub const MAX_VALUE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_BATCH_OPERATIONS: usize = 100_000;
pub const MAX_RECORD_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn mutation(key: &[u8], value: &[u8]) -> io::Result<()> {
    if key.len() > MAX_KEY_BYTES || value.len() > MAX_VALUE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "key or value exceeds Kiban's size limit",
        ));
    }
    Ok(())
}

pub(crate) fn batch(ops: &[(crate::sstable::Kind, Vec<u8>, Vec<u8>)]) -> io::Result<()> {
    if ops.len() > MAX_BATCH_OPERATIONS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "too many batch operations",
        ));
    }
    let mut bytes = 13usize;
    for (kind, key, value) in ops {
        if *kind == crate::sstable::Kind::Tombstone && !value.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "delete carries a value",
            ));
        }
        mutation(key, value)?;
        bytes = bytes
            .checked_add(9 + key.len() + value.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "batch size overflow"))?;
    }
    if bytes > MAX_RECORD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "batch exceeds record limit",
        ));
    }
    Ok(())
}
