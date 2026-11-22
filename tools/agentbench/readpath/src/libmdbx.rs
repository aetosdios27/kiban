//! libmdbx adapter for the common ordered key and JSON memory model.
use crate::{
    Options,
    api::{Backend, Result},
    model::{Memory, key},
    runner,
};
use mdbx_sys as ffi;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    ffi::{CStr, c_char, c_int, c_uint},
    io::{BufRead, Write},
    marker::PhantomData,
    mem::MaybeUninit,
    path::Path,
    ptr, slice,
    sync::OnceLock,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tuning {
    pub page_bytes: usize,
    pub growth_bytes: isize,
    pub writemap: bool,
    pub no_rdahead: bool,
    pub liforeclaim: bool,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            page_bytes: 4096,
            growth_bytes: 16 << 20,
            writemap: false,
            no_rdahead: false,
            liforeclaim: false,
        }
    }
}

pub fn open(o: &Options) -> Result<Box<dyn Backend>> {
    Ok(Box::new(Mdbx::open(o)?))
}

fn check(rc: i32) -> Result<()> {
    if rc == ffi::MDBX_SUCCESS {
        return Ok(());
    }
    let mut buffer = [0; 256];
    // SAFETY: The buffer is writable. libmdbx returns a NUL-terminated message
    // in this buffer or in static storage.
    let message =
        unsafe { CStr::from_ptr(ffi::mdbx_strerror_r(rc, buffer.as_mut_ptr(), buffer.len())) };
    Err(format!("libmdbx error {rc}: {}", message.to_string_lossy()).into())
}

fn configure_logging() -> Result<()> {
    // The old wrapper forwarded formatted messages to an unconfigured logger.
    // Keep the same native logging path and avoid stderr I/O inside timed calls.
    static SETUP: OnceLock<i32> = OnceLock::new();
    static mut BUFFER: [c_char; 1024] = [0; 1024];
    unsafe extern "C" fn discard(
        _: ffi::MDBX_log_level_t,
        _: *const c_char,
        _: c_int,
        _: *const c_char,
        _: c_uint,
    ) {
    }

    let rc = *SETUP.get_or_init(|| {
        // SAFETY: libmdbx owns use of the process-lifetime buffer after setup.
        unsafe {
            ffi::mdbx_setup_debug_nofmt(
                ffi::MDBX_LOG_DONTCHANGE,
                ffi::MDBX_DBG_DONTCHANGE,
                Some(discard),
                (&raw mut BUFFER).cast(),
                1024,
            )
        }
    });
    if rc < 0 {
        return Err(format!("libmdbx logger setup failed: {rc}").into());
    }
    Ok(())
}

fn check_cleanup(rc: i32) {
    if let Err(error) = check(rc) {
        // Cleanup can't return an error. Stop rather than acknowledge success.
        eprintln!("libmdbx cleanup failed: {error}");
        std::process::abort();
    }
}

fn value(bytes: &[u8]) -> ffi::MDBX_val {
    ffi::MDBX_val {
        iov_base: bytes.as_ptr().cast_mut().cast(),
        iov_len: bytes.len(),
    }
}

// These owners are thread-local (raw pointers are !Send and !Sync).
// A transaction exclusively borrows its environment and aborts on early return.
struct Env(*mut ffi::MDBX_env);

impl Env {
    fn open(&mut self, path: &Path, flags: ffi::MDBX_env_flags_t) -> Result<()> {
        #[cfg(unix)]
        {
            let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())?;
            // SAFETY: The environment owns a newly created handle and the path is a C string.
            check(unsafe { ffi::mdbx_env_open(self.0, path.as_ptr(), flags, 0o644) })
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let mut path: Vec<u16> = path.as_os_str().encode_wide().collect();
            if path.contains(&0) {
                return Err("libmdbx database path contains a NUL".into());
            }
            path.push(0);
            // SAFETY: The handle is newly created and the path is NUL-terminated UTF-16.
            check(unsafe { ffi::mdbx_env_openW(self.0, path.as_ptr().cast(), flags, 0o644) })
        }
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        // SAFETY: No transaction can outlive its borrow of the environment.
        check_cleanup(unsafe { ffi::mdbx_env_close_ex(self.0, false) });
    }
}

