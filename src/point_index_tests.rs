//! Real-engine tests of mixed SST formats, visibility and corruption handling.
use super::*;
use crate::sstable::builder::{FOOTER_LEN, POINT_FOOTER_EXTRA};
use crate::testutil::TempDir;
use std::collections::BTreeMap;

fn options(point_index: bool, cache: usize) -> KibanOptions {
    KibanOptions {
        point_index,
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
    if id == 0 {
        Vec::new()
    } else {
        [b"\0binary/".as_slice(), &id.to_be_bytes(), b"\xff"].concat()
    }
}

#[test]
fn point_index_randomized_mixed_formats_snapshots_and_compaction_match_model() {
    for cache in [0, 8 * 1024, 8 * 1024 * 1024] {
        let dir = TempDir::new("point-model");
        let mut model = BTreeMap::<Vec<u8>, Vec<u8>>::new();
        for phase in 0..6u64 {
            let mut db =
                Kiban::open_with_options(dir.path(), options(phase % 2 == 1, cache)).unwrap();
            let mut pinned = None;
            for op in 0..600u64 {
                let h = crate::sys::SplitMix64(phase * 600 + op).next();
                let k = key(h % 200);
                if h.is_multiple_of(7) {
                    db.delete(&k).unwrap();
                    model.remove(&k);
                } else {
                    let value = vec![op as u8; 20 + (h % 180) as usize];
                    db.put(&k, &value).unwrap();
                    model.insert(k, value);
                }
                if op % 73 == 0 {
                    db.flush().unwrap();
                }
                if op == 301 {
                    pinned = Some((db.snapshot(), model.clone()));
                }
            }
            db.flush().unwrap();
            for id in 0..250 {
                let k = key(id);
                assert_eq!(db.get(&k).unwrap(), model.get(&k).cloned());
            }
            assert_eq!(
                db.iter().map(Result::unwrap).collect::<Vec<_>>(),
                model.clone().into_iter().collect::<Vec<_>>()
            );
            let (snap, old) = pinned.take().unwrap();
            for id in 0..250 {
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
fn point_index_mixed_tables_resolve_by_sequence_across_overlapping_levels() {
    let dir = TempDir::new("point-levels");
    let mut db = Kiban::open_with_options(dir.path(), options(true, 1024 * 1024)).unwrap();
    let mut tables = Vec::new();
    for (number, level, indexed, seq, kind, value) in [
        (10, 0, true, 5, Kind::Put, b"stale".as_slice()),
        (11, 1, false, 8, Kind::Put, b"middle".as_slice()),
        (14, 1, true, 9, Kind::Put, b"legacy".as_slice()),
        (15, 2, true, 11, Kind::Put, b"flag-zero".as_slice()),
        (12, 1, true, 10, Kind::Tombstone, b"".as_slice()),
        (13, 2, true, 12, Kind::Put, b"newest".as_slice()),
    ] {
        let mut builder = if number == 15 {
            TableBuilder::with_point_index_without_value_crc()
        } else if number == 14 {
            TableBuilder::with_legacy_point_index()
        } else {
            TableBuilder::with_point_index(indexed)
        };
        builder.add(kind, b"key", value, seq).unwrap();
        let path = dir.path().join(format!("{number}.sst"));
        atomic::commit_file(&path, &builder.finish().unwrap()).unwrap();
        let table = SstTable::open(
            number,
            &path,
            db.cache.clone(),
            db.file_cache.clone(),
            false,
        )
        .unwrap();
        tables.push(StdArc::new(TableEntry {
            file_lifetime: StdArc::new(()),
            level,
            number,
            size: table.size_on_disk(),
            first_key: table.smallest_key().to_vec(),
            last_key: table.largest_key().to_vec(),
            table,
        }));
    }
    db.version = StdArc::new(Version::new(1, tables));
    for (sequence, expected) in [
        (4, None),
        (5, Some(b"stale".as_slice())),
        (8, Some(b"middle".as_slice())),
        (9, Some(b"legacy".as_slice())),
        (10, None),
        (11, Some(b"flag-zero".as_slice())),
        (12, Some(b"newest".as_slice())),
    ] {
        assert_eq!(
            get_from_version_at(&db.version, b"key", sequence, &db.read_amp).unwrap(),
            expected.map(<[u8]>::to_vec)
        );
    }
}

#[test]
fn point_index_every_section_and_footer_bit_flip_is_detected_with_or_without_admission() {
    let dir = TempDir::new("point-bit-flips");
    for mut builder in [
        TableBuilder::with_legacy_point_index(),
        TableBuilder::with_point_index_without_value_crc(),
        TableBuilder::with_point_index(true),
        TableBuilder::with_scan_index(true, 2),
    ] {
        builder.add(Kind::Put, b"a", b"one", 5).unwrap();
        builder.add(Kind::Tombstone, b"b", b"", 4).unwrap();
        let original = builder.finish().unwrap();
        let extra = original.len() - FOOTER_LEN - POINT_FOOTER_EXTRA;
        let start = u64::from_le_bytes(original[extra..extra + 8].try_into().unwrap()) as usize;
        let len = u64::from_le_bytes(original[extra + 8..extra + 16].try_into().unwrap()) as usize;
        let path = dir.path().join("1.sst");
        for capacity in [0, 1024 * 1024] {
            for at in (start..start + len).chain(extra..original.len()) {
                for bit in 0..8 {
                    let mut bytes = original.clone();
                    bytes[at] ^= 1 << bit;
                    std::fs::write(&path, bytes).unwrap();
                    let cache = StdArc::new(BlockCache::new(capacity));
                    let result = SstTable::open(
                        1,
                        &path,
                        cache.clone(),
                        StdArc::new(TableFileCache::new(1)),
                        false,
                    );
                    assert!(
                        matches!(result, Err(crate::sstable::SstError::Corrupt(_))),
                        "accepted bit {at}/{bit}, capacity {capacity}"
                    );
                    assert_eq!(cache.stats().point_index_bytes, 0);
                }
            }
        }
    }
}

#[test]
fn point_index_presence_is_independent_of_writer_switch_and_cache_admission() {
    let dir = TempDir::new("point-switch");
    for indexed in [false, true] {
        let mut db = Kiban::open_with_options(dir.path(), options(indexed, 1024 * 1024)).unwrap();
        db.put(if indexed { b"new" } else { b"old" }, b"value")
            .unwrap();
        db.flush_without_compaction().unwrap();
        db.close().unwrap();
    }
    for cache in [0, 1024 * 1024] {
        let db = Kiban::open_with_options(dir.path(), options(false, cache)).unwrap();
        assert_eq!(db.get(b"old").unwrap(), Some(b"value".to_vec()));
        assert_eq!(db.get(b"new").unwrap(), Some(b"value".to_vec()));
        assert_eq!(db.cache.stats().point_index_bytes > 0, cache > 0);
    }
}

#[test]
fn indexed_scans_share_backing_blocks_without_duplicate_cache_admission() {
    let dir = TempDir::new("point-shared-blocks");
    let opts = options(true, 8 * 1024 * 1024);
    let mut db = Kiban::open_with_options(dir.path(), opts.clone()).unwrap();
    let expected: Vec<_> = (0..500u64).map(|i| (key(i), vec![i as u8; 300])).collect();
    for (key, value) in &expected {
        db.put(key, value).unwrap();
    }
    db.flush_without_compaction().unwrap();
    db.close().unwrap();
    let db = Kiban::open_with_options(dir.path(), opts).unwrap();
    let before = db.cache.stats();
    assert!(before.point_index_bytes > 0);
    for (key, value) in &expected {
        assert_eq!(db.get(key).unwrap().as_ref(), Some(value));
    }
    assert_eq!(db.iter().collect::<Result<Vec<_>, _>>().unwrap(), expected);
    let after = db.cache.stats();
    assert_eq!(after.resident_bytes, before.resident_bytes);
    assert_eq!(after.resident_entries, 0);
    assert_eq!(after.misses, before.misses);
}

// Diagnostic only: keep this outside the measured wall-clock intervals.
fn maintenance_cpu_ticks() -> u64 {
    let stat = std::fs::read_to_string("/proc/thread-self/stat").unwrap();
    let mut fields = stat.rsplit_once(')').unwrap().1.split_whitespace();
    let user: u64 = fields.nth(11).unwrap().parse().unwrap();
    user + fields.next().unwrap().parse::<u64>().unwrap()
}

fn maintenance_perf_control(command: &str) {
    if let Some(path) = std::env::var_os("READBENCH_PERF_CTL") {
        use std::io::{BufRead, Write};
        let ack = std::fs::File::open(std::env::var_os("READBENCH_PERF_ACK").unwrap()).unwrap();
        let mut ctl = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        writeln!(ctl, "{command}").unwrap();
        let mut reply = String::new();
        std::io::BufReader::new(ack).read_line(&mut reply).unwrap();
        assert_eq!(reply.trim(), "ack");
    }
}

/// Run the already-built release test executable under taskset -c 3 with
/// KIBAN_POINT_BENCH_N set. These measurements isolate flush and compaction
/// BUILD from memtable fill. CI's `--ignored` stress job runs without the
/// variable, so the benchmark is skipped there.
#[test]
#[ignore = "CPU-3 maintenance benchmark; set KIBAN_POINT_BENCH_N to run"]
fn point_index_maintenance_measurement() {
    let Ok(n) = std::env::var("KIBAN_POINT_BENCH_N") else {
        eprintln!("skipping: KIBAN_POINT_BENCH_N is unset");
        return;
    };
    let n: u64 = n.parse().unwrap();
    let profile = std::env::var_os("KIBAN_POINT_BENCH_PROFILE").is_some();
    let selected_trial = std::env::var("KIBAN_POINT_BENCH_TRIAL")
        .ok()
        .map(|trial| trial.parse::<usize>().unwrap());
    assert!(selected_trial.is_none_or(|trial| (1..=5).contains(&trial)));
    assert!(
        std::fs::read_to_string("/proc/self/status")
            .unwrap()
            .lines()
            .any(|line| line == "Cpus_allowed_list:\t3")
    );
    for trial in 0..5 {
        if selected_trial.is_some_and(|selected| selected != trial + 1) {
            continue;
        }
        for indexed in if trial % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let dir = TempDir::new("point-maintenance");
            let mut opts = options(indexed, 512 * 1024 * 1024);
            if std::env::var_os("KIBAN_SCAN_BENCH").is_some() {
                opts.point_index = true;
                opts.scan_prefix_len = if indexed { 2 } else { 0 };
            }
            opts.binary_fuse_filter = std::env::var_os("KIBAN_FUSE_BENCH").is_some();
            opts.l0_compaction_trigger = 100;
            opts.l0_write_stall_trigger = 200;
            opts.target_file_size = 4 * 1024 * 1024;
            let mut db = Kiban::open_with_options(dir.path(), opts).unwrap();
            let mut flush_ns = 0;
            for part in 0..2 {
                for id in part * (n / 2)..(part + 1) * (n / 2) {
                    db.put(id.to_be_bytes(), vec![id as u8; 296]).unwrap();
                }
                let cpu = profile.then(maintenance_cpu_ticks);
                let tick = std::time::Instant::now();
                db.flush_without_compaction().unwrap();
                let elapsed = tick.elapsed().as_nanos();
                flush_ns += elapsed;
                if let Some(cpu) = cpu {
                    println!(
                        "{{\"type\":\"profile_phase\",\"phase\":\"flush\",\"trial\":{},\"indexed\":{},\"cpu_ticks\":{},\"wall_ns\":{}}}",
                        trial + 1,
                        indexed,
                        maintenance_cpu_ticks() - cpu,
                        elapsed
                    );
                }
            }
            let input_bytes: u64 = db.version.tables.iter().map(|t| t.size).sum();
            let plan = db.plan_compaction_at_level(0).unwrap();
            let cpu = profile.then(maintenance_cpu_ticks);
            if profile {
                maintenance_perf_control("enable");
            }
            let tick = std::time::Instant::now();
            let outputs = plan.build().unwrap();
            let build_ns = tick.elapsed().as_nanos();
            if profile {
                maintenance_perf_control("disable");
            }
            if let Some(cpu) = cpu {
                println!(
                    "{{\"type\":\"profile_phase\",\"phase\":\"build\",\"trial\":{},\"indexed\":{},\"cpu_ticks\":{},\"wall_ns\":{}}}",
                    trial + 1,
                    indexed,
                    maintenance_cpu_ticks() - cpu,
                    build_ns
                );
            }
            let output_bytes: u64 = outputs.iter().map(|t| t.size).sum();
            let output_files = outputs.len();
            let cpu = profile.then(maintenance_cpu_ticks);
            let tick = std::time::Instant::now();
            db.commit_compaction(plan, outputs).unwrap();
            let commit_ns = tick.elapsed().as_nanos();
            if let Some(cpu) = cpu {
                println!(
                    "{{\"type\":\"profile_phase\",\"phase\":\"commit\",\"trial\":{},\"indexed\":{},\"cpu_ticks\":{},\"wall_ns\":{}}}",
                    trial + 1,
                    indexed,
                    maintenance_cpu_ticks() - cpu,
                    commit_ns
                );
            }
            for id in 0..n {
                assert_eq!(
                    db.get(id.to_be_bytes()).unwrap().unwrap(),
                    vec![id as u8; 296]
                );
            }
            println!(
                "{{\"type\":\"maintenance\",\"trial\":{},\"point_index\":{},\"n\":{},\"flush_ns\":{},\"compaction_build_ns\":{},\"compaction_commit_ns\":{},\"input_bytes\":{},\"output_bytes\":{},\"output_files\":{},\"point_resident_bytes\":{},\"verified_records\":{},\"errors\":0}}",
                trial + 1,
                indexed,
                n,
                flush_ns,
                build_ns,
                commit_ns,
                input_bytes,
                output_bytes,
                output_files,
                db.cache.stats().point_index_bytes,
                n
            );
        }
    }
}

#[test]
fn point_index_conversion_reserves_outputs_for_full_key_expansion() {
    let dir = TempDir::new("point-expansion");
    let mut opts = options(false, 8 * 1024 * 1024);
    opts.l0_compaction_trigger = 100;
    opts.l0_write_stall_trigger = 200;
    let mut db = Kiban::open_with_options(dir.path(), opts.clone()).unwrap();
    for id in 0u64..16384 {
        let mut key = vec![b'a'; 512];
        key.extend_from_slice(&id.to_be_bytes());
        db.put(key, id.to_be_bytes()).unwrap();
    }
    db.flush_without_compaction().unwrap();
    let old_bound =
        db.version.tables.iter().map(|t| t.size).sum::<u64>() / opts.target_file_size + 8;
    db.close().unwrap();
    opts.point_index = true;
    let mut db = Kiban::open_with_options(dir.path(), opts).unwrap();
    let plan = db.plan_compaction_at_level(0).unwrap();
    let outputs = plan.build().unwrap();
    assert!(
        outputs.len() as u64 > old_bound,
        "fixture must exceed the old format-preserving bound"
    );
    db.commit_compaction(plan, outputs).unwrap();
    for id in 0u64..16384 {
        let mut key = vec![b'a'; 512];
        key.extend_from_slice(&id.to_be_bytes());
        assert_eq!(db.get(key).unwrap().unwrap(), id.to_be_bytes());
    }
    assert_eq!(db.iter().count(), 16384);
}

#[test]
fn remix_ranges_match_model_across_versions_memtables_and_bounds() {
    for cache in [0, 8192, 32 * 1024 * 1024] {
        let dir = TempDir::new("remix-model");
        let mut opts = options(true, cache);
        opts.scan_prefix_len = 2;
        opts.l0_compaction_trigger = 20;
        opts.l0_write_stall_trigger = 200;
        let mut db = Kiban::open_with_options(dir.path(), opts.clone()).unwrap();
        let mut model = BTreeMap::new();
        let mut snapshots = Vec::new();
        for round in 0..6u64 {
            for op in 0..400u64 {
                let h = crate::sys::SplitMix64(round * 400 + op).next();
                let mut key = vec![(h % 5) as u8, (h / 5 % 3) as u8];
                key.extend_from_slice(&(h % 130).to_be_bytes());
                if h.is_multiple_of(11) {
                    key.resize(90, 42);
                }
                if h.is_multiple_of(47) {
                    key.truncate((h % 2) as usize);
                }
                if h.is_multiple_of(7) {
                    db.delete(&key).unwrap();
                    model.remove(&key);
                } else {
                    let v = h.to_le_bytes().to_vec();
                    db.put(&key, &v).unwrap();
                    model.insert(key, v);
                }
            }
            snapshots.push((db.snapshot(), model.clone()));
            if round % 2 == 0 {
                db.freeze().unwrap();
            } else {
                db.flush().unwrap();
            }
            db.prepare_range_index();
            for a in 0..6 {
                for b in 0..4 {
                    let prefix = [a, b];
                    let expected = model
                        .iter()
                        .filter(|(key, _)| key.starts_with(&prefix))
                        .take(50)
                        .map(|(_, value)| value.clone())
                        .collect::<Vec<_>>();
                    assert_eq!(
                        db.prefix_values(&prefix, 50, |v| Ok::<_, DbError>(v.to_vec()))
                            .unwrap(),
                        expected
                    );
                }
            }
            for (snapshot, _) in &snapshots {
                snapshot
                    .version
                    .scan
                    .get_or_init(|| crate::remix::Remix::build(&snapshot.version));
            }
            for probe in 0..350u64 {
                let h = crate::sys::SplitMix64(probe + round * 350).next();
                let prefix = vec![(h % 6) as u8, (h / 6 % 4) as u8];
                let mut start = prefix.clone();
                if !h.is_multiple_of(3) {
                    start.extend_from_slice(&(h % 150).to_be_bytes());
                }
                let mut end = prefix;
                if h.is_multiple_of(4) {
                    end.extend_from_slice(&(h / 3 % 150).to_be_bytes());
                } else {
                    *end.last_mut().unwrap() += 1;
                }
                if h.is_multiple_of(9) {
                    end = vec![7];
                }
                let count = (h % 40) as usize;
                let expected = if start < end {
                    model
                        .range(start.clone()..end.clone())
                        .take(count)
                        .map(|(_, v)| v.clone())
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let actual = db
                    .range_values(&start, &end, count, |v| Ok::<_, DbError>(v.to_vec()))
                    .unwrap();
                assert_eq!(
                    actual, expected,
                    "round {round} start {start:?} end {end:?}"
                );
                for (snapshot, old) in &snapshots {
                    let expected = if start < end {
                        old.range(start.clone()..end.clone())
                            .take(count)
                            .map(|(_, v)| v.clone())
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    assert_eq!(
                        snapshot
                            .range_values(&start, &end, count, |v| Ok::<_, DbError>(v.to_vec()))
                            .unwrap(),
                        expected
                    );
                }
            }
            if let Some(plan) = db.plan_flush() {
                let output = plan.build().unwrap();
                db.commit_flush(plan, output).unwrap();
            }
        }
        db.flush().unwrap();
        if cache == 32 * 1024 * 1024 {
            assert!(
                db.version
                    .scan
                    .get_or_init(|| crate::remix::Remix::build(&db.version))
                    .is_some()
            );
        }
        drop(snapshots);
        db.close().unwrap();
        // Writers can be switched off without changing how persisted sections read.
        opts.scan_prefix_len = 0;
        let db = Kiban::open_with_options(dir.path(), opts).unwrap();
        let actual = db.iter().collect::<Result<BTreeMap<_, _>, _>>().unwrap();
        for (key, value) in &model {
            assert_eq!(
                actual.get(key),
                Some(value),
                "reopen cache={cache} key={key:?}"
            );
        }
        assert_eq!(actual.len(), model.len(), "reopen cache={cache}");
    }
}

#[test]
fn remix_mixed_formats_and_cache_reservations() {
    let dir = TempDir::new("remix-mixed");
    for round in 0..4 {
        let mut opts = options(round != 0, 32 * 1024 * 1024);
        opts.scan_prefix_len = if round == 3 { 2 } else { 0 };
        opts.l0_compaction_trigger = 100;
        opts.l0_write_stall_trigger = 200;
        let mut db = Kiban::open_with_options(dir.path(), opts).unwrap();
        for id in 0..128u64 {
            db.put([b"ab".as_slice(), &id.to_be_bytes()].concat(), [round])
                .unwrap();
        }
        db.flush_without_compaction().unwrap();
        let expected = db
            .range(b"ab", b"ac")
            .map(|r| r.unwrap().1)
            .collect::<Vec<_>>();
        assert_eq!(
            db.range_values(b"ab", b"ac", 1000, |v| Ok::<_, DbError>(v.to_vec()))
                .unwrap(),
            expected
        );
        db.close().unwrap();
    }
}

#[test]
fn remix_scan_corruption_and_checksummed_bad_locators_are_rejected() {
    let mut builder = TableBuilder::with_scan_index(true, 2);
    for id in 0..70u64 {
        builder
            .add(
                Kind::Put,
                &[b"ab".as_slice(), &id.to_be_bytes()].concat(),
                b"value",
                id + 1,
            )
            .unwrap();
    }
    let original = builder.finish().unwrap();
    let footer = original.len() - FOOTER_LEN - POINT_FOOTER_EXTRA;
    let section = u64::from_le_bytes(original[footer..footer + 8].try_into().unwrap()) as usize;
    let len = u64::from_le_bytes(original[footer + 8..footer + 16].try_into().unwrap()) as usize;
    let extension_len = u64::from_le_bytes(
        original[section + len - 13..section + len - 5]
            .try_into()
            .unwrap(),
    ) as usize;
    let extension = section + len - 13 - extension_len;
    for repair_crc in [false, true] {
        for at in extension..section + len {
            let mut bytes = original.clone();
            bytes[at] ^= 0x80;
            if repair_crc {
                let crc = crate::crc32::crc32(&bytes[section..section + len - 4]);
                bytes[section + len - 4..section + len].copy_from_slice(&crc.to_le_bytes());
                if at >= section + len - 5 {
                    continue;
                }
            }
            let dir = TempDir::new("remix-corrupt");
            let path = dir.path().join("1.sst");
            std::fs::write(&path, bytes).unwrap();
            let cache = StdArc::new(BlockCache::new(32 * 1024 * 1024));
            let files = StdArc::new(TableFileCache::new(4));
            assert!(
                SstTable::open(1, &path, cache.clone(), files, false).is_err(),
                "accepted offset {at} repaired {repair_crc}"
            );
            assert_eq!(cache.stats().point_index_bytes, 0);
        }
    }
}

#[test]
fn remix_duplicate_anchor_boundaries_and_ph3_anchor_groups() {
    let dir = TempDir::new("remix-anchors");
    let cache = StdArc::new(BlockCache::new(64 * 1024 * 1024));
    let files = StdArc::new(TableFileCache::new(4));
    let mut tables = Vec::new();
    let mut histories = BTreeMap::<Vec<u8>, Vec<(u64, Kind, Vec<u8>)>>::new();
    for run in 0..3u64 {
        let mut builder = TableBuilder::with_scan_index(true, 2);
        for id in 0..2300u64 {
            let mut key = [b"ab".as_slice(), &id.to_be_bytes()].concat();
            if id.is_multiple_of(31) {
                key.resize(95, 255);
            }
            for v in (0..3u64).rev() {
                let seq = 1 + run + 3 * v;
                let kind = if (id + seq).is_multiple_of(13) {
                    Kind::Tombstone
                } else {
                    Kind::Put
                };
                let value = if kind == Kind::Put {
                    vec![seq as u8, id as u8]
                } else {
                    Vec::new()
                };
                builder.add(kind, &key, &value, seq).unwrap();
                histories
                    .entry(key.clone())
                    .or_default()
                    .push((seq, kind, value));
            }
        }
        let path = dir.path().join(file_name(run + 1, SST_EXTENSION));
        std::fs::write(&path, builder.finish().unwrap()).unwrap();
        let table = SstTable::open(run + 1, &path, cache.clone(), files.clone(), false).unwrap();
        tables.push(StdArc::new(TableEntry {
            file_lifetime: StdArc::new(()),
            level: run as u32,
            number: run + 1,
            size: table.size_on_disk(),
            first_key: table.smallest_key().to_vec(),
            last_key: table.largest_key().to_vec(),
            table,
        }));
    }
    let version = StdArc::new(Version::new(0, tables));
    let before = cache.stats().point_index_bytes;
    assert!(crate::remix::Remix::build_cancellable(&version, || true).is_none());
    assert_eq!(cache.stats().point_index_bytes, before);
    assert!(
        version
            .scan
            .get_or_init(|| crate::remix::Remix::build(&version))
            .is_some()
    );
    let resident = cache.stats().point_index_bytes;
    for seq in 0..=10 {
        let snapshot = Snapshot {
            lease: None,
            seq,
            memtable: StdArc::new(Memtable::new()),
            immutable: None,
            version: version.clone(),
        };
        let model = histories
            .iter()
            .filter_map(|(key, vs)| {
                let (_, kind, v) = vs
                    .iter()
                    .filter(|(s, _, _)| *s <= seq)
                    .max_by_key(|(s, _, _)| s)?;
                (*kind == Kind::Put).then(|| (key.clone(), v.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        for probe in 0..400u64 {
            let id = crate::sys::SplitMix64(probe).next() % 2400;
            let start = [b"ab".as_slice(), &id.to_be_bytes()].concat();
            let end = if probe.is_multiple_of(2) {
                b"ac".to_vec()
            } else {
                [b"ab".as_slice(), &(id + 50).to_be_bytes()].concat()
            };
            let expected = model
                .range(start.clone()..end.clone())
                .take(50)
                .map(|(_, v)| v.clone())
                .collect::<Vec<_>>();
            assert_eq!(
                snapshot
                    .range_values(&start, &end, 50, |v| Ok::<_, DbError>(v.to_vec()))
                    .unwrap(),
                expected
            );
        }
        assert_eq!(cache.stats().point_index_bytes, resident);
    }
    drop(version);
    assert_eq!(cache.stats().point_index_bytes, 0);
}

#[test]
fn remix_memtable_handoff_respects_remaining_source_span() {
    let dir = TempDir::new("remix-short-span");
    let mut opts = options(true, 32 * 1024 * 1024);
    opts.scan_prefix_len = 2;
    opts.l0_compaction_trigger = 100;
    opts.l0_write_stall_trigger = 200;
    let mut db = Kiban::open_with_options(dir.path(), opts).unwrap();
    for id in (0..5u8).chain(10..15) {
        db.put([b'a', b'b', id], [id]).unwrap();
    }
    db.flush_without_compaction().unwrap();
    for id in 5..10u8 {
        db.put([b'a', b'b', id], [id]).unwrap();
    }
    db.flush_without_compaction().unwrap();
    db.put([b'a', b'b', 0], [100]).unwrap();
    assert!(db.prepare_range_index());
    let expected = std::iter::once(vec![100])
        .chain((1..10u8).map(|id| vec![id]))
        .collect::<Vec<_>>();
    assert_eq!(
        db.range_values(b"ab", b"ac", 10, |v| Ok::<_, DbError>(v.to_vec()))
            .unwrap(),
        expected
    );
}

#[test]
fn remix_prefix_values_cover_binary_short_long_empty_and_memtable_bounds() {
    for cache in [0, 32 * 1024 * 1024] {
        let dir = TempDir::new("remix-prefix-values");
        let mut opts = options(true, cache);
        opts.scan_prefix_len = 2;
        opts.l0_compaction_trigger = 100;
        opts.l0_write_stall_trigger = 200;
        let mut db = Kiban::open_with_options(dir.path(), opts).unwrap();
        let keys = [
            Vec::new(),
            b"a".to_vec(),
            b"ab".to_vec(),
            b"ab0".to_vec(),
            b"ab1".to_vec(),
            b"ac0".to_vec(),
            vec![255],
            vec![255, 255],
            vec![255, 255, 0],
            vec![255; 80],
        ];
        for (i, key) in keys.iter().enumerate() {
            db.put(key, [i as u8]).unwrap();
        }
        db.flush_without_compaction().unwrap();
        db.put(b"ab0", b"new").unwrap();
        db.delete(b"ab1").unwrap();
        db.put(b"ab2", b"mem").unwrap();
        db.put(b"ad0", b"mem-only-prefix").unwrap();
        db.put([255, 255, 1], b"binary-mem").unwrap();
        db.delete([255, 255, 0]).unwrap();
        db.prepare_range_index();
        for prefix in [
            Vec::new(),
            b"a".to_vec(),
            b"ab".to_vec(),
            b"ab0".to_vec(),
            b"ad".to_vec(),
            b"missing".to_vec(),
            vec![255],
            vec![255, 255],
            vec![255; 80],
        ] {
            for count in [0, 1, 2, 10, 100] {
                let expected = db
                    .iter()
                    .filter_map(|r| {
                        let (key, value) = r.unwrap();
                        key.starts_with(&prefix).then_some(value)
                    })
                    .take(count)
                    .collect::<Vec<_>>();
                assert_eq!(
                    db.prefix_values(&prefix, count, |v| Ok::<_, DbError>(v.to_vec()))
                        .unwrap(),
                    expected
                );
            }
        }
    }
}

#[test]
fn remix_memtable_histories_emit_one_visible_value_per_key() {
    let dir = TempDir::new("remix-mem-histories");
    let mut opts = options(true, 32 * 1024 * 1024);
    opts.scan_prefix_len = 2;
    let mut db = Kiban::open_with_options(dir.path(), opts).unwrap();
    db.put(b"ab0", b"sst").unwrap();
    db.flush_without_compaction().unwrap();
    assert!(db.prepare_range_index());
    db.memtable.set_snapshot_bounds(1, 100);
    db.put(b"ab0", b"old").unwrap();
    let seq = db.last_sequence;
    db.put(b"ab0", b"new").unwrap();
    db.put(b"ab1", b"deleted").unwrap();
    db.delete(b"ab1").unwrap();
    db.put(b"ab2", b"last").unwrap();
    let values = db
        .range_values(b"ab", b"ac", 100, |v| Ok::<_, DbError>(v.to_vec()))
        .unwrap();
    assert_eq!(values, vec![b"new".to_vec(), b"last".to_vec()]);
    let snapshot = Snapshot {
        lease: None,
        seq,
        memtable: StdArc::new(db.memtable.clone()),
        immutable: None,
        version: db.version.clone(),
    };
    assert_eq!(
        snapshot
            .range_values(b"ab", b"ac", 100, |v| Ok::<_, DbError>(v.to_vec()))
            .unwrap(),
        vec![b"old".to_vec()]
    );
}

/// PH3 tables carry the per-value CRC (flag bit 0) and the scan extension
/// (flag bit 1) together. Both resident modes (full backing with the scan
/// directory, and cache-resident locators without it) must agree with a model.
#[test]
fn ph3_value_crc_and_scan_extension_together_match_model_and_detect_corruption() {
    for (cache, scan_expected) in [(32 * 1024 * 1024, true), (192 * 1024, false)] {
        let dir = TempDir::new("ph3-crc-scan");
        let mut opts = options(true, cache);
        opts.scan_prefix_len = 2;
        opts.l0_compaction_trigger = 100;
        opts.l0_write_stall_trigger = 200;
        let mut db = Kiban::open_with_options(dir.path(), opts.clone()).unwrap();
        let mut model = BTreeMap::<Vec<u8>, Vec<u8>>::new();
        let agent = |a: u8, id: u64| [&[b'a', b'0' + a][..], &id.to_be_bytes()].concat();
        for a in 0..3u8 {
            for id in 0..400u64 {
                let value = vec![a ^ id as u8; 150];
                db.put(agent(a, id), value.clone()).unwrap();
                model.insert(agent(a, id), value);
            }
        }
        db.flush_without_compaction().unwrap();
        let snapshot = db.snapshot();
        let old = model.clone();
        for id in (0..400u64).step_by(3) {
            db.delete(agent(1, id)).unwrap();
            model.remove(&agent(1, id));
            let value = vec![0xee; 150];
            db.put(agent(2, id), value.clone()).unwrap();
            model.insert(agent(2, id), value);
        }
        db.flush_without_compaction().unwrap();
        let resident = db
            .version
            .scan
            .get_or_init(|| crate::remix::Remix::build(&db.version));
        assert_eq!(resident.is_some(), scan_expected, "cache {cache}");
        assert!(db.cache.stats().point_index_bytes > 0);
        for (k, v) in &model {
            assert_eq!(db.get(k).unwrap().as_deref(), Some(v.as_slice()));
        }
        assert_eq!(db.get(agent(1, 0)).unwrap(), None);
        for a in 0..3u8 {
            let prefix = vec![b'a', b'0' + a];
            for count in [1, 10, 500] {
                let expected = |m: &BTreeMap<Vec<u8>, Vec<u8>>| {
                    m.range(prefix.clone()..)
                        .take_while(|(k, _)| k.starts_with(&prefix))
                        .take(count)
                        .map(|(_, v)| v.clone())
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    db.prefix_values(&prefix, count, |v| Ok::<_, DbError>(v.to_vec()))
                        .unwrap(),
                    expected(&model)
                );
                assert_eq!(
                    snapshot
                        .range_values(&prefix, &[b'a', b'1' + a], count, |v| {
                            Ok::<_, DbError>(v.to_vec())
                        })
                        .unwrap(),
                    expected(&old)
                );
            }
        }
        drop(snapshot);
        db.close().unwrap();

        // Corrupting a stored value fails open (admission checks every block
        // and every per-value CRC) in both resident modes.
        let sst = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == SST_EXTENSION))
            .min()
            .unwrap();
        let mut bytes = std::fs::read(&sst).unwrap();
        bytes[200] ^= 0x40;
        std::fs::write(&sst, bytes).unwrap();
        assert!(
            Kiban::open_with_options(dir.path(), opts).is_err(),
            "cache {cache}"
        );
    }
}
