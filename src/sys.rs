//! Durability-relevant syscall wrappers with test-time fault injection
//! and optional power-loss device simulation.
//!
//! Per `docs/design/fault-injection.md`: production is passthrough;
//! tests can (a) make the Nth checked operation return `EIO`, and
//! (b) enable a simulated volatile device where writes land in an
//! overlay that only `sync` commits to the "disk" — so a simulated
//! power loss discards exactly the unsynced bytes and durability
//! becomes *exactly* assertable, not banded.
//!
//! Reads are not fault-injected: the simulated crash is process death
//! with a warm page cache, and the device simulation models exactly the
//! volatile layers between Kiban and the medium.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- faults

#[derive(Debug)]
struct FaultState {
    /// Fail when the operation counter hits any of these values.
    fail_at: HashSet<usize>,
    counter: usize,
}

thread_local! {
    static FAULT: RefCell<Option<FaultState>> = const { RefCell::new(None) };
}

/// Fails the `index`-th checked operation (0-based) of this thread.
pub fn install_fault(index: usize) {
    install_faults(&[index]);
}

/// Fails every checked operation whose index appears in `indices`.
pub fn install_faults(indices: &[usize]) {
    FAULT.with(|f| {
        *f.borrow_mut() = Some(FaultState {
            fail_at: indices.iter().copied().collect(),
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
                if state.fail_at.contains(&n) {
                    Err(io::Error::other("injected i/o failure"))
                } else {
                    Ok(())
                }
            }
        }
    })
}

// ------------------------------------------------- power-loss simulation

/// One simulated file. `committed` is what survives power loss;
/// `overlay` is written-but-unsynced bytes living in volatile memory.
#[derive(Debug, Default, Clone)]
struct SimFile {
    committed: Vec<u8>,
    overlay: Vec<u8>,
}

thread_local! {
    /// None = passthrough mode; Some = device simulation active.
    static DEVICE: RefCell<Option<HashMap<PathBuf, SimFile>>> =
        const { RefCell::new(None) };
}

/// Routes file operations through the simulated volatile device.
pub fn enable_device_sim() {
    DEVICE.with(|d| {
        *d.borrow_mut() = Some(HashMap::new());
    });
}

pub fn disable_device_sim() {
    DEVICE.with(|d| {
        *d.borrow_mut() = None;
    });
}

/// Discards every overlay: unsynced bytes never existed. Committed bytes
/// remain exactly as last synced.
pub fn power_loss() {
    DEVICE.with(|d| {
        if let Some(files) = d.borrow_mut().as_mut() {
            for f in files.values_mut() {
                f.overlay.clear();
            }
        }
    });
}

fn sim_active() -> bool {
    DEVICE.with(|d| d.borrow().is_some())
}

fn with_sim<R>(f: impl FnOnce(&mut HashMap<PathBuf, SimFile>) -> R) -> Option<R> {
    DEVICE.with(|d| d.borrow_mut().as_mut().map(f))
}

fn merged(file: &SimFile) -> Vec<u8> {
    let mut all = file.committed.clone();
    all.extend_from_slice(&file.overlay);
    all
}

/// Paths of all simulated files under `dir` (empty when simulation is
/// inactive).
pub fn simulated_files_under(dir: &Path) -> Vec<PathBuf> {
    with_sim(|files| {
        files
            .keys()
            .filter(|k| k.starts_with(dir))
            .cloned()
            .collect()
    })
    .unwrap_or_default()
}

/// Existence as the process observes it.
pub fn exists(path: &Path) -> bool {
    let known = with_sim(|files| files.contains_key(path));
    match known {
        Some(b) => b,
        None => path.exists(),
    }
}

pub fn device_sim_active() -> bool {
    sim_active()
}

fn lookup_exists(path: &Path) -> bool {
    with_sim(|files| files.contains_key(path)).unwrap_or(false)
}

