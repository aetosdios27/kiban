//! Compare prefix-compressed keys without reconstructing a temporary key.
use kiban::{
    cache::{BlockMeta, CachedBlock},
    sstable::{Kind, SstError},
};
use std::cmp::Ordering;
const ENTRY_FIXED_LEN: usize = 21;

type Match<'a> = (Kind, u64, &'a [u8]);

pub(crate) fn get<'a>(
    block: &'a CachedBlock,
    target: &[u8],
    limit: Option<u64>,
) -> Result<Option<Match<'a>>, SstError> {
    let data = &block.data;
    let meta = &block.meta;
    let start = seek_restart_index(data, meta, target)?;
    let mut pos = restart_offset_of(data, meta.restart_start, start);
    let mut previous_len = 0;
    let mut matched = 0;
    let mut ordering = Ordering::Equal;
    while pos < meta.entries_end {
        let (kind, seq, shared, non_shared, value_len) = header_at(data, meta, pos)?;
        if shared > previous_len {
            return Err(SstError::Corrupt(
                "shared length exceeds previous key".into(),
            ));
        }
        if shared <= matched {
            let suffix = &data[pos + ENTRY_FIXED_LEN..pos + ENTRY_FIXED_LEN + non_shared];
            let remaining = &target[shared..];
            let common = suffix
                .iter()
                .zip(remaining)
                .take_while(|(a, b)| a == b)
                .count();
            ordering = suffix.get(common).cmp(&remaining.get(common));
            matched = shared + common;
        }
        previous_len = shared + non_shared;
        if ordering == Ordering::Greater {
            return Ok(None);
        }
        if ordering == Ordering::Equal && limit.is_none_or(|limit| seq <= limit) {
            return Ok(Some((
                kind,
                seq,
                entry_value(data, pos, kind, non_shared, value_len),
            )));
        }
        pos += ENTRY_FIXED_LEN + non_shared + value_len;
    }
    Ok(None)
}

fn header_at(
    data: &[u8],
    meta: &BlockMeta,
    pos: usize,
) -> Result<(Kind, u64, usize, usize, usize), SstError> {
    let bad = |m: String| SstError::Corrupt(format!("data block entry at {pos}: {m}"));
    if pos
        .checked_add(ENTRY_FIXED_LEN)
        .is_none_or(|end| end > meta.entries_end)
    {
        return Err(bad("header runs past entries area".to_string()));
    }
    let kind = Kind::from_u8(data[pos]).ok_or_else(|| bad("unknown entry kind".to_string()))?;
    let seq = u64::from_le_bytes(data[pos + 13..pos + 21].try_into().unwrap());
    let shared = u32::from_le_bytes(data[pos + 1..pos + 5].try_into().unwrap()) as usize;
    let non_shared = u32::from_le_bytes(data[pos + 5..pos + 9].try_into().unwrap()) as usize;
    let value_len = u32::from_le_bytes(data[pos + 9..pos + 13].try_into().unwrap()) as usize;
    if kind == Kind::Tombstone && value_len != 0 {
        return Err(bad("tombstone with non-zero value length".to_string()));
    }
    let end = pos
        .checked_add(ENTRY_FIXED_LEN)
        .and_then(|n| n.checked_add(non_shared))
        .and_then(|n| n.checked_add(value_len));
    if end.is_none_or(|end| end > meta.entries_end) {
        return Err(bad("key/value runs past entries area".to_string()));
    }
    Ok((kind, seq, shared, non_shared, value_len))
}

fn restart_offset_of(data: &[u8], restart_start: usize, i: usize) -> usize {
    u32::from_le_bytes(
        data[restart_start + i * 4..restart_start + (i + 1) * 4]
            .try_into()
            .unwrap(),
    ) as usize
}

fn full_key_at_restart<'a>(
    data: &'a [u8],
    meta: &BlockMeta,
    i: usize,
) -> Result<&'a [u8], SstError> {
    let pos = restart_offset_of(data, meta.restart_start, i);
    let (_, _, shared, non_shared, _) = header_at(data, meta, pos)?;
    if shared != 0 {
        return Err(SstError::Corrupt(
            "restart point entry has shared bytes".to_string(),
        ));
    }
    Ok(&data[pos + ENTRY_FIXED_LEN..pos + ENTRY_FIXED_LEN + non_shared])
}

/// Restart interval where a forward search for `target` must begin: the one
/// before the FIRST restart whose key is >= `target`. Equal-key versions can
/// begin in the preceding interval and extend across many restarts;
/// starting at the last equal restart would skip newer visible versions
/// (including tombstones).
fn seek_restart_index(data: &[u8], meta: &BlockMeta, target: &[u8]) -> Result<usize, SstError> {
    let mut lo = 0usize;
    let mut hi = meta.num_restarts;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if full_key_at_restart(data, meta, mid)? < target {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    Ok(lo.saturating_sub(1))
}

fn entry_value(data: &[u8], pos: usize, kind: Kind, non_shared: usize, value_len: usize) -> &[u8] {
    match kind {
        Kind::Put => {
            let vstart = pos + ENTRY_FIXED_LEN + non_shared;
            &data[vstart..vstart + value_len]
        }
        Kind::Tombstone => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiban::sstable::{TableBuilder, VerifiedBlock};

    #[test]
    fn comparison_matches_restart_decoder_for_binary_keys_and_history() {
        let keys = [vec![], vec![0], vec![0, 0], vec![0, 255], vec![1, 0, 0]];
        let mut builder = TableBuilder::new();
        for key in &keys {
            for seq in (1..=20).rev() {
                let kind = if seq % 5 == 0 {
                    Kind::Tombstone
                } else {
                    Kind::Put
                };
                builder
                    .add(kind, key, if kind == Kind::Put { b"abc" } else { b"" }, seq)
                    .unwrap();
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
        let original = VerifiedBlock::from_cached(block.clone());
        for key in keys
            .into_iter()
            .chain([vec![0, 0, 0], vec![0, 1], vec![1], vec![255]])
        {
            for limit in (0..=21).map(Some).chain([None]) {
                let expected = original
                    .get(&key, limit)
                    .unwrap()
                    .map(|m| (m.kind, m.seq, m.value));
                assert_eq!(
                    get(&block, &key, limit).unwrap(),
                    expected,
                    "key={key:?}, limit={limit:?}"
                );
            }
        }
    }
}
