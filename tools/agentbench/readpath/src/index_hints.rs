//! In-memory separator-search prototype; no file bytes or engine behavior change.
use std::cmp::Ordering;

const LEAF_SIZE: usize = 16;
const MAX_DEPTH: usize = 8;

enum Prefix {
    Inline { bytes: [u8; 32], len: u8 },
    Long(Vec<u8>),
}

impl Prefix {
    fn new(bytes: &[u8]) -> Self {
        if bytes.len() > 32 {
            return Self::Long(bytes.to_vec());
        }
        let mut inline = [0; 32];
        inline[..bytes.len()].copy_from_slice(bytes);
        Self::Inline {
            bytes: inline,
            len: bytes.len() as u8,
        }
    }
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Inline { bytes, len } => &bytes[..usize::from(*len)],
            Self::Long(bytes) => bytes,
        }
    }
}

struct Edge {
    label: u16,
    start: usize,
    child: usize,
}
enum Node {
    Leaf {
        start: usize,
        end: usize,
    },
    Branch {
        start: usize,
        end: usize,
        at: usize,
        prefix: Prefix,
        edges: Vec<Edge>,
    },
}

pub(crate) struct IndexHints {
    nodes: Vec<Node>,
}

impl IndexHints {
    pub(crate) fn new(keys: &[impl AsRef<[u8]>]) -> Self {
        let mut hints = Self { nodes: Vec::new() };
        hints.build(keys, 0, keys.len(), 0, 0);
        hints
    }
    fn build(
        &mut self,
        keys: &[impl AsRef<[u8]>],
        start: usize,
        end: usize,
        known: usize,
        depth: usize,
    ) -> usize {
        let node = self.nodes.len();
        self.nodes.push(Node::Leaf { start, end });
        if end - start <= LEAF_SIZE || depth == MAX_DEPTH {
            return node;
        }
        let first = keys[start].as_ref();
        let last = keys[end - 1].as_ref();
        let common = first.iter().zip(last).take_while(|(a, b)| a == b).count();
        let prefix = Prefix::new(&first[known..common]);
        let mut edges = Vec::new();
        let mut from = start;
        while from < end {
            let label = label_of(keys[from].as_ref(), common);
            let to = from
                + keys[from..end].partition_point(|key| label_of(key.as_ref(), common) == label);
            let child = self.build(keys, from, to, common + usize::from(label != 0), depth + 1);
            edges.push(Edge {
                label,
                start: from,
                child,
            });
            from = to;
        }
        self.nodes[node] = Node::Branch {
            start,
            end,
            at: known,
            prefix,
            edges,
        };
        node
    }
    pub(crate) fn partition_point(&self, keys: &[impl AsRef<[u8]>], target: &[u8]) -> usize {
        let mut node = 0;
        loop {
            match &self.nodes[node] {
                Node::Leaf { start, end } => {
                    return start + keys[*start..*end].partition_point(|key| key.as_ref() < target);
                }
                Node::Branch {
                    start,
                    end,
                    at,
                    prefix,
                    edges,
                } => {
                    let prefix = prefix.bytes();
                    let tail = target.get(*at..).unwrap_or_default();
                    let shared = tail.len().min(prefix.len());
                    match tail[..shared].cmp(&prefix[..shared]) {
                        Ordering::Less => return *start,
                        Ordering::Greater => return *end,
                        Ordering::Equal => {}
                    }
                    if tail.len() < prefix.len() {
                        return *start;
                    }
                    let wanted = label_of(target, *at + prefix.len());
                    let edge = edges.partition_point(|edge| edge.label < wanted);
                    let Some(edge) = edges.get(edge) else {
                        return *end;
                    };
                    if edge.label > wanted {
                        return edge.start;
                    }
                    node = edge.child;
                }
            }
        }
    }
}

fn label_of(key: &[u8], at: usize) -> u16 {
    key.get(at).map_or(0, |&b| u16::from(b) + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separator_hints_match_binary_search_for_present_and_absent_binary_keys() {
        let mut keys = vec![Vec::new(), vec![0], vec![255], vec![7; 200]];
        for i in 0..2000u32 {
            keys.push(i.to_be_bytes().to_vec());
            keys.push(
                format!("agent/{:04}/memory/{:020}", i % 64, u64::MAX - u64::from(i)).into_bytes(),
            );
        }
        for n in 0..70 {
            keys.push(vec![b'a'; n]);
        }
        keys.sort();
        keys.dedup();
        let hints = IndexHints::new(&keys);
        let check = |target: &[u8]| {
            assert_eq!(
                hints.partition_point(&keys, target),
                keys.partition_point(|key| key.as_slice() < target),
                "{target:?}"
            )
        };
        for key in &keys {
            check(key);
            let mut after = key.clone();
            after.push(0);
            check(&after);
            if !key.is_empty() {
                check(&key[..key.len() - 1]);
            }
        }
        for a in 0..=255 {
            for b in [0, 1, 127, 255] {
                check(&[a, b]);
            }
        }
        let empty: Vec<Vec<u8>> = Vec::new();
        assert_eq!(IndexHints::new(&empty).partition_point(&empty, b"x"), 0);
        let duplicate = vec![b"same".to_vec(); 100];
        let hints = IndexHints::new(&duplicate);
        assert_eq!(hints.partition_point(&duplicate, b"same"), 0);
        assert_eq!(hints.partition_point(&duplicate, b"same0"), 100);
    }
}
