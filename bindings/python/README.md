# Use Kiban from Python and MCP

Build on Linux with Rust 1.89+ and Python 3.10+. Run these commands from the repository root:

```sh
cargo build --release -p kiban-ffi
python3 -m venv .venv
. .venv/bin/activate
python -m pip install ./bindings/python
export KIBAN_LIB="$PWD/target/release/libkiban_ffi.so"
```

The `kiban` Python package uses only `ctypes` and the standard library.
`KIBAN_LIB` selects the library explicitly. From a source checkout, the binding
also searches `target/debug` and `target/release`, in that order.
Installed packages need `KIBAN_LIB`; the shared library is built separately.

```python
from kiban import Kiban
with Kiban("./memory") as db:
    db.put("name", "Kiban")
    print(db.get("name"))  # b'Kiban'
    db.batch([("put", b"a", b"1"), ("delete", "name")])
    with db.snapshot() as view:
        db.put("a", "2")
        print(view.get("a"))  # b'1'
    print(list(db.prefix("a")))  # [(b'a', b'2')]
    db.sync()
```

Keys and values accept `bytes` or UTF-8 `str`; reads always return `bytes`.
Missing keys return `None`. `scan(start=b"", end=None)` yields ordered pairs
in the half-open range `[start, end)`. `prefix(p)` yields keys with that byte
prefix. Snapshots support `get`, `scan`, and `prefix` and remain valid after
the database closes. Close snapshots to release their directory ownership.

Scan creation captures a snapshot and copies all matching entries into an owned
iterator. Memory use is proportional to the entire result, even if you stop early.
Close a partly consumed generator to release its entries promptly.

`batch` validates all operations before one atomic write. Writes require `sync`
for crash durability. `flush` publishes a table; `checkpoint` also syncs and waits
for maintenance. Database context exit calls `close`, which checkpoints.
Errors derive from `KibanError`, with subclasses such as `LockedError`,
`KibanIOError`, and `PoisonedError`; each includes the ABI status in `.code`.
The C ABI and ownership rules are in [`kiban.h`](../../crates/kiban-ffi/kiban.h).

## Connect an MCP client

Install the server in the same environment:

```sh
python -m pip install ./bindings/python/kiban_mcp
python -m kiban_mcp /absolute/path/to/memory
```

The server uses the [official MCP Python SDK](https://github.com/modelcontextprotocol/python-sdk/tree/v1.x),
with `mcp>=1.28,<2` declared only in the MCP subpackage. It speaks stdio and opens
one directory. Diagnostics go to stderr.

For [Claude Code](https://code.claude.com/docs/en/mcp), put this in `.mcp.json`.
Other stdio MCP clients use the same command, arguments, and environment:

```json
{
  "mcpServers": {
    "kiban": {
      "command": "/absolute/path/to/repo/.venv/bin/python",
      "args": ["-m", "kiban_mcp", "/absolute/path/to/memory"],
      "env": {
        "KIBAN_LIB": "/absolute/path/to/repo/target/release/libkiban_ffi.so"
      }
    }
  }
}
```

Tools are `memory_put(key, value)`, `memory_get(key)`, `memory_delete(key)`,
`memory_list(prefix="", limit=100)`, `memory_batch(ops)`, and `memory_sync()`.
All MCP keys and values are UTF-8 strings. Binary values written through Python
or C cause a tool error when read through MCP. Missing reads return
`{"key": "...", "found": false, "value": null}`. Writes return `{"ok": true}`.
`memory_list` returns `{"entries": [{"key": "...", "value": "..."}]}`;
its limit is 0–10000 and limits returned entries, not scan memory.

A batch is a list such as
`[{"op": "put", "key": "note", "value": "text"}, {"op": "delete", "key": "old"}]`.
It returns `{"ok": true, "count": 2}`. Invalid operations reject the whole batch.
Call `memory_sync()` to acknowledge durability before ending an agent session.
Each directory allows one database owner, so give separate server processes
separate directories.

## Run the binding tests

```sh
cargo build -p kiban-ffi
cargo test -p kiban-ffi
KIBAN_LIB="$PWD/target/debug/libkiban_ffi.so" PYTHONPATH=bindings/python \
    python3 -m unittest discover -s bindings/python/tests -v
```

The MCP tests launch the server and call it through the SDK stdio client.
They skip with an installation message when `mcp` is absent.
