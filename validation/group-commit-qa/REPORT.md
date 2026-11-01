# Group-commit QA at 4170b731

This branch adds tests and test runners only. It does not change engine behavior or open a PR. The review found no ordinary durability, atomicity, ordering, or error-handling defect in the exercised paths. An injected leader panic after publication exposes an ambiguous completion result, described in finding 2.

All logs and simulated test directories are under `~/.local/share/kiban-gates/gc-qa-20261022/`. Commands below ran from this worktree. `TMPDIR` keeps test directories out of `/tmp`.

## Findings

1. **Severity: none. No issue found with acknowledged durability across WAL generations.**
   Reproducer: `qa_live_wal_generations_sync_before_ack_and_survive_rotation`.
   The test rotates through six generations. Each iteration leaves an immutable WAL and an active WAL live, writes buffered records to both generations, acknowledges a durable write, and immediately simulates strict power loss. Recovery equals the complete expected map. Capturing at acknowledgement prevents a later flush from hiding a missing durability barrier. The engine permits one immutable generation, so two WALs can be live at once. The concurrent checkpoint test also exercises freeze and rotation against grouped writers. The seeded gate requires both acknowledged writes and writes whose `AfterSync` hook ran to survive. No writer returned `Ok` for a lost record in these runs.

2. **Severity: medium. A follower can receive `Err(Corrupt)` for an already durable, visible write when its leader panics after publication.**
   Reproducer: `qa_panic_boundaries_complete_followers_and_recover_synced_records`, `Published` case.
   A forced two-member group panics at `Published`, before hand-off. Its leader unwinds. Its follower completes with `Err(DbError::Corrupt("group commit leader panicked"))`. Both records are readable and survive strict power loss. This ambiguity is outside the freeze-failure acknowledgement policy. At `AfterSync`, both records survive recovery but neither is published in the running engine. At `BeforeAppend` and `AfterAppend`, neither record survives strict loss. The test also verifies that a retry on the same handle fails because the engine gate is poisoned. Reopen recovers the synced records. Retrying an operation after reopen therefore requires the caller to resolve its ambiguous outcome. These are injected panic outcomes, not an observed ordinary I/O failure. The test asserts the current behavior; this branch does not propose an engine fix.

3. **Severity: none. No issue found with batch visibility or crash atomicity.**
   Reproducers: `qa_grouped_batches_recover_atomically_at_each_crash_boundary`, `qa_readers_and_snapshots_wait_for_whole_group_publication`, and `qa_checkpoint_close_backup_and_snapshots_with_grouped_batches`.
   Three batches share a forced group and mix puts, deletes, and repeated keys. Strict crashes at `BeforeAppend`, `AfterAppend`, `AfterSync`, `Applied(0)`, and `Published` recover exactly the old map or the complete synced map. A paused partial publication blocks a point reader and snapshot capture until the entire group is published. Snapshots and backups retain matching key pairs during concurrent batches. Atomicity here means one read or snapshot boundary. Separate `get` calls can legitimately straddle a commit.

4. **Severity: none. No issue found with sequence order for mixed operations.**
   Reproducers: `qa_mixed_batches_tombstones_and_groups_match_wal_order_and_snapshots` and `qa_sequence_exhaustion_rejects_whole_batch_and_preserves_other_group_members`.
   Thirty-two randomized schedules each use four competing writers and 48 mixed operations, plus the initial leader. An independent map applies requests in observed WAL append order, including repeated-key puts, tombstones, and batches. Point reads, snapshots, and strict recovery match that map. An old snapshot stays unchanged. Near `u64::MAX`, an oversized batch fails as a whole, later fitting members still commit, an empty batch succeeds, and an overflowing retry fails without poisoning or sequence wrap. Recovery preserves the maximum surviving sequence.

5. **Severity: none. No issue found with group failure, poisoning, retries, or strict recovery after append and sync faults.**
   Reproducer: `qa_failed_groups_refuse_all_retries_and_recover_exact_synced_prefix`.
   The test injects faults at eight positions for each of two record sizes. Three gathered batches all succeed or all fail. Large frames reach append failures; small frames reach flush and fdatasync failures. Failed groups publish nothing and leave the sequence unchanged. After a failure, `put`, `delete`, `write`, `put_durable`, `delete_durable`, `write_durable`, and `sync` all reject the poisoned handle. Recovery after strict power loss equals the exact synced map, including absence of failed-group keys and retention of a key those batches tried to delete. A retry becomes legal after reopen. A process crash or an adversarial loss can retain unacknowledged frames; this test's exact-prefix claim applies to the strict device model. Existing PR #23 tests already cover that broader ambiguity.

