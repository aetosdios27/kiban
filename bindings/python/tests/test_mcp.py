import asyncio
from contextlib import asynccontextmanager
from datetime import timedelta
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

from kiban import Kiban

HAS_MCP = importlib.util.find_spec("mcp") is not None
if HAS_MCP:
    from mcp import ClientSession, StdioServerParameters
    from mcp.client.stdio import stdio_client


@unittest.skipUnless(HAS_MCP, "mcp SDK not installed; pip install -e bindings/python/kiban_mcp")
class MCPTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kiban-mcp-")
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name) / "db"

    @asynccontextmanager
    async def session(self):
        env = os.environ.copy()
        source = str(Path(__file__).resolve().parents[1])
        env["PYTHONPATH"] = os.pathsep.join(filter(None, [source, env.get("PYTHONPATH")]))
        params = StdioServerParameters(command=sys.executable,
                                       args=["-m", "kiban_mcp", str(self.path)], env=env)
        async with stdio_client(params) as (read, write):
            async with ClientSession(read, write, read_timeout_seconds=timedelta(seconds=15)) as session:
                await session.initialize()
                yield session

    async def call(self, session, name, args=None):
        result = await session.call_tool(name, args or {})
        self.assertFalse(result.isError, result.content)
        return json.loads(result.content[0].text)

    async def test_all_tools_over_stdio_and_restart(self):
        await asyncio.wait_for(self._all_tools_over_stdio_and_restart(), timeout=45)

    async def _all_tools_over_stdio_and_restart(self):
        async with self.session() as session:
            names = {tool.name for tool in (await session.list_tools()).tools}
            self.assertEqual(names, {"memory_put", "memory_get", "memory_delete",
                                     "memory_list", "memory_batch", "memory_sync"})
            self.assertEqual(await self.call(session, "memory_get", {"key": "missing"}),
                             {"key": "missing", "value": None, "found": False})
            self.assertEqual(await self.call(session, "memory_put", {"key": "note:α", "value": "你好\u0000"}), {"ok": True})
            self.assertEqual((await self.call(session, "memory_get", {"key": "note:α"}))["value"], "你好\u0000")
            batch = [{"op": "put", "key": "note:b", "value": "b"},
                     {"op": "put", "key": "note:c", "value": "c"},
                     {"op": "delete", "key": "note:α"}]
            self.assertEqual(await self.call(session, "memory_batch", {"ops": batch}), {"ok": True, "count": 3})
            self.assertEqual(await self.call(session, "memory_list", {"prefix": "note:", "limit": 1}),
                             {"entries": [{"key": "note:b", "value": "b"}]})
            self.assertEqual(await self.call(session, "memory_list", {"prefix": "none"}), {"entries": []})
            self.assertEqual(await self.call(session, "memory_list", {"limit": 0}), {"entries": []})
            self.assertEqual(await self.call(session, "memory_delete", {"key": "note:c"}), {"ok": True})
            self.assertEqual(await self.call(session, "memory_delete", {"key": "absent"}), {"ok": True})
            self.assertFalse((await self.call(session, "memory_get", {"key": "note:c"}))["found"])
            self.assertEqual(await self.call(session, "memory_sync"), {"ok": True})
        async with self.session() as session:
            self.assertEqual((await self.call(session, "memory_get", {"key": "note:b"}))["value"], "b")
            self.assertFalse((await self.call(session, "memory_get", {"key": "note:α"}))["found"])

    async def test_tool_errors_and_batch_atomic_validation(self):
        await asyncio.wait_for(self._tool_errors_and_batch_atomic_validation(), timeout=45)

    async def _tool_errors_and_batch_atomic_validation(self):
        # Binary data can exist when the directory was also used via Python/C.
        with Kiban(self.path) as db:
            db.put("binary", b"\xff")
        async with self.session() as session:
            for name, args, message in [
                ("memory_list", {"limit": -1}, "limit"),
                ("memory_list", {"limit": 10001}, "limit"),
                ("memory_get", {"key": "binary"}, "UTF-8"),
                ("memory_list", {"prefix": "binary"}, "UTF-8"),
                ("memory_put", {"key": 7, "value": "x"}, "key"),
                ("memory_batch", {"ops": [{"op": "put", "key": "partial", "value": "v"},
                                           {"op": "delete", "key": 1}]}, "operation 1"),
                ("memory_batch", {"ops": [{"op": "put", "key": "partial", "value": "v"},
                                           {"op": "unknown", "key": "k"}]}, "operation 1"),
            ]:
                result = await session.call_tool(name, args)
                self.assertTrue(result.isError, result)
                self.assertIn(message, result.content[0].text)
            self.assertFalse((await self.call(session, "memory_get", {"key": "partial"}))["found"])
            self.assertEqual(await self.call(session, "memory_batch", {"ops": []}), {"ok": True, "count": 0})
            # The same connection remains usable after tool errors.
            self.assertEqual(await self.call(session, "memory_put", {"key": "ok", "value": ""}), {"ok": True})
            self.assertEqual((await self.call(session, "memory_get", {"key": "ok"}))["value"], "")


if __name__ == "__main__":
    unittest.main()
