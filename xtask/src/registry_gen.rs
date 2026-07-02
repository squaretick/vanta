//! `cargo xtask registry-gen` — build and minisign-sign the official Vanta tool
//! registry from real upstream checksums.
//!
//! For each seed tool the version list is discovered from the upstream index
//! ([`VersionSpec`]: nodejs.org/go.dev/releases.hashicorp.com/GitHub releases —
//! full history where checksum manifests exist, a logged latest-N cap where
//! hashing requires downloading each asset). Checksums come from the
//! publishers' manifests (`SHASUMS256.txt`, `*_SHA256SUMS`, `*_checksums.txt`,
//! `.sha256`/`.sha256sum` sidecars, go's `?mode=json`) or, as a fallback, from
//! hashing the downloaded asset. Each asset is mapped to Vanta's platform
//! target tokens and emitted as `registry/registry.toml` in the exact schema
//! [`vanta_registry::Registry::from_toml`] parses (override the output dir
//! with `VANTA_REGISTRY_OUT`). The canonical index bytes are then signed with
//! the pinned root key → `registry/registry.toml.minisig`.
//!
//! Generation is resilient: a tool/version/platform that cannot be resolved is
//! logged and skipped rather than aborting the whole run. The output is
//! deterministic (sorted), so re-running is idempotent.
//!
//! `cargo xtask keygen <secret-path>` mints a minisign-compatible root keypair:
//! the secret (unencrypted dev key) is written to `<secret-path>` and the public
//! key text is printed to stdout (to be baked into
//! `vanta_security::trust::COMPILED_IN_ROOT_KEYS`).

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use vanta_core::{Checksum, Platform};
use vanta_net::Downloader;
use vanta_provider::ProviderDef;

