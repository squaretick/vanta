//! `cargo xtask registry-gen` — build and minisign-sign the official Vanta tool
//! registry from real upstream checksums.
//!
//! For a hardcoded list of seed tools/versions this fetches the publishers'
//! published sha256 digests (node `SHASUMS256.txt`, go `?mode=json`) or hashes
//! the downloaded asset (GitHub-release tools), maps each asset to Vanta's
//! platform target tokens, and emits `registry/registry.toml` in the exact
//! schema [`vanta_registry::Registry::from_toml`] parses. The canonical index
//! bytes are then signed with the pinned root key →
//! `registry/registry.toml.minisig`.
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
    /// Hash the downloaded asset (universal fallback; used for GitHub releases).
    Download,
    /// node: parse `https://nodejs.org/dist/v{version}/SHASUMS256.txt`.
    NodeShasums,
    /// go: parse `https://go.dev/dl/?mode=json` (carries sha256 + size).
    GoJson,
    /// Fetch the `<asset-url>.sha256` sidecar (python-build-standalone).
    Sidecar,
}

/// A seed tool: its declarative provider plus the versions/platforms to seed.
struct ToolSpec {
    name: &'static str,
    summary: &'static str,
    archive: &'static str,
    strip: u32,
    bin: &'static [&'static str],
    url_template: &'static str,
    os_map: &'static [(&'static str, &'static str)],
    arch_map: &'static [(&'static str, &'static str)],
    versions: &'static [&'static str],
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

