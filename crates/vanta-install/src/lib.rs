//! `vanta-install` — the install engine.
//!
//! Drives the lifecycle stages `[4 Fetch]`..`[8 Commit]` (`docs/08-installation.md`)
//! for a resolved artifact: download (mirror-aware, resumable), verify the
//! checksum (fail-closed), materialize (extract) into a staging tree, publish it
//! atomically into the content-addressed store, and record a new generation.
//!
//! The entry point takes a resolved [`Artifact`] (produced by `vanta-resolve`).
//! Supported archive formats: `tar.gz`/`tgz` and `raw`.
#![forbid(unsafe_code)]

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use vanta_core::{Area, Artifact, Platform, StoreKey, VtaError, VtaResult};
use vanta_net::Downloader;
use vanta_security::Policy;
use vanta_state::{GenerationRecord, State, StoreEntryMeta};
use vanta_store::Store;

/// Observes the progress of an [`Engine::install_artifact_reported`] run so a
/// caller (the CLI) can render download bars and phase spinners without this
/// crate depending on a UI crate. All methods have no-op defaults; the unit
/// type `()` implements it as a fully silent reporter.
pub trait Reporter {
    /// The fetch stage is about to begin; `total` is the artifact's declared
    /// size in bytes when known (used as the download bar's length).
    fn fetch_start(&self, total: Option<u64>) {
        let _ = total;
    }
    /// `n` more bytes have been downloaded.
    fn fetch_inc(&self, n: u64) {
        let _ = n;
    }
    /// A new post-fetch phase has begun (e.g. `"verifying"`, `"extracting"`).
    fn phase(&self, name: &str) {
        let _ = name;
    }
}

/// Silent reporter: the default when no progress UI is wired in.
impl Reporter for () {}

/// Default ceiling on the total decompressed size of an archive (audit M8). A
/// gzip bomb that would expand past this aborts extraction rather than filling
/// the disk. Overridable via [`Engine::with_max_decompressed`].
pub const DEFAULT_MAX_DECOMPRESSED: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB

/// The install engine, bound to a `$VANTA_HOME`.
pub struct Engine {
    store: Store,
    state: State,
    downloader: Downloader,
    home: PathBuf,
    /// Verification policy (audit H2). When `require_signature` is set, a missing
    /// or untrusted signature is a hard error (fail-closed).
    policy: Policy,
    /// Hard ceiling on decompressed archive bytes (audit M8).
    max_decompressed: u64,
}

impl Engine {
    /// Open the engine over `home` (`$VANTA_HOME`) with the default (permissive)
    /// policy — checksum-gated, signatures verified when present. Use
    /// [`Engine::open_with_policy`] to require signatures.
    pub fn open(home: impl AsRef<Path>) -> VtaResult<Engine> {
        Self::open_with_policy(home, Policy::default())
    }

    /// Open the engine with an explicit verification [`Policy`] (audit H2).
    pub fn open_with_policy(home: impl AsRef<Path>, policy: Policy) -> VtaResult<Engine> {
        let home = home.as_ref().to_path_buf();
        let store = Store::open(&home)?;
        let state = State::open(&home.join("state.db"))?;
        let downloader = Downloader::new()?;
        Ok(Engine {
            store,
            state,
            downloader,
            home,
            policy,
            max_decompressed: DEFAULT_MAX_DECOMPRESSED,
        })
    }

    /// Override the decompressed-size ceiling (audit M8).
    pub fn with_max_decompressed(mut self, max: u64) -> Self {
        self.max_decompressed = max;
        self
    }

    /// Borrow the underlying store / state (for `gc`, `which`, etc.).
    pub fn store(&self) -> &Store {
        &self.store
    }
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Install one resolved artifact for the current platform, returning its
    /// store key. Fetch → verify → materialize → publish → commit a generation.
    /// A store hit short-circuits fetch/verify/materialize.
    pub fn install_artifact(
        &self,
        tool: &str,
        version: &str,
        artifact: &Artifact,
    ) -> VtaResult<StoreKey> {
        self.install_artifact_reported(tool, version, artifact, &())
    }

