//! `vanta-lock` — the `vanta.lock` model, canonical serialization, and the
//! manifest↔lock reconcile.
//!
//! The lock pins exact versions and per-platform artifact hashes for every
//! target so a single committed file reproduces on any OS. See
//! `docs/11-reproducibility.md` and `docs/31-lockfile-and-manifest-reference.md`.
//! Serialization is canonical (sorted tools, sorted platform keys) so the file
//! diffs cleanly in VCS.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use vanta_core::{Area, VtaError, VtaResult};

/// The current lock format version.
pub const LOCK_VERSION: u32 = 1;

/// A `vanta.lock` file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Lock {
    pub lock_version: u32,
    #[serde(default)]
    pub generated_by: String,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub registry_revision: String,
    /// One entry per locked tool, serialized as `[[tool]]`.
    #[serde(rename = "tool", default)]
    pub tools: Vec<LockedTool>,
}

/// A locked tool: the resolution plus a per-platform artifact pin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockedTool {
    pub name: String,
    pub request: String,
    pub version: String,
    pub provider: String,
    /// platform token → artifact pin (sorted for canonical output).
    #[serde(default)]
    pub platform: BTreeMap<String, PlatformPin>,
}

/// The per-platform artifact pin recorded in the lock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlatformPin {
    pub store_key: String,
    pub url: String,
    #[serde(default)]
    pub size: Option<u64>,
    pub sha256: String,
    #[serde(default)]
    pub blake3: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub bin: Vec<String>,
}

impl Lock {
    /// A new, empty lock with the current format version.
    pub fn new(generated_by: impl Into<String>, targets: Vec<String>) -> Lock {
        Lock {
            lock_version: LOCK_VERSION,
            generated_by: generated_by.into(),
            targets,
            registry_revision: String::new(),
            tools: Vec::new(),
        }
    }

    /// The set of tool names this lock pins.
    pub fn tool_names(&self) -> BTreeSet<String> {
        self.tools.iter().map(|t| t.name.clone()).collect()
    }

    /// Return a canonicalized clone: tools sorted by name (platform maps are
    /// already sorted by `BTreeMap`). Determinism keeps VCS diffs minimal.
    pub fn canonical(&self) -> Lock {
        let mut out = self.clone();
        out.tools.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Serialize to canonical TOML.
    pub fn to_toml(&self) -> VtaResult<String> {
        toml::to_string_pretty(&self.canonical())
            .map_err(|e| VtaError::new(Area::Lock, 4, format!("serialize lock: {e}")))
    }

    /// Parse a lock from TOML, rejecting a format version newer than supported.
    pub fn from_toml(src: &str) -> VtaResult<Lock> {
        let lock: Lock = toml::from_str(src)
            .map_err(|e| VtaError::new(Area::Lock, 1, format!("parse lock: {e}")))?;
        if lock.lock_version > LOCK_VERSION {
            return Err(VtaError::new(
                Area::Lock,
                2,
                format!(
                    "lock_version {} is newer than this Vanta supports ({}); upgrade Vanta",
                    lock.lock_version, LOCK_VERSION
                ),
            ));
        }
        Ok(lock)
    }

    /// Load a lock file.
    pub fn load_file(path: &Path) -> VtaResult<Lock> {
        let src = fs::read_to_string(path).map_err(|e| {
            VtaError::new(
                Area::Lock,
                1,
                format!("cannot read {}: {e}", path.display()),
            )
        })?;
        Lock::from_toml(&src)
    }

    /// Write the lock file canonically.
    pub fn write_file(&self, path: &Path) -> VtaResult<()> {
        let body = self.to_toml()?;
        fs::write(path, body).map_err(|e| {
            VtaError::new(
                Area::Lock,
                7,
                format!("cannot write {}: {e}", path.display()),
            )
        })
    }
}

/// The difference between what a manifest declares and what the lock pins.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reconcile {
    /// Declared in the manifest but absent from the lock (need re-lock).
    pub missing: Vec<String>,
    /// Present in the lock but no longer declared (prune candidates).
    pub extra: Vec<String>,
}

impl Reconcile {
    /// Whether the manifest and lock fully agree on tool membership.
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty()
    }
}

/// Compare the manifest's declared tool names against the lock. Tool-name level
/// only; deeper drift (a changed constraint a pin no longer satisfies) is checked
/// during resolution (`docs/06-resolution.md`).
pub fn reconcile(manifest_tools: &BTreeSet<String>, lock: &Lock) -> Reconcile {
    let locked = lock.tool_names();
    Reconcile {
        missing: manifest_tools.difference(&locked).cloned().collect(),
        extra: locked.difference(manifest_tools).cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Lock {
        let mut lock = Lock::new(
            "vanta 0.0.0",
            vec!["macos/aarch64".into(), "linux/x86_64/gnu".into()],
        );
        let mut platform = BTreeMap::new();
        platform.insert(
            "macos/aarch64".to_string(),
            PlatformPin {
                store_key: "blake3-aa3f".into(),
                url: "https://example.test/node.tar.xz".into(),
                size: Some(24117248),
                sha256: "5f2c".into(),
                blake3: Some("aa3f".into()),
                signature: Some("minisign:RWQf".into()),
                bin: vec!["bin/node".into()],
            },
        );
        lock.tools.push(LockedTool {
            name: "node".into(),
            request: "24".into(),
            version: "24.6.0".into(),
            provider: "official/node@3".into(),
            platform,
        });
        lock
    }

    #[test]
    fn roundtrips_through_toml() {
        let lock = sample();
        let text = lock.to_toml().unwrap();
        let parsed = Lock::from_toml(&text).unwrap();
        assert_eq!(parsed, lock.canonical());
    }

    #[test]
    fn rejects_newer_format() {
        let err = Lock::from_toml("lock_version = 999\n").unwrap_err();
        assert_eq!(err.area, Area::Lock);
        assert_eq!(err.number, 2);
    }

    #[test]
    fn canonical_sorts_tools() {
        let mut lock = Lock::new("t", vec![]);
        for n in ["terraform", "node", "go"] {
            lock.tools.push(LockedTool {
                name: n.into(),
                request: "latest".into(),
                version: "1".into(),
                provider: "p".into(),
                platform: BTreeMap::new(),
            });
        }
        let names: Vec<_> = lock.canonical().tools.into_iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["go", "node", "terraform"]);
    }

    #[test]
    fn reconcile_detects_drift() {
        let lock = sample();
        let manifest: BTreeSet<String> = ["node", "python"].iter().map(|s| s.to_string()).collect();
        let r = reconcile(&manifest, &lock);
        assert_eq!(r.missing, vec!["python".to_string()]); // declared, not locked
        assert!(r.extra.is_empty());
        assert!(!r.is_clean());
    }
}

#[cfg(test)]
mod fuzz {
    use super::*;
    proptest::proptest! {
        #[test]
        fn lock_parse_never_panics(s in ".*") { let _ = Lock::from_toml(&s); }
    }
}
