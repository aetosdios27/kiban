//! Read-path equivalence: indexed point lookups, per-level concatenated
//! scans with end-bound pruning, restart seeking and the range read-ahead
//! tier must return exactly what a reference model holds.
use super::*;
use crate::testutil::TempDir;
use std::collections::BTreeMap;

fn options(range_cache_bytes: usize) -> KibanOptions {
    KibanOptions {
        l0_compaction_trigger: 2,
        l0_write_stall_trigger: 20,
        base_level_bytes: 48 * 1024,
        level_multiplier: 4,
        // Several 4 KiB blocks per table, so read-ahead spans blocks.
        target_file_size: 24 * 1024,
        // Far below the data size: most block reads miss.
        block_cache_bytes: 16 * 1024,
        range_cache_bytes,
        range_readahead_bytes: 32 * 1024,
        ..KibanOptions::default()
    }
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % bound
    }
}

fn key(i: u64) -> Vec<u8> {
    format!("k{i:06}").into_bytes()
}

/// A probe key: either a stored key or one sorting strictly between two.
fn probe(rng: &mut Rng, space: u64) -> Vec<u8> {
    let mut k = key(rng.below(space + 2));
    if rng.below(3) == 0 {
        k.push(b'~');
    }
    k
}

fn expected_range(
    model: &BTreeMap<Vec<u8>, Vec<u8>>,
    start: &[u8],
    end: &[u8],
) -> Vec<(Vec<u8>, Vec<u8>)> {
    if start >= end {
        return Vec::new();
    }
    model
        .range(start.to_vec()..end.to_vec())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[test]
fn randomized_reads_match_reference_with_and_without_range_cache() {
    for range_cache_bytes in [0, 96 * 1024] {
        let dir = TempDir::new("read-path-equivalence");
        let mut db = Kiban::open_with_options(dir.path(), options(range_cache_bytes)).unwrap();
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ range_cache_bytes as u64);
        let mut model: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        let space = 3000;
        let mut pinned = None;
        for op in 0..12_000u64 {
            let k = key(rng.below(space));
            if rng.below(10) == 0 {
                db.delete(&k).unwrap();
                model.remove(&k);
            } else {
                let len = 20 + rng.below(160) as usize;
                let v = vec![b'a' + (op % 26) as u8; len];
                db.put(&k, &v).unwrap();
                model.insert(k, v);
            }
            if op % 400 == 399 {
                db.flush().unwrap();
            }
            if op == 6_000 {
                pinned = Some((db.snapshot(), model.clone()));
            }
        }
        // Leave recent writes in the memtable too.
        let leveled: std::collections::BTreeSet<u32> = db
            .version
            .tables
            .iter()
            .filter(|t| t.level >= 1)
            .map(|t| t.level)
            .collect();
        assert!(leveled.len() >= 2, "need several levels, got {leveled:?}");
        assert!(
            db.version.tables.iter().any(|t| t.size > 12 * 1024),
            "need multi-block tables"
        );

        for _ in 0..1500 {
            let k = probe(&mut rng, space);
            assert_eq!(db.get(&k).unwrap(), model.get(&k).cloned(), "get {k:?}");
        }
        for _ in 0..400 {
            let start = probe(&mut rng, space);
            let end = probe(&mut rng, space);
            let got: Vec<_> = db.range(&start, &end).map(Result::unwrap).collect();
            assert_eq!(
                got,
                expected_range(&model, &start, &end),
                "range {start:?}..{end:?}"
            );
        }
        let all: Vec<_> = db.iter().map(Result::unwrap).collect();
        let want: Vec<_> = model.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        assert_eq!(all, want);

        let (snap, snap_model) = pinned.take().unwrap();
        for _ in 0..500 {
            let k = probe(&mut rng, space);
            assert_eq!(db.get_at(&snap, &k).unwrap(), snap_model.get(&k).cloned());
        }
        let scanned = db.scan_at(&snap).unwrap();
        let want: Vec<_> = snap_model
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        assert_eq!(scanned, want);
        drop(snap);

        let stats = db.cache.range_cache().stats();
        if range_cache_bytes > 0 {
            assert!(stats.hits > 0, "read-ahead never served a block: {stats:?}");
            assert!(stats.source_bytes > stats.source_reads * 4096);
        } else {
            assert_eq!(stats, crate::range_cache::RangeCacheStats::default());
        }
        db.sync().unwrap();
        drop(db);

        // Shared snapshots use the same sources through a different handle.
        let shared =
            SharedKiban::open_with_options(dir.path(), options(range_cache_bytes)).unwrap();
        let snap = shared.snapshot().unwrap();
        for _ in 0..300 {
            let start = probe(&mut rng, space);
            let end = probe(&mut rng, space);
            let got: Vec<_> = snap.range(&start, &end).map(Result::unwrap).collect();
            assert_eq!(got, expected_range(&model, &start, &end));
            let k = probe(&mut rng, space);
            assert_eq!(shared.get(&k).unwrap(), model.get(&k).cloned());
        }
    }
}

fn table(dir: &TempDir, db: &Kiban, number: u64, level: u32, keys: &[(&[u8], u64)]) -> TableEntry {
    let mut builder = TableBuilder::new();
    for (k, seq) in keys {
        builder.add(Kind::Put, k, &seq.to_be_bytes(), *seq).unwrap();
    }
    let path = dir.path().join(file_name(number, SST_EXTENSION));
    atomic::commit_file(&path, &builder.finish().unwrap()).unwrap();
    let table = SstTable::open(
        number,
        &path,
        db.cache.clone(),
        db.file_cache.clone(),
        false,
    )
    .unwrap();
    TableEntry {
        file_lifetime: StdArc::new(()),
        number,
        level,
        size: table.size_on_disk(),
        first_key: table.smallest_key().to_vec(),
        last_key: table.largest_key().to_vec(),
        table,
    }
}