/// How to obtain an artifact's sha256 (and optionally size) for one platform.
#[derive(Clone, Copy)]
enum ChecksumSource {
    /// Hash the downloaded asset (universal fallback; used for GitHub releases
    /// that publish no checksum manifest). Expensive: full download per asset.
    Download,
    /// node: parse `https://nodejs.org/dist/v{version}/SHASUMS256.txt`.
    NodeShasums,
    /// go: parse `https://go.dev/dl/?mode=json` (carries sha256 + size).
    GoJson,
    /// Fetch the `<asset-url><suffix>` sidecar (e.g. `.sha256`,
    /// `.sha256sum`) whose first token is the hex digest.
    Sidecar(&'static str),
    /// One checksum-manifest per version at the given URL template
    /// (`{version}` substituted): lines of `<hash>  <filename>`. Covers gh
    /// `*_checksums.txt`, bun `SHASUMS256.txt`, hashicorp `*_SHA256SUMS`,
    /// just `SHA256SUMS`, fzf `*_checksums.txt`.
    VersionSums(&'static str),
}

/// Where the list of versions for a tool comes from.
enum VersionSpec {
    /// Static pins (used where no cheap upstream index exists).
    Pinned(&'static [&'static str]),
    /// `https://nodejs.org/dist/index.json` — every release with
    /// `major >= min_major` (full history, oldest to newest).
    NodeIndex { min_major: u64 },
    /// `https://go.dev/dl/?mode=json&include=all` — every stable `go1.X.Y`
    /// with `minor >= min_minor`.
    GoIndex { min_minor: u64 },
    /// `https://releases.hashicorp.com/{product}/index.json` — every
    /// non-prerelease version `>= min`.
    HashicorpIndex {
        product: &'static str,
        min: (u64, u64),
    },
    /// GitHub releases (newest first, one page of 100): non-draft,
    /// non-prerelease tags with `tag_prefix` stripped; keep at most `latest`
    /// and only versions `>= min` when set. Used where checksums require
    /// per-asset work, to bound generation cost; the cap is logged.
    GitHub {
        repo: &'static str,
        tag_prefix: &'static str,
        latest: usize,
        min: Option<&'static str>,
    },
}

/// Parse a `1.2.3`-ish version into numeric fields (missing → 0; suffixes
/// after the numeric core are ignored).
fn parse_ver(v: &str) -> (u64, u64, u64) {
    let mut it = v.split(['.', '-', '+']).map(|p| p.parse::<u64>().ok());
    let a = it.next().flatten().unwrap_or(0);
    let b = it.next().flatten().unwrap_or(0);
    let c = it.next().flatten().unwrap_or(0);
    (a, b, c)
}

/// `true` when `v` looks like a plain stable version (`N.N` / `N.N.N`, no
/// prerelease suffix).
fn is_stable_ver(v: &str) -> bool {
    !v.is_empty()
        && v.split('.').count() >= 2
        && v.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

impl VersionSpec {
    /// Resolve to a concrete newest-first version list.
    fn resolve(&self, dl: &Downloader, tmp: &Path) -> Result<Vec<String>, String> {
        match self {
            VersionSpec::Pinned(list) => Ok(list.iter().map(|s| s.to_string()).collect()),
            VersionSpec::NodeIndex { min_major } => {
                let text = fetch_text(
                    dl,
                    "https://nodejs.org/dist/index.json",
                    tmp,
                    32 * 1024 * 1024,
                )?;
                let v: serde_json::Value =
                    serde_json::from_str(&text).map_err(|e| format!("node index: {e}"))?;
                let mut out = Vec::new();
                for rel in v.as_array().ok_or("node index not an array")? {
                    if let Some(ver) = rel.get("version").and_then(|s| s.as_str()) {
                        let ver = ver.trim_start_matches('v');
                        if is_stable_ver(ver) && parse_ver(ver).0 >= *min_major {
                            out.push(ver.to_string());
                        }
                    }
                }
                Ok(out)
            }
            VersionSpec::GoIndex { min_minor } => {
                let text = fetch_text(
                    dl,
                    "https://go.dev/dl/?mode=json&include=all",
                    tmp,
                    32 * 1024 * 1024,
                )?;
                let v: serde_json::Value =
                    serde_json::from_str(&text).map_err(|e| format!("go index: {e}"))?;
                let mut out = Vec::new();
                for rel in v.as_array().ok_or("go index not an array")? {
                    if rel.get("stable").and_then(|s| s.as_bool()) != Some(true) {
                        continue;
                    }
                    if let Some(ver) = rel.get("version").and_then(|s| s.as_str()) {
                        let ver = ver.trim_start_matches("go");
                        if is_stable_ver(ver) && parse_ver(ver).1 >= *min_minor {
                            out.push(ver.to_string());
                        }
                    }
                }
                Ok(out)
            }
            VersionSpec::HashicorpIndex { product, min } => {
                let url = format!("https://releases.hashicorp.com/{product}/index.json");
                let text = fetch_text(dl, &url, tmp, 32 * 1024 * 1024)?;
                let v: serde_json::Value =
                    serde_json::from_str(&text).map_err(|e| format!("{product} index: {e}"))?;
                let versions = v
                    .get("versions")
                    .and_then(|m| m.as_object())
                    .ok_or("hashicorp index missing versions")?;
                let mut out: Vec<String> = versions
                    .keys()
                    .filter(|k| is_stable_ver(k))
                    .filter(|k| {
                        let (a, b, _) = parse_ver(k);
                        (a, b) >= *min
                    })
                    .cloned()
                    .collect();
                out.sort_by(|a, b| parse_ver(b).cmp(&parse_ver(a))); // newest first
                Ok(out)
            }
            VersionSpec::GitHub {
                repo,
                tag_prefix,
                latest,
                min,
            } => {
                let url = format!("https://api.github.com/repos/{repo}/releases?per_page=100");
                let text = fetch_text(dl, &url, tmp, 32 * 1024 * 1024)?;
                let v: serde_json::Value =
                    serde_json::from_str(&text).map_err(|e| format!("{repo} releases: {e}"))?;
                let min_v = min.map(parse_ver);
                let mut out = Vec::new();
                for rel in v.as_array().ok_or("releases not an array")? {
                    let draft = rel.get("draft").and_then(|b| b.as_bool()).unwrap_or(false);
                    let pre = rel
                        .get("prerelease")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false);
                    if draft || pre {
                        continue;
                    }
                    let Some(tag) = rel.get("tag_name").and_then(|s| s.as_str()) else {
                        continue;
                    };
                    let Some(ver) = tag.strip_prefix(tag_prefix) else {
                        continue;
                    };
                    if !is_stable_ver(ver) {
                        continue;
                    }
                    if let Some(m) = min_v {
                        if parse_ver(ver) < m {
                            continue;
                        }
                    }
                    out.push(ver.to_string());
                    if out.len() >= *latest {
                        eprintln!(
                            "  note {repo}: capped at latest {latest} versions (checksums need per-asset work)"
                        );
                        break;
                    }
                }
                Ok(out)
            }
        }
    }
}

/// A seed tool: its declarative provider plus the versions/platforms to seed.
struct ToolSpec {
    name: &'static str,
    summary: &'static str,
    archive: &'static str,
    /// Per-OS archive-kind overrides (canonical OS token → kind), for
    /// upstreams that ship e.g. tar.gz on linux but zip on macOS.
    archive_map: &'static [(&'static str, &'static str)],
    strip: u32,
    bin: &'static [&'static str],
    url_template: &'static str,
    os_map: &'static [(&'static str, &'static str)],
    arch_map: &'static [(&'static str, &'static str)],
    versions: VersionSpec,
    platforms: &'static [&'static str],
    checksum: ChecksumSource,
}