    /// Like [`Engine::install_artifact`], but drives `reporter` with download
    /// byte counts and phase transitions so a caller can render progress.
    pub fn install_artifact_reported(
        &self,
        tool: &str,
        version: &str,
        artifact: &Artifact,
        reporter: &dyn Reporter,
    ) -> VtaResult<StoreKey> {
        // Policy precheck (audit H2): when a signature is required, an artifact
        // lacking a signature OR a *trusted* signing key (the resolver drops
        // untrusted keys, audit C1) is refused — fail-closed, before any I/O.
        let has_trusted_sig = artifact.signature.is_some() && artifact.signature_key.is_some();
        if self.policy.require_signature && !has_trusted_sig {
            return Err(VtaError::new(
                Area::Vrf,
                3,
                format!(
                    "signature required by policy but `{tool} {version}` is unsigned \
                     or its signing key is not trusted"
                ),
            ));
        }

        // [3 Plan] — if the lock already named a key and it is present, reuse it
        // ONLY if it still verifies (audit H4): a store hit must not be trusted
        // blindly, since the entry could have been poisoned (audit H3) or the
        // lockfile's `store_key` is attacker-influenceable. On mismatch, drop the
        // bad entry and fall through to a fresh fetch + verify.
        if let Some(key) = &artifact.store_key {
            if self.store.has(key) {
                if self.store.verify_entry(key)? {
                    self.link_bins(key, &artifact.bin)?;
                    self.record(tool, version, key, &artifact.checksum.value)?;
                    return Ok(key.clone());
                }
                self.store.remove_entry(key)?;
            }
        }

        // [4 Fetch] — cap downloaded bytes at the declared size when known (M8).
        let dl = self
            .store
            .downloads_dir()
            .join(format!("incoming-{tool}-{}", std::process::id()));
        let mut urls = vec![artifact.url.clone()];
        urls.extend(artifact.mirrors.clone());
        reporter.fetch_start(artifact.size);
        self.downloader.download_any_with_progress(
            &urls,
            &dl,
            artifact.size,
            Some(&|n| reporter.fetch_inc(n)),
        )?;

        // [5 Verify] — fail closed (centralized in vanta-security).
        reporter.phase("verifying");
        if let Err(e) =
            vanta_security::verify_file(&dl, &artifact.checksum.algo, &artifact.checksum.value)
        {
            let _ = fs::remove_file(&dl);
            return Err(e);
        }
        // Signature verification when the registry pinned a signature + trusted
        // key. The key's trust is established upstream (audit C1, in the resolver);
        // by this point a present `signature_key` is one we trust.
        if let (Some(sig), Some(key_text)) = (&artifact.signature, &artifact.signature_key) {
            let key = vanta_security::parse_minisign_pubkey(key_text)?;
            let bytes = fs::read(&dl).map_err(|e| io(&dl, e))?;
            if let Err(e) = vanta_security::minisign_verify(&bytes, sig, &key) {
                let _ = fs::remove_file(&dl);
                return Err(e);
            }
        }

        // [6 Materialize]
        reporter.phase("extracting");
        let staging = self.store.new_staging()?;
        let name = artifact
            .bin
            .first()
            .map(|b| basename(b))
            .unwrap_or_else(|| tool.to_string());
        extract(
            &artifact.archive,
            &dl,
            &staging,
            &name,
            artifact.strip,
            self.max_decompressed,
        )?;
        let _ = fs::remove_file(&dl);

        // [6 Materialize, cont.] atomic publish into the store.
        let key = self.store.publish_tree(&staging)?;

        // [7 Link] expose the tool's executables on PATH via ~/.vanta/bin.
        self.link_bins(&key, &artifact.bin)?;

        // [8 Commit]
        self.record(tool, version, &key, &artifact.checksum.value)?;
        Ok(key)
    }

