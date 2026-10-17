use crate::{Options, Result, json, model, perf_control, random};
use kiban::{
    bloom::BloomFilter,
    cache::{BlockCache, CachedBlock},
    db::{Kiban, KibanOptions},
    memtable::Memtable,
    sstable::VerifiedBlock,
};
use std::{
    hash::{Hash, Hasher},
    hint::black_box,
    sync::{Arc, RwLock},
    time::Instant,
};

fn u64_at(bytes: &[u8], at: usize) -> usize {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize
}
fn u32_at(bytes: &[u8], at: usize) -> usize {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
}

struct Index {
    separator: Vec<u8>,
    offset: usize,
    len: usize,
}

impl AsRef<[u8]> for Index {
    fn as_ref(&self) -> &[u8] {
        &self.separator
    }
}

pub(super) fn run(o: &Options) -> Result<()> {
    if matches!(
        o.micro.as_str(),
        "iter-construct" | "iter-first" | "iter-ten"
    ) {
        return iter(o);
    }
    let c = &o.suite;
    let manifest = kiban::manifest::Manifest::load(&c.db)?.ok_or("missing manifest")?;
    if manifest.tables.len() != 1 {
        return Err("micro requires the single bulk SST".into());
    }
    let number = manifest.tables[0].number;
    let table = std::fs::read(c.db.join(format!("{number}.sst")))?;
    let footer = table.len() - 44;
    let index_at = u64_at(&table, footer);
    let filter_at = u64_at(&table, footer + 20);
    let filter_len = u64_at(&table, footer + 28);
    let filter = BloomFilter::decode(&table[filter_at..filter_at + filter_len - 5])
        .ok_or("invalid bloom")?;
    let count = u32_at(&table, index_at);
    let mut pos = index_at + 4;
    let mut index = Vec::with_capacity(count);
    for _ in 0..count {
        let len = u32_at(&table, pos);
        pos += 4;
        let separator = table[pos..pos + len].to_vec();
        pos += len;
        let offset = u64_at(&table, pos);
        let len = u64_at(&table, pos + 8);
        pos += 16;
        index.push(Index {
            separator,
            offset,
            len,
        });
    }
    if o.micro == "crc-hardware" {
        for e in &index {
            let end = e.offset + e.len;
            assert_eq!(
                crc32fast::hash(&table[e.offset..end - 4]),
                u32::from_le_bytes(table[end - 4..end].try_into().unwrap())
            );
        }
    }
    let cache = BlockCache::new(512 << 20);
    let mut blocks = Vec::with_capacity(index.len());
    for e in &index {
        let data: Arc<[u8]> = table[e.offset..e.offset + e.len].into();
        let block = CachedBlock {
            meta: VerifiedBlock::verify(&data)?,
            data,
        };
        cache.insert((number, e.offset as u64), block.clone());
        blocks.push(VerifiedBlock::from_cached(block));
    }
    let ids: Vec<_> = (0..100000)
        .map(|i| random::mix(c.seed ^ random::mix(1) ^ i) % c.n)
        .collect();
    let keys: Vec<_> = ids.iter().map(|&id| model::key(id).into_bytes()).collect();
    let positions: Vec<_> = keys
        .iter()
        .map(|k| index.partition_point(|e| e.separator.as_slice() < k))
        .collect();
    let values: Vec<_> = keys
        .iter()
        .zip(&positions)
        .map(|(k, &i)| blocks[i].get(k, None).unwrap().unwrap().value.to_vec())
        .collect();
    let hints = (o.micro == "index-hints").then(|| crate::index_hints::IndexHints::new(&index));
    let inline = (o.micro == "index-inline").then(|| {
        index
            .iter()
            .map(|e| {
                crate::inline_index::InlineEntry::new(&e.separator, e.offset as u64, e.len as u64)
            })
            .collect::<Vec<_>>()
    });
    let mut mem = Memtable::default();
    for (i, (key, value)) in keys.iter().zip(&values).take(10000).enumerate() {
        mem.put(key, value, i as u64 + 1);
    }
    let lock = RwLock::new(());
    let arc: Arc<[u8]> = vec![0; 4096].into();
    perf_control("enable")?;
    let started = Instant::now();
    for op in 0..c.operations {
        let at = (op as usize) % keys.len();
        let key = &keys[at];
        let i = positions[at];
        match o.micro.as_str() {
            "loop" => {
                black_box(key);
            }
            "memtable-miss" => {
                black_box(mem.entry(black_box(b"absent")));
            }
            "memtable-hit" => {
                black_box(mem.entry(black_box(&keys[at % 10000])));
            }
            "bloom" => {
                assert!(black_box(filter.may_contain(black_box(key))));
            }
            "index" => {
                black_box(index.partition_point(|e| e.separator.as_slice() < black_box(key)));
            }
            "index-hints" => {
                black_box(
                    hints
                        .as_ref()
                        .unwrap()
                        .partition_point(&index, black_box(key)),
                );
            }
            "index-inline" => {
                let entries = inline.as_ref().unwrap();
                let at = entries.partition_point(|e| e.as_ref() < black_box(key));
                black_box((entries[at].offset, entries[at].len));
            }
            "hash" => {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (number, index[i].offset as u64).hash(&mut h);
                black_box(h.finish());
            }
            "lock" => {
                drop(black_box(lock.read().unwrap()));
            }
            "arc" => {
                black_box(arc.clone());
            }
            "cache" => {
                black_box(
                    cache
                        .get(black_box(&(number, index[i].offset as u64)))
                        .unwrap(),
                );
            }
            "block-seek" => {
                black_box(blocks[i].get(black_box(key), None)?.ok_or("block miss")?);
            }
            "block-linear" => {
                black_box(
                    blocks[i]
                        .iter()
                        .find(|item| item.as_ref().is_ok_and(|e| e.2 == *key))
                        .ok_or("linear miss")??,
                );
            }
            "verify-block" => {
                let e = &index[i];
                black_box(VerifiedBlock::verify(&table[e.offset..e.offset + e.len])?);
            }
            "crc-scalar" | "crc-hardware" => {
                let e = &index[i];
                let data = &table[e.offset..e.offset + e.len - 4];
                if o.micro == "crc-scalar" {
                    black_box(kiban::crc32::crc32(black_box(data)));
                } else {
                    black_box(crc32fast::hash(black_box(data)));
                }
            }
            "copy" => {
                black_box(black_box(&values[at]).to_vec());
            }
            "decode" => {
                black_box(serde_json::from_slice::<model::Memory>(black_box(
                    &values[at],
                ))?);
            }
            _ => return Err("unknown microbench".into()),
        }
    }
    let elapsed = started.elapsed().as_nanos();
    perf_control("disable")?;
    println!(
        "{}",
        json!({"type":"micro","name":o.micro,"operations":c.operations,"ns":elapsed,"ns_per_op":elapsed as f64/c.operations as f64,"errors":0})
    );
    Ok(())
}

