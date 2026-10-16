//! C ABI for Kiban. See `kiban.h` for ownership and pointer contracts.

use kiban::db::{DbError, KibanOptions, ScanEntry, SharedKiban, SharedSnapshot, WriteBatch};
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

pub const OK: i32 = 0;
pub const NOT_FOUND: i32 = 1;
pub const DONE: i32 = 2;
pub const INVALID_ARGUMENT: i32 = 3;
pub const INVALID_HANDLE: i32 = 4;
pub const LOCKED: i32 = 5;
pub const IO: i32 = 6;
pub const CORRUPT: i32 = 7;
pub const COMMIT_FAILED: i32 = 8;
pub const COMMIT_AMBIGUOUS: i32 = 9;
pub const POISONED: i32 = 10;
pub const MAINTENANCE: i32 = 11;
pub const SEQUENCE_EXHAUSTED: i32 = 12;
pub const PANIC: i32 = 13;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KibanBuf {
    pub data: *mut u8,
    pub len: usize,
}

impl Default for KibanBuf {
    fn default() -> Self {
        Self {
            data: std::ptr::null_mut(),
            len: 0,
        }
    }
}

/// Zero selects the engine default for each option.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KibanOpenOptions {
    pub write_buffer_bytes: usize,
    pub block_cache_bytes: usize,
    pub max_open_table_files: usize,
}

/// Kind 1 is put; kind 2 is delete (value ignored).
#[repr(C)]
pub struct KibanBatchOp {
    pub kind: u32,
    pub key: *const u8,
    pub key_len: usize,
    pub value: *const u8,
    pub value_len: usize,
}

type Failure = (i32, String);
type Result<T> = std::result::Result<T, Failure>;
type Database = RwLock<Option<SharedKiban>>;
type Entries = Mutex<std::vec::IntoIter<ScanEntry>>;

#[derive(Clone)]
enum Handle {
    Db(Arc<Database>),
    Snapshot(Arc<SharedSnapshot>),
    Iter(Arc<Entries>),
}

#[derive(Default)]
struct Registry {
    next: u64,
    handles: HashMap<u64, Handle>,
    buffers: HashMap<usize, Box<[u8]>>,
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
thread_local! {
    static LAST_ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

fn registry() -> Result<std::sync::MutexGuard<'static, Registry>> {
    REGISTRY
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| (PANIC, "FFI registry poisoned".into()))
}

fn invalid(message: &str) -> Failure {
    (INVALID_ARGUMENT, message.into())
}
fn bad_handle() -> Failure {
    (
        INVALID_HANDLE,
        "invalid, closed, or wrong-kind handle".into(),
    )
}

fn db_error(error: DbError) -> Failure {
    let code = match &error {
        DbError::InvalidArgument(_) => INVALID_ARGUMENT,
        DbError::Locked => LOCKED,
        DbError::Io(_) => IO,
        DbError::Corrupt(_) => CORRUPT,
        DbError::CommitFailed(_) => COMMIT_FAILED,
        DbError::CommitAmbiguous(_) => COMMIT_AMBIGUOUS,
        DbError::Poisoned(_) => POISONED,
        DbError::Maintenance(_) => MAINTENANCE,
        DbError::SequenceExhausted => SEQUENCE_EXHAUSTED,
    };
    (code, error.to_string())
}

fn boundary(f: impl FnOnce() -> Result<()>) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(f));
    let failure = match result {
        Ok(Ok(())) => return OK,
        Ok(Err(error)) => error,
        Err(_) => (PANIC, "panic caught at Kiban FFI boundary".into()),
    };
    LAST_ERROR.with(|last| *last.borrow_mut() = failure.1);
    failure.0
}

fn insert(handle: Handle) -> Result<u64> {
    let mut reg = registry()?;
    reg.next = reg
        .next
        .checked_add(1)
        .ok_or_else(|| (PANIC, "handle IDs exhausted".into()))?;
    let id = reg.next;
    reg.handles.insert(id, handle);
    Ok(id)
}