/// The seed list: real upstream tools at current stable versions.
fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "node",
            summary: "Node.js JavaScript runtime",
            archive: "tar.gz",
            strip: 1,
            bin: &["bin/node"],
            url_template:
                "https://nodejs.org/dist/v{version}/node-v{version}-{os}-{arch}.{ext}",
            os_map: &[("macos", "darwin"), ("linux", "linux")],
            arch_map: &[("x86_64", "x64"), ("aarch64", "arm64")],
            versions: &["22.11.0", "20.18.0"],
            // node ships glibc tarballs for unix; the windows build is a .zip
            // (a different archive kind) so it is intentionally omitted.
            platforms: &[
                "linux/x86_64/gnu",
                "linux/aarch64/gnu",
                "macos/x86_64",
                "macos/aarch64",
            ],
            checksum: ChecksumSource::NodeShasums,
        },
        ToolSpec {
            name: "go",
            summary: "The Go programming language toolchain",
            archive: "tar.gz",
            strip: 1,
            bin: &["bin/go", "bin/gofmt"],
            url_template: "https://go.dev/dl/go{version}.{os}-{arch}.{ext}",
            os_map: &[("macos", "darwin")],
            arch_map: &[("x86_64", "amd64"), ("aarch64", "arm64")],
            versions: &["1.23.4", "1.22.10"],
            // unix tarballs only; the windows distribution is a .zip.
            platforms: &[
                "linux/x86_64/gnu",
                "linux/aarch64/gnu",
                "macos/x86_64",
                "macos/aarch64",
            ],
            checksum: ChecksumSource::GoJson,
        },
        ToolSpec {
            name: "ripgrep",
            summary: "ripgrep (rg) — recursive line-oriented search",
            archive: "tar.gz",
            strip: 1,
            bin: &["rg"],
            url_template:
                "https://github.com/BurntSushi/ripgrep/releases/download/{version}/ripgrep-{version}-{arch}-{os}.{ext}",
            // The musl build is a static binary that runs on any Linux, so we map
            // the canonical `linux` token to it and store it under the gnu token
            // that `Platform::current()` reports.
            os_map: &[("macos", "apple-darwin"), ("linux", "unknown-linux-musl")],
            arch_map: &[],
            versions: &["14.1.1"],
            platforms: &[
                "linux/x86_64/gnu",
                "linux/aarch64/gnu",
                "macos/x86_64",
                "macos/aarch64",
            ],
            checksum: ChecksumSource::Download,
        },
        ToolSpec {
            name: "fd",
            summary: "fd — a fast, user-friendly alternative to find",
            archive: "tar.gz",
            strip: 1,
            bin: &["fd"],
            url_template:
                "https://github.com/sharkdp/fd/releases/download/v{version}/fd-v{version}-{arch}-{os}.{ext}",
            os_map: &[("macos", "apple-darwin"), ("linux", "unknown-linux-musl")],
            arch_map: &[],
            versions: &["10.2.0"],
            platforms: &[
                "linux/x86_64/gnu",
                "linux/aarch64/gnu",
                "macos/x86_64",
                "macos/aarch64",
            ],
            checksum: ChecksumSource::Download,
        },
        ToolSpec {
            name: "jq",
            summary: "jq — command-line JSON processor",
            archive: "raw",
            strip: 0,
            bin: &["jq"],
            url_template:
                "https://github.com/jqlang/jq/releases/download/jq-{version}/jq-{os}-{arch}",
            os_map: &[],
            arch_map: &[("x86_64", "amd64"), ("aarch64", "arm64")],
            versions: &["1.7.1"],
            // raw single-file binaries; the windows asset has a `.exe` suffix and
            // is omitted (the `{ext}`-less template cannot express it).
            platforms: &[
                "linux/x86_64/gnu",
                "linux/aarch64/gnu",
                "macos/x86_64",
                "macos/aarch64",
            ],
            checksum: ChecksumSource::Download,
        },
        ToolSpec {
            name: "uv",
            summary: "uv — an extremely fast Python package and project manager",
            archive: "tar.gz",
            strip: 1,
            bin: &["uv", "uvx"],
            url_template:
                "https://github.com/astral-sh/uv/releases/download/{version}/uv-{arch}-{os}.{ext}",
            os_map: &[("macos", "apple-darwin"), ("linux", "unknown-linux-musl")],
            arch_map: &[],
            versions: &["0.5.11"],
            platforms: &[
                "linux/x86_64/gnu",
                "linux/aarch64/gnu",
                "macos/x86_64",
                "macos/aarch64",
            ],
            checksum: ChecksumSource::Download,
        },
        ToolSpec {
            name: "python",
            summary: "CPython standalone build (astral-sh/python-build-standalone)",
            archive: "tar.gz",
            strip: 0,
            bin: &["python/bin/python3"],
            // python-build-standalone embeds the release tag (a date) in the path
            // AND a `+{tag}` build-metadata suffix in the version; we pin one
            // release so every seeded python version shares the `20241016` tag.
            url_template:
                "https://github.com/astral-sh/python-build-standalone/releases/download/20241016/cpython-{version}-{arch}-{os}-install_only.tar.gz",
            os_map: &[("macos", "apple-darwin"), ("linux", "unknown-linux-gnu")],
            arch_map: &[],
            versions: &["3.12.7+20241016", "3.11.10+20241016"],
            // install_only tarballs lay out `python/bin/python3` on unix; the
            // windows layout differs, so windows is omitted to keep `bin` correct.
            platforms: &[
                "linux/x86_64/gnu",
                "linux/aarch64/gnu",
                "macos/x86_64",
                "macos/aarch64",
            ],
            checksum: ChecksumSource::Sidecar,
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

        for version in spec.versions {
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

    let registry_dir = repo_root().join("registry");
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
fn resolve_checksum(
    spec: &ToolSpec,
    version: &str,
    url: &str,
    dl: &Downloader,
    tmp: &Path,
    node_shasums: &mut BTreeMap<String, BTreeMap<String, String>>,
    go_json: &mut Option<serde_json::Value>,
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
        ChecksumSource::Sidecar => {
            let sidecar = format!("{url}.sha256");
            let text = fetch_text(dl, &sidecar, tmp, 4096)?;
            let hash = text
                .split_whitespace()
                .next()
                .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
                .ok_or("malformed .sha256 sidecar")?;
            Ok(PlatformResult {
                sha256: hash.to_lowercase(),
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
