use crate::random::mix;
use serde::{Deserialize, Serialize};

pub const AGENTS: u64 = 64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Memory {
    pub memory_id: String,
    pub agent: u32,
    pub created_at: u64,
    pub revision: u64,
    pub text: String,
}

impl Memory {
    pub fn new(id: u64, revision: u64, seed: u64) -> Self {
        let text = (0..200)
            .map(|i| (b'a' + (mix(seed ^ mix(id) ^ mix(revision) ^ i) % 26) as u8) as char)
            .collect();
        Self {
            memory_id: format!("memory-{id:020}"),
            agent: (id % AGENTS) as u32,
            created_at: id,
            revision,
            text,
        }
    }

    pub fn key(&self) -> String {
        key(self.created_at)
    }
}

// Creation order is immutable. This primary key gives ID reads and an ordered
// latest-for-agent range without maintaining a second, differently durable index.
pub fn key(id: u64) -> String {
    format!("agent/{:04}/memory/{:020}", id % AGENTS, u64::MAX - id)
}

pub fn recent_ids(n: u64, agent: u32, k: usize) -> Vec<u64> {
    (0..n)
        .rev()
        .filter(|id| id % AGENTS == u64::from(agent))
        .take(k)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_order_and_payload_identity() {
        let ids = recent_ids(200, 3, 2);
        assert_eq!(ids, [195, 131]);
        assert!(key(ids[0]) < key(ids[1]));
        let a = Memory::new(195, 0, 17);
        let b = Memory::new(195, 1, 17);
        assert_eq!(a.key(), b.key());
        assert_eq!(a.text.len(), 200);
        assert_ne!(a.text, b.text);
    }
}
