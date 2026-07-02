//! `vanta-registry` — the index of which tools exist and how to get them.
//!
//! For each tool the registry holds a [`ProviderDef`] plus the list of known
//! versions, each with per-platform checksums. The resolver reads this to pick a
//! version and render an artifact (`docs/06-resolution.md`, `docs/07-providers.md`).
//!
//! An index is loaded from a TOML document (a local file or an HTTP response).
//! Signed distribution, caching, and TUF-style metadata roles are specified in
//! `docs/15-security.md` and `docs/26-registry-and-metadata-reference.md`.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use vanta_core::{Area, VtaError, VtaResult};
use vanta_provider::ProviderDef;

/// A parsed registry index.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Registry {
    /// tool name → entry.
    #[serde(default)]
    pub tools: BTreeMap<String, ToolEntry>,

    /// Whether this index was authenticated against a pinned trust root (audit
    /// C1). Set by the loader after a detached-signature check; never read from
    /// the index document itself (hence `#[serde(skip)]`). When `true`, the
    /// per-tool `public_key` values may be trusted transitively.
    #[serde(skip)]
    pub index_verified: bool,

    /// The pinned root public-key texts the loader checked this index against.
    /// Carried so the resolver can apply the "key is itself pinned" branch of
    /// the trust model. Never sourced from the index (hence `#[serde(skip)]`).
    #[serde(skip)]
    pub trusted_root_keys: Vec<String>,
}

/// One tool's provider and version list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolEntry {
    pub provider: ProviderDef,
    #[serde(default, rename = "version")]
    pub versions: Vec<VersionEntry>,
    #[serde(default)]
    pub summary: Option<String>,
    /// Trusted minisign public key used to verify this tool's signatures.
    #[serde(default)]
    pub public_key: Option<String>,
}

/// A known version with its per-platform checksums.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionEntry {
    pub version: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub lts: bool,
    #[serde(default)]
    pub yanked: bool,
    /// platform token → checksum.
    #[serde(default)]
    pub platforms: BTreeMap<String, PlatformChecksum>,
}

/// The checksum (and size) of an artifact on one platform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlatformChecksum {
    pub sha256: String,
    #[serde(default)]
    pub size: Option<u64>,
    /// Detached minisign signature (`.minisig` contents) over the artifact.
    #[serde(default)]
    pub signature: Option<String>,
}

impl Registry {
    /// Parse a registry index from TOML.
    pub fn from_toml(src: &str) -> VtaResult<Registry> {
        toml::from_str(src).map_err(|e| VtaError::new(Area::Reg, 2, format!("parse registry: {e}")))
    }

    /// Load a registry index file.
    pub fn load_file(path: &Path) -> VtaResult<Registry> {
        let src = fs::read_to_string(path).map_err(|e| {
            VtaError::new(Area::Reg, 1, format!("cannot read {}: {e}", path.display()))
        })?;
        Registry::from_toml(&src)
    }

    /// Look up a tool entry.
    pub fn tool(&self, name: &str) -> Option<&ToolEntry> {
        self.tools.get(name)
    }

    /// All known (non-yanked) versions of a tool, in registry order.
    pub fn versions(&self, name: &str) -> Vec<&VersionEntry> {
        self.tools
            .get(name)
            .map(|e| e.versions.iter().filter(|v| !v.yanked).collect())
            .unwrap_or_default()
    }

    /// Substring search over tool names and summaries.
    pub fn search(&self, query: &str) -> Vec<&str> {
        let q = query.to_lowercase();
        self.tools
            .iter()
            .filter(|(name, entry)| {
                name.to_lowercase().contains(&q)
                    || entry
                        .summary
                        .as_deref()
                        .map(|s| s.to_lowercase().contains(&q))
                        .unwrap_or(false)
            })
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Merge another registry over this one (higher-priority overlay wins per
    /// tool name) — used to layer a private registry over the official one
    /// (`docs/14-enterprise.md`).
    pub fn overlay(&mut self, other: Registry) {
        self.tools.extend(other.tools);
    }

    /// The default, empty index used when no registry is configured. A registry
    /// is supplied via the `$VANTA_REGISTRY` file/URL or a `[registries]` entry
    /// in the configuration (`docs/07-providers.md`, `docs/14-enterprise.md`).
    pub fn builtin() -> Registry {
        Registry::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[tools.node.provider]
id = "official/node"
tool = "node"
url_template = "https://nodejs.org/dist/v{version}/node-v{version}-{os}-{arch}.{ext}"
archive = "tar.gz"
strip = 1
bin = ["bin/node"]
[tools.node.provider.os_map]
macos = "darwin"
[tools.node.provider.arch_map]
aarch64 = "arm64"

[[tools.node.version]]
version = "24.6.0"
channel = "stable"
[tools.node.version.platforms."macos/aarch64"]
sha256 = "aaaa"

[[tools.node.version]]
version = "24.5.0"
channel = "stable"
[tools.node.version.platforms."macos/aarch64"]
sha256 = "bbbb"
"#;

    #[test]
    fn parses_and_queries() {
        let reg = Registry::from_toml(SAMPLE).unwrap();
        let entry = reg.tool("node").unwrap();
        assert_eq!(entry.provider.id, "official/node");
        assert_eq!(reg.versions("node").len(), 2);
        assert_eq!(reg.search("nod"), vec!["node"]);
        assert!(reg.tool("python").is_none());
    }

    #[test]
    fn parses_archive_map_and_defaults_empty() {
        // Without an archive_map table the field defaults to empty (older
        // indexes keep parsing).
        let reg = Registry::from_toml(SAMPLE).unwrap();
        assert!(reg.tool("node").unwrap().provider.archive_map.is_empty());

        let with_map = r#"
[tools.gh.provider]
id = "official/gh"
tool = "gh"
url_template = "https://example.com/gh_{version}_{os}_{arch}.{ext}"
archive = "tar.gz"
strip = 1
bin = ["bin/gh"]
[tools.gh.provider.archive_map]
macos = "zip"

[[tools.gh.version]]
version = "2.63.0"
channel = "stable"
[tools.gh.version.platforms."macos/aarch64"]
sha256 = "cccc"
"#;
        let reg = Registry::from_toml(with_map).unwrap();
        let provider = &reg.tool("gh").unwrap().provider;
        assert_eq!(provider.archive_map.get("macos").map(String::as_str), Some("zip"));
    }
}
