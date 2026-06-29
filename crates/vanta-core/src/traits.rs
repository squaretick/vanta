//! The trait seams the engine is wired through (see
//! `docs/02-architecture.md` §dependency-injection-and-extension-seams).
//!
//! Concrete implementations live in their owning crates (`vanta-provider`,
//! `vanta-net`, `vanta-store`, `vanta-security`, `vanta-platform`); test fakes
//! live in `vanta-test`.

use crate::error::VtaResult;
use crate::platform::Platform;
use crate::types::Artifact;
use std::path::Path;

/// Describes how to discover and resolve a tool's artifacts
/// (`docs/07-providers.md`, `docs/22-provider-sdk.md`).
pub trait Provider {
    /// The provider id, e.g. `"official/node"`.
    fn id(&self) -> &str;
    /// The available version strings, newest-first ordering applied by the resolver.
    fn list_versions(&self) -> VtaResult<Vec<String>>;
    /// The artifact descriptor for a version on a platform.
    fn resolve(&self, version: &str, platform: &Platform) -> VtaResult<Artifact>;
}

/// A fetch backend (curated, github-releases, direct-url, …).
pub trait Backend {
    fn name(&self) -> &str;
    /// Fetch `url` into `dest` (a path in the download cache).
    fn fetch(&self, url: &str, dest: &Path) -> VtaResult<()>;
}

/// A byte cache keyed by an opaque string (download/metadata caches).
pub trait CacheStore {
    fn get(&self, key: &str) -> VtaResult<Option<Vec<u8>>>;
    fn put(&self, key: &str, bytes: &[u8]) -> VtaResult<()>;
}

/// Verifies an artifact signature against trusted keys (`docs/15-security.md`).
pub trait SignatureVerifier {
    /// The scheme handled, e.g. `"minisign"` or `"cosign"`.
    fn scheme(&self) -> &str;
    /// Whether `signature` is valid for `data`.
    fn verify(&self, data: &[u8], signature: &str) -> VtaResult<bool>;
}

/// A way to materialize an environment view from the store
/// (`docs/09-store.md` §link-strategies).
pub trait LinkStrategy {
    /// The strategy name, e.g. `"reflink"`, `"hardlink"`, `"symlink"`, `"copy"`.
    fn name(&self) -> &str;
    /// Whether this strategy is usable for the given target directory.
    fn probe(&self, dir: &Path) -> bool;
    /// Link `src` to `dst`.
    fn link(&self, src: &Path, dst: &Path) -> VtaResult<()>;
}