impl ToolSpec {
    fn provider(&self) -> ProviderDef {
        ProviderDef {
            id: format!("official/{}", self.name),
            tool: self.name.to_string(),
            url_template: self.url_template.to_string(),
            archive: self.archive.to_string(),
            archive_map: self
                .archive_map
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            strip: self.strip,
            bin: self.bin.iter().map(|s| s.to_string()).collect(),
            os_map: self
                .os_map
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            arch_map: self
                .arch_map
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }
}

/// The seed list: real upstream tools. Version lists come from upstream
/// indexes ([`VersionSpec`]) — full history where the publisher ships checksum
/// manifests, a logged latest-N cap where checksums require hashing each
/// downloaded asset.
fn specs() -> Vec<ToolSpec> {
    const UNIX4: &[&str] = &[
        "linux/x86_64/gnu",
        "linux/aarch64/gnu",
        "macos/x86_64",
        "macos/aarch64",
    ];
    vec![
        ToolSpec {
            name: "node",
            summary: "Node.js JavaScript runtime",
            archive: "tar.gz",
            archive_map: &[],
            strip: 1,
            bin: &["bin/node", "bin/npm", "bin/npx"],
            url_template:
                "https://nodejs.org/dist/v{version}/node-v{version}-{os}-{arch}.{ext}",
            os_map: &[("macos", "darwin"), ("linux", "linux")],
            arch_map: &[("x86_64", "x64"), ("aarch64", "arm64")],
            versions: VersionSpec::NodeIndex { min_major: 18 },
            platforms: UNIX4,
            checksum: ChecksumSource::NodeShasums,
        },
        ToolSpec {
            name: "go",
            summary: "The Go programming language toolchain",
            archive: "tar.gz",
            archive_map: &[],
            strip: 1,
            bin: &["bin/go", "bin/gofmt"],
            url_template: "https://go.dev/dl/go{version}.{os}-{arch}.{ext}",
            os_map: &[("macos", "darwin")],
            arch_map: &[("x86_64", "amd64"), ("aarch64", "arm64")],
            versions: VersionSpec::GoIndex { min_minor: 21 },
            platforms: UNIX4,
            checksum: ChecksumSource::GoJson,
        },
        ToolSpec {
            name: "python",
            summary: "CPython (python-build-standalone, install_only builds)",
            archive: "tar.gz",
            archive_map: &[],
            strip: 1,
            bin: &["bin/python3"],
            url_template:
                "https://github.com/astral-sh/python-build-standalone/releases/download/20241016/cpython-{version}-{arch}-{os}-install_only.{ext}",
            os_map: &[("macos", "apple-darwin"), ("linux", "unknown-linux-gnu")],
            arch_map: &[],
            versions: VersionSpec::Pinned(&["3.12.7+20241016", "3.11.10+20241016"]),
            platforms: UNIX4,
            checksum: ChecksumSource::Sidecar(".sha256"),
        },
        ToolSpec {
            name: "uv",
            summary: "uv — an extremely fast Python package and project manager",
            archive: "tar.gz",
            archive_map: &[],
            strip: 1,
            bin: &["uv", "uvx"],
            url_template:
                "https://github.com/astral-sh/uv/releases/download/{version}/uv-{arch}-{os}.{ext}",
            os_map: &[("macos", "apple-darwin"), ("linux", "unknown-linux-gnu")],
            arch_map: &[],
            versions: VersionSpec::GitHub {
                repo: "astral-sh/uv",
                tag_prefix: "",
                latest: 20,
                min: Some("0.5.0"),
            },
            platforms: UNIX4,
            checksum: ChecksumSource::Sidecar(".sha256"),
        },
        ToolSpec {
            name: "ripgrep",
            summary: "ripgrep (rg) — recursive line-oriented search",
            archive: "tar.gz",
            archive_map: &[],
            strip: 1,
            bin: &["rg"],
            url_template:
                "https://github.com/BurntSushi/ripgrep/releases/download/{version}/ripgrep-{version}-{arch}-{os}.{ext}",
            os_map: &[("macos", "apple-darwin"), ("linux", "unknown-linux-musl")],
            arch_map: &[],
            versions: VersionSpec::GitHub {
                repo: "BurntSushi/ripgrep",
                tag_prefix: "",
                latest: 4,
                min: None,
            },
            platforms: UNIX4,
            checksum: ChecksumSource::Download,
        },
        ToolSpec {
            name: "fd",
            summary: "fd — a fast, user-friendly alternative to find",
            archive: "tar.gz",
            archive_map: &[],
            strip: 1,
            bin: &["fd"],
            url_template:
                "https://github.com/sharkdp/fd/releases/download/v{version}/fd-v{version}-{arch}-{os}.{ext}",
            os_map: &[("macos", "apple-darwin"), ("linux", "unknown-linux-musl")],
            arch_map: &[],
            versions: VersionSpec::GitHub {
                repo: "sharkdp/fd",
                tag_prefix: "v",
                latest: 4,
                min: None,
            },
            platforms: UNIX4,
            checksum: ChecksumSource::Download,
        },
        ToolSpec {
            name: "jq",
            summary: "jq — command-line JSON processor",
            archive: "raw",
            archive_map: &[],
            strip: 0,
            bin: &["jq"],
            url_template:
                "https://github.com/jqlang/jq/releases/download/jq-{version}/jq-{os}-{arch}",
            os_map: &[],
            arch_map: &[("x86_64", "amd64"), ("aarch64", "arm64")],
            versions: VersionSpec::GitHub {
                repo: "jqlang/jq",
                tag_prefix: "jq-",
                latest: 3,
                min: None,
            },
            platforms: UNIX4,
            checksum: ChecksumSource::Download,
        },
        ToolSpec {
            name: "terraform",
            summary: "HashiCorp Terraform — infrastructure as code",
            archive: "zip",
            archive_map: &[],
            strip: 0,
            bin: &["terraform"],
            url_template:
                "https://releases.hashicorp.com/terraform/{version}/terraform_{version}_{os}_{arch}.{ext}",
            os_map: &[("macos", "darwin")],
            arch_map: &[("x86_64", "amd64"), ("aarch64", "arm64")],
            versions: VersionSpec::HashicorpIndex {
                product: "terraform",
                min: (1, 3),
            },
            platforms: UNIX4,
            checksum: ChecksumSource::VersionSums(
                "https://releases.hashicorp.com/terraform/{version}/terraform_{version}_SHA256SUMS",
            ),
        },
        ToolSpec {
            name: "gh",
            summary: "GitHub CLI",
            archive: "tar.gz",
            // gh ships tar.gz on linux but zip on macOS.
            archive_map: &[("macos", "zip")],
            strip: 1,
            bin: &["bin/gh"],
            url_template:
                "https://github.com/cli/cli/releases/download/v{version}/gh_{version}_{os}_{arch}.{ext}",
            os_map: &[("macos", "macOS")],
            arch_map: &[("x86_64", "amd64"), ("aarch64", "arm64")],
            versions: VersionSpec::GitHub {
                repo: "cli/cli",
                tag_prefix: "v",
                latest: 15,
                min: Some("2.40.0"),
            },
            platforms: UNIX4,
            checksum: ChecksumSource::VersionSums(
                "https://github.com/cli/cli/releases/download/v{version}/gh_{version}_checksums.txt",
            ),
        },
        ToolSpec {
            name: "pnpm",
            summary: "pnpm — fast, disk-efficient package manager",
            archive: "raw",
            archive_map: &[],
            strip: 0,
            bin: &["pnpm"],
            url_template:
                "https://github.com/pnpm/pnpm/releases/download/v{version}/pnpm-{os}-{arch}",
            os_map: &[("linux", "linuxstatic"), ("macos", "macos")],
            arch_map: &[("x86_64", "x64"), ("aarch64", "arm64")],
            versions: VersionSpec::GitHub {
                repo: "pnpm/pnpm",
                tag_prefix: "v",
                latest: 5,
                min: Some("9.0.0"),
            },
            platforms: UNIX4,
            checksum: ChecksumSource::Download,
        },
        ToolSpec {
            name: "deno",
            summary: "Deno — a secure JavaScript/TypeScript runtime",
            archive: "zip",
            archive_map: &[],
            strip: 0,
            bin: &["deno"],
            url_template:
                "https://github.com/denoland/deno/releases/download/v{version}/deno-{arch}-{os}.{ext}",
            os_map: &[("macos", "apple-darwin"), ("linux", "unknown-linux-gnu")],
            arch_map: &[],
            versions: VersionSpec::GitHub {
                repo: "denoland/deno",
                tag_prefix: "v",
                latest: 10,
                min: Some("2.0.0"),
            },
            platforms: UNIX4,
            checksum: ChecksumSource::Sidecar(".sha256sum"),
        },
        ToolSpec {
            name: "bun",
            summary: "Bun — fast all-in-one JavaScript runtime & toolkit",
            archive: "zip",
            archive_map: &[],
            strip: 1,
            bin: &["bun"],
            url_template:
                "https://github.com/oven-sh/bun/releases/download/bun-v{version}/bun-{os}-{arch}.{ext}",
            os_map: &[("macos", "darwin")],
            arch_map: &[("x86_64", "x64")],
            versions: VersionSpec::GitHub {
                repo: "oven-sh/bun",
                tag_prefix: "bun-v",
                latest: 10,
                min: None,
            },
            platforms: UNIX4,
            checksum: ChecksumSource::VersionSums(
                "https://github.com/oven-sh/bun/releases/download/bun-v{version}/SHASUMS256.txt",
            ),
        },
        ToolSpec {
            name: "kubectl",
            summary: "kubectl — the Kubernetes command-line tool",
            archive: "raw",
            archive_map: &[],
            strip: 0,
            bin: &["kubectl"],
            url_template: "https://dl.k8s.io/release/v{version}/bin/{os}/{arch}/kubectl",
            os_map: &[("macos", "darwin")],
            arch_map: &[("x86_64", "amd64"), ("aarch64", "arm64")],
            versions: VersionSpec::GitHub {
                repo: "kubernetes/kubernetes",
                tag_prefix: "v",
                latest: 30,
                min: Some("1.28.0"),
            },
            platforms: UNIX4,
            checksum: ChecksumSource::Sidecar(".sha256"),
        },
        ToolSpec {
            name: "helm",
            summary: "Helm — the Kubernetes package manager",
            archive: "tar.gz",
            archive_map: &[],
            strip: 1,
            bin: &["helm"],
            url_template: "https://get.helm.sh/helm-v{version}-{os}-{arch}.{ext}",
            os_map: &[("macos", "darwin")],
            arch_map: &[("x86_64", "amd64"), ("aarch64", "arm64")],
            versions: VersionSpec::GitHub {
                repo: "helm/helm",
                tag_prefix: "v",
                latest: 12,
                min: Some("3.12.0"),
            },
            platforms: UNIX4,
            checksum: ChecksumSource::Sidecar(".sha256sum"),
        },
        ToolSpec {
            name: "just",
            summary: "just — a handy command runner",
            archive: "tar.gz",
            archive_map: &[],
            strip: 0,
            bin: &["just"],
            url_template:
                "https://github.com/casey/just/releases/download/{version}/just-{version}-{arch}-{os}.{ext}",
            os_map: &[("macos", "apple-darwin"), ("linux", "unknown-linux-musl")],
            arch_map: &[],
            versions: VersionSpec::GitHub {
                repo: "casey/just",
                tag_prefix: "",
                latest: 5,
                min: None,
            },
            platforms: UNIX4,
            checksum: ChecksumSource::VersionSums(
                "https://github.com/casey/just/releases/download/{version}/SHA256SUMS",
            ),
        },
        ToolSpec {
            name: "fzf",
            summary: "fzf — a command-line fuzzy finder",
            archive: "tar.gz",
            // fzf ships tar.gz on linux but zip on macOS.
            archive_map: &[("macos", "zip")],
            strip: 0,
            bin: &["fzf"],
            url_template:
                "https://github.com/junegunn/fzf/releases/download/v{version}/fzf-{version}-{os}_{arch}.{ext}",
            os_map: &[("macos", "darwin")],
            arch_map: &[("x86_64", "amd64"), ("aarch64", "arm64")],
            versions: VersionSpec::GitHub {
                repo: "junegunn/fzf",
                tag_prefix: "v",
                latest: 5,
                min: None,
            },
            platforms: UNIX4,
            checksum: ChecksumSource::VersionSums(
                "https://github.com/junegunn/fzf/releases/download/v{version}/fzf_{version}_checksums.txt",
            ),
        },
        ToolSpec {
            name: "yq",
            summary: "yq — portable command-line YAML/JSON/XML processor",
            archive: "raw",
            archive_map: &[],
            strip: 0,
            bin: &["yq"],
            url_template:
                "https://github.com/mikefarah/yq/releases/download/v{version}/yq_{os}_{arch}",
            os_map: &[("macos", "darwin")],
            arch_map: &[("x86_64", "amd64"), ("aarch64", "arm64")],
            versions: VersionSpec::GitHub {
                repo: "mikefarah/yq",
                tag_prefix: "v",
                latest: 5,
                min: None,
            },
            platforms: UNIX4,
            checksum: ChecksumSource::Download,
        },
        ToolSpec {
            name: "zoxide",
            summary: "zoxide — a smarter cd command",
            archive: "tar.gz",
            archive_map: &[],
            strip: 0,
            bin: &["zoxide"],
            url_template:
                "https://github.com/ajeetdsouza/zoxide/releases/download/v{version}/zoxide-{version}-{arch}-{os}.{ext}",
            os_map: &[("macos", "apple-darwin"), ("linux", "unknown-linux-musl")],
            arch_map: &[],
            versions: VersionSpec::GitHub {
                repo: "ajeetdsouza/zoxide",
                tag_prefix: "v",
                latest: 4,
                min: None,
            },
            platforms: UNIX4,
            checksum: ChecksumSource::Download,
        },
        ToolSpec {
            name: "bat",
            summary: "bat — a cat clone with wings",
            archive: "tar.gz",
            archive_map: &[],
            strip: 1,
            bin: &["bat"],
            url_template:
                "https://github.com/sharkdp/bat/releases/download/v{version}/bat-v{version}-{arch}-{os}.{ext}",
            os_map: &[("macos", "apple-darwin"), ("linux", "unknown-linux-gnu")],
            arch_map: &[],
            versions: VersionSpec::GitHub {
                repo: "sharkdp/bat",
                tag_prefix: "v",
                latest: 4,
                min: None,
            },
            platforms: UNIX4,
            checksum: ChecksumSource::Download,
        },
        ToolSpec {
            name: "delta",
            summary: "delta — a syntax-highlighting pager for git and diff",
            archive: "tar.gz",
            archive_map: &[],
            strip: 1,
            bin: &["delta"],
            url_template:
                "https://github.com/dandavison/delta/releases/download/{version}/delta-{version}-{arch}-{os}.{ext}",
            os_map: &[("macos", "apple-darwin"), ("linux", "unknown-linux-gnu")],
            arch_map: &[],
            versions: VersionSpec::GitHub {
                repo: "dandavison/delta",
                tag_prefix: "",
                latest: 4,
                min: None,
            },
            platforms: UNIX4,
            checksum: ChecksumSource::Download,
        },
    ]
}

/// Resolved checksum for one platform.
struct PlatformResult {
    sha256: String,
    size: Option<u64>,
}

/// Run `cargo xtask registry-gen`.
pub fn run() -> Result<(), String> {
    let secret_path = std::env::var("VANTA_ROOT_KEY").map_err(|_| {
        "VANTA_ROOT_KEY must point at the root secret key file (run `xtask keygen` first)"
            .to_string()
    })?;
    let secret = SecretKey::load(Path::new(&secret_path))?;

    let dl = Downloader::new()
        .map_err(|e| format!("building downloader: {e}"))?
        // Fail fast on a missing asset so skips are quick (no retry backoff).
        .with_retries(0);
    let tmp = std::env::temp_dir().join("vanta-registry-gen");
    std::fs::create_dir_all(&tmp).map_err(|e| format!("creating temp dir: {e}"))?;

    // Per-version caches for the publishers' checksum manifests.
    let mut node_shasums: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut go_json: Option<serde_json::Value> = None;
    // checksum-manifest URL → (filename → hash), for VersionSums sources.
    let mut sums_cache: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();

    let mut included = 0usize;
    let mut skipped = 0usize;
    let mut out = String::new();
    out.push_str(
        "# Vanta official tool registry — GENERATED by `cargo xtask registry-gen`.\n\
         # Do not edit by hand; regenerate and re-sign instead (see registry/README.md).\n\
         # Each artifact's integrity is gated by its sha256; the whole index is\n\
         # authenticated by the detached root signature in registry.toml.minisig.\n\n",
    );

    for spec in specs() {
        let provider = spec.provider();
        let mut version_blocks = String::new();
        let mut tool_any = false;

        let versions = match spec.versions.resolve(&dl, &tmp) {
            Ok(v) if !v.is_empty() => v,
            Ok(_) => {
                eprintln!("  SKIP tool {}: upstream lists no versions", spec.name);
                continue;
            }
            Err(e) => {
                eprintln!("  SKIP tool {}: version discovery failed: {e}", spec.name);
                skipped += 1;
                continue;
            }
        };
        eprintln!("tool {}: {} version(s) discovered", spec.name, versions.len());

        for version in &versions {
            let version = version.as_str();
            let mut platform_rows: BTreeMap<String, PlatformResult> = BTreeMap::new();

            for token in spec.platforms {
                let platform = match Platform::parse(token) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("  skip {} {} [{token}]: bad token: {e}", spec.name, version);
                        skipped += 1;
                        continue;
                    }
                };
                let artifact = provider.render_artifact(
                    version,
                    &platform,
                    Checksum {
                        algo: "sha256".to_string(),
                        value: String::new(),
                    },
                    None,
                );
                match resolve_checksum(
                    &spec,
                    version,
                    &artifact.url,
                    &dl,
                    &tmp,
                    &mut node_shasums,
                    &mut go_json,
                    &mut sums_cache,
                ) {
                    Ok(res) => {
                        eprintln!(
                            "  ok   {} {} [{token}] -> {}",
                            spec.name,
                            version,
                            &res.sha256[..16.min(res.sha256.len())]
                        );
                        platform_rows.insert(token.to_string(), res);
                        included += 1;
                    }
                    Err(e) => {
                        eprintln!("  skip {} {} [{token}]: {e}", spec.name, version);
                        skipped += 1;
                    }
                }
            }

            if platform_rows.is_empty() {
                continue;
            }
            tool_any = true;
            version_blocks.push_str(&format!("[[tools.{}.version]]\n", spec.name));
            version_blocks.push_str(&format!("version = {}\n", toml_str(version)));
            version_blocks.push_str("channel = \"stable\"\n");
            for (token, res) in &platform_rows {
                version_blocks.push_str(&format!(
                    "[tools.{}.version.platforms.{}]\n",
                    spec.name,
                    toml_str(token)
                ));
                version_blocks.push_str(&format!("sha256 = {}\n", toml_str(&res.sha256)));
                if let Some(size) = res.size {
                    version_blocks.push_str(&format!("size = {size}\n"));
                }
            }
            version_blocks.push('\n');
        }