fn lookup(id: u64) -> Result<Handle> {
    registry()?.handles.get(&id).cloned().ok_or_else(bad_handle)
}

fn with_db<T>(id: u64, f: impl FnOnce(&SharedKiban) -> Result<T>) -> Result<T> {
    let Handle::Db(db) = lookup(id)? else {
        return Err(bad_handle());
    };
    let guard = db
        .read()
        .map_err(|_| (PANIC, "database handle poisoned".into()))?;
    f(guard.as_ref().ok_or_else(bad_handle)?)
}

fn snapshot(id: u64) -> Result<Arc<SharedSnapshot>> {
    let Handle::Snapshot(snap) = lookup(id)? else {
        return Err(bad_handle());
    };
    Ok(snap)
}

// Pointer validity beyond null, length, and alignment is the caller's contract.
unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8]> {
    if len == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() || len > isize::MAX as usize {
        return Err(invalid("invalid byte slice"));
    }
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

unsafe fn output<'a, T>(ptr: *mut T, initial: T) -> Result<&'a mut T> {
    if ptr.is_null() || !(ptr as usize).is_multiple_of(std::mem::align_of::<T>()) {
        return Err(invalid("null or misaligned output pointer"));
    }
    unsafe {
        ptr.write(initial);
        Ok(&mut *ptr)
    }
}

fn buffer(value: Vec<u8>) -> Result<KibanBuf> {
    if value.is_empty() {
        return Ok(KibanBuf::default());
    }
    let mut value = value.into_boxed_slice();
    let result = KibanBuf {
        data: value.as_mut_ptr(),
        len: value.len(),
    };
    registry()?.buffers.insert(result.data as usize, value);
    Ok(result)
}

fn found(value: Option<Vec<u8>>) -> Result<KibanBuf> {
    buffer(value.ok_or_else(|| (NOT_FOUND, "key not found".into()))?)
}

/// # Safety
/// All pointers must satisfy the contracts in `kiban.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kiban_open(
    path: *const u8,
    path_len: usize,
    options: *const KibanOpenOptions,
    out: *mut u64,
) -> i32 {
    boundary(|| {
        let out = unsafe { output(out, 0)? };
        let path = unsafe { bytes(path, path_len)? };
        if path.is_empty() || path.contains(&0) {
            return Err(invalid("path must be nonempty without NUL"));
        }
        let mut opts = KibanOptions::default();
        if !options.is_null() {
            if !(options as usize).is_multiple_of(std::mem::align_of::<KibanOpenOptions>()) {
                return Err(invalid("misaligned options"));
            }
            let options = unsafe { *options };
            if options.write_buffer_bytes != 0 {
                opts.write_buffer_bytes = options.write_buffer_bytes;
            }
            if options.block_cache_bytes != 0 {
                opts.block_cache_bytes = options.block_cache_bytes;
            }
            if options.max_open_table_files != 0 {
                opts.max_open_table_files = options.max_open_table_files;
            }
        }
        let db = SharedKiban::open_with_options(std::ffi::OsStr::from_bytes(path), opts)
            .map_err(db_error)?;
        *out = insert(Handle::Db(Arc::new(RwLock::new(Some(db)))))?;
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn kiban_close(id: u64) -> i32 {
    boundary(|| {
        let Handle::Db(db) = lookup(id)? else {
            return Err(bad_handle());
        };
        let mut guard = db
            .write()
            .map_err(|_| (PANIC, "database handle poisoned".into()))?;
        let db = guard.take().ok_or_else(bad_handle)?;
        registry()?.handles.remove(&id);
        db.close().map_err(db_error)
    })
}

/// # Safety
/// All pointers must satisfy the contracts in `kiban.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kiban_put(
    id: u64,
    key: *const u8,
    key_len: usize,
    value: *const u8,
    value_len: usize,
) -> i32 {
    boundary(|| {
        with_db(id, |db| {
            db.put(unsafe { bytes(key, key_len)? }, unsafe {
                bytes(value, value_len)?
            })
            .map_err(db_error)
        })
    })
}

/// # Safety
/// All pointers must satisfy the contracts in `kiban.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kiban_get(
    id: u64,
    key: *const u8,
    key_len: usize,
    out: *mut KibanBuf,
) -> i32 {
    boundary(|| {
        let out = unsafe { output(out, KibanBuf::default())? };
        *out = with_db(id, |db| {
            found(db.get(unsafe { bytes(key, key_len)? }).map_err(db_error)?)
        })?;
        Ok(())
    })
}

/// # Safety
/// All pointers must satisfy the contracts in `kiban.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kiban_delete(id: u64, key: *const u8, key_len: usize) -> i32 {
    boundary(|| {
        with_db(id, |db| {
            db.delete(unsafe { bytes(key, key_len)? }).map_err(db_error)
        })
    })
}

