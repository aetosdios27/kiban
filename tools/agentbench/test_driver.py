import json
import os
import pathlib
import sys
import socket
import tempfile
import unittest
from unittest.mock import patch

import hydra
import report
from run import Campaign, server, load_level


class DriverTests(unittest.TestCase):
    def test_one_minute_gate_ignores_decaying_historical_load(self):
        self.assertLess(load_level((1.2, 12, 22), '1m'), 2)
        self.assertGreaterEqual(load_level((2.1, 1, 1), '1m'), 2)
        self.assertGreaterEqual(load_level((1.2, 12, 22), 'all'), 2)

    def test_failure_is_not_a_fast_success(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            with self.assertRaises(RuntimeError):
                Campaign(root, 5).command([sys.executable, '-c', 'raise SystemExit(7)'])
            self.assertTrue((root / 'commands.jsonl').exists())
            self.assertTrue((root / 'load.jsonl').exists())

    def test_timeout_reaps_child(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            with self.assertRaisesRegex(RuntimeError, 'timed out'):
                Campaign(root, 0).command([sys.executable, '-c', 'import time; time.sleep(20)'])

    def test_invalid_campaign_has_no_comparison_numbers(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            (root / 'status.json').write_text(json.dumps({'status': 'invalid', 'runs': [], 'error': 'bad value'}))
            report.generate(root)
            text = (root / 'REPORT.md').read_text()
            self.assertIn('bad value', text)
            self.assertNotIn('ops/s |', text)

    def test_server_can_restart_after_http_connection_closes(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            script = root / 'fake-server'
            script.write_text('#!/usr/bin/env python3\n' + """
import os, signal, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers.get('Content-Length', 0)))
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'{"data":"agentbench-noop"}')
    def log_message(self, *args): pass
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
HTTPServer.allow_reuse_address = True
HTTPServer(('127.0.0.1', int(os.environ['HELIX_PORT'])), Handler).serve_forever()
""")
            script.chmod(0o755)
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', 0))
                port = sock.getsockname()[1]
            campaign = Campaign(root, 5)
            for number in range(2):
                with server(script, root / 'data', port, root / f'server-{number}.log', campaign):
                    pass

    def test_missing_load_samples_are_marked_partial(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            raw = root / 'r2-kiban.jsonl'
            raw.write_text('{}\n')
            os.utime(raw, (10, 10))
            (root / 'commands.jsonl').write_text(json.dumps({'argv': ['bench', 'r2-kiban.config.json'], 'time': 0}) + '\n')
            (root / 'load.jsonl').write_text('\n'.join(json.dumps({
                'time': f'1970-01-01T00:00:0{second}+00:00', 'load': [27, 26, 25], 'measuring': True
            }) for second in [0, 1, 2]) + '\n')
            result = report.load_summary(root, [{'trial': 2, 'backend': 'kiban', 'file': raw.name}])[0]
            self.assertEqual(result['coverage'], 'partial')
            self.assertEqual(result['samples'], 3)
            self.assertEqual(result['load'][0]['min'], 27)
            self.assertEqual(result['largest_sample_gap_seconds'], 8)

    def test_no_key_never_contacts_hydra(self):
        with patch.dict(os.environ, {}, clear=True), patch('urllib.request.urlopen') as request:
            self.assertEqual(hydra.probe()['status'], 'skipped')
            request.assert_not_called()


if __name__ == '__main__':
    unittest.main()