/// The level index must never trust a binary search over a level whose
/// tables overlap: lookups and scans fall back to every candidate table.
#[test]
fn overlapping_level_falls_back_to_probing_every_table() {
    let dir = TempDir::new("overlapping-level-fallback");
    let mut db = Kiban::open(dir.path()).unwrap();
    let tables = vec![
        StdArc::new(table(&dir, &db, 10, 1, &[(b"a", 1), (b"m", 2), (b"q", 3)])),
        StdArc::new(table(&dir, &db, 11, 1, &[(b"f", 4), (b"m", 5), (b"z", 6)])),
        StdArc::new(table(&dir, &db, 12, 2, &[(b"b", 7), (b"c", 8)])),
    ];
    db.version = StdArc::new(Version::new(1, tables));
    assert!(!db.version.levels[0].disjoint);
    assert!(db.version.levels[1].disjoint);
    db.last_sequence = 8;
    let get = |k: &[u8]| {
        db.get(k)
            .unwrap()
            .map(|v| u64::from_be_bytes(v.try_into().unwrap()))
    };
    assert_eq!(get(b"m"), Some(5));
    assert_eq!(get(b"q"), Some(3));
    assert_eq!(get(b"f"), Some(4));
    assert_eq!(get(b"c"), Some(8));
    assert_eq!(get(b"n"), None);
    let keys: Vec<Vec<u8>> = db.range(b"b", b"r").map(|r| r.unwrap().0).collect();
    let want: Vec<&[u8]> = vec![b"b", b"c", b"f", b"m", b"q"];
    assert_eq!(keys, want);
}

/// Tables entirely outside `[start, end)` are never opened by a range read.
#[test]
fn range_reads_skip_tables_outside_the_bounds() {
    let dir = TempDir::new("range-pruning");
    let mut db = Kiban::open(dir.path()).unwrap();
    let tables = vec![
        StdArc::new(table(&dir, &db, 20, 1, &[(b"a", 1), (b"b", 2)])),
        StdArc::new(table(&dir, &db, 21, 1, &[(b"c", 3), (b"d", 4)])),
        StdArc::new(table(&dir, &db, 22, 1, &[(b"e", 5), (b"f", 6)])),
    ];
    db.version = StdArc::new(Version::new(1, tables));
    db.last_sequence = 6;
    let (l0, leveled) = db.version.overlapping(b"c", Some(b"d"));
    assert!(l0.is_empty());
    let numbers: Vec<u64> = leveled[0].1.iter().map(|t| t.number).collect();
    assert_eq!(numbers, vec![21]);
    let (_, leveled) = db.version.overlapping(b"bb", None);
    let numbers: Vec<u64> = leveled[0].1.iter().map(|t| t.number).collect();
    assert_eq!(numbers, vec![21, 22]);
    let keys: Vec<Vec<u8>> = db.range(b"b", b"e").map(|r| r.unwrap().0).collect();
    assert_eq!(keys, vec![b"b".to_vec(), b"c".to_vec(), b"d".to_vec()]);
}

/// A sequential scan over a cold table issues far fewer positional reads
/// than it reads blocks, and a second scan after block-cache eviction is
/// served from the range tier.
#[test]
fn read_ahead_coalesces_block_reads_and_survives_block_eviction() {
    let dir = TempDir::new("read-ahead-coalescing");
    let mut builder = TableBuilder::new();
    let value = [b'v'; 100];
    for i in 0..2000u64 {
        builder.add(Kind::Put, &key(i), &value, i + 1).unwrap();
    }
    let path = dir.path().join("1.sst");
    atomic::commit_file(&path, &builder.finish().unwrap()).unwrap();
    let cache = StdArc::new(BlockCache::with_range_cache(8 * 1024, 1 << 20, 64 * 1024));
    let files = StdArc::new(TableFileCache::new(4));
    let table = SstTable::open(1, &path, cache.clone(), files, false).unwrap();
    let blocks = table.size_on_disk() / 4096;
    assert!(blocks >= 40);

    let count = |t: &SstTable| t.iter().map(Result::unwrap).count();
    assert_eq!(count(&table), 2000);
    let first = cache.range_cache().stats();
    assert!(
        first.source_reads * 4 < blocks,
        "{} reads for ~{blocks} blocks",
        first.source_reads
    );
    assert_eq!(count(&table), 2000);
    let second = cache.range_cache().stats();
    // Only blocks never admitted to the tier are re-read: the block-cached
    // first block `SstTable::open` loaded, the iterator's two single-block
    // reads (so short scans read nothing extra) and a final block that no
    // following block could extend into a window.
    assert!(
        second.source_reads - first.source_reads <= 4,
        "rescan re-read the file: {first:?} -> {second:?}"
    );
    assert!(second.hits > first.hits);

    // Seeking into the middle decodes from the right restart interval.
    let from: Vec<_> = table
        .iter_from(&key(1234))
        .take(3)
        .map(|r| r.unwrap().2)
        .collect();
    assert_eq!(from, vec![key(1234), key(1235), key(1236)]);
    let mut between = key(1234);
    between.push(b'~');
    let first_after = table.iter_from(&between).next().unwrap().unwrap().2;
    assert_eq!(first_after, key(1235));
}