/// # Safety
/// All pointers must satisfy the contracts in `kiban.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kiban_write_batch(id: u64, ops: *const KibanBatchOp, count: usize) -> i32 {
    boundary(|| {
        if count > isize::MAX as usize / std::mem::size_of::<KibanBatchOp>()
            || (count != 0
                && (ops.is_null()
                    || !(ops as usize).is_multiple_of(std::mem::align_of::<KibanBatchOp>())))
        {
            return Err(invalid("invalid batch array"));
        }
        let ops = if count == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(ops, count) }
        };
        let mut batch = WriteBatch::new();
        for op in ops {
            let key = unsafe { bytes(op.key, op.key_len)? };
            match op.kind {
                1 => {
                    batch.put(key, unsafe { bytes(op.value, op.value_len)? });
                }
                2 => {
                    batch.delete(key);
                }
                _ => return Err(invalid("batch kind must be 1 (put) or 2 (delete)")),
            }
        }
        with_db(id, |db| db.write(batch).map_err(db_error))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn kiban_sync(id: u64) -> i32 {
    boundary(|| with_db(id, |db| db.sync().map_err(db_error)))
}
#[unsafe(no_mangle)]
pub extern "C" fn kiban_flush(id: u64) -> i32 {
    boundary(|| with_db(id, |db| db.flush().map_err(db_error)))
}
#[unsafe(no_mangle)]
pub extern "C" fn kiban_checkpoint(id: u64) -> i32 {
    boundary(|| with_db(id, |db| db.checkpoint().map_err(db_error)))
}

/// # Safety
/// All pointers must satisfy the contracts in `kiban.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kiban_snapshot_create(id: u64, out: *mut u64) -> i32 {
    boundary(|| {
        let out = unsafe { output(out, 0)? };
        *out = with_db(id, |db| {
            insert(Handle::Snapshot(Arc::new(db.snapshot().map_err(db_error)?)))
        })?;
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn kiban_snapshot_release(id: u64) -> i32 {
    boundary(|| {
        let mut reg = registry()?;
        if !matches!(reg.handles.get(&id), Some(Handle::Snapshot(_))) {
            return Err(bad_handle());
        }
        reg.handles.remove(&id);
        Ok(())
    })
}

/// # Safety
/// All pointers must satisfy the contracts in `kiban.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kiban_get_at_snapshot(
    id: u64,
    key: *const u8,
    key_len: usize,
    out: *mut KibanBuf,
) -> i32 {
    boundary(|| {
        let out = unsafe { output(out, KibanBuf::default())? };
        *out = found(
            snapshot(id)?
                .get(unsafe { bytes(key, key_len)? })
                .map_err(db_error)?,
        )?;
        Ok(())
    })
}

// Capture a fresh view for DB handles; existing snapshots can also be scanned.
fn scan_view(id: u64) -> Result<Arc<SharedSnapshot>> {
    match lookup(id)? {
        Handle::Snapshot(snap) => Ok(snap),
        Handle::Db(_) => with_db(id, |db| Ok(Arc::new(db.snapshot().map_err(db_error)?))),
        Handle::Iter(_) => Err(bad_handle()),
    }
}

/// # Safety
/// All pointers must satisfy the contracts in `kiban.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kiban_scan_range(
    id: u64,
    start: *const u8,
    start_len: usize,
    end: *const u8,
    end_len: usize,
    out: *mut u64,
) -> i32 {
    boundary(|| {
        let out = unsafe { output(out, 0)? };
        let start = unsafe { bytes(start, start_len)? };
        let snap = scan_view(id)?;
        let entries: std::result::Result<Vec<_>, _> = if end.is_null() {
            if end_len != 0 {
                return Err(invalid("null end requires zero length"));
            }
            snap.iter()
                .filter(|item| {
                    item.as_ref()
                        .map_or(true, |(key, _)| key.as_slice() >= start)
                })
                .collect()
        } else {
            let end = unsafe { bytes(end, end_len)? };
            snap.range(start, end).collect()
        };
        *out = insert(Handle::Iter(Arc::new(Mutex::new(
            entries.map_err(db_error)?.into_iter(),
        ))))?;
        Ok(())
    })
}

