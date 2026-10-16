"""Kiban ctypes binding. Keys and values are bytes; str inputs use UTF-8."""

import ctypes as C
import os
from pathlib import Path
import weakref


class KibanError(Exception):
    """A Kiban operation failed; code is the C ABI status."""

    def __init__(self, message, code=None):
        super().__init__(message)
        self.code = code


class InvalidArgumentError(KibanError):
    pass

class InvalidHandleError(KibanError):
    pass

class LockedError(KibanError):
    pass

class KibanIOError(KibanError):
    pass

class CorruptError(KibanError):
    pass

class CommitFailedError(KibanError):
    pass

class CommitAmbiguousError(KibanError):
    pass

class PoisonedError(KibanError):
    pass

class MaintenanceError(KibanError):
    pass

class SequenceExhaustedError(KibanError):
    pass

class KibanPanicError(KibanError):
    pass



_ERRORS = dict(enumerate((InvalidArgumentError, InvalidHandleError, LockedError,
                         KibanIOError, CorruptError, CommitFailedError,
                         CommitAmbiguousError, PoisonedError, MaintenanceError,
                         SequenceExhaustedError, KibanPanicError), start=3))


class _Buf(C.Structure):
    _fields_ = [("data", C.c_void_p), ("len", C.c_size_t)]


class _Options(C.Structure):
    _fields_ = [("write_buffer_bytes", C.c_size_t),
                ("block_cache_bytes", C.c_size_t),
                ("max_open_table_files", C.c_size_t)]


class _Op(C.Structure):
    _fields_ = [("kind", C.c_uint32), ("key", C.c_char_p),
                ("key_len", C.c_size_t), ("value", C.c_char_p),
                ("value_len", C.c_size_t)]


def _load():
    override = os.environ.get("KIBAN_LIB")
    root = Path(__file__).resolve().parents[3]
    candidates = [Path(override)] if override else [
        root / "target" / profile / "libkiban_ffi.so"
        for profile in ("debug", "release")
    ]
    for path in candidates:
        if path.is_file():
            try:
                lib = C.CDLL(str(path.resolve()))
            except OSError as error:
                raise KibanError(f"Cannot load {path}: {error}") from error
            break
    else:
        raise KibanError("Kiban library not found. Build with cargo build -p kiban-ffi "
                         "and set KIBAN_LIB to the absolute libkiban_ffi.so path.")
    h, n, p = C.c_uint64, C.c_size_t, C.c_char_p
    signatures = {
        "open": [p, n, C.POINTER(_Options), C.POINTER(h)],
        "close": [h], "put": [h, p, n, p, n],
        "get": [h, p, n, C.POINTER(_Buf)], "delete": [h, p, n],
        "write_batch": [h, C.POINTER(_Op), n],
        "sync": [h], "flush": [h], "checkpoint": [h],
        "snapshot_create": [h, C.POINTER(h)], "snapshot_release": [h],
        "get_at_snapshot": [h, p, n, C.POINTER(_Buf)],
        "scan_range": [h, p, n, p, n, C.POINTER(h)],
        "scan_prefix": [h, p, n, C.POINTER(h)],
        "iter_next": [h, C.POINTER(_Buf), C.POINTER(_Buf)],
        "iter_free": [h], "last_error": [C.POINTER(_Buf)],
        "free_buf": [_Buf],
    }
    for name, args in signatures.items():
        fn = getattr(lib, "kiban_" + name)
        fn.argtypes, fn.restype = args, C.c_int32
    return lib


def _take(lib, buf):
    try:
        return C.string_at(buf.data, buf.len) if buf.len else b""
    finally:
        lib.kiban_free_buf(buf)


def _check(lib, code):
    if code == 0:
        return
    buf = _Buf()
    if lib.kiban_last_error(C.byref(buf)) == 0:
        message = _take(lib, buf).decode("utf-8", errors="replace")
    else:
        message = "Could not retrieve Kiban error"
    raise _ERRORS.get(code, KibanError)(message, code)


def _bytes(value):
    if isinstance(value, str):
        return value.encode("utf-8")
    if isinstance(value, bytes):
        return value
    raise TypeError("keys and values must be bytes or str")