        if !tool_any {
            eprintln!(
                "  SKIP tool {} entirely (no resolvable platforms)",
                spec.name
            );
            continue;
        }

        out.push_str(&render_tool_header(&spec, &provider));
        out.push_str(&version_blocks);
    }

    // VANTA_REGISTRY_OUT redirects output (e.g. a dev-key run for local
    // testing) so the published, root-signed registry/ files are not clobbered.
    let registry_dir = std::env::var("VANTA_REGISTRY_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root().join("registry"));
    std::fs::create_dir_all(&registry_dir).map_err(|e| format!("creating registry dir: {e}"))?;
    let toml_path = registry_dir.join("registry.toml");
    std::fs::write(&toml_path, out.as_bytes())
        .map_err(|e| format!("writing {}: {e}", toml_path.display()))?;

    // Sign the canonical index bytes with the pinned root key.
    let sig = secret.sign_detached(out.as_bytes());
    let sig_path = registry_dir.join("registry.toml.minisig");
    std::fs::write(&sig_path, sig.as_bytes())
        .map_err(|e| format!("writing {}: {e}", sig_path.display()))?;

    eprintln!(
        "\nregistry-gen: wrote {} and {} ({included} artifacts included, {skipped} skipped)",
        toml_path.display(),
        sig_path.display()
    );
    Ok(())
}

