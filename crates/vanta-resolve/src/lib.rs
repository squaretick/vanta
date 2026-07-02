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
        let entry = self
            .registry
            .tool(&request.tool)
            .ok_or_else(|| self.unknown_tool_error(&request.tool))?;

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
                self.no_matching_version_error(&request.tool, &request.version.to_string(), entry)
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
                // C1: the per-artifact signing key is only trusted if the index
                // that carried it was authenticated against a pinned root
                // (transitive trust), or the key is itself pinned. Otherwise it
                // is attacker-influenceable, so we drop it (`None`) — the install
                // engine then treats the artifact as unsigned and, under a
                // signature-requiring policy, refuses it (fail-closed).
                artifact.signature_key = entry
                    .public_key
                    .as_deref()
                    .filter(|k| {
                        vanta_security::trust::artifact_key_is_trusted(
                            k,
                            self.registry.index_verified,
                            &self.registry.trusted_root_keys,
                        )
                    })
                    .map(str::to_string);
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

    /// Build the `unknown tool` error, enriched with close-match suggestions and
    /// the list of tools the registry actually knows, so a typo or a
    /// not-yet-supported tool produces an actionable message instead of a
    /// dead end.
    fn unknown_tool_error(&self, requested: &str) -> VtaError {
        let names: Vec<&str> = self.registry.tools.keys().map(String::as_str).collect();

        // "Did you mean": names within a small edit distance (scaled to the
        // requested length) or that contain the request as a substring, best
        // first, capped to a few.
        let want = requested.to_lowercase();
        let budget = (want.len() / 3).max(2);
        let mut scored: Vec<(usize, &str)> = names
            .iter()
            .filter_map(|n| {
                let d = levenshtein(&want, &n.to_lowercase());
                let contains = n.to_lowercase().contains(&want) || want.contains(&n.to_lowercase());
                if d <= budget || contains {
                    Some((if contains { 0 } else { d }, *n))
                } else {
                    None
                }
            })
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
        let suggestions: Vec<&str> = scored.into_iter().take(3).map(|(_, n)| n).collect();

        let mut msg = format!("unknown tool `{requested}`");
        // Tools people ask for that have no official prebuilt binaries: point
        // at the canonical installer instead of a dead end.
        let hint = match requested {
            "rust" | "rustc" | "cargo" => {
                Some("rust is distributed via rustup (https://rustup.rs); a vanta source-build provider is planned")
            }
            "ruby" => Some(
                "ruby publishes no official prebuilt binaries; a vanta source-build provider is planned",
            ),
            "java" | "jdk" => Some("try `vanta search` for a packaged JDK, or use SDKMAN meanwhile"),
            _ => None,
        };
        if let Some(h) = hint {
            msg.push_str(&format!("\n  note: {h}"));
        }
        if !suggestions.is_empty() {
            msg.push_str(&format!("\n  did you mean: {}?", suggestions.join(", ")));
        }
        if names.is_empty() {
            msg.push_str(
                "\n  the registry index is empty (offline, or no registry configured)",
            );
        } else {
            // Cap the listing so a large future registry stays readable.
            let shown = 20;
            let list = names
                .iter()
                .take(shown)
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
            msg.push_str(&format!("\n  available tools: {list}"));
            if names.len() > shown {
                msg.push_str(&format!(", … (+{} more)", names.len() - shown));
            }
            msg.push_str("\n  run `vanta search <term>` to search the registry");
        }
        VtaError::new(Area::Res, 3, msg)
    }

    /// Build the `no version satisfies` error, listing the versions the registry
    /// actually carries (newest first) so the user can pick a real one instead
    /// of guessing. A stale index (e.g. asking for `24` when only `22`/`20` are
    /// seeded) then reads as an obvious mismatch.
    fn no_matching_version_error(
        &self,
        tool: &str,
        want: &str,
        entry: &vanta_registry::ToolEntry,
    ) -> VtaError {
        let mut available: Vec<&str> = entry
            .versions
            .iter()
            .filter(|v| !v.yanked)
            .map(|v| v.version.as_str())
            .collect();
        available.sort_by(|a, b| cmp_version(b, a)); // newest first

        let mut msg = format!("no version of `{tool}` satisfies `{want}`");
        if available.is_empty() {
            msg.push_str("\n  the registry lists no (non-yanked) versions for this tool");
        } else {
            let shown = 10;
            let list = available
                .iter()
                .take(shown)
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
            msg.push_str(&format!("\n  available: {list}"));
            if available.len() > shown {
                msg.push_str(&format!(", … (+{} more)", available.len() - shown));
            }
            msg.push_str(&format!(
                "\n  try `vanta add {tool}@{}` or widen the constraint",
                available[0]
            ));
        }
        VtaError::new(Area::Res, 1, msg)
    }
}

/// Levenshtein edit distance between two strings (for "did you mean").
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
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

    // C1: a registry index carries a per-tool `public_key`. It must only be
    // propagated as a trusted signing key when the index was authenticated
    // against a pinned root (or the key is itself pinned).
    const SIGNED_SAMPLE: &str = r#"
[tools.node]
public_key = "untrusted comment: attacker\nRWQfattackerkeytext=="

[tools.node.provider]
id = "official/node"
tool = "node"
url_template = "https://nodejs.org/dist/v{version}/node-v{version}-{os}-{arch}.{ext}"
archive = "tar.gz"
bin = ["bin/node"]

[[tools.node.version]]
version = "24.6.0"
channel = "stable"
[tools.node.version.platforms."macos/aarch64"]
sha256 = "66"
signature = "untrusted comment: sig\nRWQfsig=="
"#;

    #[test]
    fn attacker_key_with_unsigned_index_is_dropped() {
        // Index NOT verified against a pinned root → the attacker-supplied
        // signing key must not be trusted.
        let reg = Registry::from_toml(SIGNED_SAMPLE).unwrap();
        assert!(!reg.index_verified);
        let resolver = Resolver::new(&reg);
        let res = resolver
            .resolve(&Request::parse("node@24.6.0").unwrap(), &[mac()])
            .unwrap();
        let art = artifact_for(&res, &mac()).unwrap();
        assert_eq!(art.signature_key, None); // untrusted key rejected
    }

    #[test]
    fn key_from_verified_index_is_trusted() {
        // Index verified against a pinned root → its key is trusted transitively.
        let mut reg = Registry::from_toml(SIGNED_SAMPLE).unwrap();
        reg.index_verified = true;
        let resolver = Resolver::new(&reg);
        let res = resolver
            .resolve(&Request::parse("node@24.6.0").unwrap(), &[mac()])
            .unwrap();
        let art = artifact_for(&res, &mac()).unwrap();
        assert!(art.signature_key.is_some());
    }
}
