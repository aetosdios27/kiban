//! Durability-relevant syscall wrappers with test-time fault injection
//! and optional power-loss device simulation.
//!
//! Per `ARCHITECTURE.md`: production is passthrough;
//! tests can (a) make the Nth checked operation return `EIO`, and
//! (b) enable a simulated volatile device. File bytes land in an overlay
//! that only a file sync commits; creates, renames and removals are
//! durable only after a sync of the parent directory.
//!
//! [`power_loss`] is strict: every unsynced byte and namespace change is
//! lost, so recovery must equal the last synced state exactly.
//! [`power_loss_seeded`] is adversarial and reproducible: an ordered prefix
//! of namespace changes survives and unsynced bytes may be dropped, kept,
//! cut short, zero-filled or torn by sector.
//!
//! Reads are not fault-injected. Truncation via `set_len` is treated as
//! immediately durable, which is weaker than a real device.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------- faults

#[derive(Debug)]
struct FaultState {
    /// Fail when the operation counter hits any of these values.
    fail_at: HashSet<usize>,
    counter: usize,
    #[cfg(test)]
    fail_read_opens: bool,
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
            #[cfg(test)]
            fail_read_opens: false,
        });
    });
}

/// Extend the same deterministic sweep to fallible handle preparation.
/// Ordinary durability sweeps keep their original syscall numbering.
#[cfg(test)]
pub(crate) fn install_faults_with_read_opens(indices: &[usize]) {
    install_faults(indices);
    FAULT.with(|f| f.borrow_mut().as_mut().unwrap().fail_read_opens = true);
}