fn iter(o: &Options) -> Result<()> {
    let c = &o.suite;
    let db = Kiban::open_with_options(
        &c.db,
        KibanOptions {
            block_cache_bytes: 512 << 20,
            ..KibanOptions::default()
        },
    )?;
    for id in 0..c.n {
        black_box(db.get(model::key(id))?.ok_or("missing warmup value")?);
    }
    let ranges: Vec<_> = (0..64)
        .map(|a| {
            (
                format!("agent/{a:04}/memory/"),
                format!("agent/{a:04}/memory0"),
            )
        })
        .collect();
    perf_control("enable")?;
    let started = Instant::now();
    for op in 0..c.operations {
        let (start, end) = &ranges[(op % 64) as usize];
        if o.micro == "iter-construct" {
            drop(black_box(db.range(start.as_bytes(), end.as_bytes())));
        } else {
            for entry in db
                .range(start.as_bytes(), end.as_bytes())
                .take(if o.micro == "iter-first" { 1 } else { 10 })
            {
                black_box(entry?);
            }
        }
    }
    let elapsed = started.elapsed().as_nanos();
    perf_control("disable")?;
    println!(
        "{}",
        json!({"type":"micro","name":o.micro,"layout":o.layout,"operations":c.operations,"ns":elapsed,"ns_per_op":elapsed as f64/c.operations as f64,"errors":0})
    );
    Ok(())
}