    /// Link a store entry's declared executables into `~/.vanta/bin` (placed on
    /// PATH by the shell hook). Per-directory environment views are composed by
    /// `vanta-env` (`docs/10-environments.md`).
    fn link_bins(&self, key: &StoreKey, bins: &[String]) -> VtaResult<()> {
        let bin_dir = self.home.join("bin");
        fs::create_dir_all(&bin_dir).map_err(|e| io(&bin_dir, e))?;
        let entry = self.store.entry_path(key);
        for bin in bins {
            let src = entry.join(bin);
            if src.exists() {
                let dst = bin_dir.join(basename(bin));
                vanta_store::link_best(&src, &dst)?;
            }
        }
        Ok(())
    }

    fn record(&self, tool: &str, version: &str, key: &StoreKey, sha256: &str) -> VtaResult<()> {
        let platform = Platform::current().token();
        self.state.put_store_entry(
            key.as_str(),
            &StoreEntryMeta {
                tool: tool.to_string(),
                version: version.to_string(),
                platform,
                size: 0,
                sha256: sha256.to_string(),
            },
        )?;
        let parent = self.state.current()?;
        let id = parent.map(|c| c + 1).unwrap_or(1);
        self.state.append_generation(&GenerationRecord {
            id,
            parent,
            command: format!("vanta add {tool}@{version}"),
            reason: "add".to_string(),
            tools: vec![(tool.to_string(), key.as_str().to_string())],
        })?;
        self.state.set_current(id)?;
        Ok(())
    }

    /// Store keys referenced by the active generation.
    fn active_store_keys(&self) -> VtaResult<Vec<StoreKey>> {
        let mut keys = Vec::new();
        if let Some(current) = self.state.current()? {
            if let Some(gen) = self.state.get_generation(current)? {
                for (_, k) in gen.tools {
                    if let Ok(sk) = StoreKey::new(k) {
                        keys.push(sk);
                    }
                }
            }
        }
        Ok(keys)
    }

