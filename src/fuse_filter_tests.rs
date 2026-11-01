//! Filter formats through real engine publication, visibility and recovery.
use super::*;
use crate::sstable::builder::FOOTER_LEN;
use crate::testutil::TempDir;
use std::collections::BTreeMap;

fn options(fuse: bool, indexed: bool, cache: usize) -> KibanOptions {
    KibanOptions {
        binary_fuse_filter: fuse,
        point_index: indexed,
        block_cache_bytes: cache,
        target_file_size: 16 * 1024,
        base_level_bytes: 32 * 1024,
        level_multiplier: 3,
        l0_compaction_trigger: 2,
        l0_write_stall_trigger: 20,
        ..KibanOptions::default()
    }
}
fn key(id: u64) -> Vec<u8> {
    match id % 4 {
        0 => id.to_be_bytes().to_vec(),
        1 => [b"agent/shared/prefix/".as_slice(), &id.to_be_bytes()].concat(),
        2 => [vec![0xff; 1024], id.to_be_bytes().to_vec()].concat(),
        _ if id == 3 => vec![],
        _ => [b"\0\xff".as_slice(), &id.to_le_bytes()].concat(),
    }
}
#[test]
fn fuse_mixed_formats_versions_tombstones_snapshots_compaction_match_unfiltered_scan() {
    for cache in [0, 8192, 8 * 1024 * 1024] {
        let dir = TempDir::new("fuse-mixed");
        let mut model = BTreeMap::new();
        for phase in 0..8u64 {
            let mut opts = options(phase & 1 != 0, phase & 2 != 0, cache);
            opts.scan_prefix_len = if opts.point_index && phase & 4 != 0 {
                2
            } else {
                0
            };
            let mut db = Kiban::open_with_options(dir.path(), opts).unwrap();
            let mut pinned = None;
            for op in 0..400u64 {
                let h = crate::sys::SplitMix64(phase * 400 + op).next();
                let k = key(h % 100);
                if h.is_multiple_of(5) {
                    db.delete(&k).unwrap();
                    model.remove(&k);
                } else {
                    let v = vec![op as u8; 20 + (h % 180) as usize];
                    db.put(&k, &v).unwrap();
                    model.insert(k, v);
                }
                if op % 67 == 0 {
                    db.flush().unwrap();
                }
                if op == 201 {
                    pinned = Some((db.snapshot(), model.clone()));
                }
            }
            db.flush().unwrap();
            for id in 0..150 {
                let k = key(id);
                assert_eq!(db.get(&k).unwrap(), model.get(&k).cloned());
            }
            assert_eq!(
                db.iter().map(Result::unwrap).collect::<Vec<_>>(),
                model.clone().into_iter().collect::<Vec<_>>()
            );
            let (snap, old) = pinned.unwrap();
            for id in 0..150 {
                let k = key(id);
                assert_eq!(db.get_at(&snap, &k).unwrap(), old.get(&k).cloned());
            }
            assert_eq!(
                db.scan_at(&snap).unwrap(),
                old.into_iter().collect::<Vec<_>>()
            );
            assert!(db.cache.resident_bytes() <= cache);
            drop(snap);
            db.close().unwrap();
        }
    }
}