/// Render the per-tool provider header block.
fn render_tool_header(spec: &ToolSpec, provider: &ProviderDef) -> String {
    let mut s = String::new();
    s.push_str(&format!("[tools.{}]\n", spec.name));
    s.push_str(&format!("summary = {}\n\n", toml_str(spec.summary)));
    s.push_str(&format!("[tools.{}.provider]\n", spec.name));
    s.push_str(&format!("id = {}\n", toml_str(&provider.id)));
    s.push_str(&format!("tool = {}\n", toml_str(&provider.tool)));
    s.push_str(&format!(
        "url_template = {}\n",
        toml_str(&provider.url_template)
    ));
    s.push_str(&format!("archive = {}\n", toml_str(&provider.archive)));
    s.push_str(&format!("strip = {}\n", provider.strip));
    let bins: Vec<String> = provider.bin.iter().map(|b| toml_str(b)).collect();
    s.push_str(&format!("bin = [{}]\n", bins.join(", ")));
    if !provider.archive_map.is_empty() {
        s.push_str(&format!("\n[tools.{}.provider.archive_map]\n", spec.name));
        for (k, v) in &provider.archive_map {
            s.push_str(&format!("{} = {}\n", toml_str(k), toml_str(v)));
        }
    }
    if !provider.os_map.is_empty() {
        s.push_str(&format!("\n[tools.{}.provider.os_map]\n", spec.name));
        for (k, v) in &provider.os_map {
            s.push_str(&format!("{} = {}\n", toml_str(k), toml_str(v)));
        }
    }
    if !provider.arch_map.is_empty() {
        s.push_str(&format!("\n[tools.{}.provider.arch_map]\n", spec.name));
        for (k, v) in &provider.arch_map {
            s.push_str(&format!("{} = {}\n", toml_str(k), toml_str(v)));
        }
    }
    s.push('\n');
    s
}