6. **Severity: none. No issue found with lost or duplicate completion, leadership, or detached followers in the exercised schedules.**
   Reproducers: `qa_finish_delivers_each_slot_once_without_synthesizing_failures`, `qa_detached_follower_still_completes_and_leadership_remains_available`, and `loom_qa_guard_drop_completes_each_follower_and_promotes_only_front`.
   The direct slot test catches a missing result immediately and forbids an extra synthesized completion on success. A detached follower finishes and the queue remains usable. Dropping a Rust `JoinHandle` detaches its thread; it does not cancel a writer already executing `submit`. The loom guard-drop model completes its follower once, promotes the queue front once, and releases leadership when drained. Existing loom models exercise two and three writers, including one-request groups. Existing PR #23 coverage verifies panic safety after hand-off. No starvation was observed. Finite tests and bounded loom schedules do not prove fairness under an indefinitely unfair OS scheduler.

7. **Severity: none. No issue found with stalls, checkpoint, close, backup, or snapshots.**
   Reproducers: `qa_stalled_group_wakes_on_progress_or_sticky_failure_and_reads_continue` and `qa_checkpoint_close_backup_and_snapshots_with_grouped_batches`.
   At the L0 hard ceiling, one leader stalls and three followers queue. Point reads, snapshots, and backup still finish. Releasing compaction lets all writers finish. Alternatively, `Maintenance::fail` wakes the group and rejects every member, preserves the first sticky error, and rejects a later buffered write. The concurrent protocol test runs 180 batches against eight checkpoints, eight cloned-handle closes, eight backup exports, and repeated snapshot captures with a small write buffer and L0 ceiling. Exported key pairs stay consistent, all final writes recover, and another clone remains writable after `close()`.

8. **Severity: low. Coverage lacked several exact or deterministic assertions; the new tests close those gaps.**
   Reproducers: the tests above and `qa_invalid_batch_is_rejected_before_queue_and_corrected_retry_succeeds`.
   The prior fault test required acknowledged keys to survive, but did not require failed keys to be absent after strict loss or exercise every buffered mutation API after the failure. The crash oracle required acknowledgements to survive but did not independently require the currently synced group to survive before acknowledgement. Its batch plans lacked repeated versions of the same key inside one batch. Those assertions and plans are now present. An invalid batch with a valid prefix is rejected before queueing, changes no sequence or visible record, and permits a corrected retry. The new direct slot assertion also turns a timeout-only lost-result failure into an immediate assertion failure.

PR #23's freeze-failure acknowledgement policy, panic safety after hand-off, append-failure poisoning, same-key group order, and failed-sync sequence handling are existing coverage. They are not new findings here.

## Added tests and crash changes

Ten integration tests are in `src/group_commit_qa_tests.rs`, included by the existing test-only group-commit module. Two queue tests and one loom test are appended under `cfg(test)` in `src/group_commit.rs`. Each test's contract is described in the findings above.

`src/group_commit_tests.rs` also extends the existing long crash gate:

- Generated batches overwrite, delete, and restore a key within one WAL batch while retaining a unique per-operation prefix witness.
- Each `AfterSync` records a separate per-thread synced prefix. Recovery must cover the maximum of the acknowledged and synced prefixes.
- The existing global WAL-prefix and per-thread whole-operation checks remain in place.

At this base revision, `group_commit_crash_long` lives in `src/group_commit_tests.rs`. The ordinary direct and shared simulation tests in `src/simulation_tests.rs` ran as part of the full library suite.

## Mutation results

The baseline command targeted all 20 existing group-commit tests before the new modules were included. One was the ignored long gate. Every mutation was applied alone and reverted before the next. No mutation survived the existing focused suite. A lost follower result was detected by timeout rather than a completed assertion. The new tests were then checked separately against all seven breaks.

