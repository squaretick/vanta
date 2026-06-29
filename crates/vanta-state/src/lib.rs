//! `vanta-state` — the redb-backed persistent state store.
//!
//! Holds the index/history/cache tables described in
//! `docs/23-data-and-state-model.md`: the store index, generation history, GC
//! roots, the resolution cache, and a `meta` table (schema version + the current
//! generation pointer). Records are serialized to bytes with `serde_json` and
//! stored under typed redb tables; all writes are transactional.
//!
//! The store/caches (raw content-addressed bytes) live on the filesystem, not
//! here — this crate is metadata only.
#![forbid(unsafe_code)]

use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::path::Path;
use vanta_core::{Area, VtaError, VtaResult};

/// The state-DB schema version (`docs/23` §schema-versioning-and-migration).
pub const SCHEMA_VERSION: u32 = 1;

const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");
const STORE_INDEX: TableDefinition<&str, &[u8]> = TableDefinition::new("store_index");
const GENERATIONS: TableDefinition<u64, &[u8]> = TableDefinition::new("generations");
const RESOLUTION_CACHE: TableDefinition<&str, &[u8]> = TableDefinition::new("resolution_cache");

/// Metadata catalogued for a materialized store entry (`docs/23`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreEntryMeta {
    pub tool: String,
    pub version: String,
    pub platform: String,
    pub size: u64,
    pub sha256: String,
}

/// An immutable generation record (`docs/12-updates.md`, `docs/23`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationRecord {
    pub id: u64,
    pub parent: Option<u64>,
    pub command: String,
    pub reason: String,
    /// tool name → store key.
    pub tools: Vec<(String, String)>,
}

/// The persistent state database.
pub struct State {
    db: Database,
}

impl State {
    /// Open (creating if absent) the state database at `path` and ensure the
    /// schema-version marker is present.
    pub fn open(path: &Path) -> VtaResult<State> {
        let db = Database::create(path).map_err(db_err)?;
        let state = State { db };
        state.init()?;
        Ok(state)
    }

    fn init(&self) -> VtaResult<()> {
        let txn = self.db.begin_write().map_err(db_err)?;
        {
            let mut meta = txn.open_table(META).map_err(db_err)?;
            if meta.get("schema_version").map_err(db_err)?.is_none() {
                meta.insert("schema_version", SCHEMA_VERSION.to_string().as_bytes())
                    .map_err(db_err)?;
            }
        }
        txn.commit().map_err(db_err)?;
        Ok(())
    }

    /// The on-disk schema version (0 if unset).
    pub fn schema_version(&self) -> VtaResult<u32> {
        self.meta_get("schema_version")
            .map(|opt| opt.and_then(|s| s.parse().ok()).unwrap_or(0))
    }

    /// Record (or replace) the metadata for a store entry, keyed by its store key.
    pub fn put_store_entry(&self, store_key: &str, meta: &StoreEntryMeta) -> VtaResult<()> {
        let bytes = serde_json::to_vec(meta).map_err(enc_err)?;
        let txn = self.db.begin_write().map_err(db_err)?;
        {
            let mut t = txn.open_table(STORE_INDEX).map_err(db_err)?;
            t.insert(store_key, bytes.as_slice()).map_err(db_err)?;
        }
        txn.commit().map_err(db_err)?;
        Ok(())
    }

    /// Look up store-entry metadata by store key.
    pub fn get_store_entry(&self, store_key: &str) -> VtaResult<Option<StoreEntryMeta>> {
        let txn = self.db.begin_read().map_err(db_err)?;
        let t = txn.open_table(STORE_INDEX).map_err(db_err)?;
        match t.get(store_key).map_err(db_err)? {
            Some(guard) => Ok(Some(
                serde_json::from_slice(guard.value()).map_err(enc_err)?,
            )),
            None => Ok(None),
        }
    }

    /// Append a generation record (keyed by its id).
    pub fn append_generation(&self, rec: &GenerationRecord) -> VtaResult<()> {
        let bytes = serde_json::to_vec(rec).map_err(enc_err)?;
        let txn = self.db.begin_write().map_err(db_err)?;
        {
            let mut t = txn.open_table(GENERATIONS).map_err(db_err)?;
            t.insert(rec.id, bytes.as_slice()).map_err(db_err)?;
        }
        txn.commit().map_err(db_err)?;
        Ok(())
    }

