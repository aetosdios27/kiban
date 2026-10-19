//! Leader/follower write groups.
//!
//! Callers enqueue a request. The first caller to find no active leader leads:
//! it takes a bounded prefix of the queue (its own request first), runs one
//! `commit` over that whole group, then hands leadership to the next queued
//! caller and returns each follower its own result. Followers sleep on a
//! private slot until their result arrives or they are promoted to lead.
//!
//! Invariants (loom-checked in `loom_tests`):
//! - at most one `commit` runs at any time, so groups are totally ordered;
//! - groups take requests in arrival order and every request is in exactly one;
//! - `leader_active` is false only while the queue is empty, so a queued
//!   request always has a leader or a pending promotion (no lost wakeup);
//! - a request's result is produced by the commit that contained it, and a
//!   panicking commit fails every member of its group instead of stranding it.
//!
//! The queue knows nothing about the engine: durability ordering (append,
//! sync, then publish) belongs to the `commit` closure.

use std::collections::VecDeque;

#[cfg(feature = "loom")]
use loom::sync::{Arc, Condvar, Mutex, MutexGuard};
#[cfg(not(feature = "loom"))]
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

/// Upper bounds for one group. The first request is always taken, so a single
/// oversized request still commits alone.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GroupLimits {
    pub(crate) max_requests: usize,
    pub(crate) max_bytes: usize,
}

pub(crate) const DEFAULT_LIMITS: GroupLimits = GroupLimits {
    max_requests: 1024,
    max_bytes: 1 << 20,
};

fn relock<T>(result: std::sync::LockResult<MutexGuard<'_, T>>) -> MutexGuard<'_, T> {
    result.unwrap_or_else(std::sync::PoisonError::into_inner)
}

enum SlotState<Res> {
    Waiting,
    Lead,
    Done(Res),
}

struct Slot<Res> {
    state: Mutex<SlotState<Res>>,
    wake: Condvar,
}

impl<Res> Slot<Res> {
    fn new() -> Self {
        Slot {
            state: Mutex::new(SlotState::Waiting),
            wake: Condvar::new(),
        }
    }

    fn set(&self, next: SlotState<Res>) {
        *relock(self.state.lock()) = next;
        self.wake.notify_one();
    }

    fn wait(&self) -> SlotState<Res> {
        let mut state = relock(self.state.lock());
        while matches!(*state, SlotState::Waiting) {
            state = relock(self.wake.wait(state));
        }
        std::mem::replace(&mut *state, SlotState::Waiting)
    }
}

struct Entry<Req, Res> {
    req: Req,
    bytes: usize,
    slot: Arc<Slot<Res>>,
}

struct State<Req, Res> {
    queue: VecDeque<Entry<Req, Res>>,
    leader_active: bool,
}

pub(crate) struct GroupQueue<Req, Res> {
    state: Mutex<State<Req, Res>>,
    limits: GroupLimits,
}

impl<Req, Res> GroupQueue<Req, Res> {
    pub(crate) fn new(limits: GroupLimits) -> Self {
        assert!(limits.max_requests > 0);
        GroupQueue {
            state: Mutex::new(State {
                queue: VecDeque::new(),
                leader_active: false,
            }),
            limits,
        }
    }

    /// Requests waiting for a leader (not yet taken into a group).
    #[cfg(test)]
    pub(crate) fn queued(&self) -> usize {
        relock(self.state.lock()).queue.len()
    }

    /// Enqueues `req` and returns its result once a group containing it has
    /// committed. `commit` receives the group in arrival order and must return
    /// exactly one result per request; it runs only if this caller leads.
    /// `fail` supplies the result for followers of a commit that panicked.
    pub(crate) fn submit(
        &self,
        req: Req,
        bytes: usize,
        commit: impl FnOnce(&mut [Req]) -> Vec<Res>,
        fail: impl Fn() -> Res,
    ) -> Res {
        let slot = Arc::new(Slot::new());
        let lead_now = {
            let mut state = relock(self.state.lock());
            state.queue.push_back(Entry {
                req,
                bytes,
                slot: slot.clone(),
            });
            !std::mem::replace(&mut state.leader_active, true)
        };
        if !lead_now {
            match slot.wait() {
                SlotState::Done(result) => return result,
                SlotState::Lead => {}
                SlotState::Waiting => unreachable!("wait returns only Done or Lead"),
            }
        }
        // Either the queue was empty and leadership free, or the previous
        // leader promoted us from the queue front. Our request is first.
        let (mut reqs, slots) = self.take_group();
        debug_assert!(Arc::ptr_eq(&slots[0], &slot));
        let mut leader = Leader {
            queue: self,
            slots,
            fail: &fail,
            handed_off: false,
            delivered: 0,
        };
        let mut results = commit(&mut reqs);
        assert_eq!(
            results.len(),
            reqs.len(),
            "commit must answer every request"
        );
        leader.finish(&mut results)
    }

    fn take_group(&self) -> (Vec<Req>, Vec<Arc<Slot<Res>>>) {
        let mut state = relock(self.state.lock());
        let mut reqs = Vec::new();
        let mut slots = Vec::new();
        let mut bytes = 0usize;
        while let Some(next) = state.queue.front() {
            if !reqs.is_empty()
                && (reqs.len() >= self.limits.max_requests
                    || bytes.saturating_add(next.bytes) > self.limits.max_bytes)
            {
                break;
            }
            let entry = state.queue.pop_front().expect("front just observed");
            bytes = bytes.saturating_add(entry.bytes);
            reqs.push(entry.req);
            slots.push(entry.slot);
        }
        (reqs, slots)
    }

    /// Passes leadership to the oldest waiter, or marks it free when none.
    fn hand_off(&self) {
        let mut state = relock(self.state.lock());
        match state.queue.front() {
            Some(next) => next.slot.set(SlotState::Lead),
            None => state.leader_active = false,
        }
    }
}

/// Completes a group exactly once, even if `commit` unwinds, or if anything
/// unwinds between handing off leadership and delivering the last follower:
/// `Drop` fails every follower not yet delivered and hands off only if
/// `finish` had not already.
struct Leader<'a, Req, Res> {
    queue: &'a GroupQueue<Req, Res>,
    slots: Vec<Arc<Slot<Res>>>,
    fail: &'a dyn Fn() -> Res,
    handed_off: bool,
    /// Followers (`slots[1..]`) already given their result.
    delivered: usize,
}

