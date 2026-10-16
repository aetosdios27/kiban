//! Exclusive directory ownership, acquired before recovery changes any files.
use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
};

#[derive(Debug)]
pub(crate) struct Ownership {
    // Dropping the last file handle releases the OS lock, including after a crash.
    _file: Option<File>,
}

impl Ownership {
    pub(crate) fn acquire(dir: &Path) -> Result<Self, crate::db::DbError> {
        // The simulated device models a process crash without actual processes.
        if crate::sys::device_sim_active() {
            return Ok(Self { _file: None });
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("LOCK"))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: Some(file) }),
            Err(std::fs::TryLockError::WouldBlock) => Err(crate::db::DbError::Locked),
            Err(std::fs::TryLockError::Error(e)) => {
                Err(crate::db::DbError::Io(io::Error::other(e)))
            }
        }
    }
}
