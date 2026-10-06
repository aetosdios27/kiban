use kiban::db::{Kiban, KibanOptions};
use kiban::{manifest::Manifest, sys};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;

pub type Model = BTreeMap<Vec<u8>, Vec<u8>>;
pub type Allowed = BTreeMap<Vec<u8>, Vec<Option<Vec<u8>>>>;
pub fn options() -> KibanOptions {
    KibanOptions {
        write_buffer_bytes: 4096,
        target_file_size: 4096,
        base_level_bytes: 16_384,
        level_multiplier: 4,
        block_cache_bytes: 64 * 1024,
        l0_compaction_trigger: 2,
        l0_write_stall_trigger: 6,
        ..KibanOptions::default()
    }
}
pub fn apply(model: &mut Model, key: Vec<u8>, value: Option<Vec<u8>>) {
    if let Some(v) = value {
        model.insert(key, v);
    } else {
        model.remove(&key);
    }
}
pub fn mutation(seed: u64, id: u64, keys: u64) -> (Vec<u8>, Option<Vec<u8>>) {
    let r = super::workload::mix(seed ^ super::workload::mix(id));
    let key = (r % keys).to_be_bytes().to_vec();
    let value = if r.is_multiple_of(7) {
        None
    } else {
        let mut v = Vec::with_capacity(64);
        v.extend_from_slice(&id.to_le_bytes());
        for i in 1..8 {
            v.extend_from_slice(&super::workload::mix(r ^ i).to_le_bytes());
        }
        Some(v)
    };
    (key, value)
}
pub fn scan(db: &Kiban) -> io::Result<Model> {
    let mut model = Model::new();
    let mut prev: Option<Vec<u8>> = None;
    for r in db.iter() {
        let (k, v) = r.map_err(io::Error::other)?;
        if prev.as_ref().is_some_and(|p| p >= &k) {
            return Err(io::Error::other("duplicate/unordered live scan key"));
        }
        prev = Some(k.clone());
        model.insert(k, v);
    }
    Ok(model)
}
pub fn audit(dir: &Path) -> io::Result<()> {
    let m = Manifest::load(dir)
        .map_err(io::Error::other)?
        .ok_or_else(|| io::Error::other("missing MANIFEST"))?;
    let mut numbers = BTreeSet::new();
    for &n in &m.wal_numbers {
        if !numbers.insert(n) || !sys::exists(&dir.join(format!("{n}.wal"))) {
            return Err(io::Error::other("invalid live WAL set"));
        }
    }
    for t in &m.tables {
        if !numbers.insert(t.number) || !sys::exists(&dir.join(format!("{}.sst", t.number))) {
            return Err(io::Error::other("duplicate/missing MANIFEST artifact"));
        }
    }
    Ok(())
}
pub fn verify(db: &Kiban, floor: &Model, allowed: &Allowed, keys: u64) -> io::Result<Model> {
    let got = scan(db)?;
    for (k, v) in &got {
        if k.len() != 8 || u64::from_be_bytes(k.as_slice().try_into().unwrap()) >= keys {
            return Err(io::Error::other("unexpected key after recovery"));
        }
        let point = db.get(k).map_err(io::Error::other)?;
        if point.as_ref() != Some(v) {
            return Err(io::Error::other(format!(
                "get/scan mismatch key={k:?} scan={v:?} get={point:?}"
            )));
        }
    }
    for id in 0..keys {
        let k = id.to_be_bytes();
        let actual = got.get(k.as_slice());
        let expected = floor.get(k.as_slice());
        let extra = allowed
            .get(k.as_slice())
            .is_some_and(|a| a.iter().any(|v| v.as_ref() == actual));
        if actual != expected && !extra {
            return Err(io::Error::other(format!(
                "recovered key {id} outside durable floor/attempt band"
            )));
        }
        if db.get(k).map_err(io::Error::other)?.as_ref() != actual {
            return Err(io::Error::other(format!(
                "get/scan disagreement at key {id}"
            )));
        }
    }
    audit(db.dir())?;
    Ok(got)
}