    /// Bundle the active generation's store entries into a portable archive
    /// (`docs/13-offline.md`). Returns the number of entries written.
    pub fn bundle_current(&self, out: &Path) -> VtaResult<usize> {
        let keys = self.active_store_keys()?;
        let file = fs::File::create(out).map_err(|e| io(out, e))?;
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);
        let list = keys
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let mut header = tar::Header::new_gnu();
        header.set_size(list.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "KEYS", list.as_bytes())
            .map_err(|e| inst(format!("bundle KEYS: {e}")))?;
        for key in &keys {
            let dir = self.store.entry_path(key);
            if dir.is_dir() {
                builder
                    .append_dir_all(key.as_str(), &dir)
                    .map_err(|e| inst(format!("bundle {key}: {e}")))?;
            }
        }
        let enc = builder
            .into_inner()
            .map_err(|e| inst(format!("bundle finalize: {e}")))?;
        enc.finish()
            .map_err(|e| inst(format!("bundle gzip: {e}")))?;
        Ok(keys.len())
    }

    /// Restore store entries from a bundle, verifying each entry's integrity
    /// against its content-addressed key. Returns the number newly imported.
    pub fn restore(&self, bundle: &Path) -> VtaResult<usize> {
        let file = fs::File::open(bundle).map_err(|e| io(bundle, e))?;
        let gz = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(gz);
        let staging = self.store.new_staging()?;
        archive
            .unpack(&staging)
            .map_err(|e| inst(format!("restore unpack: {e}")))?;
        let keys_txt =
            fs::read_to_string(staging.join("KEYS")).map_err(|e| io(&staging.join("KEYS"), e))?;
        let mut restored = 0;
        for line in keys_txt.lines() {
            let key = line.trim();
            if key.is_empty() {
                continue;
            }
            // `StoreKey::new` enforces the fixed-width lowercase-hex shape (M7),
            // so `staging.join(key)` below cannot traverse out of staging.
            let sk = StoreKey::new(key)?;
            let dst = self.store.entry_path(&sk);
            if dst.exists() {
                // Already present (and immutable + verified at insert); nothing
                // to import for this key.
                continue;
            }
            let src = staging.join(key);
            if !src.is_dir() {
                continue;
            }
            // Audit H3: verify the staged subtree hashes to its claimed key
            // BEFORE publishing it into the canonical store. A bundle whose
            // contents do not match the `blake3-<hash>` dir name is rejected and
            // the store is left unchanged (the staging dir is removed below).
            let actual = vanta_store::hash_tree(&src)?;
            if actual != sk.as_str() {
                let _ = fs::remove_dir_all(&staging);
                return Err(VtaError::new(
                    Area::Vrf,
                    1,
                    format!("bundled entry {key} failed integrity verification (content mismatch)"),
                ));
            }
            // Bundled entries are read-only; add write so the dir can be moved.
            let _ = vanta_store::ensure_writable(&src);
            fs::rename(&src, &dst).map_err(|e| io(&dst, e))?;
            restored += 1;
        }
        let _ = fs::remove_dir_all(&staging);
        Ok(restored)
    }

    /// Remove a tool: record a new generation without it and unlink its primary
    /// executable. Returns whether the tool was present.
    pub fn remove(&self, tool: &str) -> VtaResult<bool> {
        let current = match self.state.current()? {
            Some(c) => c,
            None => return Ok(false),
        };
        let gen = match self.state.get_generation(current)? {
            Some(g) => g,
            None => return Ok(false),
        };
        if !gen.tools.iter().any(|(t, _)| t == tool) {
            return Ok(false);
        }
        let tools: Vec<(String, String)> = gen
            .tools
            .iter()
            .filter(|(t, _)| t != tool)
            .cloned()
            .collect();
        let id = current + 1;
        self.state.append_generation(&GenerationRecord {
            id,
            parent: Some(current),
            command: format!("vanta remove {tool}"),
            reason: "remove".to_string(),
            tools,
        })?;
        self.state.set_current(id)?;
        let _ = fs::remove_file(self.home.join("bin").join(tool));
        Ok(true)
    }
}

fn inst(msg: String) -> VtaError {
    VtaError::new(Area::Inst, 1, msg)
}

/// Materialize an artifact's bytes into `dest` according to its archive kind,
/// stripping `strip` leading path components (the provider's layout).
/// `max_decompressed` caps the total decompressed bytes (audit M8).
pub fn extract(
    archive: &str,
    src: &Path,
    dest: &Path,
    raw_name: &str,
    strip: u32,
    max_decompressed: u64,
) -> VtaResult<()> {
    match archive {
        "tar.gz" | "tgz" => extract_targz(src, dest, strip, max_decompressed),
        "raw" => {
            fs::create_dir_all(dest).map_err(|e| io(dest, e))?;
            let out = dest.join(raw_name);
            fs::copy(src, &out).map_err(|e| io(&out, e))?;
            set_executable(&out);
            Ok(())
        }
        other => Err(VtaError::new(
            Area::Inst,
            3,
            format!("unsupported archive kind `{other}` (supported: tar.gz, tgz, raw)"),
        )),
    }
}