    /// Fetch a generation record by id.
    pub fn get_generation(&self, id: u64) -> VtaResult<Option<GenerationRecord>> {
        let txn = self.db.begin_read().map_err(db_err)?;
        let t = txn.open_table(GENERATIONS).map_err(db_err)?;
        match t.get(id).map_err(db_err)? {
            Some(guard) => Ok(Some(
                serde_json::from_slice(guard.value()).map_err(enc_err)?,
            )),
            None => Ok(None),
        }
    }

    /// Set the current (active) generation pointer.
    pub fn set_current(&self, id: u64) -> VtaResult<()> {
        let txn = self.db.begin_write().map_err(db_err)?;
        {
            let mut meta = txn.open_table(META).map_err(db_err)?;
            meta.insert("current", id.to_string().as_bytes())
                .map_err(db_err)?;
        }
        txn.commit().map_err(db_err)?;
        Ok(())
    }

    /// The current (active) generation id, if any.
    pub fn current(&self) -> VtaResult<Option<u64>> {
        self.meta_get("current")
            .map(|opt| opt.and_then(|s| s.parse().ok()))
    }

    /// Store a raw resolution-cache entry (opaque bytes keyed by a config hash).
    pub fn put_resolution(&self, config_hash: &str, bytes: &[u8]) -> VtaResult<()> {
        let txn = self.db.begin_write().map_err(db_err)?;
        {
            let mut t = txn.open_table(RESOLUTION_CACHE).map_err(db_err)?;
            t.insert(config_hash, bytes).map_err(db_err)?;
        }
        txn.commit().map_err(db_err)?;
        Ok(())
    }

    /// Fetch a raw resolution-cache entry.
    pub fn get_resolution(&self, config_hash: &str) -> VtaResult<Option<Vec<u8>>> {
        let txn = self.db.begin_read().map_err(db_err)?;
        let t = txn.open_table(RESOLUTION_CACHE).map_err(db_err)?;
        Ok(t.get(config_hash)
            .map_err(db_err)?
            .map(|g| g.value().to_vec()))
    }

    fn meta_get(&self, key: &str) -> VtaResult<Option<String>> {
        let txn = self.db.begin_read().map_err(db_err)?;
        let t = txn.open_table(META).map_err(db_err)?;
        Ok(t.get(key)
            .map_err(db_err)?
            .map(|g| String::from_utf8_lossy(g.value()).into_owned()))
    }
}

fn db_err<E: Display>(e: E) -> VtaError {
    VtaError::new(Area::Store, 10, format!("state db: {e}"))
}

fn enc_err<E: Display>(e: E) -> VtaError {
    VtaError::new(Area::Store, 11, format!("state encode/decode: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(tag: &str) -> std::path::PathBuf {
        let p =
            std::env::temp_dir().join(format!("vanta-state-{}-{}.redb", tag, std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn schema_initialized() {
        let path = temp_db("schema");
        let s = State::open(&path).unwrap();
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn store_entry_roundtrip() {
        let path = temp_db("store");
        let s = State::open(&path).unwrap();
        let meta = StoreEntryMeta {
            tool: "node".into(),
            version: "24.6.0".into(),
            platform: "macos/aarch64".into(),
            size: 24117248,
            sha256: "5f2c".into(),
        };
        s.put_store_entry("blake3-aa3f", &meta).unwrap();
        assert_eq!(s.get_store_entry("blake3-aa3f").unwrap(), Some(meta));
        assert_eq!(s.get_store_entry("blake3-missing").unwrap(), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn generations_and_current() {
        let path = temp_db("gen");
        let s = State::open(&path).unwrap();
        let rec = GenerationRecord {
            id: 1,
            parent: None,
            command: "vanta add node@24".into(),
            reason: "add".into(),
            tools: vec![("node".into(), "blake3-aa3f".into())],
        };
        s.append_generation(&rec).unwrap();
        s.set_current(1).unwrap();
        assert_eq!(s.current().unwrap(), Some(1));
        assert_eq!(s.get_generation(1).unwrap(), Some(rec));
        assert_eq!(s.get_generation(2).unwrap(), None);
        let _ = std::fs::remove_file(&path);
    }
}
