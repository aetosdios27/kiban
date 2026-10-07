use std::io;
use std::path::Path;

// File-scoped advisory eviction, never global drop_caches. A clean page may
// remain resident; this is deliberately called "reduced", never "cold".
#[cfg(target_os = "linux")]
pub fn reduce(dir: &Path) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn posix_fadvise(fd: i32, offset: i64, len: i64, advice: i32) -> i32;
    }
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        if !e.file_type()?.is_file() {
            continue;
        }
        let f = std::fs::File::open(e.path())?;
        f.sync_all()?;
        // SAFETY: valid owned FD, Linux POSIX_FADV_DONTNEED=4, full file range.
        let rc = unsafe { posix_fadvise(f.as_raw_fd(), 0, 0, 4) };
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc));
        }
    }
    Ok(())
}
#[cfg(not(target_os = "linux"))]
pub fn reduce(_: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "file cache reduction requires Linux",
    ))
}
