//! `vanta-store` — the content-addressed store and download cache.
//!
//! Materialized tools live in immutable, content-addressed directories
//! (`store/blake3-<hex>/`); publication is atomic (stage → rename); identical
//! content is deduplicated; GC is reachability from roots. See `docs/09-store.md`.
#![forbid(unsafe_code)]

pub mod hash;
pub mod link;

pub use hash::{hash_bytes, hash_tree};
pub use link::link_best;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use vanta_core::{Area, StoreKey, VtaError, VtaResult};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The content-addressed store rooted at `$VANTA_HOME`.
pub struct Store {
    home: PathBuf,
}

impl Store {
    /// Open (creating the directory skeleton if needed) a store at `home`.
    pub fn open(home: impl AsRef<Path>) -> VtaResult<Store> {
        let store = Store {
            home: home.as_ref().to_path_buf(),
        };
        for dir in [store.store_dir(), store.downloads_dir(), store.envs_dir()] {
            fs::create_dir_all(&dir).map_err(|e| io(&dir, e))?;
        }
        Ok(store)
    }

    pub fn store_dir(&self) -> PathBuf {
        self.home.join("store")
    }
    pub fn downloads_dir(&self) -> PathBuf {
        self.home.join("cache").join("downloads")
    }
    pub fn envs_dir(&self) -> PathBuf {
        self.home.join("envs")
    }

    /// The directory a store key occupies.
    pub fn entry_path(&self, key: &StoreKey) -> PathBuf {
        self.store_dir().join(key.as_str())
    }

    /// Whether the store already contains an entry for `key`.
    pub fn has(&self, key: &StoreKey) -> bool {
        self.entry_path(key).is_dir()
    }

    /// Create a fresh, unique staging directory under the store. The installer
    /// unpacks into it, then calls [`Store::publish_tree`].
    pub fn new_staging(&self) -> VtaResult<PathBuf> {
        let path = self.store_dir().join(format!(".tmp-{}", unique()));
        fs::create_dir_all(&path).map_err(|e| io(&path, e))?;
        Ok(path)
    }

    /// Publish a staged tree into the store, content-addressed and atomically.
    /// If an identical entry already exists, the staged copy is discarded
    /// (dedup) and the existing key returned.
    pub fn publish_tree(&self, staged: &Path) -> VtaResult<StoreKey> {
        let key = StoreKey::new(hash_tree(staged)?)?;
        let dest = self.entry_path(&key);
        if dest.exists() {
            let _ = fs::remove_dir_all(staged);
            return Ok(key);
        }
        fs::create_dir_all(self.store_dir()).map_err(|e| io(&self.store_dir(), e))?;
        // Same-filesystem rename is atomic: a reader sees all-or-nothing.
        fs::rename(staged, &dest)
            .map_err(|e| VtaError::new(Area::Store, 4, format!("publishing store entry: {e}")))?;
        let _ = set_readonly_recursive(&dest);
        Ok(key)
    }

    /// Re-hash an entry and confirm it still matches its key (integrity check).
    pub fn verify_entry(&self, key: &StoreKey) -> VtaResult<bool> {
        let path = self.entry_path(key);
        if !path.is_dir() {
            return Ok(false);
        }
        Ok(hash_tree(&path)? == key.as_str())
    }

    /// Store a downloaded blob in the content-addressed download cache, returning
    /// its `blake3-<hex>` cache key. Idempotent.
    pub fn cache_put_blob(&self, bytes: &[u8]) -> VtaResult<String> {
        let key = hash_bytes(bytes);
        let path = self.downloads_dir().join(&key);
        if !path.exists() {
            let tmp = self.downloads_dir().join(format!(".tmp-{}", unique()));
            fs::write(&tmp, bytes).map_err(|e| io(&tmp, e))?;
            fs::rename(&tmp, &path).map_err(|e| io(&path, e))?;
        }
        Ok(key)
    }