/// Applies `f` to the simulated file at `path`, mapping absence to a
/// NotFound error.
fn lookup<R>(path: &Path, f: impl FnOnce(&SimFile) -> R) -> io::Result<R> {
    with_sim(|files| match files.get(path) {
        Some(file) => Some(Ok(f(file))),
        None => Some(Err(io::Error::new(io::ErrorKind::NotFound, "no such file"))),
    })
    .expect("sim active when sim path present")
    .unwrap()
}

// ---------------------------------------------------------------- files

/// A file handle. In simulation mode it addresses the overlay device and
/// never touches the real filesystem; otherwise it wraps `std::fs::File`.
pub struct File {
    inner: Option<std::fs::File>,
    sim_path: Option<PathBuf>,
    sim_pos: u64,
}

impl File {
    pub fn create_new(path: &Path) -> io::Result<File> {
        eprintln!("CREATE_NEW {:?}", path);
        check()?;
        let existed = with_sim(|files| files.contains_key(path));
        if let Some(true) = existed {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "file exists"));
        }
        if existed == Some(false) {
            with_sim(|files| {
                files.insert(path.to_path_buf(), SimFile::default());
            });
            return Ok(File {
                inner: None,
                sim_path: Some(path.to_path_buf()),
                sim_pos: 0,
            });
        }
        let inner = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?;
        Ok(File {
            inner: Some(inner),
            sim_path: None,
            sim_pos: 0,
        })
    }

    pub fn open_rw(path: &Path) -> io::Result<File> {
        if sim_active() {
            if !lookup_exists(path) {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no such file"));
            }
            return Ok(File {
                inner: None,
                sim_path: Some(path.to_path_buf()),
                sim_pos: 0,
            });
        }
        let inner = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        Ok(File {
            inner: Some(inner),
            sim_path: None,
            sim_pos: 0,
        })
    }

    pub fn open_read(path: &Path) -> io::Result<File> {
        let known = with_sim(|files| files.contains_key(path));
        if sim_active() {
            // Unknown paths are allowed here: directories are not
            // simulated, and their only use is an (effectively no-op)
            // sync for directory-entry durability.
            let _ = known;
            return Ok(File {
                inner: None,
                sim_path: Some(path.to_path_buf()),
                sim_pos: 0,
            });
        }
        Ok(File {
            inner: Some(std::fs::File::open(path)?),
            sim_path: None,
            sim_pos: 0,
        })
    }

    pub fn try_clone(&self) -> io::Result<File> {
        match (&self.inner, &self.sim_path) {
            (Some(f), _) => Ok(File {
                inner: Some(f.try_clone()?),
                sim_path: None,
                sim_pos: self.sim_pos,
            }),
            (None, Some(p)) => Ok(File {
                inner: None,
                sim_path: Some(p.clone()),
                sim_pos: self.sim_pos,
            }),
            _ => Err(io::Error::other("unbacked file")),
        }
    }

    pub fn sync_all(&self) -> io::Result<()> {
        check()?;
        self.commit_overlay();
        match &self.inner {
            Some(f) => f.sync_all(),
            None => Ok(()), // simulated dirs / unsimulated paths
        }
    }

    pub fn sync_data(&self) -> io::Result<()> {
        check()?;
        self.commit_overlay();
        match &self.inner {
            Some(f) => f.sync_data(),
            None => Ok(()),
        }
    }

    pub fn set_len(&self, len: u64) -> io::Result<()> {
        check()?;
        if let Some(p) = &self.sim_path {
            with_sim(|files| {
                if let Some(f) = files.get_mut(p) {
                    let committed_len = f.committed.len();
                    let new_total = len as usize;
                    if new_total <= committed_len {
                        f.committed.truncate(new_total);
                        f.overlay.clear();
                    } else {
                        let new_overlay = new_total - committed_len;
                        f.overlay.resize(new_overlay, 0);
                    }
                }
            });
        }
        match &self.inner {
            Some(f) => f.set_len(len),
            None => Ok(()),
        }
    }

    /// File length as the process observes it (committed + overlay).
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> io::Result<u64> {
        if let Some(p) = &self.sim_path {
            return lookup(p.as_path(), |f| merged(f).len() as u64);
        }
        match &self.inner {
            Some(f) => Ok(f.metadata()?.len()),
            None => Err(io::Error::other("unbacked file")),
        }
    }

    /// Positioned read served through the simulated device when active.
    pub fn read_range_at(&self, path_for_sim: &Path, offset: u64, len: u64) -> io::Result<Vec<u8>> {
        if self.sim_path.is_some() || sim_active() {
            let data = lookup(path_for_sim, merged)?;
            let start = offset as usize;
            let end = (offset + len) as usize;
            if end > data.len() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "read past end of file",
                ));
            }
            return Ok(data[start..end].to_vec());
        }
        let mut buf = vec![0u8; len as usize];
        use std::os::unix::fs::FileExt;
        self.inner
            .as_ref()
            .expect("real file")
            .read_exact_at(&mut buf, offset)?;
        Ok(buf)
    }

    fn commit_overlay(&self) {
        if let Some(p) = &self.sim_path {
            with_sim(|files| {
                if let Some(f) = files.get_mut(p) {
                    f.committed.extend_from_slice(&f.overlay);
                    f.overlay.clear();
                }
            });
        }
    }
}

