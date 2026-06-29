//! `vanta-provider` — the declarative provider model.
//!
//! A provider describes how to turn a `version` + `Platform` into a concrete
//! [`Artifact`]: a URL template (with `{version}`/`{os}`/`{arch}`/`{ext}`
//! placeholders), the archive kind, the bin paths, and per-token name maps that
//! translate Vanta's canonical platform tokens into the upstream's spelling
//! (e.g. `macos`→`darwin`, `aarch64`→`arm64`). See `docs/07-providers.md` and
//! `docs/22-provider-sdk.md`.
//!
//! This is the declarative path (no code). Providers that need custom logic use a
//! sandboxed WASM hook ([`Sandbox`], see `docs/22-provider-sdk.md`).
#![forbid(unsafe_code)]

pub mod wasm;
pub use wasm::Sandbox;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use vanta_core::{Artifact, Checksum, Platform};

/// A declarative provider definition (one tool).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderDef {
    /// Provider id, e.g. `"official/node"`.
    pub id: String,
    /// The tool this provider serves.
    pub tool: String,
    /// URL template with `{version}`/`{os}`/`{arch}`/`{ext}` placeholders.
    pub url_template: String,
    /// Archive kind: `tar.gz` / `tgz` / `zip` / `raw`.
    pub archive: String,
    /// Components to strip when materializing (recorded for the store layout).
    #[serde(default)]
    pub strip: u32,
    /// Executables to expose (paths relative to the laid-out tree).
    #[serde(default)]
    pub bin: Vec<String>,
    /// Map a canonical OS token to the upstream spelling (`macos` → `darwin`).
    #[serde(default)]
    pub os_map: BTreeMap<String, String>,
    /// Map a canonical arch token to the upstream spelling (`aarch64` → `arm64`).
    #[serde(default)]
    pub arch_map: BTreeMap<String, String>,
}

impl ProviderDef {
    /// Render the artifact for `version` on `platform`, attaching `checksum`.
    /// `size` is optional metadata carried into the lock.
    pub fn render_artifact(
        &self,
        version: &str,
        platform: &Platform,
        checksum: Checksum,
        size: Option<u64>,
    ) -> Artifact {
        let os = self.map_os(platform);
        let arch = self.map_arch(platform);
        let url = self
            .url_template
            .replace("{version}", version)
            .replace("{os}", &os)
            .replace("{arch}", &arch)
            .replace("{ext}", ext_for(&self.archive));
        Artifact {
            url,
            mirrors: Vec::new(),
            archive: self.archive.clone(),
            size,
            checksum,
            signature: None,
            signature_key: None,
            bin: self.bin.clone(),
            strip: self.strip,
            store_key: None,
        }
    }

    fn map_os(&self, platform: &Platform) -> String {
        let key = platform.os.as_str();
        self.os_map
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    fn map_arch(&self, platform: &Platform) -> String {
        let key = platform.arch.as_str();
        self.arch_map
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }
}

/// The file extension implied by an archive kind (for `{ext}` substitution).
pub fn ext_for(archive: &str) -> &'static str {
    match archive {
        "tar.gz" | "tgz" => "tar.gz",
        "tar.xz" => "tar.xz",
        "zip" => "zip",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vanta_core::{Arch, Libc, Os};

    fn node_provider() -> ProviderDef {
        let mut os_map = BTreeMap::new();
        os_map.insert("macos".into(), "darwin".into());
        let mut arch_map = BTreeMap::new();
        arch_map.insert("aarch64".into(), "arm64".into());
        ProviderDef {
            id: "official/node".into(),
            tool: "node".into(),
            url_template: "https://nodejs.org/dist/v{version}/node-v{version}-{os}-{arch}.{ext}"
                .into(),
            archive: "tar.gz".into(),
            strip: 1,
            bin: vec!["bin/node".into()],
            os_map,
            arch_map,
        }
    }

    #[test]
    fn renders_url_with_token_maps() {
        let p = node_provider();
        let plat = Platform {
            os: Os::Macos,
            arch: Arch::Aarch64,
            libc: Libc::None,
        };
        let art = p.render_artifact(
            "24.6.0",
            &plat,
            Checksum {
                algo: "sha256".into(),
                value: "abc".into(),
            },
            Some(100),
        );
        assert_eq!(
            art.url,
            "https://nodejs.org/dist/v24.6.0/node-v24.6.0-darwin-arm64.tar.gz"
        );
        assert_eq!(art.archive, "tar.gz");
        assert_eq!(art.bin, vec!["bin/node".to_string()]);
        assert_eq!(art.checksum.value, "abc");
    }
}
