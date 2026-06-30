//! `vanta-security` — verification and policy (the fail-closed gate).
//!
//! Provides the checksum gate (SHA-256 / BLAKE3), Ed25519/minisign signature
//! verification (see [`sign`]), and the organization policy model. An artifact
//! that fails any required check is rejected rather than trusted. See
//! `docs/15-security.md` and `docs/21-threat-model.md`.
#![forbid(unsafe_code)]

pub mod sign;
pub mod trust;
pub use sign::{minisign_verify, parse_minisign_pubkey, MinisignKey};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use vanta_core::{Area, VtaError, VtaResult};

/// Stream a file through SHA-256, returning lowercase hex.
pub fn sha256_file(path: &Path) -> VtaResult<String> {
    let mut hasher = Sha256::new();
    hash_into(path, &mut |chunk| hasher.update(chunk))?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Stream a file through BLAKE3, returning lowercase hex.
pub fn blake3_file(path: &Path) -> VtaResult<String> {
    let mut hasher = blake3::Hasher::new();
    hash_into(path, &mut |chunk| {
        hasher.update(chunk);
    })?;
    Ok(hasher.finalize().to_hex().to_string())
}

fn hash_into(path: &Path, sink: &mut dyn FnMut(&[u8])) -> VtaResult<()> {
    let mut file = File::open(path)
        .map_err(|e| VtaError::new(Area::Vrf, 1, format!("opening {}: {e}", path.display())))?;
    let mut buf = [0u8; 65536];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| VtaError::new(Area::Vrf, 1, format!("reading {}: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        sink(&buf[..n]);
    }
    Ok(())
}

/// Verify a file against an expected checksum, fail-closed. Unknown algorithms
/// are rejected (never silently passed). `VTA-VRF-0001` on mismatch.
pub fn verify_file(path: &Path, algo: &str, expected: &str) -> VtaResult<()> {
    let got = match algo.to_ascii_lowercase().as_str() {
        "sha256" => sha256_file(path)?,
        "blake3" => blake3_file(path)?,
        other => {
            return Err(VtaError::new(
                Area::Vrf,
                2,
                format!("unsupported checksum algorithm `{other}`"),
            ))
        }
    };
    if got.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(VtaError::new(
            Area::Vrf,
            1,
            format!("checksum mismatch ({algo}): expected {expected}, got {got}"),
        ))
    }
}

/// Org policy governing what may be installed (`docs/14-enterprise.md`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub require_signature: bool,
    pub forbid_no_verify: bool,
    pub allow_source_builds: Option<bool>,
    pub min_slsa_level: Option<u8>,
    /// Allowed tool name patterns (`*` suffix wildcard). Empty = allow all.
    pub allow_tools: Vec<String>,
    pub deny_tools: Vec<String>,
    pub allow_licenses: Vec<String>,
}

impl Policy {
    pub fn from_toml(src: &str) -> VtaResult<Policy> {
        toml::from_str(src).map_err(|e| VtaError::new(Area::Vrf, 3, format!("parse policy: {e}")))
    }

    /// Whether a tool name is permitted (deny wins; empty allow = allow all).
    pub fn allows_tool(&self, tool: &str) -> bool {
        if self.deny_tools.iter().any(|p| matches_pattern(p, tool)) {
            return false;
        }
        self.allow_tools.is_empty() || self.allow_tools.iter().any(|p| matches_pattern(p, tool))
    }

    /// Enforce the tool/license rules, returning a policy-denied error if violated.
    pub fn check(&self, tool: &str, license: Option<&str>) -> VtaResult<()> {
        if !self.allows_tool(tool) {
            return Err(VtaError::new(
                Area::Res,
                6,
                format!("tool `{tool}` is denied by org policy"),
            ));
        }
        if let (false, Some(lic)) = (self.allow_licenses.is_empty(), license) {
            if !self
                .allow_licenses
                .iter()
                .any(|l| l.eq_ignore_ascii_case(lic))
            {
                return Err(VtaError::new(
                    Area::Res,
                    6,
                    format!("license `{lic}` for `{tool}` is not in the policy allow-list"),
                ));
            }
        }
        Ok(())
    }
}

fn matches_pattern(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => pattern == name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_sha256_roundtrip() {
        let path = std::env::temp_dir().join(format!("vanta-sec-{}.bin", std::process::id()));
        std::fs::write(&path, b"hello world").unwrap();
        let digest = sha256_file(&path).unwrap();
        assert!(verify_file(&path, "sha256", &digest).is_ok());
        assert!(verify_file(&path, "sha256", "deadbeef").is_err());
        assert!(verify_file(&path, "md5", &digest).is_err()); // unsupported = fail closed
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn policy_tool_rules() {
        let p = Policy {
            allow_tools: vec!["node".into(), "acme/*".into()],
            deny_tools: vec!["leftpad".into()],
            ..Default::default()
        };
        assert!(p.allows_tool("node"));
        assert!(p.allows_tool("acme/deploy"));
        assert!(!p.allows_tool("python")); // not in allow-list
        assert!(!p.allows_tool("leftpad")); // denied
        assert!(p.check("node", None).is_ok());
        assert!(p.check("python", None).is_err());
    }

    #[test]
    fn policy_license_allowlist() {
        let p = Policy {
            allow_licenses: vec!["MIT".into(), "Apache-2.0".into()],
            ..Default::default()
        };
        assert!(p.check("node", Some("MIT")).is_ok());
        assert!(p.check("node", Some("GPL-3.0")).is_err());
    }
}