#[test]
fn fuse_filter_every_header_payload_and_trailer_bit_flip_is_detected_before_lookup() {
    let dir = TempDir::new("fuse-corrupt");
    for indexed in [false, true] {
        let mut b = TableBuilder::with_point_index(indexed).with_filter(true);
        for id in 0u64..40 {
            b.add(Kind::Put, &id.to_be_bytes(), b"value", 1).unwrap();
        }
        let good = b.finish().unwrap();
        let footer = good.len() - FOOTER_LEN;
        let offset =
            u64::from_le_bytes(good[footer + 20..footer + 28].try_into().unwrap()) as usize;
        let len = u64::from_le_bytes(good[footer + 28..footer + 36].try_into().unwrap()) as usize;
        assert_eq!(
            u32::from_le_bytes(good[footer + 16..footer + 20].try_into().unwrap()),
            if indexed { 6 } else { 5 }
        );
        for cache in [0, 8 * 1024 * 1024] {
            for byte in offset..offset + len {
                for bit in 0..8 {
                    let mut bad = good.clone();
                    bad[byte] ^= 1 << bit;
                    let path = dir.path().join("1.sst");
                    std::fs::write(&path, &bad).unwrap();
                    assert!(
                        SstTable::open(
                            1,
                            &path,
                            StdArc::new(BlockCache::new(cache)),
                            StdArc::new(TableFileCache::new(10)),
                            false
                        )
                        .is_err(),
                        "index={indexed} cache={cache} byte={byte} bit={bit}"
                    );
                }
            }
            // Structurally bad fields with a valid CRC must be rejected too.
            for at in [8, 9, 10, 11, 12, 32, 36, 40, 44] {
                let mut bad = good.clone();
                bad[offset + at] ^= 0x80;
                let crc = crate::crc32::crc32(&bad[offset..offset + len - 4]);
                bad[offset + len - 4..offset + len].copy_from_slice(&crc.to_le_bytes());
                let path = dir.path().join("2.sst");
                std::fs::write(&path, &bad).unwrap();
                assert!(
                    SstTable::open(
                        2,
                        &path,
                        StdArc::new(BlockCache::new(cache)),
                        StdArc::new(TableFileCache::new(10)),
                        false
                    )
                    .is_err()
                );
            }
            for cut in offset..offset + len {
                let path = dir.path().join("3.sst");
                std::fs::write(&path, &good[..cut]).unwrap();
                assert!(
                    SstTable::open(
                        3,
                        &path,
                        StdArc::new(BlockCache::new(cache)),
                        StdArc::new(TableFileCache::new(10)),
                        false
                    )
                    .is_err()
                );
            }
        }
    }
}

/// A table written with BOTH `with_point_index(true)` and `with_filter(true)`
/// consults the fuse filter before its resident point index (`SstTable::get`).
/// The filter's "absent" answer is trusted — the peeling construction has zero
/// false negatives and the builder verifies the published filter against every
/// source key — so an absent key must come back as not-found through this
/// shortcut, while every present version, tombstone and snapshot-visible
/// version must still be answered by the unchanged point-index path.
#[test]
fn fuse_absent_answers_short_circuit_resident_point_index_and_keep_versioned_visibility() {
    let dir = TempDir::new("fuse-before-point");
    let mut b = TableBuilder::with_point_index(true).with_filter(true);
    b.add(Kind::Put, b"alive", b"new", 8).unwrap();
    b.add(Kind::Put, b"alive", b"old", 2).unwrap();
    b.add(Kind::Tombstone, b"gone", b"", 6).unwrap();
    let good = b.finish().unwrap();
    let footer = good.len() - FOOTER_LEN;
    // Footer version 6 selects the fuse+point layout; the filter block header
    // must be revision 1, fuse8 (algorithm 2).
    assert_eq!(
        u32::from_le_bytes(good[footer + 16..footer + 20].try_into().unwrap()),
        6
    );
    let offset = u64::from_le_bytes(good[footer + 20..footer + 28].try_into().unwrap()) as usize;
    assert_eq!(&good[offset..offset + 8], b"KIBANFL2");
    assert_eq!(&good[offset + 8..offset + 12], &[1, 2, 1, 8]);
    let path = dir.path().join("1.sst");
    std::fs::write(&path, &good).unwrap();
    // A cache well above the whole table: the point index stays resident.
    let table = SstTable::open(
        1,
        &path,
        StdArc::new(BlockCache::new(8 * 1024 * 1024)),
        StdArc::new(TableFileCache::new(10)),
        false,
    )
    .unwrap();
    // Absent keys: not-found through the filter-first shortcut, at every
    // snapshot limit the table could ever expose.
    for absent in [&b"absent"[..], b"zz", &[0xff, 0xff], b"yy"] {
        for limit in [None, Some(0), Some(u64::MAX)] {
            assert!(
                table.get(absent, limit).unwrap().is_none(),
                "absent key {absent:?} limit={limit:?}"
            );
        }
    }
    // Present paths are the unchanged point-index behavior.
    assert_eq!(table.get(b"alive", None).unwrap().unwrap().value, b"new");
    assert_eq!(table.get(b"alive", Some(8)).unwrap().unwrap().value, b"new");
    assert_eq!(table.get(b"alive", Some(7)).unwrap().unwrap().value, b"old");
    assert_eq!(table.get(b"alive", Some(2)).unwrap().unwrap().value, b"old");
    assert!(table.get(b"alive", Some(1)).unwrap().is_none());
    let tomb = table.get(b"gone", None).unwrap().unwrap();
    assert_eq!(tomb.kind, Kind::Tombstone);
    assert_eq!(tomb.seq, 6);
    assert!(tomb.value.is_empty());
    assert!(table.get(b"gone", Some(5)).unwrap().is_none());
}

