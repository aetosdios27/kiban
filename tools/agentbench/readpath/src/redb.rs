use crate::{
    Options,
    api::{Backend, Result},
    model::{Memory, key},
};
use redb::{
    CompactionMode, Database, Durability, Key, ReadableDatabase, ReadableTable, TableDefinition,
    TypeName, Value,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{borrow::Cow, cmp::Ordering, marker::PhantomData, ops::Bound};

pub const VERSION: &str = "5.0.0";

#[derive(Debug)]
struct DescendingBytes;

impl Value for DescendingBytes {
    type SelfType<'a> = &'a [u8];
    type AsBytes<'a> = &'a [u8];

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> &'a [u8]
    where
        Self: 'a,
    {
        data
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a &'b [u8]) -> &'a [u8] {
        value
    }

    fn type_name() -> TypeName {
        TypeName::new("kiban.readbench.DescendingBytes")
    }
}

impl Key for DescendingBytes {
    fn compare(left: &[u8], right: &[u8]) -> Ordering {
        right.cmp(left)
    }

    fn separator<'a>(left: &'a [u8], right: &'a [u8]) -> Cow<'a, [u8]> {
        // The first difference (or one byte beyond a shared prefix) already
        // orders this prefix of left strictly above right in byte order.
        let shared = left.iter().zip(right).take_while(|(l, r)| l == r).count();
        Cow::Borrowed(&left[..shared + 1])
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Compact {
    #[default]
    None,
    Balanced,
    Maximum,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Tuning {
    compact: Compact,
    quick_repair: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) plain_keys: bool,
}

struct Redb<K: Key + 'static, const REVERSE: bool> {
    db: Database,
    normal: bool,
    durable: bool,
    tuning: Tuning,
    key_type: PhantomData<K>,
}

pub(crate) fn open(o: &Options, create: bool) -> Result<Box<dyn Backend>> {
    if o.redb.plain_keys {
        Ok(Box::new(Redb::<&[u8], false>::open(o, create)?))
    } else {
        Ok(Box::new(Redb::<DescendingBytes, true>::open(o, create)?))
    }
}

impl<K: Key + 'static, const REVERSE: bool> Redb<K, REVERSE> {
    const MEMORIES: TableDefinition<'static, K, &'static [u8]> = TableDefinition::new("memories");

    fn open(o: &Options, create: bool) -> Result<Self> {
        let path = o.suite.db.join("memories.redb");
        let mut builder = Database::builder();
        builder.set_cache_size(o.cache_bytes());
        let db = if create {
            std::fs::create_dir_all(&o.suite.db)?;
            builder.create(path)?
        } else {
            builder.open(path)?
        };
        eprintln!(
            "{}",
            json!({"type":"backend_config","backend":"redb","version":VERSION,
                "cache_bytes":o.cache_bytes(),"durability":if o.durable {"Immediate"} else {"None"},
                "tuning":o.redb})
        );
        Ok(Self {
            db,
            normal: o.layout == "normal",
            durable: o.durable,
            tuning: o.redb,
            key_type: PhantomData,
        })
    }

    fn stats(&self, phase: &str) -> Result<()> {
        let txn = self.db.begin_write()?;
        let stats = txn.stats()?;
        eprintln!(
            "{}",
            json!({"type":"redb_storage","phase":phase,"page_size":stats.page_size(),
                "allocated_pages":stats.allocated_pages(),"stored_bytes":stats.stored_bytes(),
                "metadata_bytes":stats.metadata_bytes(),"fragmented_bytes":stats.fragmented_bytes()})
        );
        txn.abort()?;
        Ok(())
    }

    fn compact_fixture(mut b: Self) -> Result<()> {
        b.stats("before_compact")?;
        let mode = match b.tuning.compact {
            Compact::None => return Err("compact requires balanced or maximum".into()),
            Compact::Balanced => CompactionMode::Balanced,
            Compact::Maximum => CompactionMode::Maximum,
        };
        let changed = b.db.compact(mode)?;
        eprintln!("{}", json!({"type":"redb_compact","changed":changed}));
        b.stats("after_compact")?;
        b.db.close()?;
        Ok(())
    }
}

pub(crate) fn compact_fixture(o: &Options) -> Result<()> {
    // Compaction operates on the database without opening a typed table.
    Redb::<&[u8], false>::compact_fixture(Redb::open(o, false)?)
}

impl<K: for<'a> Key<SelfType<'a> = &'a [u8]> + 'static, const REVERSE: bool> Backend
    for Redb<K, REVERSE>
{
    fn bulk_load(&mut self, n: u64, seed: u64) -> Result<()> {
        if self.normal {
            for id in 0..n {
                self.put(&Memory::new(id, 0, seed), true)?;
                if (id + 1) % 100000 == 0 {
                    eprintln!("redb: normal preload {}/{n}", id + 1);
                }
            }
            self.finish()?;
        } else {
            let txn = self.db.begin_write()?;
            {
                let mut table = txn.open_table(Self::MEMORIES)?;
                for id in 0..n {
                    let memory = Memory::new(id, 0, seed);
                    let value = serde_json::to_vec(&memory)?;
                    table.insert(memory.key().as_bytes(), value.as_slice())?;
                }
            }
            txn.commit()?;
        }
        self.stats("loaded")
    }

    fn put(&mut self, memory: &Memory, _insert: bool) -> Result<()> {
        let key = memory.key();
        let value = serde_json::to_vec(memory)?;
        let mut txn = self.db.begin_write()?;
        let durability = if self.durable {
            txn.set_quick_repair(self.tuning.quick_repair);
            Durability::Immediate
        } else {
            Durability::None
        };
        txn.set_durability(durability)?;
        txn.open_table(Self::MEMORIES)?
            .insert(key.as_bytes(), value.as_slice())?;
        txn.commit()?;
        Ok(())
    }

    fn get(&mut self, id: u64) -> Result<Memory> {
        let key = key(id);
        let txn = self.db.begin_read()?;
        let table = txn.open_table(Self::MEMORIES)?;
        let value = table.get(key.as_bytes())?.ok_or("missing redb memory")?;
        Ok(serde_json::from_slice(value.value())?)
    }

    fn contains_key(&mut self, key: &[u8]) -> Result<bool> {
        let txn = self.db.begin_read()?;
        Ok(txn.open_table(Self::MEMORIES)?.get(key)?.is_some())
    }

    fn recent(&mut self, agent: u32, k: usize) -> Result<Vec<Memory>> {
        let start = format!("agent/{agent:04}/memory/");
        let end = format!("agent/{agent:04}/memory0");
        let txn = self.db.begin_read()?;
        let table = txn.open_table(Self::MEMORIES)?;
        // Keys encode MAX-id. Traverse in natural byte order for both comparators.
        let start = Bound::Included(start.as_bytes());
        let end = Bound::Excluded(end.as_bytes());
        let decode = |entry: std::result::Result<_, redb::StorageError>| {
            let (_, value): (_, redb::AccessGuard<'_, &[u8]>) = entry?;
            Ok(serde_json::from_slice(value.value())?)
        };
        if REVERSE {
            table
                .range((end, start))?
                .rev()
                .take(k)
                .map(decode)
                .collect()
        } else {
            table.range((start, end))?.take(k).map(decode).collect()
        }
    }

    fn finish(&mut self) -> Result<()> {
        if !self.durable {
            // An Immediate commit persists the preceding None commits.
            self.db.begin_write()?.commit()?;
        }
        Ok(())
    }
}

// The controller waits for the last ACK and SIGKILLs this process while it
// still owns the database. Recovery uses the same adapter in a new process.
pub(crate) fn crash_check(o: &Options) -> Result<()> {
    use std::io::{self, Write};

    if o.suite.backend != "redb" || !o.durable {
        return Err("crash check requires durable redb".into());
    }
    let c = &o.suite;
    let mut b = open(o, false)?;
    if o.action == "crash-verify" {
        let plan = crate::runner::Plan {
            base_population: c.n,
            insertions: c.operations,
            write_operations: 0,
        };
        // Use the ordinary full insert-only model, including all latest-K slices.
        let mut inserts = c.clone();
        inserts.operations = 0;
        crate::runner::verify_persisted_plan(b.as_mut(), &inserts, plan, &mut io::stdout())?;
        return Ok(());
    }
    crate::runner::verify_initial(b.as_mut(), c)?;
    for id in c.n..c.n + c.operations {
        b.put(&Memory::new(id, 0, c.seed), true)?;
        crate::runner::check(&b.get(id)?, id, 0, c.seed)?;
        println!("ACK {id}");
        io::stdout().flush()?;
    }
    // The controller keeps stdin open and silent until after it sends SIGKILL.
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Err("controller released durable writer without SIGKILL".into())
}