1. `skip_fdatasync`: remove the WAL's `sync_data()` call while retaining userspace flush. Existing suite **caught**, exit 101, 13 passed and 6 failed. For example, a durable buffered record recovered as `None` instead of `Some("buffered")`. New tests **caught**, exit 101, 5 passed and 5 failed.
2. `publish_before_sync`: move the sync after the memtable publication loop and mirror unlock. Existing suite **caught**, exit 101, 16 passed and 3 failed. Failure showed `Some("v1")` where an unsynced write had to be `None`. New tests **caught**, exit 101, 7 passed and 3 failed, including strict crash at `Applied(0)`.
3. `reverse_publish`: reverse the request/sequence publication iterator. Existing suite **caught**, exit 101, 17 passed and 2 failed. The per-key ascending-sequence assertion fired. New tests **caught**, exit 101, 7 passed and 3 failed, including mixed tombstone batches.
4. `skip_poison`: preserve the returned append/sync error but omit recording the sticky poison. Existing suite **caught**, exit 101, 16 passed and 3 failed. New tests **caught**, exit 101, 9 passed and 1 failed at `assert!(db.is_poisoned())`.
5. `drop_follower_result`: discard the result and increment `delivered` without completing the slot. Existing suite **caught as a hang**, killed at 45 seconds, exit 124. The crash test remained waiting on its first forced group. The new direct slot test **caught**, exit 101, with `SlotState::Done(20)` absent. It finished in under the displayed test timer's resolution.
6. `advance_sequence_early`: assign `last_sequence` before the WAL sync. Existing suite **caught**, exit 101, 18 passed and 1 failed, sequence `2` versus expected `1` after failure. New tests **caught**, exit 101, 9 passed and 1 failed.
7. `complete_slot_twice`: publish a synthesized failure to a follower and then its real result. Existing suite **caught**, exit 101, 18 passed and 1 failed when a crash-test writer received the synthesized error. This detection depends on scheduling. The new direct slot test **caught deterministically**, exit 101, because the healthy completion invoked the failure callback, `1` versus expected `0`.

These are the exact mutations implemented by `mutations.py`; they are not claims about every possible equivalent corruption. There were no survivors requiring an additional survivor-specific test. The new slot test strengthens detection for mutations 5 and 7.

## Commands and result tails

Baseline:

```sh
mkdir -p ~/.local/share/kiban-gates/gc-qa-20261022/tmp
TMPDIR=$HOME/.local/share/kiban-gates/gc-qa-20261022/tmp cargo test --lib group_commit -- --test-threads=1 > ~/.local/share/kiban-gates/gc-qa-20261022/baseline.log 2>&1
```

```text
test result: ok. 19 passed; 0 failed; 1 ignored; 0 measured; 383 filtered out; finished in 4.71s
```

Existing-suite mutations:

```sh
nohup python3 validation/group-commit-qa/mutations.py --phase baseline-focused --filter group_commit --timeout 45 > ~/.local/share/kiban-gates/gc-qa-20261022/mutation-focused.log 2>&1 < /dev/null
```

Each child command was `cargo test --lib group_commit -- --test-threads=1`. The result tails were:

```text
skip_fdatasync:        FAILED. 13 passed; 6 failed; 1 ignored
publish_before_sync:  FAILED. 16 passed; 3 failed; 1 ignored
reverse_publish:      FAILED. 17 passed; 2 failed; 1 ignored
skip_poison:          FAILED. 16 passed; 3 failed; 1 ignored
drop_follower_result: running 20 tests; first crash test never completed; exit 124
advance_sequence_early: FAILED. 18 passed; 1 failed; 1 ignored
complete_slot_twice:  FAILED. 18 passed; 1 failed; 1 ignored
```

An initial unfiltered mutation run used `nohup python3 validation/group-commit-qa/mutations.py --phase baseline --timeout 90`. Its first mutation reached `power_loss_recovers_exactly_the_last_synced_state ... FAILED` but the whole suite exceeded 90 seconds. The second mutation run was interrupted. Those incomplete runs are excluded from the results above. Their logs remain under the gate directory.

New-test mutations:

```sh
nohup python3 validation/group-commit-qa/mutations.py --phase regression --filter qa_ --mutation skip_fdatasync --mutation publish_before_sync --mutation reverse_publish --mutation skip_poison --mutation advance_sequence_early --mutation complete_slot_twice --timeout 45 > ~/.local/share/kiban-gates/gc-qa-20261022/mutation-regression.log 2>&1 < /dev/null
nohup python3 validation/group-commit-qa/mutations.py --phase regression-slot --filter qa_finish_delivers --mutation drop_follower_result --mutation complete_slot_twice --timeout 45 > ~/.local/share/kiban-gates/gc-qa-20261022/mutation-regression-slot.log 2>&1 < /dev/null
```

The first command ran before the final input-validation and exhaustion tests were added. Every child exited 101 due to the expected test assertion, with no compilation failures or timeouts. The second command produced these immediate failures:

```text
drop_follower_result:
assertion failed: matches!(*relock(follower.state.lock()), SlotState::Done(20))
test result: FAILED. 0 passed; 1 failed
complete_slot_twice:
assertion `left == right` failed: healthy completion must not produce a second failure result
  left: 1
 right: 0
test result: FAILED. 0 passed; 1 failed
```

After restoring the engine, the full library run was:

