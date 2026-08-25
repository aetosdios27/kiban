//! Durability-relevant syscall wrappers with test-time fault injection.
//!
//! Per `docs/design/fault-injection.md` D1: production is passthrough;
//! tests install a failure index that makes the Nth checked operation
//! return `EIO`. Reads are not intercepted.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use std::cell::RefCell;

#[derive(Debug)]
struct FaultState {
    /// Fail once the operation counter reaches this value.
    fail_at: usize,
    counter: usize,
}

thread_local! {
    static FAULT: RefCell<Option<FaultState>> = const { RefCell::new(None) };
}

/// Fails the `index`-th checked operation (0-based) of this thread.
pub fn install_fault(index: usize) {
    FAULT.with(|f| {
        *f.borrow_mut() = Some(FaultState {
            fail_at: index,
            counter: 0,
        });
    });
}

pub fn clear_fault() {
    FAULT.with(|f| {
        *f.borrow_mut() = None;
    });
}

/// Total checked operations performed by this thread since install.
pub fn op_count() -> usize {
    FAULT.with(|f| f.borrow().as_ref().map_or(0, |s| s.counter))
}

fn check() -> io::Result<()> {
    FAULT.with(|f| {
        let mut b = f.borrow_mut();
        match b.as_mut() {
            None => Ok(()),
            Some(state) => {
                let n = state.counter;
                state.counter += 1;
                if n == state.fail_at {
                    Err(io::Error::new(io::ErrorKind::Other, "injected i/o failure"))
                } else {
                    Ok(())
                }
            }
        }
    })
}

/// A file whose writes and syncs pass through the fault checker.
pub struct File {
    inner: std::fs::File,
}

impl File {
    pub fn create_new(path: &Path) -> io::Result<File> {
        check()?;
        Ok(File {
            inner: std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path)?,
        })
    }

    pub fn open_rw(path: &Path) -> io::Result<File> {
        check()?;
        Ok(File {
            inner: std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)?,
        })
    }

    pub fn open_read(path: &Path) -> io::Result<File> {
        Ok(File {
            inner: std::fs::File::open(path)?,
        })
    }

    pub fn try_clone(&self) -> io::Result<File> {
        Ok(File {
            inner: self.inner.try_clone()?,
        })
    }

    pub fn sync_all(&self) -> io::Result<()> {
        check()?;
        self.inner.sync_all()
    }

    pub fn sync_data(&self) -> io::Result<()> {
        check()?;
        self.inner.sync_data()
    }

    pub fn set_len(&self, len: u64) -> io::Result<()> {
        check()?;
        self.inner.set_len(len)
    }

    pub fn metadata(&self) -> io::Result<std::fs::Metadata> {
        self.inner.metadata()
    }
}

impl Write for File {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        check()?;
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // reads are not intercepted (fault-injection.md D1)
        self.inner.read(buf)
    }
}

impl Seek for File {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl AsRef<std::fs::File> for File {
    fn as_ref(&self) -> &std::fs::File {
        &self.inner
    }
}

pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    check()?;
    std::fs::rename(from, to)
}

pub fn remove_file(path: &Path) -> io::Result<()> {
    check()?;
    std::fs::remove_file(path)
}

pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    std::fs::read(path)
}
