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
        (12, 1, true, 10, Kind::Tombstone, b"".as_slice()),
        (13, 2, true, 12, Kind::Put, b"newest".as_slice()),
    ] {
        let mut builder = if number == 14 {
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
        TableBuilder::with_point_index(true),
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

/// Run the already-built release test executable under taskset -c 8 with
/// KIBAN_POINT_BENCH_N set. These measurements isolate flush and compaction
/// BUILD from memtable fill. CI's `--ignored` stress job runs without the
/// variable, so the benchmark is skipped there.
#[test]
#[ignore = "CPU-8 maintenance benchmark; set KIBAN_POINT_BENCH_N to run"]
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
            .any(|line| line == "Cpus_allowed_list:\t8")
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