class _Reader:
    def get(self, key):
        """Return bytes, or None when the key does not exist."""
        key = _bytes(key)
        buf = _Buf()
        fn = getattr(self._lib, self._get_fn)
        code = fn(self._handle, key, len(key), C.byref(buf))
        if code == 1:
            return None
        _check(self._lib, code)
        return _take(self._lib, buf)

    def scan(self, start=b"", end=None):
        """Yield (bytes, bytes) in [start, end); None means no upper bound.

        The C layer materializes the result at first iteration. Close a
        partially consumed generator to release its iterator promptly.
        """
        start = _bytes(start)
        end = None if end is None else _bytes(end)
        handle = C.c_uint64()
        _check(self._lib, self._lib.kiban_scan_range(
            self._handle, start, len(start), end, 0 if end is None else len(end),
            C.byref(handle)))
        yield from self._entries(handle.value)

    def prefix(self, prefix=b""):
        """Yield byte pairs with this byte prefix, in key order."""
        prefix = _bytes(prefix)
        handle = C.c_uint64()
        _check(self._lib, self._lib.kiban_scan_prefix(
            self._handle, prefix, len(prefix), C.byref(handle)))
        yield from self._entries(handle.value)

    def _entries(self, handle):
        lib = self._lib
        try:
            while True:
                key, value = _Buf(), _Buf()
                code = lib.kiban_iter_next(handle, C.byref(key), C.byref(value))
                if code == 2:
                    return
                _check(lib, code)
                # Free both buffers before yielding or copying can raise.
                try:
                    pair = (C.string_at(key.data, key.len) if key.len else b"",
                            C.string_at(value.data, value.len) if value.len else b"")
                finally:
                    lib.kiban_free_buf(key)
                    lib.kiban_free_buf(value)
                yield pair
        finally:
            lib.kiban_iter_free(handle)

    def __enter__(self):
        if not self._handle:
            raise InvalidHandleError("handle is closed", 4)
        return self

    def __exit__(self, exc_type, exc_value, traceback):
        self.close()

    def close(self):
        """Release once; repeated close calls do nothing."""
        if self._handle:
            handle, self._handle = self._handle, 0
            self._finalizer.detach()
            _check(self._lib, getattr(self._lib, self._close_fn)(handle))


class Snapshot(_Reader):
    """An independent read-only snapshot. Use as a context manager."""
    _get_fn = "kiban_get_at_snapshot"
    _close_fn = "kiban_snapshot_release"

    def __init__(self, lib, handle):
        self._lib, self._handle = lib, handle
        self._finalizer = weakref.finalize(self, lib.kiban_snapshot_release, handle)


class Kiban(_Reader):
    """Open a database directory. Explicit close checkpoints the database.

    Zero option values select engine defaults. Finalization is a fallback;
    use a context manager to observe close errors and guarantee a barrier.
    """
    _get_fn = "kiban_get"
    _close_fn = "kiban_close"

    def __init__(self, path, *, write_buffer_bytes=0, block_cache_bytes=0,
                 max_open_table_files=0):
        self._handle = 0
        self._lib = _load()
        values = (write_buffer_bytes, block_cache_bytes, max_open_table_files)
        for value in values:
            if not isinstance(value, int) or not 0 <= value <= C.c_size_t(-1).value:
                raise ValueError("options must be integers that fit size_t")
        opts, handle = _Options(*values), C.c_uint64()
        path = os.fsencode(path)
        _check(self._lib, self._lib.kiban_open(path, len(path), C.byref(opts), C.byref(handle)))
        self._handle = handle.value
        self._finalizer = weakref.finalize(self, self._lib.kiban_close, handle.value)

    def put(self, key, value):
        key, value = _bytes(key), _bytes(value)
        _check(self._lib, self._lib.kiban_put(self._handle, key, len(key), value, len(value)))

    def delete(self, key):
        key = _bytes(key)
        _check(self._lib, self._lib.kiban_delete(self._handle, key, len(key)))

    def batch(self, ops):
        """Atomically apply ('put', key, value) or ('delete', key) tuples.

        Validate all operations before writing. Call sync for durability.
        """
        prepared = []
        for op in ops:
            if isinstance(op, (tuple, list)) and len(op) == 3 and op[0] == "put":
                key, value = _bytes(op[1]), _bytes(op[2])
                prepared.append(_Op(1, key, len(key), value, len(value)))
            elif isinstance(op, (tuple, list)) and len(op) == 2 and op[0] == "delete":
                key = _bytes(op[1])
                prepared.append(_Op(2, key, len(key), None, 0))
            else:
                raise ValueError("expected ('put', key, value) or ('delete', key)")
        array = (_Op * len(prepared))(*prepared)
        _check(self._lib, self._lib.kiban_write_batch(self._handle, array, len(array)))

    def sync(self):
        """Make all accepted writes durable."""
        _check(self._lib, self._lib.kiban_sync(self._handle))

    def flush(self):
        _check(self._lib, self._lib.kiban_flush(self._handle))

    def checkpoint(self):
        _check(self._lib, self._lib.kiban_checkpoint(self._handle))

    def snapshot(self):
        handle = C.c_uint64()
        _check(self._lib, self._lib.kiban_snapshot_create(self._handle, C.byref(handle)))
        return Snapshot(self._lib, handle.value)