/// Obtain the sha256 (and optional size) for one rendered artifact URL.
#[allow(clippy::too_many_arguments)]
fn resolve_checksum(
    spec: &ToolSpec,
    version: &str,
    url: &str,
    dl: &Downloader,
    tmp: &Path,
    node_shasums: &mut BTreeMap<String, BTreeMap<String, String>>,
    go_json: &mut Option<serde_json::Value>,
    sums_cache: &mut BTreeMap<String, BTreeMap<String, String>>,
) -> Result<PlatformResult, String> {
    let filename = url.rsplit('/').next().unwrap_or(url).to_string();
    match spec.checksum {
        ChecksumSource::NodeShasums => {
            let map = node_shasums.entry(version.to_string()).or_default();
            if map.is_empty() {
                let shasums_url = format!("https://nodejs.org/dist/v{version}/SHASUMS256.txt");
                let text = fetch_text(dl, &shasums_url, tmp, 8 * 1024 * 1024)?;
                for line in text.lines() {
                    let mut it = line.split_whitespace();
                    if let (Some(hash), Some(file)) = (it.next(), it.next()) {
                        map.insert(file.to_string(), hash.to_string());
                    }
                }
            }
            let sha = map
                .get(&filename)
                .ok_or_else(|| format!("{filename} absent from SHASUMS256.txt"))?;
            Ok(PlatformResult {
                sha256: sha.clone(),
                size: None,
            })
        }
        ChecksumSource::GoJson => {
            if go_json.is_none() {
                let text = fetch_text(
                    dl,
                    "https://go.dev/dl/?mode=json&include=all",
                    tmp,
                    8 * 1024 * 1024,
                )?;
                let v: serde_json::Value =
                    serde_json::from_str(&text).map_err(|e| format!("parse go json: {e}"))?;
                *go_json = Some(v);
            }
            let releases = go_json
                .as_ref()
                .unwrap()
                .as_array()
                .ok_or("go json not an array")?;
            for rel in releases {
                if let Some(files) = rel.get("files").and_then(|f| f.as_array()) {
                    for f in files {
                        if f.get("filename").and_then(|n| n.as_str()) == Some(filename.as_str()) {
                            let sha = f
                                .get("sha256")
                                .and_then(|s| s.as_str())
                                .ok_or("go file missing sha256")?;
                            let size = f.get("size").and_then(|s| s.as_u64());
                            return Ok(PlatformResult {
                                sha256: sha.to_string(),
                                size,
                            });
                        }
                    }
                }
            }
            Err(format!("{filename} absent from go.dev json"))
        }
        ChecksumSource::Sidecar(suffix) => {
            let sidecar = format!("{url}{suffix}");
            let text = fetch_text(dl, &sidecar, tmp, 4096)?;
            let hash = text
                .split_whitespace()
                .next()
                .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
                .ok_or_else(|| format!("malformed {suffix} sidecar"))?;
            Ok(PlatformResult {
                sha256: hash.to_lowercase(),
                size: None,
            })
        }
        ChecksumSource::VersionSums(template) => {
            let sums_url = template.replace("{version}", version);
            let map = sums_cache.entry(sums_url.clone()).or_default();
            if map.is_empty() {
                let text = fetch_text(dl, &sums_url, tmp, 8 * 1024 * 1024)?;
                for line in text.lines() {
                    let mut it = line.split_whitespace();
                    if let (Some(hash), Some(file)) = (it.next(), it.next()) {
                        if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                            // Some manifests mark binary mode with a leading `*`.
                            let file = file.trim_start_matches('*');
                            // Some list paths; match on the basename.
                            let base = file.rsplit('/').next().unwrap_or(file);
                            map.insert(base.to_string(), hash.to_lowercase());
                        }
                    }
                }
            }
            let sha = map
                .get(&filename)
                .ok_or_else(|| format!("{filename} absent from {sums_url}"))?;
            Ok(PlatformResult {
                sha256: sha.clone(),
                size: None,
            })
        }
        ChecksumSource::Download => {
            let dest = tmp.join(&filename);
            dl.download_capped(url, &dest, Some(256 * 1024 * 1024))
                .map_err(|e| format!("download {url}: {e}"))?;
            let sha = sha256_file(&dest)?;
            let size = std::fs::metadata(&dest).ok().map(|m| m.len());
            let _ = std::fs::remove_file(&dest);
            Ok(PlatformResult { sha256: sha, size })
        }
    }
}

