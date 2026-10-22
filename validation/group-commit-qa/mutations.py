#!/usr/bin/env python3
"""Run isolated, reversible group-commit mutations; never retain engine edits."""
import argparse
import json
import os
from pathlib import Path
import signal
import hashlib
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
GATE = Path.home() / '.local/share/kiban-gates/gc-qa-20261022'


def replace_once(text, old, new):
    assert text.count(old) == 1, (old, text.count(old))
    return text.replace(old, new, 1)


def mutate(name, sources):
    db, queue, wal = (sources[p] for p in ('src/db.rs', 'src/group_commit.rs', 'src/wal.rs'))
    if name == 'skip_fdatasync':
        wal = replace_once(wal, '        self.writer\n            .get_ref()\n            .sync_data()\n            .map_err(SyncPhase::Fdatasync)?;', '')
    elif name == 'publish_before_sync':
        db = replace_once(db, '        self.sync_wals()?;\n        #[cfg(test)]\n        self.group_hook(GroupStage::AfterSync, ops);', '')
        db = replace_once(db, '        self.last_sequence = last;\n        drop(mirror);', '        drop(mirror);\n        self.sync_wals()?;\n        #[cfg(test)]\n        self.group_hook(GroupStage::AfterSync, ops);\n        self.last_sequence = last;')
    elif name == 'reverse_publish':
        db = replace_once(db, 'for (op, first) in ops.iter().zip(&firsts) {', 'for (op, first) in ops.iter().zip(&firsts).rev() {')
    elif name == 'skip_poison':
        # Return the same error without making refusal sticky.
        start, end = db.index('    fn commit_group('), db.index('    pub fn get(', db.index('    fn commit_group('))
        part = db[start:end]
        part = part.replace('self.poison(PoisonCause::WalAppendFailed(e.to_string()));\n                return Err(DbError::Poisoned(self.poisoned.clone().unwrap()));', 'return Err(DbError::Poisoned(PoisonCause::WalAppendFailed(e.to_string())));')
        part = part.replace('self.poison(PoisonCause::WalAppendFailed(e.to_string()));\n                Err(DbError::Poisoned(self.poisoned.clone().unwrap()))', 'Err(DbError::Poisoned(PoisonCause::WalAppendFailed(e.to_string())))')
        part = part.replace('self.poison(PoisonCause::WalSyncFailed(e.to_string()));\n                Err(DbError::Poisoned(self.poisoned.clone().unwrap()))', 'Err(DbError::Poisoned(PoisonCause::WalSyncFailed(e.to_string())))')
        assert 'self.poison(' not in part
        db = db[:start] + part + db[end:]
    elif name == 'drop_follower_result':
        queue = replace_once(queue, '            self.slots[1 + self.delivered].set(SlotState::Done(result));', '            drop(result);')
    elif name == 'advance_sequence_early':
        db = replace_once(db, '        self.sync_wals()?;\n        #[cfg(test)]\n        self.group_hook(GroupStage::AfterSync, ops);', '        self.last_sequence = last;\n        self.sync_wals()?;\n        #[cfg(test)]\n        self.group_hook(GroupStage::AfterSync, ops);')
    elif name == 'complete_slot_twice':
        queue = replace_once(queue, '            self.slots[1 + self.delivered].set(SlotState::Done(result));', '            self.slots[1 + self.delivered].set(SlotState::Done((self.fail)()));\n            self.slots[1 + self.delivered].set(SlotState::Done(result));')
    else:
        raise ValueError(name)
    return dict(zip(sources, (db, queue, wal)))


NAMES = ['skip_fdatasync', 'publish_before_sync', 'reverse_publish', 'skip_poison', 'drop_follower_result', 'advance_sequence_early', 'complete_slot_twice']
parser = argparse.ArgumentParser()
parser.add_argument('--phase', required=True)
parser.add_argument('--mutation', choices=NAMES, action='append')
parser.add_argument('--filter', default='')
parser.add_argument('--timeout', type=int, default=120)
args = parser.parse_args()
GATE.mkdir(parents=True, exist_ok=True)
sources = {p: (ROOT / p).read_text() for p in ('src/db.rs', 'src/group_commit.rs', 'src/wal.rs')}
(GATE / f'{args.phase}-source-hashes.json').write_text(json.dumps({p: hashlib.sha256(s.encode()).hexdigest() for p, s in sources.items()}, indent=2) + '\n')
try:
    for name in args.mutation or NAMES:
        for p, content in mutate(name, sources).items():
            (ROOT / p).write_text(content)
        command = ['cargo', 'test', '--lib', args.filter, '--', '--test-threads=1']
        log = GATE / f'{args.phase}-{name}.log'
        started = time.monotonic()
        with log.open('w') as output:
            output.write('$ ' + ' '.join(command) + '\n')
            output.flush()
            env = dict(os.environ, TMPDIR=str(GATE / 'tmp'))
            process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = process.wait(timeout=args.timeout)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
                code = 124
        result = dict(phase=args.phase, mutation=name, command=command, exit=code, seconds=round(time.monotonic()-started, 2), log=str(log))
        with (GATE / 'mutations.jsonl').open('a') as output:
            output.write(json.dumps(result) + '\n')
        print(json.dumps(result), flush=True)
        for p, content in sources.items():
            (ROOT / p).write_text(content)
finally:
    for p, content in sources.items():
        (ROOT / p).write_text(content)
