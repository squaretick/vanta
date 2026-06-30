//! Core value types: store keys, scopes, checksums, artifacts, resolutions,
//! and generations.
//!
//! See `docs/09-store.md`, `docs/11-reproducibility.md`, and
//! `docs/23-data-and-state-model.md`.

use crate::error::{Area, VtaError, VtaResult};
use crate::platform::Platform;
use std::fmt;
use std::path::PathBuf;

/// A tool name (e.g. `"node"`).
pub type ToolName = String;

/// A content-addressed store key of the form `blake3-<hex>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StoreKey(String);

/// The number of lowercase hex characters in a BLAKE3-256 digest (32 bytes).
const BLAKE3_HEX_LEN: usize = 64;

impl StoreKey {
    /// Construct a store key, validating the `blake3-` prefix **and** that the
    /// suffix is exactly [`BLAKE3_HEX_LEN`] lowercase hex characters.
    ///
    /// This is a security boundary (audit M7/L12): the key is later joined onto
    /// filesystem paths (`store/<key>`, `staging/<key>`, the shim's
    /// `store.join(key)`). Accepting anything but a fixed-width lowercase-hex
    /// digest would allow path-traversal payloads such as `blake3-../../etc` to
    /// flow into those joins. We therefore reject any non-conforming suffix
    /// rather than trusting the prefix alone.
    pub fn new(s: impl Into<String>) -> VtaResult<StoreKey> {
        let s = s.into();
        let invalid = |s: &str| {
            VtaError::new(
                Area::Store,
                1,
                format!(
                    "invalid store key `{s}` (expected `blake3-<{BLAKE3_HEX_LEN} lowercase hex>`)"
                ),
            )
        };
        let suffix = s.strip_prefix("blake3-").ok_or_else(|| invalid(&s))?;
        if suffix.len() != BLAKE3_HEX_LEN
            || !suffix
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid(&s));
        }
        Ok(StoreKey(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StoreKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a change applies (`docs/02-architecture.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// A project rooted at the given directory (the nearest `vanta.toml`).
    Project(PathBuf),
    /// The global scope (`~/.vanta`).
    Global,
}

/// An artifact checksum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checksum {
    /// `"sha256"` or `"blake3"`.
    pub algo: String,
    /// Lowercase hex digest.
    pub value: String,
}

/// The concrete, verifiable bytes for one tool@version on one platform
/// (see `docs/26-registry-and-metadata-reference.md`).
#[derive(Debug, Clone)]
pub struct Artifact {
    pub url: String,
    pub mirrors: Vec<String>,
    /// Archive kind token, e.g. `"tar.xz"`, `"zip"`, `"raw"`.
    pub archive: String,
    pub size: Option<u64>,
    pub checksum: Checksum,
    /// Detached signature over the artifact (minisign `.minisig` contents), if any.
    pub signature: Option<String>,
    /// The trusted public key (minisign) used to verify `signature`, if any.
    pub signature_key: Option<String>,
    /// Executables to expose (paths relative to the laid-out tree).
    pub bin: Vec<String>,
    /// Leading path components to strip when extracting (archive layout).
    pub strip: u32,
    /// The content-addressed key, set once materialized / recorded in the lock.
    pub store_key: Option<StoreKey>,
}

/// The deterministic output of resolving a request (`docs/06-resolution.md`).
#[derive(Debug, Clone)]
pub struct Resolution {
    pub tool: ToolName,
    /// The exact resolved version.
    pub version: String,
    /// Provider id + provider version, e.g. `"official/node@3"`.
    pub provider: String,
    /// One artifact per target platform.
    pub per_platform: Vec<(Platform, Artifact)>,
}

/// A monotonically-increasing generation id (rendered zero-padded to 4 digits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GenId(pub u64);

impl fmt::Display for GenId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}", self.0)
    }
}

/// Why a generation was created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reason {
    Add,
    Remove,
    Update,
    Sync,
    Rollback,
    Restore,
}

/// An immutable snapshot of an environment (see `docs/23-data-and-state-model.md`).
#[derive(Debug, Clone)]
pub struct Generation {
    pub id: GenId,
    pub parent: Option<GenId>,
    pub scope: Scope,
    /// The resolved, materialized tool set: tool → store key.
    pub tools: Vec<(ToolName, StoreKey)>,
    /// The command that produced this generation.
    pub command: String,
    pub reason: Reason,
}