/// Download a (small) text resource and return its contents.
fn fetch_text(dl: &Downloader, url: &str, tmp: &Path, max: u64) -> Result<String, String> {
    let dest = tmp.join(format!("fetch-{:x}.txt", fnv1a(url)));
    dl.download_capped(url, &dest, Some(max))
        .map_err(|e| format!("fetch {url}: {e}"))?;
    let s = std::fs::read_to_string(&dest).map_err(|e| format!("read {url}: {e}"))?;
    let _ = std::fs::remove_file(&dest);
    Ok(s)
}

/// sha256 a file as lowercase hex.
fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(hex(&h.finalize()))
}

/// A minimal TOML basic-string literal. Our generated values never contain
/// control characters; we escape only `\` and `"`.
fn toml_str(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Repository root: the workspace dir (xtask's manifest parent's parent).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

// ---------------------------------------------------------------------------
// Minisign-compatible root keypair (dev key). The public key and signature are
// in minisign wire format (algo `Ed`, legacy detached signature over the file)
// which `vanta_security::sign::minisign_verify` accepts. The secret is stored
// unencrypted in a tiny key=base64 file the generator reads; it is NEVER placed
// in the repository.
// ---------------------------------------------------------------------------

const PUBKEY_COMMENT: &str = "untrusted comment: vanta registry root key";

/// The loaded root secret key.
pub struct SecretKey {
    key_id: [u8; 8],
    signing: SigningKey,
}

impl SecretKey {
    /// Load a secret key file (`keyid=<b64>` / `seed=<b64>`).
    pub fn load(path: &Path) -> Result<SecretKey, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading root secret {}: {e}", path.display()))?;
        let mut key_id: Option<[u8; 8]> = None;
        let mut seed: Option<[u8; 32]> = None;
        for line in text.lines() {
            let line = line.trim();
            if let Some(v) = line.strip_prefix("keyid=") {
                let raw = STANDARD
                    .decode(v.trim())
                    .map_err(|e| format!("keyid b64: {e}"))?;
                key_id = Some(
                    raw.try_into()
                        .map_err(|_| "keyid must be 8 bytes".to_string())?,
                );
            } else if let Some(v) = line.strip_prefix("seed=") {
                let raw = STANDARD
                    .decode(v.trim())
                    .map_err(|e| format!("seed b64: {e}"))?;
                seed = Some(
                    raw.try_into()
                        .map_err(|_| "seed must be 32 bytes".to_string())?,
                );
            }
        }
        let key_id = key_id.ok_or("secret key file missing `keyid=`")?;
        let seed = seed.ok_or("secret key file missing `seed=`")?;
        Ok(SecretKey {
            key_id,
            signing: SigningKey::from_bytes(&seed),
        })
    }

    /// Produce a detached minisign signature file (legacy `Ed`) over `data`.
    pub fn sign_detached(&self, data: &[u8]) -> String {
        let sig = self.signing.sign(data).to_bytes();
        let mut raw = Vec::with_capacity(74);
        raw.extend_from_slice(b"Ed");
        raw.extend_from_slice(&self.key_id);
        raw.extend_from_slice(&sig);
        format!(
            "untrusted comment: vanta registry index signature\n{}\ntrusted comment: vanta official registry\n{}\n",
            STANDARD.encode(&raw),
            STANDARD.encode([0u8; 64])
        )
    }

    /// The matching minisign public-key text.
    pub fn public_key_text(&self) -> String {
        let pk = self.signing.verifying_key().to_bytes();
        let mut raw = Vec::with_capacity(42);
        raw.extend_from_slice(b"Ed");
        raw.extend_from_slice(&self.key_id);
        raw.extend_from_slice(&pk);
        format!("{PUBKEY_COMMENT}\n{}", STANDARD.encode(&raw))
    }
}