struct Txn<'env> {
    raw: *mut ffi::MDBX_txn,
    _env: PhantomData<&'env mut Env>,
}

impl<'env> Txn<'env> {
    fn begin(env: &'env mut Env, flags: ffi::MDBX_txn_flags_t) -> Result<Self> {
        let mut raw = ptr::null_mut();
        // SAFETY: env is open and exclusively borrowed. The out pointer is writable.
        check(unsafe {
            ffi::mdbx_txn_begin_ex(env.0, ptr::null_mut(), flags, &mut raw, ptr::null_mut())
        })?;
        Ok(Self {
            raw,
            _env: PhantomData,
        })
    }

    fn commit(mut self) -> Result<()> {
        // SAFETY: The transaction is live and owned by the current thread.
        let rc = unsafe { ffi::mdbx_txn_commit_ex(self.raw, ptr::null_mut()) };
        // libmdbx consumes the handle even on commit failure except for a thread mismatch.
        if rc != ffi::MDBX_THREAD_MISMATCH {
            self.raw = ptr::null_mut();
        }
        check(rc)
    }

    fn put(&mut self, dbi: ffi::MDBX_dbi, key: &[u8], bytes: &[u8]) -> Result<()> {
        // SAFETY: The DBI belongs to the borrowed environment. UPSERT reads the key
        // and payload during the call and does not write to the borrowed slices.
        check(unsafe {
            ffi::mdbx_put(
                self.raw,
                dbi,
                &value(key),
                &mut value(bytes),
                ffi::MDBX_UPSERT,
            )
        })
    }

    fn lookup(&self, dbi: ffi::MDBX_dbi, key: &[u8]) -> Result<Option<ffi::MDBX_val>> {
        let mut bytes = MaybeUninit::uninit();
        // SAFETY: The DBI belongs to the borrowed environment. On success libmdbx
        // writes a value descriptor that remains valid in this read transaction.
        let rc = unsafe { ffi::mdbx_get(self.raw, dbi, &value(key), bytes.as_mut_ptr()) };
        if rc == ffi::MDBX_NOTFOUND {
            return Ok(None);
        }
        check(rc)?;
        // SAFETY: The successful lookup initialized the descriptor.
        Ok(Some(unsafe { bytes.assume_init() }))
    }
}

impl Drop for Txn<'_> {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: The live transaction is owned by the current thread.
            check_cleanup(unsafe { ffi::mdbx_txn_abort_ex(self.raw, ptr::null_mut()) });
        }
    }
}

struct Cursor(*mut ffi::MDBX_cursor);

impl Drop for Cursor {
    fn drop(&mut self) {
        // SAFETY: The cursor is live and owned by the current thread.
        unsafe { ffi::mdbx_cursor_close(self.0) };
    }
}

// SAFETY: The descriptor must refer to bytes valid for the returned borrow.
unsafe fn bytes(v: &ffi::MDBX_val) -> &[u8] {
    if v.iov_len == 0 {
        return &[];
    }
    // SAFETY: Guaranteed by the caller for this descriptor.
    unsafe { slice::from_raw_parts(v.iov_base.cast(), v.iov_len) }
}

struct Mdbx {
    env: Env,
    dbi: ffi::MDBX_dbi,
}