impl Write for File {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        check()?;
        if let Some(p) = &self.sim_path {
            if with_sim(|files| files.get_mut(p).is_some()) != Some(true) {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no such file"));
            }
            with_sim(|files| {
                if let Some(f) = files.get_mut(p) {
                    f.overlay.extend_from_slice(buf);
                }
            });
            return Ok(buf.len());
        }
        self.inner.as_mut().expect("real file").write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(p) = &self.sim_path {
            let data = lookup(p.as_path(), merged)?;
            let pos = self.sim_pos as usize;
            if pos >= data.len() {
                return Ok(0);
            }
            let n = (data.len() - pos).min(buf.len());
            buf[..n].copy_from_slice(&data[pos..pos + n]);
            self.sim_pos += n as u64;
            return Ok(n);
        }
        self.inner.as_mut().expect("real file").read(buf)
    }
}

impl Seek for File {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match self.inner.as_mut() {
            Some(f) => f.seek(pos),
            None => {
                let new = match pos {
                    SeekFrom::Start(o) => o,
                    SeekFrom::End(d) => (self.len()? as i64 + d) as u64,
                    SeekFrom::Current(d) => (self.sim_pos as i64 + d) as u64,
                };
                self.sim_pos = new;
                Ok(new)
            }
        }
    }
}

/// Whole-file read routed through the simulated device when active.
pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    let known = with_sim(|files| files.contains_key(path));
    eprintln!("READ {path:?} known={known:?}");
    if known == Some(true) {
        let handle = File::open_read(path)?;
        let len = handle.len()?;
        return handle.read_range_at(path, 0, len);
    }
    if known == Some(false) && sim_active() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no such file"));
    }
    std::fs::read(path)
}

pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    check()?;
    with_sim(|files| {
        if let Some(f) = files.remove(from) {
            files.insert(to.to_path_buf(), f);
        }
    });
    match (sim_active(), known_real(from)) {
        (true, false) => Ok(()),
        _ => std::fs::rename(from, to),
    }
}

fn known_real(_p: &Path) -> bool {
    // In sim mode real paths may not exist at all; renames of simulated
    // files must not touch the real filesystem.
    !sim_active()
}

pub fn remove_file(path: &Path) -> io::Result<()> {
    eprintln!("REMOVE {:?}", path);
    check()?;
    with_sim(|files| {
        files.remove(path);
    });
    if sim_active() {
        return Ok(());
    }
    std::fs::remove_file(path)
}