```sh
TMPDIR=$HOME/.local/share/kiban-gates/gc-qa-20261022/tmp cargo test --lib -- --test-threads=4 > ~/.local/share/kiban-gates/gc-qa-20261022/full-lib.log 2>&1
```

```text
test result: ok. 408 passed; 0 failed; 7 ignored; 0 measured; 0 filtered out; finished in 48.49s
```

The new tests were also run with `cargo test --lib qa_ -- --test-threads=1` during development, and the reader rendezvous separately with `cargo test --lib qa_readers_and_snapshots -- --test-threads=1`. Early test-only mistakes used manual maintenance without a wake and tried to hold a manual stall whose waiter could run maintenance itself. Those tests failed, were corrected to use an explicit wake or a paused real worker, and passed in the final full suite. No engine change was made for those failures.

Loom:

```sh
TMPDIR=$HOME/.local/share/kiban-gates/gc-qa-20261022/tmp cargo test --features loom --lib group_commit:: -- --test-threads=1 > ~/.local/share/kiban-gates/gc-qa-20261022/loom-final.log 2>&1
```

```text
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 410 filtered out; finished in 7.03s
```

Long crash gates:

```sh
nohup python3 validation/group-commit-qa/seeds.py --seeds 200000 --shards 16 > ~/.local/share/kiban-gates/gc-qa-20261022/seeds-runner.log 2>&1 < /dev/null
nohup python3 validation/group-commit-qa/seeds.py --seeds 200000 --shards 16 --start 200000 --label seeds-final > ~/.local/share/kiban-gates/gc-qa-20261022/seeds-final-runner.log 2>&1 < /dev/null
```

Each runner builds with `cargo test --release --lib --no-run --message-format=json` and runs the returned test executable directly, avoiding concurrent Cargo build locks. Every child uses `group_commit_crash_long --ignored --nocapture --test-threads=1`, `TMPDIR` under the gate directory, `KIBAN_SIM_SEEDS=12500`, and a distinct `KIBAN_SIM_SEED_START`. The first range was 0 through 199999. The final-source range was 200000 through 399999. Both runs passed, for 400,000 distinct seeds in total. All 32 children exited 0. The first summary reports 200000 seeds and `passed: true`; the final summary reports the same. Their manifests are `seeds-summary.json` and `seeds-final-summary.json` in the gate directory.

Representative first and last shard tails, with all other shard markers checked by the runner:

```text
seeds-000000-12500.log
running 1 test
test db::group_commit_tests::group_commit_crash_long ... checked 12500 group-commit crash seeds from 0
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 411 filtered out; finished in 87.66s

seeds-200000-12500.log
running 1 test
test db::group_commit_tests::group_commit_crash_long ... checked 12500 group-commit crash seeds from 200000
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 414 filtered out; finished in 78.86s

seeds-387500-12500.log
running 1 test
test db::group_commit_tests::group_commit_crash_long ... checked 12500 group-commit crash seeds from 387500
ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 414 filtered out; finished in 79.40s
```

The runner checks each child's exit code, exact seed-count marker, and Rust test result before adding its seeds to the total. The final summary also records SHA-256 hashes of the four relevant source files.

Source and formatting checks:

```sh
python3 validation/group-commit-qa/verify_engine.py
git diff -- src/db.rs src/background.rs src/wal.rs
git diff --check
rustfmt --edition 2024 --check src/group_commit.rs src/group_commit_tests.rs src/group_commit_qa_tests.rs
```

```text
PASS: all tracked src/ bytes match 4170b731, except test-only group_commit additions and group_commit_tests.rs.
```

The engine-file diff and whitespace checks produced no output. The existing contents of `src/group_commit.rs` match the base byte-for-byte; its only additions are the two explicitly test-gated modules. All temporary mutations were reverted before final validation and commit.

## Scope of the evidence

The long gate uses the existing simulated device's adversarial namespace and sector-loss model, five crash stages, target groups 0 through 2, two through six writers, six through twelve operations per writer, and odd-seed memtable rotations. Thread schedules remain OS-dependent; the seed reproduces the workload and device-loss choices. A stage not reached falls back to a crash after completion, as in the original gate. The ordinary 350-seed test checks that every crash stage reaches live multi-writer groups.

Fault injection returns an error before the selected simulated syscall. It does not model every real device's behavior after an actual EIO. `set_len` is immediately durable in this simulator. Allocation aborts, process termination at arbitrary machine instructions, and indefinite scheduler starvation are outside these tests. The loom model bounds preemptions and checks queue synchronization rather than modeling the entire LSM or filesystem.