fn extract_targz(src: &Path, dest: &Path, strip: u32, max_decompressed: u64) -> VtaResult<()> {
    use std::path::PathBuf;
    let file = fs::File::open(src).map_err(|e| io(src, e))?;
    // M8: bound total decompressed bytes so a gzip bomb aborts rather than
    // filling the disk.
    let gz = LimitReader::new(flate2::read::GzDecoder::new(file), max_decompressed);
    let mut archive = tar::Archive::new(gz);
    // We re-apply a sanitized mode after unpack (M5: strip setuid/setgid), so we
    // do not need tar to preserve raw permission bits.
    archive.set_preserve_permissions(true);
    let dest_canon = dest.canonicalize().map_err(|e| io(dest, e))?;
    let entries = archive
        .entries()
        .map_err(|e| VtaError::new(Area::Inst, 1, format!("reading archive: {e}")))?;
    for entry in entries {
        let mut entry = entry
            .map_err(|e| VtaError::new(Area::Inst, 1, format!("reading archive entry: {e}")))?;
        let entry_type = entry.header().entry_type();
        let path = entry
            .path()
            .map_err(|e| VtaError::new(Area::Inst, 1, format!("entry path: {e}")))?
            .into_owned();
        let stripped: PathBuf = path.components().skip(strip as usize).collect();
        if stripped.as_os_str().is_empty() {
            continue;
        }
        // Reject anything that could escape the destination (zip-slip / traversal).
        if escapes(&stripped) {
            return Err(traversal());
        }
        // M5: link entries (symlink/hardlink) get an extra check — their *target*
        // must not be absolute or contain `..`. `entry.unpack` would otherwise
        // create a link pointing outside the staging tree, which a later entry
        // could write through.
        if matches!(entry_type, tar::EntryType::Symlink | tar::EntryType::Link) {
            let target = entry
                .link_name()
                .map_err(|e| VtaError::new(Area::Inst, 1, format!("link target: {e}")))?
                .map(|c| c.into_owned())
                .unwrap_or_default();
            if target.is_absolute() || escapes(&target) {
                return Err(VtaError::new(
                    Area::Inst,
                    1,
                    format!(
                        "archive link entry `{}` has an unsafe target `{}` (rejected)",
                        stripped.display(),
                        target.display()
                    ),
                ));
            }
        }
        let out = dest.join(&stripped);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
            // M5: after the parent exists, canonicalize it and confirm the
            // realpath still lies under the staging root (defeats symlinked
            // ancestors pointing elsewhere).
            let parent_canon = parent.canonicalize().map_err(|e| io(parent, e))?;
            if !parent_canon.starts_with(&dest_canon) {
                return Err(traversal());
            }
        }
        entry
            .unpack(&out)
            .map_err(|e| VtaError::new(Area::Inst, 1, format!("unpacking entry: {e}")))?;
        // M5: strip setuid/setgid/sticky bits from materialized files.
        strip_special_bits(&out);
    }
    Ok(())
}

/// Whether a relative path contains a component that would escape its base.
fn escapes(p: &Path) -> bool {
    use std::path::Component;
    p.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

fn traversal() -> VtaError {
    VtaError::new(
        Area::Inst,
        1,
        "archive entry escapes destination (path traversal rejected)".to_string(),
    )
}

/// A reader that errors once more than `limit` bytes have been read (audit M8).
struct LimitReader<R> {
    inner: R,
    remaining: u64,
}

impl<R> LimitReader<R> {
    fn new(inner: R, limit: u64) -> Self {
        LimitReader {
            inner,
            remaining: limit,
        }
    }
}

impl<R: Read> Read for LimitReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        let n64 = n as u64;
        if n64 > self.remaining {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "decompressed size exceeds configured maximum (possible decompression bomb)",
            ));
        }
        self.remaining -= n64;
        Ok(n)
    }
}

/// Strip setuid/setgid/sticky bits from a materialized path (audit M5).
#[cfg(unix)]
fn strip_special_bits(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    // Symlinks carry no meaningful permission bits; skip (and avoid following).
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return;
        }
        let mode = meta.permissions().mode();
        let safe = mode & 0o777; // drop 0o7000 (setuid/setgid/sticky)
        if safe != mode {
            let mut perms = meta.permissions();
            perms.set_mode(safe);
            let _ = fs::set_permissions(path, perms);
        }
    }
}

#[cfg(not(unix))]
fn strip_special_bits(_path: &Path) {}

fn basename(p: &str) -> String {
    p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o755);
        let _ = fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) {}

