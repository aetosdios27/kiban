//! SQLite baselines: integer rowid plus recent index, or clustered WITHOUT ROWID.
//! One connection is owned by the calling worker; no connection is shared.
use crate::{
    Options,
    api::{Backend, Result},
    model::{AGENTS, Memory},
};
use rusqlite::{Connection, OpenFlags, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    #[default]
    Rowid,
    Clustered,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Sync {
    Off,
    #[default]
    Normal,
    Full,
}

impl Sync {
    fn number(self) -> i64 {
        match self {
            Self::Off => 0,
            Self::Normal => 1,
            Self::Full => 2,
        }
    }
}

pub struct Sqlite {
    connection: Connection,
    layout: Layout,
}

fn decode(row: &Row<'_>) -> rusqlite::Result<Memory> {
    let bytes = row.get_ref(0)?.as_blob()?;
    serde_json::from_slice(bytes).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, Box::new(e))
    })
}

impl Sqlite {
    pub(super) fn open(o: &Options) -> Result<Self> {
        std::fs::create_dir_all(&o.suite.db)?;
        let connection = Connection::open_with_flags(
            o.suite.db.join("memories.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.set_prepared_statement_cache_capacity(16);
        connection.pragma_update(None, "page_size", 4096)?;
        let mode: String = connection.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        if mode != "wal" {
            return Err("SQLite refused WAL".into());
        }
        connection.pragma_update(None, "synchronous", o.sqlite_sync.number())?;
        // Positive cache_size counts pages: match the engine budget within one page.
        connection.pragma_update(None, "cache_size", (o.cache_bytes() / 4096) as i64)?;
        connection.pragma_update(None, "mmap_size", i64::try_from(o.sqlite_mmap_bytes)?)?;
        connection.pragma_update(None, "wal_autocheckpoint", 1000)?;
        connection.pragma_update(None, "temp_store", "MEMORY")?;
        let schema = match o.sqlite_layout {
            Layout::Rowid => {
                "CREATE TABLE IF NOT EXISTS memories(\
                id INTEGER PRIMARY KEY, agent INTEGER NOT NULL, value BLOB NOT NULL);\
                CREATE INDEX IF NOT EXISTS memories_recent ON memories(agent, id DESC);"
            }
            Layout::Clustered => {
                "CREATE TABLE IF NOT EXISTS memories(\
                agent INTEGER NOT NULL, ts INTEGER NOT NULL, value BLOB NOT NULL,\
                PRIMARY KEY(agent, ts DESC)) WITHOUT ROWID;"
            }
        };
        connection.execute_batch(schema)?;
        let actual_schema = connection
            .prepare("SELECT sql FROM sqlite_schema WHERE name='memories'")?
            .query_row([], |r| r.get::<_, String>(0))?;
        if actual_schema.contains("WITHOUT ROWID") != (o.sqlite_layout == Layout::Clustered) {
            return Err("SQLite fixture layout differs from requested layout".into());
        }
        let mut pragmas = serde_json::Map::new();
        for name in [
            "synchronous",
            "cache_size",
            "page_size",
            "mmap_size",
            "wal_autocheckpoint",
            "temp_store",
            "locking_mode",
            "journal_mode",
        ] {
            let value = connection.query_row(&format!("PRAGMA {name}"), [], |r| {
                Ok(match r.get_ref(0)? {
                    rusqlite::types::ValueRef::Integer(n) => json!(n),
                    rusqlite::types::ValueRef::Text(s) => json!(String::from_utf8_lossy(s)),
                    _ => serde_json::Value::Null,
                })
            })?;
            pragmas.insert(name.into(), value);
        }
        if pragmas["synchronous"] != json!(o.sqlite_sync.number())
            || pragmas["cache_size"] != json!(o.cache_bytes() / 4096)
            || pragmas["page_size"] != json!(4096)
            || pragmas["mmap_size"].as_u64().unwrap_or(0) < o.sqlite_mmap_bytes
        {
            return Err("SQLite effective pragmas differ from requested configuration".into());
        }
        let compile_options = connection
            .prepare("PRAGMA compile_options")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let source_id: String =
            connection.query_row("SELECT sqlite_source_id()", [], |r| r.get(0))?;
        let point = match o.sqlite_layout {
            Layout::Rowid => "EXPLAIN QUERY PLAN SELECT value FROM memories WHERE id=1",
            Layout::Clustered => {
                "EXPLAIN QUERY PLAN SELECT value FROM memories WHERE agent=1 AND ts=1"
            }
        };
        let recent = match o.sqlite_layout {
            Layout::Rowid => {
                "EXPLAIN QUERY PLAN SELECT value FROM memories WHERE agent=1 ORDER BY id DESC LIMIT 10"
            }
            Layout::Clustered => {
                "EXPLAIN QUERY PLAN SELECT value FROM memories WHERE agent=1 ORDER BY ts DESC LIMIT 10"
            }
        };
        let mut plans = Vec::new();
        for sql in [point, recent] {
            plans.push(
                connection
                    .prepare(sql)?
                    .query_map([], |r| r.get::<_, String>(3))?
                    .collect::<rusqlite::Result<Vec<_>>>()?,
            );
        }
        eprintln!(
            "{}",
            json!({"type":"backend_config", "backend":"sqlite", "sqlite_version":rusqlite::version(),
            "sqlite_source_id":source_id,"rusqlite":"0.40.2","compile_options":compile_options,
            "layout":o.sqlite_layout,"schema":schema,"pragmas":pragmas,"query_plans":plans,
            "target_cache_bytes":o.cache_bytes(),"effective_cache_bytes":(o.cache_bytes()/4096)*4096})
        );
        Ok(Self {
            connection,
            layout: o.sqlite_layout,
        })
    }

    fn insert_sql(&self) -> &'static str {
        match self.layout {
            Layout::Rowid => "INSERT INTO memories(id, agent, value) VALUES(?1, ?2, ?3)",
            Layout::Clustered => "INSERT INTO memories(ts, agent, value) VALUES(?1, ?2, ?3)",
        }
    }
}

impl Backend for Sqlite {
    fn bulk_load(&mut self, n: u64, seed: u64) -> Result<()> {
        let sql = self.insert_sql();
        if self.layout == Layout::Rowid {
            self.connection
                .execute_batch("DROP INDEX memories_recent")?;
        }
        let txn = self.connection.transaction()?;
        {
            let mut statement = txn.prepare_cached(sql)?;
            // Match each table's primary order, just as the LSM bulk flush
            // sorts its keys. Preparation order is outside all call timers.
            let ids: Box<dyn Iterator<Item = u64>> = match self.layout {
                Layout::Rowid => Box::new(0..n),
                Layout::Clustered => Box::new((0..AGENTS.min(n)).flat_map(|agent| {
                    let count = (n - 1 - agent) / AGENTS + 1;
                    (0..count).rev().map(move |seq| agent + seq * AGENTS)
                })),
            };
            for id in ids {
                let memory = Memory::new(id, 0, seed);
                statement.execute(params![
                    id as i64,
                    memory.agent,
                    serde_json::to_vec(&memory)?
                ])?;
            }
        }
        txn.commit()?;
        if self.layout == Layout::Rowid {
            self.connection
                .execute_batch("CREATE INDEX memories_recent ON memories(agent, id DESC)")?;
        }
        self.connection.execute_batch("ANALYZE")?;
        Ok(())
    }

