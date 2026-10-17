//! Isolated immutable-SST point-read experiment. No engine changes.
#[path = "../../src/model.rs"]
#[allow(dead_code)]
mod model;
#[path = "../../../../examples/support/random.rs"]
#[allow(dead_code)]
mod random;
use heed3::{Database, Env, EnvFlags, EnvOpenOptions, types::Bytes};
use kiban::{
    db::{Kiban, KibanOptions},
    manifest::Manifest,
    sstable::{Kind, VerifiedBlock},
};
use ptr_hash::{DefaultPtrHash, PtrHashParams};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashSet},
    fs::File,
    hint::black_box,
    io::{BufRead, Write},
    path::Path,
    time::Instant,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const N: u64 = 1_000_000;
const SEED: u64 = 20261016;
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
fn u32at(b: &[u8], p: usize) -> usize {
    u32::from_le_bytes(b[p..p + 4].try_into().unwrap()) as usize
}
fn u64at(b: &[u8], p: usize) -> usize {
    u64::from_le_bytes(b[p..p + 8].try_into().unwrap()) as usize
}
fn hash(key: &[u8]) -> (u64, u16) {
    let h = xxhash_rust::xxh3::xxh3_128(key);
    (h as u64, (h >> 64) as u16)
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Slot {
    block: u32,
    entry: u32,
    fingerprint: u16,
    reserved: u16,
}
struct Table {
    mphf: DefaultPtrHash,
    slots: Vec<Slot>,
    arena: Vec<u8>,
    first: Vec<u8>,
    last: Vec<u8>,
    level: u32,
}
struct Phash {
    levels: Vec<Vec<Table>>,
}
impl Phash {
    fn open(path: &Path) -> Result<Self> {
        let manifest = Manifest::load(path)?.ok_or("no manifest")?;
        let mut levels = BTreeMap::<u32, Vec<Table>>::new();
        let (
            mut total,
            mut mphf_bits,
            mut arena_bytes,
            mut original_bytes,
            mut build_ns,
            mut mphf_ns,
        ) = (0, 0., 0, 0, 0u128, 0u128);
        let mut layout = BTreeMap::<u32, usize>::new();
        for tr in manifest.tables.iter().rev() {
            let started = Instant::now();
            let f = File::open(path.join(format!("{}.sst", tr.number)))?;
            // Input is read-only for this experiment and never passed to the engine.
            let raw = unsafe { memmap2::Mmap::map(&f)? };
            original_bytes += raw.len();
            let footer = raw.len() - 44;
            assert_eq!(&raw[footer + 36..], b"KIBANSST");
            let mut pos = u64at(&raw, footer);
            let count = u32at(&raw, pos);
            pos += 4;
            let mut arena = Vec::with_capacity(raw.len() * 12 / 10);
            let (mut keys, mut pending, mut first, mut last) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for _ in 0..count {
                let len = u32at(&raw, pos);
                pos += 4 + len;
                let off = u64at(&raw, pos);
                let len = u64at(&raw, pos + 8);
                pos += 16;
                // CRC and structural verification are outside every timer.
                let block = VerifiedBlock::from_raw(raw[off..off + len].to_vec())?;
                let block_offset = u32::try_from(arena.len())?;
                for record in block.iter() {
                    let (kind, _, key, value) = record?;
                    assert_eq!(kind, Kind::Put, "only initial put-only datasets supported");
                    if first.is_empty() {
                        first = key.clone();
                    }
                    last = key.clone();
                    let (h, fp) = hash(&key);
                    keys.push(h);
                    pending.push(Slot {
                        block: block_offset,
                        entry: u32::try_from(arena.len() - block_offset as usize)?,
                        fingerprint: fp,
                        reserved: 0,
                    });
                    arena.extend_from_slice(&u16::try_from(key.len())?.to_le_bytes());
                    arena.extend_from_slice(&u16::try_from(value.len())?.to_le_bytes());
                    arena.extend_from_slice(&key);
                    arena.extend_from_slice(&value);
                }
            }
            assert_eq!(
                keys.iter().copied().collect::<HashSet<_>>().len(),
                keys.len(),
                "duplicate keys/hash collision"
            );
            let tick = Instant::now();
            let mphf = DefaultPtrHash::new(&keys, PtrHashParams::default());
            mphf_ns += tick.elapsed().as_nanos();
            let mut slots = vec![Slot::default(); keys.len()];
            let mut occupied = vec![false; keys.len()];
            for (h, slot) in keys.iter().zip(pending) {
                let i = mphf.index(h);
                assert!(!occupied[i]);
                occupied[i] = true;
                slots[i] = slot;
            }
            assert!(occupied.into_iter().all(|v| v));
            let (pilots, remap) = mphf.bits_per_element();
            mphf_bits += (pilots + remap) * keys.len() as f64;
            total += keys.len();
            arena_bytes += arena.len();
            build_ns += started.elapsed().as_nanos();
            *layout.entry(tr.level).or_default() += 1;
            levels.entry(tr.level).or_default().push(Table {
                mphf,
                slots,
                arena,
                first,
                last,
                level: tr.level,
            });
        }
        assert_eq!(total, N as usize, "expected disjoint initial records");
        for tables in levels.values_mut() {
            if tables[0].level > 0 {
                tables.sort_by(|a, b| a.first.cmp(&b.first));
                for p in tables.windows(2) {
                    assert!(p[0].last < p[1].first);
                }
            }
        }
        println!(
            "{}",
            json!({"type":"build", "keys":total,"layout":layout,"mphf_bits_per_key":mphf_bits/total as f64,"slot_bytes_per_key":std::mem::size_of::<Slot>(),"arena_bytes":arena_bytes,"original_sst_bytes":original_bytes,"build_ns":build_ns,"mphf_build_ns":mphf_ns,"threads":1})
        );
        Ok(Self {
            levels: levels.into_values().collect(),
        })
    }
    fn get(&self, key: &[u8]) -> Option<&[u8]> {
        let (h, fp) = hash(key);
        for tables in &self.levels {
            let candidates = if tables[0].level == 0 {
                tables.as_slice()
            } else {
                let at = tables.partition_point(|t| t.last.as_slice() < key);
                if at == tables.len() {
                    continue;
                }
                &tables[at..at + 1]
            };
            for t in candidates {
                if key < t.first.as_slice() || key > t.last.as_slice() {
                    continue;
                }
                let s = &t.slots[t.mphf.index(&h)];
                if s.fingerprint != fp {
                    continue;
                }
                let p = s.block as usize + s.entry as usize;
                let kl = u16::from_le_bytes(t.arena[p..p + 2].try_into().unwrap()) as usize;
                let vl = u16::from_le_bytes(t.arena[p + 2..p + 4].try_into().unwrap()) as usize;
                if &t.arena[p + 4..p + 4 + kl] == key {
                    return Some(&t.arena[p + 4 + kl..p + 4 + kl + vl]);
                }
            }
        }
        None
    }
}
struct Lmdb {
    env: Env,
    records: Database<Bytes, Bytes>,
}
impl Lmdb {
    fn open(path: &Path) -> Result<Self> {
        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(10 * 1024 * 1024 * 1024)
                .max_dbs(200)
                .max_readers(200)
                .flags(EnvFlags::READ_ONLY)
                .open(path)
                .map_err(|e| format!("LMDB open: {e:?}"))?
        };
        let txn = env.read_txn()?;
        let records = env
            .open_database(&txn, Some("memories"))?
            .ok_or("missing records")?;
        txn.commit()?;
        Ok(Self { env, records })
    }
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let txn = self.env.read_txn()?;
        let value = self.records.get(&txn, key)?.map(<[u8]>::to_vec);
        txn.commit()?;
        Ok(value)
    }
}
enum Backend {
    Lmdb(Lmdb),
    Kiban(Kiban),
    Mphf(Phash),
}
impl Backend {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        match self {
            Self::Lmdb(b) => b.get(key),
            Self::Kiban(b) => Ok(b.get(key)?),
            Self::Mphf(b) => Ok(b.get(key).map(<[u8]>::to_vec)),
        }
    }
}
fn control(command: &str) -> Result<()> {
    if let Some(path) = std::env::var_os("READBENCH_PERF_CTL") {
        let ack = File::open(std::env::var_os("READBENCH_PERF_ACK").ok_or("missing ack")?)?;
        let mut ctl = std::fs::OpenOptions::new().write(true).open(path)?;
        writeln!(ctl, "{command}")?;
        let mut reply = String::new();
        std::io::BufReader::new(ack).read_line(&mut reply)?;
        assert_eq!(reply.trim(), "ack");
    }
    Ok(())
}
fn verify(b: &Backend, miss: bool, ops: usize) -> Result<()> {
    for id in 0..N {
        let k = model::key(id);
        let actual = b.get(k.as_bytes())?.ok_or("missing hit")?;
        let expected = serde_json::to_vec(&model::Memory::new(id, 0, SEED))?;
        assert_eq!(actual, expected, "value mismatch {id}");
    }
    if miss {
        for op in 0..ops {
            let id = N + random::mix(SEED ^ random::mix(1) ^ op as u64) % N;
            assert!(b.get(model::key(id).as_bytes())?.is_none());
        }
    }
    Ok(())
}
fn reads(variant: &str, path: &Path, miss: bool, ops: usize) -> Result<()> {
    let b = match variant {
        "lmdb" => Backend::Lmdb(Lmdb::open(path)?),
        "kiban" => Backend::Kiban(Kiban::open_with_options(
            path,
            KibanOptions {
                block_cache_bytes: 512 * 1024 * 1024,
                ..KibanOptions::default()
            },
        )?),
        "mphf" => Backend::Mphf(Phash::open(path)?),
        _ => return Err("unknown variant".into()),
    };
    verify(&b, miss, ops)?;
    let ids: Vec<_> = (0..ops * 10)
        .map(|op| random::mix(SEED ^ random::mix(1) ^ op as u64) % N + if miss { N } else { 0 })
        .collect();
    let keys: Vec<_> = ids.iter().map(|&id| model::key(id)).collect();
    // Each identical sequence is warmed independently after full validation.
    for k in &keys {
        black_box(b.get(k.as_bytes())?);
    }
    let mut samples = Vec::with_capacity(ops);
    for k in keys.iter().take(ops) {
        let tick = Instant::now();
        let v = b.get(black_box(k.as_bytes()))?;
        black_box(&v);
        samples.push(tick.elapsed().as_nanos() as u64);
        drop(v);
    }
    let mut e2e = Vec::with_capacity(ops);
    for &id in ids.iter().take(ops) {
        let tick = Instant::now();
        let k = model::key(black_box(id));
        let v = b.get(k.as_bytes())?;
        let parsed = v
            .as_ref()
            .map(|v| serde_json::from_slice::<model::Memory>(v).unwrap());
        black_box(&parsed);
        e2e.push(tick.elapsed().as_nanos() as u64);
        drop(parsed);
    }
    // Perf gates exclude clocks, samples, setup, validation, and JSON parsing.
    control("enable")?;
    let tick = Instant::now();
    let mut digest = 0u64;
    for k in &keys {
        let v = b.get(black_box(k.as_bytes()))?;
        digest = digest.wrapping_add(black_box(v.as_ref()).map_or(0, |v| v.len() as u64));
        black_box(v);
    }
    let ns = tick.elapsed().as_nanos();
    control("disable")?;
    let stats = |s: &mut Vec<u64>| {
        let mean = s.iter().sum::<u64>() as f64 / s.len() as f64;
        s.sort_unstable();
        json!({"p50_ns":s[s.len()/2],"p99_ns":s[s.len()*99/100],"mean_ns":mean})
    };
    // Repeat the entire checked workload after timing, including full key checks in MPHF.
    for (&id, k) in ids.iter().zip(&keys) {
        let v = b.get(k.as_bytes())?;
        if miss {
            assert!(v.is_none());
        } else {
            assert_eq!(
                v.unwrap(),
                serde_json::to_vec(&model::Memory::new(id, 0, SEED))?
            );
        }
    }
    println!(
        "{}",
        json!({"type":"read","variant":variant,"missing":miss,"operations":ops,"counter_operations":ops*10,"raw":stats(&mut samples),"e2e":stats(&mut e2e),"throughput_ops_s":(ops*10) as f64*1e9/ns as f64,"batch_mean_ns":ns as f64/(ops*10) as f64,"digest":digest,"verified_all_hits":N,"verified_timed_results":ops*10,"errors":0})
    );
    Ok(())
}
#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct Node {
    next: usize,
    pad: [u8; 56],
}
fn floor(ops: usize) {
    let n = 512 * 1024 * 1024 / 64;
    let mut order: Vec<u32> = (0..n as u32).collect();
    let mut rng = SEED;
    for i in (1..n).rev() {
        rng = random::mix(rng);
        order.swap(i, rng as usize % (i + 1));
    }
    let mut nodes = vec![
        Node {
            next: 0,
            pad: [0; 56]
        };
        n
    ];
    for i in 0..n {
        nodes[order[i] as usize].next = order[(i + 1) % n] as usize;
    }
    let mut at = order[0] as usize;
    drop(order);
    for _ in 0..n {
        at = black_box(nodes[at].next);
    }
    for trial in 0..5 {
        for len in if trial % 2 == 0 {
            vec![1, 2, 3, 4, 6, 8]
        } else {
            vec![8, 6, 4, 3, 2, 1]
        } {
            let tick = Instant::now();
            for _ in 0..ops {
                for _ in 0..len {
                    at = unsafe { black_box(nodes.get_unchecked(at).next) };
                }
            }
            let ns = tick.elapsed().as_nanos();
            println!(
                "{}",
                json!({"type":"floor","trial":trial+1,"length":len,"operations":ops,"ns_per_chain":ns as f64/ops as f64,"digest":black_box(at),"bytes":n*64})
            );
        }
    }
}
fn main() -> Result<()> {
    assert_eq!(
        unsafe { libc::sched_getcpu() },
        8,
        "all runs must use taskset CPU 8"
    );
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("floor") {
        floor(args.get(2).map(|s| s.parse().unwrap()).unwrap_or(200000));
        return Ok(());
    }
    reads(
        &args[1],
        Path::new(&args[2]),
        args[3] == "miss",
        args.get(4).map(|s| s.parse().unwrap()).unwrap_or(100000),
    )
}