/// # Safety
/// All pointers must satisfy the contracts in `kiban.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kiban_scan_prefix(
    id: u64,
    prefix: *const u8,
    prefix_len: usize,
    out: *mut u64,
) -> i32 {
    boundary(|| {
        let out = unsafe { output(out, 0)? };
        let prefix = unsafe { bytes(prefix, prefix_len)? };
        let snap = scan_view(id)?;
        let end = prefix.iter().rposition(|&byte| byte != 255).map(|i| {
            let mut end = prefix[..=i].to_vec();
            end[i] += 1;
            end
        });
        let entries: std::result::Result<Vec<_>, _> = match end {
            Some(end) => snap.range(prefix, &end).collect(),
            None => snap
                .iter()
                .filter(|item| {
                    item.as_ref()
                        .map_or(true, |(key, _)| key.starts_with(prefix))
                })
                .collect(),
        };
        *out = insert(Handle::Iter(Arc::new(Mutex::new(
            entries.map_err(db_error)?.into_iter(),
        ))))?;
        Ok(())
    })
}

/// # Safety
/// All pointers must satisfy the contracts in `kiban.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kiban_iter_next(id: u64, key: *mut KibanBuf, value: *mut KibanBuf) -> i32 {
    boundary(|| {
        if key == value {
            return Err(invalid("key and value outputs must be distinct"));
        }
        let key = unsafe { output(key, KibanBuf::default())? };
        let value = unsafe { output(value, KibanBuf::default())? };
        let Handle::Iter(iter) = lookup(id)? else {
            return Err(bad_handle());
        };
        let (k, v) = iter
            .lock()
            .map_err(|_| (PANIC, "iterator poisoned".into()))?
            .next()
            .ok_or_else(|| (DONE, "iterator exhausted".into()))?;
        *key = buffer(k)?;
        *value = buffer(v)?;
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn kiban_iter_free(id: u64) -> i32 {
    boundary(|| {
        let mut reg = registry()?;
        if !matches!(reg.handles.get(&id), Some(Handle::Iter(_))) {
            return Err(bad_handle());
        }
        reg.handles.remove(&id);
        Ok(())
    })
}

/// # Safety
/// All pointers must satisfy the contracts in `kiban.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kiban_last_error(out: *mut KibanBuf) -> i32 {
    boundary(|| {
        let out = unsafe { output(out, KibanBuf::default())? };
        *out = buffer(LAST_ERROR.with(|last| last.borrow().as_bytes().to_vec()))?;
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn kiban_free_buf(buf: KibanBuf) -> i32 {
    boundary(|| {
        if buf.data.is_null() && buf.len == 0 {
            return Ok(());
        }
        let mut reg = registry()?;
        let stored = reg
            .buffers
            .get(&(buf.data as usize))
            .ok_or_else(|| invalid("unknown or freed buffer"))?;
        if stored.len() != buf.len {
            return Err(invalid("incorrect buffer length"));
        }
        reg.buffers.remove(&(buf.data as usize));
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_panics() {
        assert_eq!(boundary(|| panic!("test panic")), PANIC);
        assert_eq!(kiban_sync(0), INVALID_HANDLE);
    }
}