    fn put(&mut self, memory: &Memory, insert: bool) -> Result<()> {
        let value = serde_json::to_vec(memory)?;
        // An implicit transaction commits this single mutation before returning.
        // Update only the payload: creation sequence and agent are immutable.
        let changed = if insert {
            self.connection
                .prepare_cached(self.insert_sql())?
                .execute(params![memory.created_at as i64, memory.agent, value])?
        } else {
            let sql = match self.layout {
                Layout::Rowid => "UPDATE memories SET value=?3 WHERE id=?1 AND agent=?2",
                Layout::Clustered => "UPDATE memories SET value=?3 WHERE ts=?1 AND agent=?2",
            };
            self.connection.prepare_cached(sql)?.execute(params![
                memory.created_at as i64,
                memory.agent,
                value
            ])?
        };
        if changed != 1 {
            return Err("SQLite mutation did not affect exactly one row".into());
        }
        Ok(())
    }

    fn get(&mut self, id: u64) -> Result<Memory> {
        let memory = match self.layout {
            Layout::Rowid => self
                .connection
                .prepare_cached("SELECT value FROM memories WHERE id=?1")?
                .query_row([id as i64], decode)?,
            Layout::Clustered => self
                .connection
                .prepare_cached("SELECT value FROM memories WHERE agent=?1 AND ts=?2")?
                .query_row(params![(id % AGENTS) as u32, id as i64], decode)?,
        };
        Ok(memory)
    }

