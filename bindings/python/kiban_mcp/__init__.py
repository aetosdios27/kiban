"""UTF-8 agent memory tools over the official MCP SDK, using stdio."""

from contextlib import closing
from functools import wraps
from itertools import islice

from kiban import KibanError
from mcp.server.fastmcp import FastMCP
from mcp.server.fastmcp.exceptions import ToolError


def _errors(fn):
    @wraps(fn)
    def call(*args, **kwargs):
        try:
            return fn(*args, **kwargs)
        except KibanError as error:
            raise ToolError(f"{fn.__name__}: {type(error).__name__}: {error}") from error
        except UnicodeError as error:
            raise ToolError(f"{fn.__name__}: keys and values must be valid UTF-8") from error
    return call


def create_server(db):
    """Build tools for one already-open database; the caller owns its lifetime."""
    server = FastMCP("Kiban memory", log_level="WARNING", instructions=(
        "Persistent key-value memory. Keys and values are UTF-8 strings. "
        "Writes are buffered; call memory_sync to make accepted writes durable. "
        "memory_batch applies all operations atomically."))

    @server.tool()
    @_errors
    def memory_put(key: str, value: str) -> dict:
        """Store a UTF-8 string. Call memory_sync for crash durability."""
        db.put(key, value)
        return {"ok": True}

    @server.tool()
    @_errors
    def memory_get(key: str) -> dict:
        """Read a UTF-8 value. A missing key returns found=false and value=null."""
        value = db.get(key)
        return {"key": key, "found": value is not None,
                "value": None if value is None else value.decode("utf-8")}

    @server.tool()
    @_errors
    def memory_delete(key: str) -> dict:
        """Delete a key; deleting a missing key succeeds. Sync for durability."""
        db.delete(key)
        return {"ok": True}

    @server.tool()
    @_errors
    def memory_list(prefix: str = "", limit: int = 100) -> dict:
        """List up to limit entries in byte-key order with a UTF-8 prefix.

        Limit must be between 0 and 10000. The binding materializes matching
        entries before applying the limit; use narrow prefixes for large data.
        """
        if not 0 <= limit <= 10000:
            raise ToolError("memory_list: limit must be between 0 and 10000")
        entries = []
        if limit:
            with closing(db.prefix(prefix)) as scan:
                entries = [{"key": key.decode("utf-8"), "value": value.decode("utf-8")}
                           for key, value in islice(scan, limit)]
        return {"entries": entries}

    @server.tool()
    @_errors
    def memory_batch(ops: list[dict]) -> dict:
        """Atomically apply [{op:'put',key:str,value:str}, {op:'delete',key:str}].

        All operations are validated before writing. Sync for crash durability.
        """
        batch = []
        for i, op in enumerate(ops):
            kind = op.get("op")
            expected = {"op", "key", "value"} if kind == "put" else {"op", "key"}
            if (kind not in ("put", "delete") or set(op) != expected
                    or not all(isinstance(value, str) for value in op.values())):
                raise ToolError(f"memory_batch: operation {i} must be "
                                "{op:'put',key:str,value:str} or {op:'delete',key:str}")
            batch.append(("put", op["key"], op["value"]) if kind == "put"
                         else ("delete", op["key"]))
        db.batch(batch)
        return {"ok": True, "count": len(batch)}

    @server.tool()
    @_errors
    def memory_sync() -> dict:
        """Make all previously accepted writes durable."""
        db.sync()
        return {"ok": True}

    return server