fn io(path: &Path, e: std::io::Error) -> VtaError {
    VtaError::new(Area::Inst, 2, format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("vanta-install-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn engine_opens_and_creates_state() {
        let h = home("open");
        let e = Engine::open(&h).unwrap();
        assert_eq!(
            e.state().schema_version().unwrap(),
            vanta_state::SCHEMA_VERSION
        );
        let _ = fs::remove_dir_all(&h);
    }

    #[test]
    fn extracts_targz_then_publishes() {
        use flate2::write::GzEncoder;
        use flate2::Compression;

        // Build a small .tar.gz in memory: one file `bin/tool`.
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        let mut header = tar::Header::new_gnu();
        let payload = b"#!/bin/sh\necho hi\n";
        header.set_size(payload.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "bin/tool", &payload[..])
            .unwrap();
        let gz = builder.into_inner().unwrap();
        let bytes = gz.finish().unwrap();

        let h = home("targz");
        let store = Store::open(&h).unwrap();
        let archive_path = store.downloads_dir().join("a.tar.gz");
        fs::write(&archive_path, &bytes).unwrap();

        let staging = store.new_staging().unwrap();
        extract(
            "tar.gz",
            &archive_path,
            &staging,
            "tool",
            0,
            DEFAULT_MAX_DECOMPRESSED,
        )
        .unwrap();
        assert!(staging.join("bin/tool").exists());

        let key = store.publish_tree(&staging).unwrap();
        assert!(store.has(&key));
        assert!(store.verify_entry(&key).unwrap());
        let _ = fs::remove_dir_all(&h);
    }

    #[test]
    fn rejects_unsupported_archive() {
        let err = extract(
            "tar.xz",
            Path::new("/x"),
            Path::new("/y"),
            "t",
            0,
            DEFAULT_MAX_DECOMPRESSED,
        )
        .unwrap_err();
        assert_eq!(err.area, Area::Inst);
    }

    // M5: an archive containing a symlink whose target escapes the tree (here an
    // absolute path), followed by a write through that link, must be rejected.
    #[test]
    fn rejects_symlink_escape_archive() {
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        // A symlink `evil` -> `/tmp/escape-target` (absolute).
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Symlink);
        link.set_size(0);
        link.set_mode(0o777);
        builder
            .append_link(&mut link, "evil", "/tmp/vanta-escape-target")
            .unwrap();
        // A regular write through the link path.
        let payload = b"pwned";
        let mut f = tar::Header::new_gnu();
        f.set_size(payload.len() as u64);
        f.set_mode(0o644);
        f.set_cksum();
        builder.append_data(&mut f, "evil", &payload[..]).unwrap();
        let bytes = builder.into_inner().unwrap().finish().unwrap();

        let h = home("symlink");
        let store = Store::open(&h).unwrap();
        let archive_path = store.downloads_dir().join("evil.tar.gz");
        fs::write(&archive_path, &bytes).unwrap();
        let staging = store.new_staging().unwrap();
        let err = extract(
            "tar.gz",
            &archive_path,
            &staging,
            "tool",
            0,
            DEFAULT_MAX_DECOMPRESSED,
        )
        .unwrap_err();
        assert_eq!(err.area, Area::Inst);
        assert!(!Path::new("/tmp/vanta-escape-target").exists());
        let _ = fs::remove_dir_all(&h);
    }

    // M8: a highly compressible archive that decompresses past the cap aborts.
    #[test]
    fn rejects_decompression_bomb() {
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        let big = vec![0u8; 1_000_000]; // 1 MB of zeros, compresses tiny
        let mut header = tar::Header::new_gnu();
        header.set_size(big.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, "big", &big[..]).unwrap();
        let bytes = builder.into_inner().unwrap().finish().unwrap();

        let h = home("bomb");
        let store = Store::open(&h).unwrap();
        let archive_path = store.downloads_dir().join("bomb.tar.gz");
        fs::write(&archive_path, &bytes).unwrap();
        let staging = store.new_staging().unwrap();
        // Cap well below the 1 MB payload → extraction must fail.
        let err = extract("tar.gz", &archive_path, &staging, "tool", 0, 4096).unwrap_err();
        assert_eq!(err.area, Area::Inst);
        let _ = fs::remove_dir_all(&h);
    }
}