impl<Req, Res> Leader<'_, Req, Res> {
    /// Hands off first so the next group's WAL work overlaps waking this
    /// group's followers, then delivers follower results. Returns our own.
    fn finish(&mut self, results: &mut Vec<Res>) -> Res {
        let mut results = results.drain(..);
        let own = results.next().expect("leader result");
        self.queue.hand_off();
        self.handed_off = true;
        #[cfg(all(test, not(feature = "loom")))]
        tests::panic_after_hand_off_if_requested();
        for result in results {
            self.slots[1 + self.delivered].set(SlotState::Done(result));
            self.delivered += 1;
        }
        own
    }
}

impl<Req, Res> Drop for Leader<'_, Req, Res> {
    fn drop(&mut self) {
        if !self.handed_off {
            self.queue.hand_off();
        }
        for slot in &self.slots[1 + self.delivered..] {
            slot.set(SlotState::Done((self.fail)()));
        }
    }
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Barrier, Mutex as StdMutex};

    thread_local! {
        static PANIC_AFTER_HAND_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// Lets a test unwind inside `Leader::finish` after leadership moved on
    /// and before any follower was delivered. Per thread: only the leader's
    /// own commit closure arms it.
    pub(super) fn panic_after_hand_off_if_requested() {
        if PANIC_AFTER_HAND_OFF.with(|flag| flag.replace(false)) {
            panic!("injected panic after hand_off");
        }
    }

    #[test]
    fn lone_caller_leads_its_own_group_of_one() {
        let queue = GroupQueue::new(DEFAULT_LIMITS);
        let out = queue.submit(7u32, 1, |g| g.iter().map(|r| r * 2).collect(), || 0);
        assert_eq!(out, 14);
        assert_eq!(queue.queued(), 0);
        // Leadership was released: the next caller leads again.
        let out = queue.submit(1u32, 1, |g| g.iter().map(|r| r + 1).collect(), || 0);
        assert_eq!(out, 2);
    }

    #[test]
    fn followers_join_the_group_the_leader_commits() {
        const WRITERS: usize = 8;
        let queue = GroupQueue::new(DEFAULT_LIMITS);
        let groups = StdMutex::new(Vec::<Vec<usize>>::new());
        let started = Barrier::new(2);
        std::thread::scope(|scope| {
            // The first caller blocks inside its commit until the rest queue up.
            let first = scope.spawn(|| {
                queue.submit(
                    0usize,
                    1,
                    |g| {
                        started.wait();
                        while queue.queued() < WRITERS - 1 {
                            std::thread::yield_now();
                        }
                        groups.lock().unwrap().push(g.to_vec());
                        g.iter().map(|r| r + 100).collect()
                    },
                    || usize::MAX,
                )
            });
            started.wait();
            let rest: Vec<_> = (1..WRITERS)
                .map(|i| {
                    let (queue, groups) = (&queue, &groups);
                    scope.spawn(move || {
                        queue.submit(
                            i,
                            1,
                            |g| {
                                groups.lock().unwrap().push(g.to_vec());
                                g.iter().map(|r| r + 100).collect()
                            },
                            || usize::MAX,
                        )
                    })
                })
                .collect();
            assert_eq!(first.join().unwrap(), 100);
            for (i, handle) in rest.into_iter().enumerate() {
                assert_eq!(handle.join().unwrap(), i + 1 + 100);
            }
        });
        let groups = groups.into_inner().unwrap();
        assert_eq!(groups[0], vec![0]);
        // Everyone else queued behind the first commit and shares one group,
        // led by whichever writer was promoted.
        assert_eq!(groups.len(), 2, "{groups:?}");
        assert_eq!(groups[1].len(), WRITERS - 1);
    }

    #[test]
    fn groups_respect_request_and_byte_limits_but_always_take_one() {
        let limits = GroupLimits {
            max_requests: 3,
            max_bytes: 10,
        };
        let queue = GroupQueue::new(limits);
        let sizes = StdMutex::new(Vec::new());
        let gate = Barrier::new(2);
        std::thread::scope(|scope| {
            let queue = &queue;
            let (sizes, gate) = (&sizes, &gate);
            scope.spawn(move || {
                queue.submit(
                    (0usize, 1usize),
                    1,
                    |g| {
                        gate.wait();
                        while queue.queued() < 7 {
                            std::thread::yield_now();
                        }
                        sizes.lock().unwrap().push(g.len());
                        vec![(); g.len()]
                    },
                    || (),
                );
            });
            gate.wait();
            // Queue order is not controlled; sizes alone drive the limits.
            for i in 0..7usize {
                let bytes = if i == 0 { 50 } else { 4 };
                scope.spawn(move || {
                    queue.submit(
                        (i, bytes),
                        bytes,
                        |g| {
                            sizes.lock().unwrap().push(g.len());
                            vec![(); g.len()]
                        },
                        || (),
                    );
                });
            }
        });
        let sizes = sizes.into_inner().unwrap();
        assert_eq!(sizes.iter().sum::<usize>(), 8);
        assert!(sizes.iter().all(|&n| (1..=3).contains(&n)), "{sizes:?}");
    }

    #[test]
    fn panicking_commit_fails_followers_and_frees_leadership() {
        let queue = GroupQueue::new(DEFAULT_LIMITS);
        let entered = Barrier::new(2);
        let calls = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            let leader = scope.spawn(|| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    queue.submit(
                        1u32,
                        1,
                        |_| {
                            entered.wait();
                            while queue.queued() < 1 {
                                std::thread::yield_now();
                            }
                            calls.fetch_add(1, Ordering::SeqCst);
                            panic!("commit blew up");
                        },
                        || 0,
                    )
                }))
            });
            entered.wait();
            // Queued behind the doomed leader: promoted, then commits normally.
            let follower = scope.spawn(|| queue.submit(2u32, 1, |g| g.to_vec(), || 0));
            assert!(leader.join().unwrap().is_err());
            assert_eq!(follower.join().unwrap(), 2);
        });
        assert_eq!(queue.submit(3u32, 1, |g| g.to_vec(), || 0), 3);
    }

    #[test]
    fn followers_of_a_panicked_group_get_the_failure_result() {
        let queue = GroupQueue::new(DEFAULT_LIMITS);
        let entered = Barrier::new(2);
        let outcomes = StdMutex::new(Vec::new());
        std::thread::scope(|scope| {
            let queue = &queue;
            let outcomes = &outcomes;
            // A holds leadership until B and C are both queued, so B and C
            // land in one group whose commit panics.
            scope.spawn(|| {
                queue.submit(
                    0u32,
                    1,
                    |g| {
                        entered.wait();
                        while queue.queued() < 2 {
                            std::thread::yield_now();
                        }
                        g.to_vec()
                    },
                    || 99,
                );
            });
            entered.wait();
            for req in [1u32, 2] {
                scope.spawn(move || {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        queue.submit(req, 1, |_| panic!("boom"), || 99)
                    }));
                    outcomes.lock().unwrap().push(outcome.ok());
                });
            }
        });
        let mut outcomes = outcomes.into_inner().unwrap();
        outcomes.sort();
        assert_eq!(outcomes, vec![None, Some(99)]);
        assert_eq!(queue.submit(3u32, 1, |g| g.to_vec(), || 99), 3);
    }

    #[test]
    fn panic_after_hand_off_still_fails_undelivered_followers_and_frees_leadership() {
        let queue = GroupQueue::new(DEFAULT_LIMITS);
        let entered = Barrier::new(2);
        let follower_outcomes = StdMutex::new(Vec::new());
        let leader_outcome = StdMutex::new(None);
        std::thread::scope(|scope| {
            let queue = &queue;
            let follower_outcomes = &follower_outcomes;
            let leader_outcome = &leader_outcome;
            // A holds leadership until B and C queue behind it; B then leads
            // the group {B, C} and unwinds after handing off, with C
            // undelivered.
            scope.spawn(|| {
                queue.submit(
                    0u32,
                    1,
                    |g| {
                        entered.wait();
                        while queue.queued() < 2 {
                            std::thread::yield_now();
                        }
                        g.to_vec()
                    },
                    || 99,
                );
            });
            entered.wait();
            for req in [1u32, 2] {
                scope.spawn(move || {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        queue.submit(
                            req,
                            1,
                            |g| {
                                PANIC_AFTER_HAND_OFF.with(|flag| flag.set(true));
                                g.to_vec()
                            },
                            || 99,
                        )
                    }));
                    match outcome {
                        Ok(value) => follower_outcomes.lock().unwrap().push(value),
                        Err(_) => *leader_outcome.lock().unwrap() = Some(req),
                    }
                });
            }
        });
        // One of B/C led and panicked; the other was a follower and must have
        // been failed (99), not left waiting.
        assert!(leader_outcome.into_inner().unwrap().is_some());
        assert_eq!(follower_outcomes.into_inner().unwrap(), vec![99]);
        // Leadership was released: the next caller leads again.
        assert_eq!(queue.submit(3u32, 1, |g| g.to_vec(), || 99), 3);
    }
}

