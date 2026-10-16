use kiban_ffi::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct Db {
    id: u64,
    path: std::path::PathBuf,
}
impl Db {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "kiban-ffi-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let bytes = path.as_os_str().as_encoded_bytes();
        let mut id = 0;
        let opts = KibanOpenOptions {
            write_buffer_bytes: 4096,
            block_cache_bytes: 4096,
            max_open_table_files: 8,
        };
        assert_eq!(
            unsafe { kiban_open(bytes.as_ptr(), bytes.len(), &opts, &mut id) },
            OK
        );
        Self { id, path }
    }
    fn put(&self, key: &[u8], value: &[u8]) {
        assert_eq!(
            unsafe {
                kiban_put(
                    self.id,
                    key.as_ptr(),
                    key.len(),
                    value.as_ptr(),
                    value.len(),
                )
            },
            OK
        );
    }
}
impl Drop for Db {
    fn drop(&mut self) {
        if self.id != 0 {
            assert_eq!(kiban_close(self.id), OK);
        }
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}
unsafe fn take(buf: KibanBuf) -> Vec<u8> {
    let value = if buf.len == 0 {
        vec![]
    } else {
        unsafe { std::slice::from_raw_parts(buf.data, buf.len).to_vec() }
    };
    assert_eq!(kiban_free_buf(buf), OK);
    value
}
fn get(id: u64, key: &[u8], snap: bool) -> (i32, Vec<u8>) {
    let mut out = KibanBuf::default();
    let code = unsafe {
        if snap {
            kiban_get_at_snapshot(id, key.as_ptr(), key.len(), &mut out)
        } else {
            kiban_get(id, key.as_ptr(), key.len(), &mut out)
        }
    };
    (code, unsafe { take(out) })
}
fn entries(id: u64) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut result = vec![];
    loop {
        let mut k = KibanBuf::default();
        let mut v = KibanBuf::default();
        let code = unsafe { kiban_iter_next(id, &mut k, &mut v) };
        if code == DONE {
            break;
        }
        assert_eq!(code, OK);
        result.push((unsafe { take(k) }, unsafe { take(v) }));
    }
    assert_eq!(kiban_iter_free(id), OK);
    assert_eq!(kiban_iter_free(id), INVALID_HANDLE);
    result
}

#[test]
fn crud_snapshot_batch_and_reopen() {
    let db = Db::new();
    db.put(b"a\0", b"\0\xff");
    db.put(b"empty", b"");
    assert_eq!(get(db.id, b"a\0", false), (OK, vec![0, 255]));
    assert_eq!(get(db.id, b"empty", false), (OK, vec![]));
    assert_eq!(get(db.id, b"missing", false).0, NOT_FOUND);
    let mut snap = 0;
    assert_eq!(unsafe { kiban_snapshot_create(db.id, &mut snap) }, OK);
    let ops = [
        KibanBatchOp {
            kind: 1,
            key: b"b".as_ptr(),
            key_len: 1,
            value: b"v".as_ptr(),
            value_len: 1,
        },
        KibanBatchOp {
            kind: 2,
            key: [b'a', 0].as_ptr(),
            key_len: 2,
            value: std::ptr::null(),
            value_len: 0,
        },
    ];
    assert_eq!(
        unsafe { kiban_write_batch(db.id, ops.as_ptr(), ops.len()) },
        OK
    );
    assert_eq!(get(db.id, b"a\0", false).0, NOT_FOUND);
    assert_eq!(get(snap, b"a\0", true), (OK, vec![0, 255]));
    assert_eq!(get(db.id, b"b", false), (OK, b"v".to_vec()));
    assert_eq!(kiban_sync(db.id), OK);
    assert_eq!(kiban_flush(db.id), OK);
    assert_eq!(kiban_checkpoint(db.id), OK);
    assert_eq!(kiban_close(db.id), OK);
    assert_eq!(kiban_close(db.id), INVALID_HANDLE);
    assert_eq!(get(db.id, b"b", false).0, INVALID_HANDLE);
    assert_eq!(get(snap, b"a\0", true).0, OK);
    assert_eq!(kiban_snapshot_release(snap), OK);
    assert_eq!(kiban_snapshot_release(snap), INVALID_HANDLE);
    assert_eq!(get(snap, b"a\0", true).0, INVALID_HANDLE);
    let path = db.path.as_os_str().as_encoded_bytes();
    let mut reopened = 0;
    assert_eq!(
        unsafe { kiban_open(path.as_ptr(), path.len(), std::ptr::null(), &mut reopened) },
        OK
    );
    assert_eq!(get(reopened, b"b", false), (OK, b"v".to_vec()));
    assert_eq!(unsafe { kiban_delete(reopened, b"b".as_ptr(), 1) }, OK);
    assert_eq!(get(reopened, b"b", false).0, NOT_FOUND);
    assert_eq!(kiban_close(reopened), OK);
    let mut db = db;
    db.id = 0;
}