    /// The path of a cached blob, if present.
    pub fn cache_get_path(&self, cache_key: &str) -> Option<PathBuf> {
        let p = self.downloads_dir().join(cache_key);
        p.exists().then_some(p)
    }

    /// Garbage-collect: remove every store entry not reachable from `roots`, plus
    /// stale staging dirs. Returns the number of entries removed.
    pub fn gc(&self, roots: &HashSet<StoreKey>) -> VtaResult<usize> {
        let mut removed = 0;
        let dir = self.store_dir();
        for entry in fs::read_dir(&dir).map_err(|e| io(&dir, e))? {
            let entry = entry.map_err(|e| io(&dir, e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if name.starts_with(".tmp-") {
                let _ = fs::remove_dir_all(&path);
                continue;
            }
            if name.starts_with("blake3-") {
                if let Ok(key) = StoreKey::new(name) {
                    if !roots.contains(&key) {
                        let _ = make_writable_recursive(&path);
                        if fs::remove_dir_all(&path).is_ok() {
                            removed += 1;
                        }
                    }
                }
            }
        }
        Ok(removed)
    }
}

fn unique() -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{}-{}", std::process::id(), nanos, n)
}

fn set_readonly_recursive(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            set_readonly_recursive(&entry?.path())?;
        }
    }
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_readonly(true);
    fs::set_permissions(path, perms)
}

/// Recursively add write permission (used before moving a restored entry into
/// place; the executable bit is preserved so the content hash still verifies).
pub fn ensure_writable(path: &Path) -> std::io::Result<()> {
    make_writable_recursive(path)
}

#[allow(clippy::permissions_set_readonly_false)] // intentional: make a GC target deletable
fn make_writable_recursive(path: &Path) -> std::io::Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_readonly(false);
    fs::set_permissions(path, perms)?;
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            make_writable_recursive(&entry?.path())?;
        }
    }
    Ok(())
}

fn io(path: &Path, e: std::io::Error) -> VtaError {
    VtaError::new(Area::Store, 2, format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("vanta-store-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn publish_dedup_and_verify() {
        let h = home("publish");
        let store = Store::open(&h).unwrap();

        let staged = store.new_staging().unwrap();
        fs::write(staged.join("tool"), b"binary-bytes").unwrap();
        let key = store.publish_tree(&staged).unwrap();
        assert!(key.as_str().starts_with("blake3-"));
        assert!(store.has(&key));
        assert!(store.verify_entry(&key).unwrap());

        // Identical content publishes to the same key (dedup); staged dir consumed.
        let staged2 = store.new_staging().unwrap();
        fs::write(staged2.join("tool"), b"binary-bytes").unwrap();
        let key2 = store.publish_tree(&staged2).unwrap();
        assert_eq!(key, key2);
        assert!(!staged2.exists());

        let _ = fs::remove_dir_all(&h);
    }

    #[test]
    fn cache_blob_roundtrip() {
        let h = home("cache");
        let store = Store::open(&h).unwrap();
        let k = store.cache_put_blob(b"download").unwrap();
        assert!(store.cache_get_path(&k).is_some());
        assert!(store.cache_get_path("blake3-absent").is_none());
        let _ = fs::remove_dir_all(&h);
    }

    #[test]
    fn gc_removes_unreachable() {
        let h = home("gc");
        let store = Store::open(&h).unwrap();
        let s = store.new_staging().unwrap();
        fs::write(s.join("f"), b"keepme").unwrap();
        let keep = store.publish_tree(&s).unwrap();
        let s2 = store.new_staging().unwrap();
        fs::write(s2.join("f"), b"dropme").unwrap();
        let drop = store.publish_tree(&s2).unwrap();

        let mut roots = HashSet::new();
        roots.insert(keep.clone());
        let removed = store.gc(&roots).unwrap();
        assert_eq!(removed, 1);
        assert!(store.has(&keep));
        assert!(!store.has(&drop));
        let _ = fs::remove_dir_all(&h);
    }
}