/// Exhaustive interleaving models. Run with
/// `cargo test --features loom --lib -- --test-threads=1 loom_`.
#[cfg(all(test, feature = "loom"))]
mod loom_tests {
    use super::*;
    use loom::sync::atomic::{AtomicBool, Ordering};
    use loom::thread;

    struct Shared {
        queue: GroupQueue<u32, u32>,
        committing: AtomicBool,
        groups: Mutex<Vec<Vec<u32>>>,
    }

    fn submit(shared: &Arc<Shared>, req: u32) -> u32 {
        shared.queue.submit(
            req,
            1,
            |group| {
                assert!(
                    !shared.committing.swap(true, Ordering::SeqCst),
                    "two commits overlapped"
                );
                shared.groups.lock().unwrap().push(group.to_vec());
                let out = group.iter().map(|r| r + 1000).collect();
                shared.committing.store(false, Ordering::SeqCst);
                out
            },
            || u32::MAX,
        )
    }

    fn model(writers: u32, limits: GroupLimits, preemptions: usize) {
        let mut builder = loom::model::Builder::new();
        builder.preemption_bound = Some(preemptions);
        builder.check(move || {
            let shared = Arc::new(Shared {
                queue: GroupQueue::new(limits),
                committing: AtomicBool::new(false),
                groups: Mutex::new(Vec::new()),
            });
            let handles: Vec<_> = (0..writers)
                .map(|i| {
                    let shared = shared.clone();
                    thread::spawn(move || (i, submit(&shared, i)))
                })
                .collect();
            for handle in handles {
                let (i, result) = handle.join().unwrap();
                assert_eq!(result, i + 1000, "result must come from its own request");
            }
            let groups = shared.groups.lock().unwrap();
            let mut flat: Vec<u32> = groups.iter().flatten().copied().collect();
            assert!(groups.iter().all(|g| !g.is_empty()));
            flat.sort_unstable();
            assert_eq!(flat, (0..writers).collect::<Vec<_>>(), "exactly once");
            let state = shared.queue.state.lock().unwrap();
            assert!(state.queue.is_empty() && !state.leader_active);
        });
    }

    #[test]
    fn loom_two_writers_commit_once_each_with_no_overlap() {
        model(2, DEFAULT_LIMITS, 4);
    }

    #[test]
    fn loom_three_writers_never_strand_a_follower() {
        model(3, DEFAULT_LIMITS, 3);
    }

    #[test]
    fn loom_group_limit_of_one_hands_leadership_down_the_queue() {
        model(
            3,
            GroupLimits {
                max_requests: 1,
                max_bytes: 1 << 20,
            },
            3,
        );
    }
}
