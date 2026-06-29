//! `vanta-sdk` — the provider-author SDK (`docs/22-provider-sdk.md`).
//!
//! Types and helpers a provider author uses to describe how to discover and
//! resolve a tool. For declarative providers this mirrors the registry manifest;
//! for WASM hooks these are the values a guest returns to the host. Kept
//! dependency-light so it can target `wasm32` guests.
#![forbid(unsafe_code)]

/// What a `resolve(version, os, arch)` hook returns for one platform.
#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactDesc {
    pub url: String,
    /// `tar.gz` / `tgz` / `zip` / `raw`.
    pub archive: String,
    pub sha256: String,
    /// Executables to expose (relative to the laid-out tree).
    pub bin: Vec<String>,
    /// Leading path components to strip on extraction.
    pub strip: u32,
}

/// The contract a provider author implements (declaratively or via a WASM hook).
pub trait ToolProvider {
    /// Available version strings (host applies ordering).
    fn list_versions(&self) -> Vec<String>;
    /// The artifact for a version on a platform, or `None` if unsupported.
    fn resolve(&self, version: &str, os: &str, arch: &str) -> Option<ArtifactDesc>;
}

/// Substitute `{version}`/`{os}`/`{arch}`/`{ext}` in a URL template — the helper
/// most declarative providers need.
pub fn render_url(template: &str, version: &str, os: &str, arch: &str, ext: &str) -> String {
    template
        .replace("{version}", version)
        .replace("{os}", os)
        .replace("{arch}", arch)
        .replace("{ext}", ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Demo;
    impl ToolProvider for Demo {
        fn list_versions(&self) -> Vec<String> {
            vec!["1.0.0".into(), "1.1.0".into()]
        }
        fn resolve(&self, version: &str, os: &str, arch: &str) -> Option<ArtifactDesc> {
            Some(ArtifactDesc {
                url: render_url(
                    "https://x.test/demo-{version}-{os}-{arch}.{ext}",
                    version,
                    os,
                    arch,
                    "tar.gz",
                ),
                archive: "tar.gz".into(),
                sha256: "abc".into(),
                bin: vec!["bin/demo".into()],
                strip: 1,
            })
        }
    }

    #[test]
    fn render_substitutes_all_placeholders() {
        assert_eq!(
            render_url(
                "a/{version}/{os}-{arch}.{ext}",
                "1.2.3",
                "macos",
                "arm64",
                "tar.gz"
            ),
            "a/1.2.3/macos-arm64.tar.gz"
        );
    }

    #[test]
    fn author_trait_roundtrip() {
        let p = Demo;
        assert_eq!(p.list_versions().len(), 2);
        let a = p.resolve("1.1.0", "linux", "x64").unwrap();
        assert_eq!(a.url, "https://x.test/demo-1.1.0-linux-x64.tar.gz");
        assert_eq!(a.strip, 1);
    }
}