#[test]
fn ranges_prefixes_and_owned_iterator() {
    let db = Db::new();
    for key in [b"".as_slice(), b"a", b"ab", b"b", b"\xff", b"\xff\xff"] {
        db.put(key, key);
    }
    let mut iter = 0;
    assert_eq!(
        unsafe { kiban_scan_range(db.id, b"a".as_ptr(), 1, b"b".as_ptr(), 1, &mut iter) },
        OK
    );
    db.put(b"ac", b"new");
    assert_eq!(
        entries(iter)
            .iter()
            .map(|x| x.0.clone())
            .collect::<Vec<_>>(),
        vec![b"a".to_vec(), b"ab".to_vec()]
    );
    assert_eq!(
        unsafe { kiban_scan_range(db.id, b"a".as_ptr(), 1, b"".as_ptr(), 0, &mut iter) },
        OK
    );
    assert!(entries(iter).is_empty());
    assert_eq!(
        unsafe { kiban_scan_range(db.id, b"b".as_ptr(), 1, std::ptr::null(), 0, &mut iter) },
        OK
    );
    assert_eq!(entries(iter).len(), 3);
    assert_eq!(
        unsafe { kiban_scan_prefix(db.id, b"\xff".as_ptr(), 1, &mut iter) },
        OK
    );
    assert_eq!(entries(iter).len(), 2);
    let mut snap = 0;
    assert_eq!(unsafe { kiban_snapshot_create(db.id, &mut snap) }, OK);
    assert_eq!(
        unsafe { kiban_scan_prefix(snap, std::ptr::null(), 0, &mut iter) },
        OK
    );
    assert_eq!(kiban_snapshot_release(snap), OK);
    assert_eq!(entries(iter).len(), 7);
}

#[test]
fn rejects_invalid_arguments_and_handles_without_mutation() {
    let db = Db::new();
    assert_eq!(kiban_sync(0), INVALID_HANDLE);
    assert_eq!(kiban_flush(u64::MAX), INVALID_HANDLE);
    assert_eq!(kiban_iter_free(db.id), INVALID_HANDLE);
    assert_eq!(kiban_snapshot_release(db.id), INVALID_HANDLE);
    unsafe {
        assert_eq!(
            kiban_get(db.id, b"k".as_ptr(), 1, std::ptr::null_mut()),
            INVALID_ARGUMENT
        );
        assert_eq!(
            kiban_put(db.id, std::ptr::null(), 1, std::ptr::null(), 0),
            INVALID_ARGUMENT
        );
        assert_eq!(
            kiban_write_batch(db.id, std::ptr::null(), 1),
            INVALID_ARGUMENT
        );
        assert_eq!(kiban_write_batch(db.id, std::ptr::null(), 0), OK);
        let ops = [
            KibanBatchOp {
                kind: 1,
                key: b"k".as_ptr(),
                key_len: 1,
                value: b"v".as_ptr(),
                value_len: 1,
            },
            KibanBatchOp {
                kind: 99,
                key: b"k".as_ptr(),
                key_len: 1,
                value: std::ptr::null(),
                value_len: 0,
            },
        ];
        assert_eq!(kiban_write_batch(db.id, ops.as_ptr(), 2), INVALID_ARGUMENT);
        assert_eq!(get(db.id, b"k", false).0, NOT_FOUND);
        let path = db.path.as_os_str().as_encoded_bytes();
        let mut id = 99;
        assert_eq!(
            kiban_open(path.as_ptr(), path.len(), std::ptr::null(), &mut id),
            LOCKED
        );
        assert_eq!(id, 0);
        assert_eq!(
            kiban_open(std::ptr::null(), 0, std::ptr::null(), &mut id),
            INVALID_ARGUMENT
        );
        let mut msg = KibanBuf::default();
        assert_eq!(kiban_last_error(&mut msg), OK);
        assert!(String::from_utf8(take(msg)).unwrap().contains("path"));
    }
}

#[test]
fn buffers_and_errors_are_checked_and_thread_local() {
    let db = Db::new();
    db.put(b"k", b"value");
    let mut out = KibanBuf::default();
    assert_eq!(unsafe { kiban_get(db.id, b"k".as_ptr(), 1, &mut out) }, OK);
    assert_eq!(
        kiban_free_buf(KibanBuf {
            len: out.len + 1,
            ..out
        }),
        INVALID_ARGUMENT
    );
    assert_eq!(kiban_free_buf(out), OK);
    assert_eq!(kiban_free_buf(out), INVALID_ARGUMENT);
    assert_eq!(kiban_sync(0), INVALID_HANDLE);
    std::thread::spawn(|| {
        assert_eq!(
            unsafe { kiban_open(std::ptr::null(), 0, std::ptr::null(), &mut 0) },
            INVALID_ARGUMENT
        );
    })
    .join()
    .unwrap();
    let mut msg = KibanBuf::default();
    assert_eq!(unsafe { kiban_last_error(&mut msg) }, OK);
    assert!(
        String::from_utf8(unsafe { take(msg) })
            .unwrap()
            .contains("handle")
    );
    std::thread::scope(|scope| {
        for i in 0..4 {
            let db = &db;
            scope.spawn(move || {
                for j in 0..10 {
                    db.put(format!("{i}:{j}").as_bytes(), b"v");
                }
            });
        }
    });
}

#[test]
fn header_covers_every_export() {
    let source = include_str!("../src/lib.rs");
    let header = include_str!("../kiban.h");
    let mut count = 0;
    for tail in source.split("extern \"C\" fn ").skip(1) {
        let symbol = tail.split('(').next().unwrap();
        assert!(
            header.contains(&format!(" {symbol}(")),
            "missing {symbol} in header"
        );
        count += 1;
    }
    assert_eq!(count, 18);
}