/// Run `cargo xtask keygen <secret-path>`: mint a root keypair, write the secret
/// to `<secret-path>`, and print the public key text to stdout.
pub fn keygen(out_path: &str) -> Result<(), String> {
    // 40 bytes of OS randomness: 8-byte key id + 32-byte ed25519 seed.
    let mut buf = [0u8; 40];
    read_random(&mut buf)?;
    let mut key_id = [0u8; 8];
    key_id.copy_from_slice(&buf[0..8]);
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&buf[8..40]);

    let secret = SecretKey {
        key_id,
        signing: SigningKey::from_bytes(&seed),
    };

    let body = format!(
        "# vanta registry root SECRET key — UNENCRYPTED dev key.\n\
         # Keep this file offline; never commit it to the repository.\n\
         keyid={}\n\
         seed={}\n",
        STANDARD.encode(key_id),
        STANDARD.encode(seed)
    );
    std::fs::write(out_path, body).map_err(|e| format!("writing {out_path}: {e}"))?;
    // Best-effort restrictive permissions.
    set_owner_only(Path::new(out_path));

    println!("{}", secret.public_key_text());
    eprintln!("keygen: wrote secret key to {out_path} (public key printed to stdout)");
    Ok(())
}

/// Read exactly `buf.len()` bytes of OS randomness.
fn read_random(buf: &mut [u8]) -> Result<(), String> {
    use std::io::Read;
    let mut f = std::fs::File::open("/dev/urandom").map_err(|e| format!("/dev/urandom: {e}"))?;
    f.read_exact(buf)
        .map_err(|e| format!("reading randomness: {e}"))
}

#[cfg(unix)]
fn set_owner_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) {}
