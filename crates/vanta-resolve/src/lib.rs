//! `vanta-resolve` — turn a [`Request`] into a deterministic [`Resolution`].
//!
//! Resolution reads the registry, filters versions by the request's constraint,
//! picks the maximum satisfying version by SemVer ordering, and renders a
//! per-platform artifact for every requested target. The result is fully pinned
//! and lockable. See `docs/06-resolution.md`.
//!
//! Most tools are independent leaves; inter-tool dependency graphs are resolved
//! where a provider declares them.
#![forbid(unsafe_code)]

use std::cmp::Ordering;
use vanta_core::{Area, Checksum, Platform, Request, Resolution, VersionReq, VtaError, VtaResult};
use vanta_registry::{Registry, VersionEntry};

/// Resolves requests against a registry.
pub struct Resolver<'a> {
    registry: &'a Registry,
}

impl<'a> Resolver<'a> {
    pub fn new(registry: &'a Registry) -> Resolver<'a> {
        Resolver { registry }
    }

    /// Resolve `request` for every platform in `targets`.
    pub fn resolve(&self, request: &Request, targets: &[Platform]) -> VtaResult<Resolution> {
        let entry = self.registry.tool(&request.tool).ok_or_else(|| {
            VtaError::new(Area::Res, 3, format!("unknown tool `{}`", request.tool))
        })?;

        let candidates: Vec<&VersionEntry> = entry
            .versions
            .iter()
            .filter(|v| !v.yanked && satisfies(&request.version, v))
            .collect();

        let chosen = candidates
            .iter()
            .copied()
            .max_by(|a, b| cmp_version(&a.version, &b.version))
            .ok_or_else(|| {
                VtaError::new(
                    Area::Res,
                    1,
                    format!(
                        "no version of `{}` satisfies `{}`",
                        request.tool, request.version
                    ),
                )
            })?;

        let mut per_platform = Vec::new();
        for platform in targets {
            if let Some(pc) = chosen.platforms.get(&platform.token()) {
                let checksum = Checksum {
                    algo: "sha256".to_string(),
                    value: pc.sha256.clone(),
                };
                let mut artifact =
                    entry
                        .provider
                        .render_artifact(&chosen.version, platform, checksum, pc.size);
                artifact.signature = pc.signature.clone();
                artifact.signature_key = entry.public_key.clone();
                per_platform.push((*platform, artifact));
            }
        }

        if per_platform.is_empty() {
            return Err(VtaError::new(
                Area::Res,
                5,
                format!(
                    "no artifact for `{}` {} on any requested platform",
                    request.tool, chosen.version
                ),
            ));
        }

        Ok(Resolution {
            tool: request.tool.clone(),
            version: chosen.version.clone(),
            provider: entry.provider.id.clone(),
            per_platform,
        })
    }
}

/// Pick the artifact for a specific platform out of a resolution.
pub fn artifact_for<'r>(
    resolution: &'r Resolution,
    platform: &Platform,
) -> Option<&'r vanta_core::Artifact> {
    resolution
        .per_platform
        .iter()
        .find(|(p, _)| p == platform)
        .map(|(_, a)| a)
}

/// Whether a version entry satisfies a request's constraint.
fn satisfies(req: &VersionReq, entry: &VersionEntry) -> bool {
    match req {
        VersionReq::Exact(s) => &entry.version == s,
        VersionReq::Prefix(p) => &entry.version == p || entry.version.starts_with(&format!("{p}.")),
        VersionReq::Latest => is_stable(entry),
        VersionReq::Lts => entry.lts,
        VersionReq::Channel(c) => entry.channel.as_deref() == Some(c.as_str()),
        VersionReq::Range(r) => sem_match(r, &entry.version),
        VersionReq::System => false,
        _ => false, // `VersionReq` is #[non_exhaustive]
    }
}

fn is_stable(entry: &VersionEntry) -> bool {
    let channel_ok = matches!(entry.channel.as_deref(), None | Some("stable"));
    let pre_ok = semver::Version::parse(&entry.version)
        .map(|v| v.pre.is_empty())
        .unwrap_or(true);
    channel_ok && pre_ok
}

fn sem_match(range: &str, version: &str) -> bool {
    match (
        semver::VersionReq::parse(range),
        semver::Version::parse(version),
    ) {
        (Ok(req), Ok(ver)) => req.matches(&ver),
        _ => false,
    }
}

/// Order two version strings: SemVer where both parse, else lexical.
fn cmp_version(a: &str, b: &str) -> Ordering {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        _ => a.cmp(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vanta_core::{Arch, Libc, Os};

    const SAMPLE: &str = r#"
[tools.node.provider]
id = "official/node"
tool = "node"
url_template = "https://nodejs.org/dist/v{version}/node-v{version}-{os}-{arch}.{ext}"
archive = "tar.gz"
bin = ["bin/node"]

[[tools.node.version]]
version = "24.5.0"
channel = "stable"
[tools.node.version.platforms."macos/aarch64"]
sha256 = "55"

[[tools.node.version]]
version = "24.6.0"
channel = "stable"
[tools.node.version.platforms."macos/aarch64"]
sha256 = "66"

[[tools.node.version]]
version = "25.0.0"
channel = "stable"
[tools.node.version.platforms."macos/aarch64"]
sha256 = "00"
"#;

    fn mac() -> Platform {
        Platform {
            os: Os::Macos,
            arch: Arch::Aarch64,
            libc: Libc::None,
        }
    }

    fn resolve(spec: &str) -> VtaResult<Resolution> {
        let reg = Registry::from_toml(SAMPLE).unwrap();
        let resolver = Resolver::new(&reg);
        resolver.resolve(&Request::parse(spec).unwrap(), &[mac()])
    }

    #[test]
    fn prefix_picks_newest_in_series() {
        assert_eq!(resolve("node@24").unwrap().version, "24.6.0");
    }

    #[test]
    fn two_component_prefix_pins_series() {
        assert_eq!(resolve("node@24.5").unwrap().version, "24.5.0");
    }

    #[test]
    fn latest_picks_global_newest() {
        assert_eq!(resolve("node@latest").unwrap().version, "25.0.0");
    }

    #[test]
    fn exact_pins() {
        assert_eq!(resolve("node@24.5.0").unwrap().version, "24.5.0");
    }

    #[test]
    fn range_constraint() {
        // >=24, <25 → newest is 24.6.0
        assert_eq!(resolve("node@>=24, <25").unwrap().version, "24.6.0");
    }

    #[test]
    fn renders_artifact_for_target() {
        let res = resolve("node@24").unwrap();
        let art = artifact_for(&res, &mac()).unwrap();
        assert_eq!(
            art.url,
            "https://nodejs.org/dist/v24.6.0/node-v24.6.0-macos-aarch64.tar.gz"
        );
        assert_eq!(art.checksum.value, "66");
    }

    #[test]
    fn unknown_tool_errors() {
        assert_eq!(resolve("python@3").unwrap_err().area, Area::Res);
    }

    #[test]
    fn no_match_errors() {
        let err = resolve("node@99").unwrap_err();
        assert_eq!(err.area, Area::Res);
        assert_eq!(err.number, 1);
    }
}
