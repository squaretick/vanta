//! `vanta-net` — HTTP downloads over rustls: resumable, retrying, mirror-aware.
//!
//! The installer fetches artifacts through a [`Downloader`]; bytes stream to a
//! `<dest>.part` file (resumed via HTTP range on retry) and are atomically
//! renamed into place on success. Verification (checksum/signature) is the
//! caller's responsibility (`vanta-security` / `vanta-store`); this crate only
//! moves bytes. Parallelism is provided by the installer running downloads on
//! worker threads (`docs/08-installation.md`). See `docs/16-performance.md`.
#![forbid(unsafe_code)]

use reqwest::blocking::Client;
use reqwest::header::RANGE;
use reqwest::StatusCode;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use vanta_core::{Area, VtaError, VtaResult};

/// A reusable HTTP downloader.
pub struct Downloader {
    client: Client,
    retries: u32,
}

impl Downloader {
    /// Build a downloader with sane defaults (TLS via rustls, connect timeout).
    pub fn new() -> VtaResult<Downloader> {
        let client = Client::builder()
            .user_agent(concat!("vanta/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| VtaError::new(Area::Net, 4, format!("building HTTP client: {e}")))?;
        Ok(Downloader { client, retries: 3 })
    }

    /// Override the per-URL retry count (default 3).
    pub fn with_retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// Download `url` into `dest`, resuming a partial `<dest>.part` if present and
    /// retrying transient failures with backoff.
    pub fn download(&self, url: &str, dest: &Path) -> VtaResult<()> {
        let mut last: Option<VtaError> = None;
        for attempt in 0..=self.retries {
            match self.fetch_one(url, dest) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last = Some(e);
                    if attempt < self.retries {
                        std::thread::sleep(backoff(attempt));
                    }
                }
            }
        }
        Err(last.unwrap_or_else(|| VtaError::new(Area::Net, 1, format!("download failed: {url}"))))
    }

    /// Try a primary URL then mirrors/alternates in order, returning on the first
    /// success. A mirror that serves wrong bytes is caught by the caller's hash
    /// verification, so falling through mirrors is safe (`docs/13-offline.md`).
    pub fn download_any(&self, urls: &[String], dest: &Path) -> VtaResult<()> {
        let mut last: Option<VtaError> = None;
        for url in urls {
            match self.download(url, dest) {
                Ok(()) => return Ok(()),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| {
            VtaError::new(Area::Net, 1, "no URLs supplied to download_any".to_string())
        }))
    }

    fn fetch_one(&self, url: &str, dest: &Path) -> VtaResult<()> {
        let part = part_path(dest);
        let have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);

        let mut req = self.client.get(url);
        if have > 0 {
            req = req.header(RANGE, format!("bytes={have}-"));
        }
        let mut resp = req
            .send()
            .map_err(|e| VtaError::new(Area::Net, 1, format!("requesting {url}: {e}")))?;

        let status = resp.status();
        let resuming = have > 0 && status == StatusCode::PARTIAL_CONTENT;
        if !(status.is_success() || resuming) {
            return Err(VtaError::new(
                Area::Net,
                1,
                format!("HTTP {status} for {url}"),
            ));
        }

        if let Some(parent) = part.parent() {
            fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        let mut file = if resuming {
            fs::OpenOptions::new()
                .append(true)
                .open(&part)
                .map_err(|e| io(&part, e))?
        } else {
            let _ = fs::remove_file(&part);
            fs::File::create(&part).map_err(|e| io(&part, e))?
        };

        std::io::copy(&mut resp, &mut file)
            .map_err(|e| VtaError::new(Area::Net, 1, format!("writing {}: {e}", part.display())))?;
        file.sync_all().ok();
        fs::rename(&part, dest).map_err(|e| io(dest, e))?;
        Ok(())
    }
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_os_string();
    s.push(".part");
    PathBuf::from(s)
}

fn backoff(attempt: u32) -> Duration {
    // 0.5s, 1s, 2s, 4s … capped.
    let secs = (1u64 << attempt.min(4)) as f64 * 0.5;
    Duration::from_secs_f64(secs)
}

fn io(path: &Path, e: std::io::Error) -> VtaError {
    VtaError::new(Area::Net, 1, format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_builds() {
        assert!(Downloader::new().is_ok());
    }

    #[test]
    fn part_path_appends_suffix() {
        assert_eq!(
            part_path(Path::new("/tmp/a.bin")),
            PathBuf::from("/tmp/a.bin.part")
        );
    }

    #[test]
    fn download_any_empty_errors() {
        let d = Downloader::new().unwrap();
        assert!(d.download_any(&[], Path::new("/tmp/none")).is_err());
    }
}