#[cfg(test)]
fn check_read_open() -> io::Result<()> {
    let enabled = FAULT.with(|f| f.borrow().as_ref().is_some_and(|s| s.fail_read_opens));
    if enabled { check() } else { Ok(()) }
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

/// One simulated inode. `committed` is what survives power loss;
/// `overlay` is written-but-unsynced bytes living in volatile memory.
#[derive(Debug, Default, Clone)]
struct SimFile {
    committed: Vec<u8>,
    overlay: Vec<u8>,
}

/// A directory-entry change that is visible but not yet durable.
#[derive(Debug, Clone)]
enum NsOp {
    Create {
        path: PathBuf,
        inode: u64,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
        inode: u64,
    },
    Remove {
        path: PathBuf,
    },
}

impl NsOp {
    /// The directory whose fsync makes this change durable.
    fn dir(&self) -> &Path {
        let path = match self {
            NsOp::Create { path, .. } | NsOp::Remove { path } => path,
            NsOp::Rename { to, .. } => to,
        };
        path.parent().unwrap_or(Path::new(""))
    }

    fn apply(&self, names: &mut HashMap<PathBuf, u64>) {
        match self {
            NsOp::Create { path, inode } => {
                names.insert(path.clone(), *inode);
            }
            NsOp::Rename { from, to, inode } => {
                if names.get(from) == Some(inode) {
                    names.remove(from);
                }
                names.insert(to.clone(), *inode);
            }
            NsOp::Remove { path } => {
                names.remove(path);
            }
        }
    }
}

/// A volatile block device under a POSIX-like namespace: file data is
/// durable only after a file sync, and creates, renames and removals are
/// durable only after a sync of the parent directory.
#[derive(Debug, Default)]
struct Device {
    /// The namespace as the running process observes it.
    names: HashMap<PathBuf, u64>,
    /// The namespace that survives power loss.
    durable: HashMap<PathBuf, u64>,
    /// Visible namespace changes not yet made durable, in issue order.
    pending: Vec<NsOp>,
    inodes: HashMap<u64, SimFile>,
    next_inode: u64,
    next_nonce: u64,
}

impl Device {
    fn contains_key(&self, path: &Path) -> bool {
        self.names.contains_key(path)
    }

    fn get(&self, path: &Path) -> Option<&SimFile> {
        self.names.get(path).and_then(|i| self.inodes.get(i))
    }

    fn get_mut(&mut self, path: &Path) -> Option<&mut SimFile> {
        let inode = *self.names.get(path)?;
        self.inodes.get_mut(&inode)
    }

    fn keys(&self) -> impl Iterator<Item = &PathBuf> {
        self.names.keys()
    }

    fn create(&mut self, path: &Path) {
        let inode = self.next_inode;
        self.next_inode += 1;
        self.inodes.insert(inode, SimFile::default());
        self.names.insert(path.to_path_buf(), inode);
        self.pending.push(NsOp::Create {
            path: path.to_path_buf(),
            inode,
        });
    }

    fn rename(&mut self, from: &Path, to: &Path) {
        if let Some(inode) = self.names.remove(from) {
            self.names.insert(to.to_path_buf(), inode);
            self.pending.push(NsOp::Rename {
                from: from.to_path_buf(),
                to: to.to_path_buf(),
                inode,
            });
        }
    }

    fn remove(&mut self, path: &Path) {
        if self.names.remove(path).is_some() {
            self.pending.push(NsOp::Remove {
                path: path.to_path_buf(),
            });
        }
    }

    /// Directory fsync: every pending change to entries of `dir` becomes
    /// durable, in issue order.
    fn sync_dir(&mut self, dir: &Path) {
        let mut rest = Vec::with_capacity(self.pending.len());
        for op in self.pending.drain(..) {
            if op.dir() == dir {
                op.apply(&mut self.durable);
            } else {
                rest.push(op);
            }
        }
        self.pending = rest;
    }

    /// Replaces the visible state with what survives the crash. `keep_ops`
    /// pending namespace changes persist (a journal commits in order);
    /// `tear` decides what remains of each inode's unsynced bytes.
    fn crash(&mut self, keep_ops: usize, mut tear: impl FnMut(&[u8]) -> Vec<u8>) {
        for op in self.pending.drain(..).take(keep_ops) {
            op.apply(&mut self.durable);
        }
        self.pending.clear();
        self.names = self.durable.clone();
        let live: HashSet<u64> = self.names.values().copied().collect();
        self.inodes.retain(|inode, _| live.contains(inode));
        // Inode order, so a seeded tear is independent of map iteration order.
        let mut order: Vec<u64> = self.inodes.keys().copied().collect();
        order.sort_unstable();
        for inode in order {
            let file = self.inodes.get_mut(&inode).expect("live inode");
            let overlay = std::mem::take(&mut file.overlay);
            let survived = tear(&overlay);
            file.committed.extend_from_slice(&survived);
        }
    }
}

type SimDevice = Arc<Mutex<Device>>;

thread_local! {
    /// None = passthrough mode; Some = device simulation active. The device
    /// is shared by `Arc` so engine-owned worker threads can adopt the same
    /// simulated disk as the thread that enabled it; fault injection stays
    /// per-thread.
    static DEVICE: RefCell<Option<SimDevice>> = const { RefCell::new(None) };
}

/// Routes file operations through the simulated volatile device.
pub fn enable_device_sim() {
    DEVICE.with(|d| {
        *d.borrow_mut() = Some(Arc::new(Mutex::new(Device::default())));
    });
}

/// A handle to the calling thread's simulated device, for threads the engine
/// spawns on its behalf.
#[derive(Clone)]
pub(crate) struct DeviceHandle(Option<SimDevice>);

impl DeviceHandle {
    /// Makes the calling thread use the captured device (a no-op when the
    /// capturing thread had none).
    pub(crate) fn adopt(self) {
        if self.0.is_some() {
            DEVICE.with(|d| *d.borrow_mut() = self.0);
        }
    }
}

pub(crate) fn device_handle() -> DeviceHandle {
    DeviceHandle(DEVICE.with(|d| d.borrow().clone()))
}

pub fn disable_device_sim() {
    DEVICE.with(|d| {
        *d.borrow_mut() = None;
    });
}

/// Strict power loss: unsynced bytes and namespace changes not followed by
/// a directory sync never existed. Committed state remains exactly as last
/// synced.
pub fn power_loss() {
    with_sim(|device| device.crash(0, |_| Vec::new()));
}

/// Adversarial power loss, fully determined by `seed`. An ordered prefix of
/// the pending namespace changes persists, and each file keeps all, none, a
/// prefix, a prefix followed by zeros, or an arbitrary set of 512-byte
/// sectors (lost sectors read as zeros) of its unsynced bytes.
pub fn power_loss_seeded(seed: u64) {
    let mut rng = SplitMix64(seed);
    with_sim(|device| {
        let keep_ops = rng.below(device.pending.len() as u64 + 1) as usize;
        device.crash(keep_ops, |overlay| tear(overlay, rng.next()));
    });
}

/// What survives of one file's unsynced bytes for tear decision `r`.
fn tear(overlay: &[u8], r: u64) -> Vec<u8> {
    const SECTOR: usize = 512;
    let mut rng = SplitMix64(r);
    let len = overlay.len();
    match rng.below(5) {
        0 => Vec::new(),
        1 => overlay.to_vec(),
        2 => overlay[..rng.below(len as u64 + 1) as usize].to_vec(),
        3 => {
            let keep = rng.below(len as u64 + 1) as usize;
            let mut out = overlay[..keep].to_vec();
            out.resize(len, 0);
            out
        }
        _ => {
            let mut out = overlay.to_vec();
            for chunk in out.chunks_mut(SECTOR) {
                if rng.below(2) == 0 {
                    chunk.fill(0);
                }
            }
            out
        }
    }
}

/// Small deterministic generator for crash decisions (no dependencies).
pub(crate) struct SplitMix64(pub(crate) u64);

impl SplitMix64 {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform-enough value in `0..n` (n > 0) for test decisions.
    pub(crate) fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn sim_active() -> bool {
    DEVICE.with(|d| d.borrow().is_some())
}

fn with_sim<R>(f: impl FnOnce(&mut Device) -> R) -> Option<R> {
    let device = DEVICE.with(|d| d.borrow().clone())?;
    let mut files = device.lock().expect("simulated device");
    Some(f(&mut files))
}

/// Private WAL identity. Simulation uses a device-local counter so replaying
/// the same sequence of operations produces exactly the same file bytes.
pub(crate) fn wal_nonce() -> u64 {
    if let Some(nonce) = with_sim(|device| {
        let nonce = device.next_nonce;
        device.next_nonce = nonce.checked_add(1).expect("WAL nonce counter exhausted");
        nonce
    }) {
        return nonce;
    }
    use std::hash::BuildHasher;
    std::collections::hash_map::RandomState::new()
        .hash_one((std::time::SystemTime::now(), std::process::id()))
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
        check()?;
        let existed = with_sim(|files| files.contains_key(path));
        if let Some(true) = existed {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "file exists"));
        }
        if existed == Some(false) {
            with_sim(|files| files.create(path));
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
        #[cfg(test)]
        check_read_open()?;
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
        #[cfg(test)]
        check_read_open()?;
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

    /// Positioned read into the caller's buffer. Bulk maintenance readers use
    /// the same device abstraction as foreground reads, without cache leases.
    pub(crate) fn read_exact_at(
        &self,
        path_for_sim: &Path,
        buf: &mut [u8],
        offset: u64,
    ) -> io::Result<()> {
        if self.sim_path.is_some() || sim_active() {
            let data = lookup(path_for_sim, merged)?;
            let start = usize::try_from(offset)
                .map_err(|_| io::Error::other("read offset exceeds address space"))?;
            let end = start
                .checked_add(buf.len())
                .ok_or_else(|| io::Error::other("read range overflow"))?;
            if end > data.len() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "read past end of file",
                ));
            }
            buf.copy_from_slice(&data[start..end]);
            return Ok(());
        }
        use std::os::unix::fs::FileExt;
        self.inner
            .as_ref()
            .expect("real file")
            .read_exact_at(buf, offset)
    }

    /// Positioned read served through the simulated device when active.
    pub fn read_range_at(&self, path_for_sim: &Path, offset: u64, len: u64) -> io::Result<Vec<u8>> {
        let len = usize::try_from(len).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "read exceeds address space")
        })?;
        let mut buf = Vec::new();
        buf.try_reserve_exact(len).map_err(io::Error::other)?;
        buf.resize(len, 0);
        self.read_exact_at(path_for_sim, &mut buf, offset)?;
        Ok(buf)
    }

    /// File sync commits the file's bytes; a sync of a path that is not a
    /// simulated file is a directory sync and commits its entries.
    fn commit_overlay(&self) {
        if let Some(p) = &self.sim_path {
            with_sim(|files| match files.get_mut(p) {
                Some(f) => {
                    let overlay = std::mem::take(&mut f.overlay);
                    f.committed.extend_from_slice(&overlay);
                }
                None => files.sync_dir(p),
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
    with_sim(|files| files.rename(from, to));
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
    check()?;
    with_sim(|files| files.remove(path));
    if sim_active() {
        return Ok(());
    }
    std::fs::remove_file(path)
}
