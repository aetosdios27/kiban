//! Packed, optional search metadata for an immutable resident data block.
use kiban::{
    cache::CachedBlock,
    sstable::{Kind, SstError, VerifiedBlock},
};

struct Entry {
    suffix: u32,
    key_len: u32,
    value: u32,
    value_len: u32,
    seq: u64,
    kind: Kind,
}

pub(crate) struct BlockIndex {
    prefix: Vec<u8>,
    suffixes: Vec<u8>,
    entries: Vec<Entry>,
}

struct Pending {
    key: Vec<u8>,
    seq: u64,
    kind: Kind,
    value: usize,
    value_len: usize,
}

impl BlockIndex {
    pub(crate) fn build(block: &CachedBlock) -> Result<Option<Self>, SstError> {
        // Large history/value blocks retain the existing restart search.
        // Bound transient decompression as well as permanent metadata.
        if block.data.len() > 16 * 1024 {
            return Ok(None);
        }
        let mut pending: Vec<Pending> = Vec::new();
        let mut position = 0;
        let mut key_bytes = 0usize;
        for item in VerifiedBlock::from_cached(block.clone()).iter() {
            let (kind, seq, key, _value) = item?;
            key_bytes += key.len();
            if key_bytes > block.data.len() * 8 {
                return Ok(None);
            }
            if pending.last().is_some_and(|previous| {
                key < previous.key || (key == previous.key && seq >= previous.seq)
            }) {
                return Err(SstError::Corrupt("unordered block search entries".into()));
            }
            let header = block
                .data
                .get(position..position + 21)
                .ok_or_else(|| SstError::Corrupt("short block search header".into()))?;
            let non_shared = u32::from_le_bytes(header[5..9].try_into().unwrap()) as usize;
            let value_len = u32::from_le_bytes(header[9..13].try_into().unwrap()) as usize;
            let value = position + 21 + non_shared;
            pending.push(Pending {
                key,
                seq,
                kind,
                value,
                value_len,
            });
            position = value + value_len;
        }
        if pending.len() < 8 {
            return Ok(None);
        }
        let first = &pending[0].key;
        let last = &pending.last().unwrap().key;
        let prefix_len = first.iter().zip(last).take_while(|(a, b)| a == b).count();
        let suffix_len = key_bytes - prefix_len * pending.len();
        let bytes = std::mem::size_of::<Self>()
            + pending.len() * std::mem::size_of::<Entry>()
            + prefix_len
            + suffix_len;
        if bytes > block.data.len() / 2 {
            return Ok(None);
        }
        let prefix = first[..prefix_len].to_vec();
        let mut suffixes = Vec::with_capacity(suffix_len);
        let mut entries = Vec::with_capacity(pending.len());
        for entry in pending {
            // The bounded block and decompression sizes above fit u32.
            entries.push(Entry {
                suffix: suffixes.len() as u32,
                key_len: (entry.key.len() - prefix_len) as u32,
                value: entry.value as u32,
                value_len: entry.value_len as u32,
                kind: entry.kind,
                seq: entry.seq,
            });
            suffixes.extend_from_slice(&entry.key[prefix_len..]);
        }
        Ok(Some(Self {
            prefix,
            suffixes,
            entries,
        }))
    }

    pub(crate) fn get<'a>(
        &self,
        data: &'a [u8],
        key: &[u8],
        limit: Option<u64>,
    ) -> Option<(Kind, u64, &'a [u8])> {
        if !key.starts_with(&self.prefix) {
            return None;
        }
        let target = &key[self.prefix.len()..];
        let suffix = |entry: &Entry| {
            &self.suffixes[entry.suffix as usize..(entry.suffix + entry.key_len) as usize]
        };
        let from = self.entries.partition_point(|entry| suffix(entry) < target);
        for entry in &self.entries[from..] {
            if suffix(entry) != target {
                return None;
            }
            if limit.is_none_or(|limit| entry.seq <= limit) {
                return Some((
                    entry.kind,
                    entry.seq,
                    &data[entry.value as usize..(entry.value + entry.value_len) as usize],
                ));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiban::sstable::TableBuilder;

    #[test]
    fn packed_block_search_preserves_binary_keys_versions_and_tombstones() {
        let mut builder = TableBuilder::new();
        let mut expected = Vec::new();
        for i in 0..12u8 {
            let key = vec![b'k', i];
            for seq in (1..=3).rev() {
                let kind = if seq == 2 && i % 3 == 0 {
                    Kind::Tombstone
                } else {
                    Kind::Put
                };
                let value = if kind == Kind::Put {
                    vec![i; 64]
                } else {
                    Vec::new()
                };
                builder.add(kind, &key, &value, seq).unwrap();
                expected.push((kind, seq, key.clone(), value));
            }
        }
        let table = builder.finish().unwrap();
        let footer = table.len() - 44;
        let end = u64::from_le_bytes(table[footer + 20..footer + 28].try_into().unwrap()) as usize;
        let data: std::sync::Arc<[u8]> = table[..end].into();
        let block = CachedBlock {
            meta: VerifiedBlock::verify(&data).unwrap(),
            data,
        };
        let index = BlockIndex::build(&block).unwrap().unwrap();
        for key in [
            Vec::new(),
            vec![b'j'],
            vec![b'k'],
            vec![b'k', 255],
            vec![255],
        ]
        .into_iter()
        .chain(expected.iter().map(|entry| entry.2.clone()))
        {
            for limit in 0..=4 {
                let wanted = expected
                    .iter()
                    .find(|entry| entry.2 == key && entry.1 <= limit)
                    .map(|entry| (entry.0, entry.1, entry.3.as_slice()));
                assert_eq!(
                    index.get(&block.data, &key, Some(limit)),
                    wanted,
                    "key={key:?}, limit={limit}"
                );
            }
        }
    }
}