impl Mdbx {
    fn open(o: &Options) -> Result<Self> {
        configure_logging()?;
        std::fs::create_dir_all(&o.suite.db)?;
        let tuning = &o.libmdbx;
        let page_bytes = isize::try_from(tuning.page_bytes)?;
        let (sync_mode, sync_flags) = if o.durable {
            ("Durable", ffi::MDBX_SYNC_DURABLE)
        } else {
            ("SafeNoSync", ffi::MDBX_SAFE_NOSYNC)
        };
        // Retain the previous wrapper's native environment options.
        let mut flags = ffi::MDBX_NOSTICKYTHREADS | sync_flags;
        for (enabled, flag) in [
            (tuning.writemap, ffi::MDBX_WRITEMAP),
            (tuning.no_rdahead, ffi::MDBX_NORDAHEAD),
            (tuning.liforeclaim, ffi::MDBX_LIFORECLAIM),
        ] {
            if enabled {
                flags |= flag;
            }
        }
        let mut raw = ptr::null_mut();
        // SAFETY: The output points to writable storage. The owner takes over immediately.
        check(unsafe { ffi::mdbx_env_create(&mut raw) })?;
        let mut env = Env(raw);
        // SAFETY: env owns the newly created handle.
        unsafe {
            check(ffi::mdbx_env_set_geometry(
                env.0,
                1 << 20,
                -1,
                10 << 30,
                tuning.growth_bytes,
                0,
                page_bytes,
            ))?;
            check(ffi::mdbx_env_set_option(
                env.0,
                ffi::MDBX_opt_max_readers,
                200,
            ))?;
        }
        env.open(&o.suite.db, flags)?;

        let mut effective = 0;
        let mut stat = MaybeUninit::<ffi::MDBX_stat>::zeroed();
        let mut info = MaybeUninit::<ffi::MDBX_envinfo>::zeroed();
        // SAFETY: env is open, and the writable outputs match the native struct sizes.
        let (stat, info) = unsafe {
            check(ffi::mdbx_env_get_flags(env.0, &mut effective))?;
            check(ffi::mdbx_env_stat_ex(
                env.0,
                ptr::null(),
                stat.as_mut_ptr(),
                size_of::<ffi::MDBX_stat>(),
            ))?;
            check(ffi::mdbx_env_info_ex(
                env.0,
                ptr::null(),
                info.as_mut_ptr(),
                size_of::<ffi::MDBX_envinfo>(),
            ))?;
            (stat.assume_init(), info.assume_init())
        };
        let geometry = info.mi_geo;
        let checked_flags = ffi::MDBX_UTTERLY_NOSYNC
            | ffi::MDBX_NOMETASYNC
            | ffi::MDBX_NOSTICKYTHREADS
            | ffi::MDBX_WRITEMAP
            | ffi::MDBX_NORDAHEAD
            | ffi::MDBX_LIFORECLAIM;
        // Native combine_durability_flags adds NOMETASYNC with SafeNoSync.
        let expected = flags | if o.durable { 0 } else { ffi::MDBX_NOMETASYNC };
        if effective & checked_flags != expected
            || stat.ms_psize as usize != tuning.page_bytes
            || geometry.grow != tuning.growth_bytes as u64
        {
            return Err(format!(
                "libmdbx effective configuration mismatch: flags={effective:#x}, page={}, growth={}",
                stat.ms_psize, geometry.grow
            ).into());
        }

        let txn = Txn::begin(&mut env, ffi::MDBX_TXN_RDONLY)?;
        let mut dbi = 0;
        // SAFETY: The transaction is live. A null name opens the default table, whose
        // DBI remains valid after commit until the environment closes.
        check(unsafe {
            ffi::mdbx_dbi_open(txn.raw, ptr::null(), ffi::MDBX_DB_DEFAULTS, &mut dbi)
        })?;
        txn.commit()?;
        eprintln!(
            "{}",
            json!({"type":"backend_config","backend":"libmdbx","crate":"mdbx-sys 14.4.0",
                "version":"0.14.4","sync":sync_mode,"flags":effective,
                "tuning":tuning,"page_bytes":stat.ms_psize,"entries":stat.ms_entries,
                "tree_depth":stat.ms_depth,"leaf_pages":stat.ms_leaf_pages,
                "branch_pages":stat.ms_branch_pages,"overflow_pages":stat.ms_overflow_pages,
                "txnid":info.mi_recent_txnid,"geometry":{"min":geometry.lower,
                "current":geometry.current,"max":geometry.upper,
                "growth":geometry.grow,"shrink":geometry.shrink}})
        );
        Ok(Self { env, dbi })
    }
}

impl Backend for Mdbx {
    fn bulk_load(&mut self, n: u64, seed: u64) -> Result<()> {
        let mut txn = Txn::begin(&mut self.env, ffi::MDBX_TXN_READWRITE)?;
        for id in 0..n {
            let memory = Memory::new(id, 0, seed);
            txn.put(
                self.dbi,
                memory.key().as_bytes(),
                &serde_json::to_vec(&memory)?,
            )?;
        }
        txn.commit()
    }

    fn put(&mut self, memory: &Memory, _insert: bool) -> Result<()> {
        let key = memory.key();
        let bytes = serde_json::to_vec(memory)?;
        let mut txn = Txn::begin(&mut self.env, ffi::MDBX_TXN_READWRITE)?;
        txn.put(self.dbi, key.as_bytes(), &bytes)?;
        txn.commit()
    }

