#!/usr/bin/env python3
"""Standalone bounded RSS diagnostic against an existing Helix HTTP server.

No database/client library dependencies. Responses are checked then discarded.
This is a memory reproduction, never an engine latency measurement.
"""
import argparse
import http.client
import json
import os
import pathlib
import time
import urllib.parse


def memory(pid):
    fields = {}
    for line in pathlib.Path(f'/proc/{pid}/status').read_text().splitlines():
        name, _, value = line.partition(':')
        if name in ('VmRSS', 'VmHWM', 'VmSwap'):
            fields[name] = int(value.split()[0]) * 1024
    return fields


def query(connection, path, body):
    connection.request('POST', path, json.dumps(body), {'Content-Type': 'application/json'})
    response = connection.getresponse()
    payload = response.read()
    if response.status != 200:
        raise RuntimeError(f'HTTP {response.status}: {payload[:200]!r}')
    result = json.loads(payload)
    if 'error' in result or 'errors' in result:
        raise RuntimeError(str(result))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--endpoint', default='http://127.0.0.1:16971')
    parser.add_argument('--server-pid', type=int, required=True)
    parser.add_argument('--population', type=int, required=True)
    parser.add_argument('--agents', type=int, default=64)
    parser.add_argument('--queries', type=int, default=1000)
    parser.add_argument('--k', type=int, default=10)
    parser.add_argument('--rss-limit-mib', type=int, default=6144)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args()
    if args.population < args.agents or min(args.agents,args.queries,args.k,args.rss_limit_mib) <= 0:
        parser.error('Require population >= agents and positive counts/limit')
    args.output.mkdir(parents=True, exist_ok=False)
    url = urllib.parse.urlsplit(args.endpoint)
    if url.scheme != 'http' or url.path not in ('', '/'):
        raise ValueError('Use a local HTTP endpoint without a path')
    connection = http.client.HTTPConnection(url.hostname, url.port, timeout=30)
    started = time.monotonic()
    completed = 0
    status = 'incomplete'
    def sample(count):
        return {'queries_completed': count, 'elapsed_seconds': time.monotonic()-started,
                'server': memory(args.server_pid), 'client': memory(os.getpid()),
                'load': os.getloadavg()}
    try:
        with (args.output/'rss.jsonl').open('w', buffering=1) as out:
            out.write(json.dumps(sample(0))+'\n')
            for i in range(args.queries):
                agent = i % args.agents
                result = query(connection, '/Recent', {'agent': agent, 'k': args.k})
                records = result['memories']
                last = args.population-1-((args.population-1-agent) % args.agents)
                expected = list(range(last, max(-1,last-args.k*args.agents), -args.agents))
                assert len(records) == len(expected)
                for record, ident in zip(records, expected):
                    assert record['memory_id'] == f'memory-{ident:020}'
                    assert record['agent'] == agent and record['created_at'] == ident
                    assert isinstance(record['revision'], int) and record['revision'] >= 0
                    assert len(record['text']) == 200
                # No responses survive the iteration; only a fixed-size current list.
                del record, records, result, expected
                completed = i+1
                row = sample(completed)
                out.write(json.dumps(row)+'\n')
                if row['server']['VmRSS'] > args.rss_limit_mib*1024*1024:
                    status = 'rss_limit'
                    break
            else:
                status = 'complete'
    finally:
        connection.close()
        (args.output/'summary.json').write_text(json.dumps({
            'status': status, 'queries_completed': completed, 'comparison_eligible': False,
            'reason': 'HTTP memory diagnostic; not an engine comparison',
            'parameters': vars(args) | {'output': str(args.output)},
            'final': sample(completed)}, indent=2)+'\n')
    print(json.dumps({'status': status, 'queries_completed': completed}))


if __name__ == '__main__':
    main()
