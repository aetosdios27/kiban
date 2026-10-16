import ctypes
import gc
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from concurrent.futures import ThreadPoolExecutor

from kiban import Kiban, KibanError, InvalidHandleError, LockedError


class BindingTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kiban-python-")
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name) / "db"
        self.db = Kiban(self.path, write_buffer_bytes=4096, max_open_table_files=8)
        self.addCleanup(self.db.close)

    def test_site_example(self):
        directory = Path(self.tmp.name) / "site-example"
        # site:begin python
        from kiban import Kiban

        with Kiban(directory) as db:
            db.put("hello", "world")
            db.sync()
            assert db.get("hello") == b"world"
            with db.snapshot() as snapshot:
                db.put("hello", "later")
                assert snapshot.get("hello") == b"world"
            db.batch([("put", "a", "1"), ("put", "b", "2")])
            assert list(db.scan("a", "c")) == [(b"a", b"1"), (b"b", b"2")]
        # site:end

    def test_crud_binary_empty_and_str(self):
        for key, value in [(b"k\0\xff", b"v\0\xff"), (b"", b""), ("café", "你好")]:
            self.db.put(key, value)
            self.assertEqual(self.db.get(key), value.encode() if isinstance(value, str) else value)
            self.db.delete(key)
            self.assertIsNone(self.db.get(key))
        self.db.delete("missing")
        self.assertIsNone(self.db.get("missing"))
        with self.assertRaises(TypeError):
            self.db.put(1, b"v")

    def test_batch_and_validation_before_write(self):
        self.db.batch([("put", "a", "old"), ("put", "b", "old")])
        self.db.batch([("delete", "a"), ("put", "b", "new")])
        self.assertIsNone(self.db.get("a"))
        self.assertEqual(self.db.get("b"), b"new")
        with self.assertRaises(ValueError):
            self.db.batch([("put", "c", "value"), ("unknown", "b")])
        self.assertIsNone(self.db.get("c"))
        self.db.batch([])

    def test_snapshot_and_lifetime(self):
        self.db.put("a", "old")
        with self.db.snapshot() as snap:
            self.db.put("a", "new")
            self.db.flush()
            self.assertEqual(snap.get("a"), b"old")
            self.assertEqual(list(snap.scan()), [(b"a", b"old")])
            self.assertEqual(list(snap.prefix("a")), [(b"a", b"old")])
            self.db.close()
            self.assertEqual(snap.get("a"), b"old")
            with self.assertRaises(LockedError):
                Kiban(self.path)
        with self.assertRaises(InvalidHandleError):
            snap.get("a")
        with Kiban(self.path) as db:
            self.assertEqual(db.get("a"), b"new")

    def test_ranges_prefixes_and_generator_release(self):
        for key in [b"", b"a", b"ab", b"b", b"\xff", b"\xff\xff"]:
            self.db.put(key, key)
        self.assertEqual([k for k, _ in self.db.scan("a", "b")], [b"a", b"ab"])
        self.assertEqual(list(self.db.scan("b", "a")), [])
        self.assertEqual(list(self.db.scan("a", b"")), [])
        self.assertEqual([k for k, _ in self.db.scan("b")], [b"b", b"\xff", b"\xff\xff"])
        self.assertEqual([k for k, _ in self.db.prefix(b"\xff")], [b"\xff", b"\xff\xff"])
        self.assertEqual(len(list(self.db.prefix())), 6)
        scan = self.db.scan()
        self.assertEqual(next(scan), (b"", b""))
        self.db.close()
        self.assertEqual(next(scan), (b"a", b"a"))
        scan.close()

    def test_close_reopen_and_error_types(self):
        with self.assertRaises(LockedError) as ctx:
            Kiban(self.path)
        self.assertEqual(ctx.exception.code, 5)
        self.db.put("a", "saved")
        self.db.sync()
        self.db.flush()
        self.db.checkpoint()
        self.db.close()
        self.db.close()
        for op in [lambda: self.db.get("a"), lambda: self.db.put("a", "v"), self.db.sync,
                   self.db.snapshot, lambda: list(self.db.scan())]:
            with self.assertRaises(InvalidHandleError):
                op()
        with Kiban(self.path) as db:
            self.assertEqual(db.get("a"), b"saved")

    def test_threads(self):
        def run(i):
            for j in range(20):
                key = f"{i}:{j}"
                self.db.put(key, key)
                self.assertEqual(self.db.get(key), key.encode())
        with ThreadPoolExecutor(max_workers=4) as pool:
            list(pool.map(run, range(4)))
        self.assertEqual(len(list(self.db.scan())), 80)

    def test_loader_and_option_errors(self):
        with patch.dict(os.environ, {"KIBAN_LIB": str(self.path / "missing.so")}):
            with self.assertRaisesRegex(KibanError, "library not found"):
                Kiban(self.path / "other")
        for value in [-1, 2 ** (8 * ctypes.sizeof(ctypes.c_size_t)), "1"]:
            with self.assertRaises(ValueError):
                Kiban(self.path / "other", write_buffer_bytes=value)

    def test_finalization_releases_handles(self):
        path = Path(self.tmp.name) / "finalized"
        db = Kiban(path)
        db.put("a", "v")
        snap = db.snapshot()
        del db, snap
        gc.collect()
        with Kiban(path) as reopened:
            self.assertEqual(reopened.get("a"), b"v")


if __name__ == "__main__":
    unittest.main()