    fn contains_key(&mut self, key: &[u8]) -> Result<bool> {
        let txn = Txn::begin(&mut self.env, ffi::MDBX_TXN_RDONLY)?;
        let found = txn.lookup(self.dbi, key)?.is_some();
        txn.commit()?;
        Ok(found)
    }

    fn get(&mut self, id: u64) -> Result<Memory> {
        let txn = Txn::begin(&mut self.env, ffi::MDBX_TXN_RDONLY)?;
        let data = txn
            .lookup(self.dbi, key(id).as_bytes())?
            .ok_or("missing libmdbx memory")?;
        // SAFETY: The successful lookup returned mapped bytes from the live read snapshot.
        let result = serde_json::from_slice(unsafe { bytes(&data) })?;
        txn.commit()?;
        Ok(result)
    }

    fn recent(&mut self, agent: u32, k: usize) -> Result<Vec<Memory>> {
        let txn = Txn::begin(&mut self.env, ffi::MDBX_TXN_RDONLY)?;
        let prefix = format!("agent/{agent:04}/memory/");
        let mut result = Vec::with_capacity(k);
        {
            let mut raw = ptr::null_mut();
            // SAFETY: The cursor uses the cached DBI and is dropped before the read txn.
            check(unsafe { ffi::mdbx_cursor_open(txn.raw, self.dbi, &mut raw) })?;
            let cursor = Cursor(raw);
            let mut key = value(prefix.as_bytes());
            let mut data = value(&[]);
            let mut op = ffi::MDBX_SET_RANGE;
            for _ in 0..k {
                // SAFETY: Both value descriptors are writable and the read cursor is live.
                let rc = unsafe { ffi::mdbx_cursor_get(cursor.0, &mut key, &mut data, op) };
                if matches!(rc, ffi::MDBX_NOTFOUND | ffi::MDBX_ENODATA) {
                    break;
                }
                check(rc)?;
                // SAFETY: Successful seek/next returns snapshot bytes. Consume them
                // before moving the cursor or ending the transaction.
                if !unsafe { bytes(&key) }.starts_with(prefix.as_bytes()) {
                    break;
                }
                result.push(serde_json::from_slice(unsafe { bytes(&data) })?);
                op = ffi::MDBX_NEXT;
            }
        }
        txn.commit()?;
        Ok(result)
    }

    fn finish(&mut self) -> Result<()> {
        // SAFETY: The environment is open and there is no active transaction.
        let rc = unsafe { ffi::mdbx_env_sync_ex(self.env.0, true, false) };
        if rc == ffi::MDBX_RESULT_TRUE {
            return Ok(()); // Already synced.
        }
        check(rc)
    }
}

// The supervisor releases one mutation at a time and kills after an ack,
// while the child is waiting for the next command without closing the database.
pub fn crash_check(o: &Options, verify: bool) -> Result<()> {
    if o.suite.backend != "libmdbx" || !o.durable {
        return Err("crash check requires durable libmdbx".into());
    }
    let c = &o.suite;
    let mut db = open(o)?;
    let mut revisions = vec![0; (c.n + c.operations) as usize];
    if verify {
        for offset in 0..c.operations {
            revisions[(c.n - 1 - offset) as usize] = 1;
        }
        runner::verify(db.as_mut(), c, c.n + c.operations, &revisions)?;
        println!(
            "{}",
            json!({"type":"crash_verified","records":c.n+c.operations,"mutations":2*c.operations})
        );
        return Ok(());
    }
    runner::verify_initial(db.as_mut(), c)?;
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    writeln!(output, "ready")?;
    output.flush()?;
    for id in (0..c.operations).flat_map(|offset| [c.n + offset, c.n - 1 - offset]) {
        let mut command = String::new();
        if input.read_line(&mut command)? == 0 {
            return Err("crash check supervisor disconnected".into());
        }
        let insert = id >= c.n;
        db.put(&Memory::new(id, u64::from(!insert), c.seed), insert)?;
        writeln!(output, "ack {id}")?;
        output.flush()?;
    }
    input.read_line(&mut String::new())?;
    Err("crash check child was not killed".into())
}
