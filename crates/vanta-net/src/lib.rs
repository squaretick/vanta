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
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use vanta_core::{Area, VtaError, VtaResult};

/// Maximum number of HTTP redirects to follow (audit M6).
const MAX_REDIRECTS: usize = 10;

/// A reusable HTTP downloader.
pub struct Downloader {
    client: Client,
    retries: u32,
    /// When `true`, plaintext `http://` URLs to non-loopback hosts are permitted
    /// (audit M6 insecure opt-in). Default `false`: only `https://` (and
    /// loopback `http://`, for local dev/test servers) is allowed.
    allow_http: bool,
}

impl Downloader {
    /// Build a secure downloader (TLS via rustls, connect timeout). Plaintext
    /// `http://` to non-loopback hosts is rejected and `https→http` downgrade
    /// redirects are refused.
    pub fn new() -> VtaResult<Downloader> {
        Self::build(false)
    }

    /// Build a downloader that additionally permits plaintext `http://` to any
    /// host. This is the **dangerous** insecure opt-in (audit M6/C1): callers
    /// must surface it to the operator. `https→http` downgrade redirects are
    /// still refused.
    pub fn insecure() -> VtaResult<Downloader> {
        Self::build(true)
    }

    fn build(allow_http: bool) -> VtaResult<Downloader> {
        // Redirect policy (M6): cap the chain and never follow an https→http
        // downgrade, regardless of `allow_http`.
        let redirect = reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if attempt.url().scheme() == "http"
                && attempt
                    .previous()
                    .last()
                    .map(|u| u.scheme() == "https")
                    .unwrap_or(false)
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        });
        // NOTE: we intentionally do NOT call reqwest's `.https_only(true)` here.
        // That would reject *all* `http://`, including the loopback dev/test
        // servers Vanta's own integration tests (and local mirrors) rely on.
        // Instead TLS is enforced per-request in `scheme_ok` (reject plaintext to
        // non-loopback hosts) and the custom redirect policy above refuses any
        // https→http downgrade — together giving the same guarantee for real
        // hosts without breaking loopback.
        let client = Client::builder()
            .user_agent(concat!("vanta/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            .redirect(redirect)
            .build()
            .map_err(|e| VtaError::new(Area::Net, 4, format!("building HTTP client: {e}")))?;
        Ok(Downloader {
            client,
            retries: 3,
            allow_http,
        })
    }

    /// Override the per-URL retry count (default 3).
    pub fn with_retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// Download `url` into `dest`, resuming a partial `<dest>.part` if present and
    /// retrying transient failures with backoff. No size ceiling.
    pub fn download(&self, url: &str, dest: &Path) -> VtaResult<()> {
        self.download_capped(url, dest, None)
    }

    /// Like [`Downloader::download`] but aborts with an error if more than
    /// `max` bytes would be written (audit M8). `None` means no ceiling.
    pub fn download_capped(&self, url: &str, dest: &Path, max: Option<u64>) -> VtaResult<()> {
        self.scheme_ok(url)?;
        let mut last: Option<VtaError> = None;
        for attempt in 0..=self.retries {
            match self.fetch_one(url, dest, max) {
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
    /// `max`, when set, caps the bytes accepted from any single URL (M8).
    pub fn download_any(&self, urls: &[String], dest: &Path, max: Option<u64>) -> VtaResult<()> {
        let mut last: Option<VtaError> = None;
        for url in urls {
            // L11: never resume across a mirror switch — a stale `.part` from a
            // previous host would otherwise be concatenated with this host's
            // bytes. Start each URL from a clean slate.
            let _ = fs::remove_file(part_path(dest));
            match self.download_capped(url, dest, max) {
                Ok(()) => return Ok(()),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| {
            VtaError::new(Area::Net, 1, "no URLs supplied to download_any".to_string())
        }))
    }

    /// Reject plaintext `http://` to non-loopback hosts unless the insecure
    /// opt-in is set (audit M6).
    fn scheme_ok(&self, url: &str) -> VtaResult<()> {
        if let Some(rest) = url.strip_prefix("http://") {
            if !self.allow_http && !is_loopback_authority(rest) {
                return Err(VtaError::new(
                    Area::Net,
                    5,
                    format!(
                        "refusing plaintext http:// download of {url} \
                         (https required; set the insecure opt-in to override)"
                    ),
                ));
            }
        }
        Ok(())
    }

    fn fetch_one(&self, url: &str, dest: &Path, max: Option<u64>) -> VtaResult<()> {
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

        // M8: enforce the declared size as a hard ceiling on total bytes.
        let remaining =
            match max {
                Some(m) => Some(m.checked_sub(if resuming { have } else { 0 }).ok_or_else(
                    || VtaError::new(Area::Net, 6, format!("download of {url} exceeds size cap")),
                )?),
                None => None,
            };

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

        let written = match remaining {
            // Read one byte past the limit so we can detect an oversize body.
            Some(limit) => {
                let mut limited = (&mut resp).take(limit.saturating_add(1));
                let n = std::io::copy(&mut limited, &mut file).map_err(|e| {
                    VtaError::new(Area::Net, 1, format!("writing {}: {e}", part.display()))
                })?;
                if n > limit {
                    let _ = fs::remove_file(&part);
                    return Err(VtaError::new(
                        Area::Net,
                        6,
                        format!("download of {url} exceeds declared size {limit} bytes"),
                    ));
                }
                n
            }
            None => std::io::copy(&mut resp, &mut file).map_err(|e| {
                VtaError::new(Area::Net, 1, format!("writing {}: {e}", part.display()))
            })?,
        };
        let _ = written;
        file.sync_all().ok();
        fs::rename(&part, dest).map_err(|e| io(dest, e))?;
        Ok(())
    }
}

/// Whether the authority part of an `http://` URL (everything after the scheme)
/// names a loopback host. Used to keep local dev/test servers usable while still
/// rejecting plaintext to public hosts.
fn is_loopback_authority(rest: &str) -> bool {
    // authority is up to the first '/', '?' or '#'.
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(rest)
        .trim_end_matches('.');
    // strip userinfo
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(stripped) = host_port.strip_prefix('[') {
        // IPv6 literal: [::1]:port
        stripped.split(']').next().unwrap_or(stripped)
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };
    host == "localhost" || host == "::1" || host.starts_with("127.")
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
        assert!(d.download_any(&[], Path::new("/tmp/none"), None).is_err());
    }

    #[test]
    fn rejects_plaintext_http_scheme() {
        // M6: a secure downloader refuses http:// to a non-loopback host before
        // any network I/O.
        let d = Downloader::new().unwrap();
        let err = d
            .download("http://example.org/x", Path::new("/tmp/should-not-write"))
            .unwrap_err();
        assert_eq!(err.area, Area::Net);
        assert_eq!(err.number, 5);
        // https is accepted at the scheme gate (it will fail later on network,
        // but not on scheme).
        assert!(matches!(d.scheme_ok("https://example.org/x"), Ok(())));
    }

    #[test]
    fn loopback_http_is_allowed_scheme() {
        // Local dev/test servers serve plaintext on 127.0.0.1 — permitted.
        assert!(is_loopback_authority("127.0.0.1:8080/x"));
        assert!(is_loopback_authority("localhost/x"));
        assert!(is_loopback_authority("[::1]:9/x"));
        assert!(!is_loopback_authority("example.org/x"));
        assert!(!is_loopback_authority("127x.evil.com/x"));
    }

    #[test]
    fn insecure_allows_http() {
        let d = Downloader::insecure().unwrap();
        assert!(matches!(d.scheme_ok("http://example.org/x"), Ok(())));
    }

    #[test]
    fn size_cap_aborts_oversize_download() {
        // M8: a body larger than the declared ceiling is rejected.
        use std::collections::HashMap;
        let mut files = HashMap::new();
        files.insert("/big".to_string(), vec![0u8; 10_000]);
        let port = vanta_test::serve(files);
        let d = Downloader::new().unwrap();
        let dest = std::env::temp_dir().join(format!("vanta-net-cap-{}.bin", std::process::id()));
        let _ = fs::remove_file(&dest);
        let url = format!("http://127.0.0.1:{port}/big");
        // Cap below the body size → error, no file published.
        let err = d.download_capped(&url, &dest, Some(1000)).unwrap_err();
        assert_eq!(err.number, 6);
        assert!(!dest.exists());
        // Cap at/above the body size → succeeds.
        assert!(d.download_capped(&url, &dest, Some(10_000)).is_ok());
        let _ = fs::remove_file(&dest);
    }
}
