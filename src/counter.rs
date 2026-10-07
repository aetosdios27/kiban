//! Exact relaxed observational totals without one shared hot cache line.
//!
//! Colliding threads still use atomics; shard placement establishes no engine
//! synchronization. Observations sum eight slots, and are not a transactional
//! snapshot while increments are in flight, just like the former relaxed fields.
use crate::engine_lock::{ENGINE_READ_SHARDS, counter_shard};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[derive(Debug, Default)]
#[repr(align(64))]
struct Slot(AtomicU64);

#[derive(Debug, Default)]
pub(crate) struct Counter {
    slots: [Slot; ENGINE_READ_SHARDS],
}
impl Counter {
    pub(crate) fn increment(&self) {
        self.slots[counter_shard()].0.fetch_add(1, Relaxed);
    }
    pub(crate) fn load(&self) -> u64 {
        self.slots
            .iter()
            .fold(0u64, |sum, slot| sum.wrapping_add(slot.0.load(Relaxed)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_increments_remain_exact_with_more_threads_than_slots() {
        let counter = std::sync::Arc::new(Counter::default());
        let threads: Vec<_> = (0..32)
            .map(|_| {
                let counter = counter.clone();
                std::thread::spawn(move || {
                    for _ in 0..10000 {
                        counter.increment();
                    }
                })
            })
            .collect();
        let mut previous = 0;
        while threads.iter().any(|t| !t.is_finished()) {
            let current = counter.load();
            assert!(current >= previous);
            previous = current;
            std::thread::yield_now();
        }
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(counter.load(), 320000);
    }
}