#[test]
fn fuse_single_key_and_duplicate_versions_preserve_sequence_and_tombstone() {
    let dir = TempDir::new("fuse-single");
    for indexed in [false, true] {
        let mut b = TableBuilder::with_point_index(indexed).with_filter(true);
        b.add(Kind::Tombstone, b"", b"", 9).unwrap();
        b.add(Kind::Put, b"", b"previous", 8).unwrap();
        b.add(Kind::Put, b"", b"old", 1).unwrap();
        let path = dir.path().join("1.sst");
        std::fs::write(&path, b.finish().unwrap()).unwrap();
        let table = SstTable::open(
            1,
            &path,
            StdArc::new(BlockCache::new(0)),
            StdArc::new(TableFileCache::new(10)),
            false,
        )
        .unwrap();
        assert_eq!(
            table.get(b"", Some(9)).unwrap().unwrap().kind,
            Kind::Tombstone
        );
        assert_eq!(table.get(b"", Some(8)).unwrap().unwrap().value, b"previous");
        assert_eq!(table.get(b"", Some(1)).unwrap().unwrap().value, b"old");
        assert!(table.get(b"", Some(0)).unwrap().is_none());
    }
}

#[test]
fn fuse_flush_and_compaction_recover_after_every_publication_syscall_failure() {
    use super::crash_sweep_tests::{
        assert_band, run_scenario_with_faults_and_options, sweep_options,
    };
    for indexed in [false, true] {
        let opts = KibanOptions {
            binary_fuse_filter: true,
            point_index: indexed,
            ..sweep_options()
        };
        let clean_dir = TempDir::new("fuse-crash-clean");
        let clean = run_scenario_with_faults_and_options(clean_dir.path(), &[], opts.clone());
        assert!(!clean.failed());
        assert!(clean.ops > 20);
        for n in 0..clean.ops {
            let dir = TempDir::new("fuse-crash");
            let outcome = run_scenario_with_faults_and_options(dir.path(), &[n], opts.clone());
            let db = Kiban::open_with_options(dir.path(), opts.clone()).unwrap();
            let recovered: BTreeMap<_, _> = db.iter().map(Result::unwrap).collect();
            assert_band("fuse publication", &[n], &recovered, &outcome.tracker);
            for (key, value) in recovered {
                assert_eq!(db.get(key).unwrap(), Some(value), "syscall={n}");
            }
        }
    }
}

#[test]
fn fuse_point_section_and_footer_bit_flips_are_detected_for_every_point_flavour() {
    let dir = TempDir::new("fuse-point-flips");
    let path = dir.path().join("1.sst");
    for make in [
        TableBuilder::with_legacy_point_index as fn() -> TableBuilder,
        TableBuilder::with_point_index_without_value_crc,
        || TableBuilder::with_point_index(true),
        || TableBuilder::with_scan_index(true, 2),
    ] {
        let mut b = make().with_filter(true);
        b.add(Kind::Put, b"a", b"one", 5).unwrap();
        b.add(Kind::Tombstone, b"b", b"", 4).unwrap();
        let good = b.finish().unwrap();
        let extra = good.len() - FOOTER_LEN - crate::sstable::builder::POINT_FOOTER_EXTRA;
        let start = u64::from_le_bytes(good[extra..extra + 8].try_into().unwrap()) as usize;
        let len = u64::from_le_bytes(good[extra + 8..extra + 16].try_into().unwrap()) as usize;
        for capacity in [0, 1 << 20] {
            for at in (start..start + len).chain(extra..good.len()) {
                for bit in 0..8 {
                    let mut bad = good.clone();
                    bad[at] ^= 1 << bit;
                    std::fs::write(&path, bad).unwrap();
                    let opened = SstTable::open(
                        1,
                        &path,
                        StdArc::new(BlockCache::new(capacity)),
                        StdArc::new(TableFileCache::new(2)),
                        false,
                    );
                    assert!(
                        opened.is_err(),
                        "accepted byte {at} bit {bit} cap {capacity}"
                    );
                }
            }
        }
    }
}
