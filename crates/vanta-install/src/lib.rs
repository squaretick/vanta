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
use std::path::{Path, PathBuf};
use vanta_core::{Area, Artifact, Platform, StoreKey, VtaError, VtaResult};
use vanta_net::Downloader;
use vanta_state::{GenerationRecord, State, StoreEntryMeta};
use vanta_store::Store;

/// The install engine, bound to a `$VANTA_HOME`.
pub struct Engine {
    store: Store,
    state: State,
    downloader: Downloader,
    home: PathBuf,
}

impl Engine {
    /// Open the engine over `home` (`$VANTA_HOME`).
    pub fn open(home: impl AsRef<Path>) -> VtaResult<Engine> {
        let home = home.as_ref().to_path_buf();
        let store = Store::open(&home)?;
        let state = State::open(&home.join("state.db"))?;
        let downloader = Downloader::new()?;
        Ok(Engine {
            store,
            state,
            downloader,
            home,
        })
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
        // [3 Plan] — if the lock already named a key and it is present, reuse it.
        if let Some(key) = &artifact.store_key {
            if self.store.has(key) {
                self.link_bins(key, &artifact.bin)?;
                self.record(tool, version, key, &artifact.checksum.value)?;
                return Ok(key.clone());
            }
        }

        // [4 Fetch]
        let dl = self
            .store
            .downloads_dir()
            .join(format!("incoming-{tool}-{}", std::process::id()));
        let mut urls = vec![artifact.url.clone()];
        urls.extend(artifact.mirrors.clone());
        self.downloader.download_any(&urls, &dl)?;

        // [5 Verify] — fail closed (centralized in vanta-security).
        if let Err(e) =
            vanta_security::verify_file(&dl, &artifact.checksum.algo, &artifact.checksum.value)
        {
            let _ = fs::remove_file(&dl);
            return Err(e);
        }
        // Signature verification when the registry pinned a signature + trusted key.
        if let (Some(sig), Some(key_text)) = (&artifact.signature, &artifact.signature_key) {
            let key = vanta_security::parse_minisign_pubkey(key_text)?;
            let bytes = fs::read(&dl).map_err(|e| io(&dl, e))?;
            if let Err(e) = vanta_security::minisign_verify(&bytes, sig, &key) {
                let _ = fs::remove_file(&dl);
                return Err(e);
            }
        }

        // [6 Materialize]
        let staging = self.store.new_staging()?;
        let name = artifact
            .bin
            .first()
            .map(|b| basename(b))
            .unwrap_or_else(|| tool.to_string());
        extract(&artifact.archive, &dl, &staging, &name, artifact.strip)?;
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
            let sk = StoreKey::new(key)?;
            let dst = self.store.entry_path(&sk);
            let src = staging.join(key);
            if !dst.exists() && src.is_dir() {
                // Bundled entries are read-only; add write so the dir can be moved.
                let _ = vanta_store::ensure_writable(&src);
                fs::rename(&src, &dst).map_err(|e| io(&dst, e))?;
                restored += 1;
            }
            if !self.store.verify_entry(&sk)? {
                return Err(VtaError::new(
                    Area::Vrf,
                    1,
                    format!("restored entry {key} failed integrity verification"),
                ));
            }
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
pub fn extract(
    archive: &str,
    src: &Path,
    dest: &Path,
    raw_name: &str,
    strip: u32,
) -> VtaResult<()> {
    match archive {
        "tar.gz" | "tgz" => extract_targz(src, dest, strip),
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

fn extract_targz(src: &Path, dest: &Path, strip: u32) -> VtaResult<()> {
    use std::path::{Component, PathBuf};
    let file = fs::File::open(src).map_err(|e| io(src, e))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    archive.set_preserve_permissions(true);
    let entries = archive
        .entries()
        .map_err(|e| VtaError::new(Area::Inst, 1, format!("reading archive: {e}")))?;
    for entry in entries {
        let mut entry = entry
            .map_err(|e| VtaError::new(Area::Inst, 1, format!("reading archive entry: {e}")))?;
        let path = entry
            .path()
            .map_err(|e| VtaError::new(Area::Inst, 1, format!("entry path: {e}")))?
            .into_owned();
        let stripped: PathBuf = path.components().skip(strip as usize).collect();
        if stripped.as_os_str().is_empty() {
            continue;
        }
        // Reject anything that could escape the destination (zip-slip / traversal).
        if stripped.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(VtaError::new(
                Area::Inst,
                1,
                "archive entry escapes destination (path traversal rejected)".to_string(),
            ));
        }
        let out = dest.join(&stripped);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        entry
            .unpack(&out)
            .map_err(|e| VtaError::new(Area::Inst, 1, format!("unpacking entry: {e}")))?;
    }
    Ok(())
}

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
        extract("tar.gz", &archive_path, &staging, "tool", 0).unwrap();
        assert!(staging.join("bin/tool").exists());

        let key = store.publish_tree(&staging).unwrap();
        assert!(store.has(&key));
        assert!(store.verify_entry(&key).unwrap());
        let _ = fs::remove_dir_all(&h);
    }

    #[test]
    fn rejects_unsupported_archive() {
        let err = extract("tar.xz", Path::new("/x"), Path::new("/y"), "t", 0).unwrap_err();
        assert_eq!(err.area, Area::Inst);
    }
}