    fn recent(&mut self, agent: u32, k: usize) -> Result<Vec<Memory>> {
        let sql = match self.layout {
            Layout::Rowid => "SELECT value FROM memories WHERE agent=?1 ORDER BY id DESC LIMIT ?2",
            Layout::Clustered => {
                "SELECT value FROM memories WHERE agent=?1 ORDER BY ts DESC LIMIT ?2"
            }
        };
        let mut statement = self.connection.prepare_cached(sql)?;
        Ok(statement
            .query_map(params![agent, k as i64], decode)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn finish(&mut self) -> Result<()> {
        // Outside operation timers, persist relaxed-mode writes and include
        // checkpoint cost in the reported completion time.
        self.connection.pragma_update(None, "synchronous", 2)?;
        let (busy, _, _): (i64, i64, i64) =
            self.connection
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?;
        if busy != 0 {
            return Err("SQLite checkpoint was busy".into());
        }
        let integrity: String = self
            .connection
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(format!("SQLite integrity: {integrity}").into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, prepare, reads, writes};

    #[test]
    fn layouts_and_sync_modes_preserve_payloads_and_commits() -> Result<()> {
        let root = std::env::temp_dir().join(format!("readbench-sqlite-{}", std::process::id()));
        std::fs::create_dir_all(&root)?;
        for layout in [Layout::Rowid, Layout::Clustered] {
            for sync in [Sync::Off, Sync::Normal, Sync::Full] {
                let label = format!("{layout:?}-{sync:?}");
                let o = Options {
                    suite: Config {
                        backend: "sqlite".into(),
                        db: root.join(&label),
                        endpoint: String::new(),
                        n: 640,
                        operations: 100,
                        k: 10,
                        seed: 20261016,
                        output: root.join(format!("{label}-read.jsonl")),
                        verify_only: false,
                    },
                    action: "read".into(),
                    layout: "bulk".into(),
                    cache_mib: Some(32),
                    mmap_max_level: None,
                    readers: 1,
                    point_index: false,
                    stage: 0,
                    durable: false,
                    micro: String::new(),
                    sqlite_layout: layout,
                    sqlite_sync: sync,
                    sqlite_mmap_bytes: 1024 * 1024 * 1024,
                    rocks: crate::rocks::Tuning::default(),
                };
                prepare(&o)?;
                reads(&o)?;
                let mut w = o;
                w.suite.output = root.join(format!("{label}-write.jsonl"));
                writes(&w)?;
                let rows: Vec<serde_json::Value> = std::fs::read_to_string(&w.suite.output)?
                    .lines()
                    .map(serde_json::from_str)
                    .collect::<std::result::Result<_, _>>()?;
                assert_eq!(rows[5]["verified_records"], 740);
                assert_eq!(rows[6]["type"], "reopen_verified");
                let mut db = Sqlite::open(&w)?;
                assert!(db.put(&Memory::new(0, 0, w.suite.seed), true).is_err());
                assert!(db.put(&Memory::new(9999, 0, w.suite.seed), false).is_err());
                db.put(&Memory::new(5, 999, w.suite.seed), false)?;
                db.finish()?;
                drop(db);
                assert!(
                    reads(&w)
                        .unwrap_err()
                        .to_string()
                        .contains("value mismatch")
                );
            }
        }
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
