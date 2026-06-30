//! Pinned-root trust model for the registry index (audit C1).
//!
//! The registry index is the system's trust root: it names artifact URLs,
//! checksums, detached signatures, **and** the public keys that verify those
//! signatures. If the index itself is unauthenticated, an attacker who controls
//! the index (MITM on a plaintext URL, a malicious `$VANTA_REGISTRY`, a
//! compromised mirror) can supply their own keypair and sign malicious bytes —
//! signature verification then provides zero protection.
//!
//! This module implements a minimal but real pinned-root model:
//!
//!  1. A small set of **root public keys** is pinned out-of-band — compiled in
//!     ([`COMPILED_IN_ROOT_KEYS`]) and/or loaded from the user-owned trusted
//!     config at `<trust_dir>/roots.toml` ([`load_root_keys`]). Roots are
//!     **never** sourced from the fetched index.
//!  2. A fetched index must carry a detached signature that
//!     [`index_signed_by_root`] verifies against one of the pinned roots before
//!     its entries are trusted.
//!  3. A per-artifact signing key (carried in the index) is only trusted if the
//!     index that carried it was itself verified against a pinned root
//!     (transitive trust), **or** that key is itself in the pinned set
//!     ([`artifact_key_is_trusted`]). Otherwise it is treated as unverified.
//!
//! Verification is fail-closed throughout: any parse/length/scheme failure
//! denies trust rather than granting it.

use crate::sign::{minisign_verify, parse_minisign_pubkey};
use serde::Deserialize;
use std::path::Path;

/// Compiled-in trusted root public keys (minisign format, one full key text per
/// entry — the same shape minisign writes, including the `untrusted comment:`
/// line).
///
/// The pinned Vanta release registry root key(s). A fetched network index must
/// carry a detached signature that verifies against one of these before its
/// entries are trusted. The matching secret is held offline by the registry
/// maintainer (see `registry/README.md`); it is never stored in the repository.
///
/// To rotate, append the new public key here (keep the old one until every
/// published index is re-signed), rebuild, then regenerate + re-sign the
/// registry with `cargo xtask registry-gen`.
pub const COMPILED_IN_ROOT_KEYS: &[&str] = &[
    // Vanta official registry root (minisign Ed25519).
    "untrusted comment: vanta registry root key\nRWTKdzWEeVXHgj5NxXCdfaJwJYJ5rdpNJ+MJ4IINh2RlSVBgOOt7QbKL",
];

/// The on-disk shape of `<trust_dir>/roots.toml`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RootsFile {
    /// Full minisign public-key texts, one per pinned root.
    keys: Vec<String>,
}

/// Load the set of pinned root public-key texts: the compiled-in roots plus any
/// the operator placed in the user-owned `<trust_dir>/roots.toml`. These are the
/// only keys ever trusted to authenticate an index; they are never read from a
/// fetched index.
///
/// A missing or malformed `roots.toml` contributes no keys (fail-closed: we do
/// not invent trust on error).
pub fn load_root_keys(trust_dir: &Path) -> Vec<String> {
    let mut roots: Vec<String> = COMPILED_IN_ROOT_KEYS
        .iter()
        .map(|s| s.to_string())
        .collect();
    let path = trust_dir.join("roots.toml");
    if let Ok(src) = std::fs::read_to_string(&path) {
        if let Ok(parsed) = toml::from_str::<RootsFile>(&src) {
            roots.extend(parsed.keys);
        }
    }
    roots
}

/// Whether `index_bytes` carries a detached `signature` produced by one of the
/// pinned `roots`. Tries every root and returns `true` on the first that
/// verifies; returns `false` if none do (including when `roots` is empty).
pub fn index_signed_by_root(index_bytes: &[u8], signature: &str, roots: &[String]) -> bool {
    roots.iter().any(|root| match parse_minisign_pubkey(root) {
        Ok(key) => minisign_verify(index_bytes, signature, &key).is_ok(),
        Err(_) => false,
    })
}

/// Whether a per-artifact signing key (carried by the index) may be trusted.
///
/// Trusted iff the index was verified against a pinned root (`index_verified`,
/// transitive trust) **or** the key itself is one of the pinned roots. Otherwise
/// the key is attacker-influenceable and must be treated as unverified.
pub fn artifact_key_is_trusted(artifact_key: &str, index_verified: bool, roots: &[String]) -> bool {
    index_verified || key_in_roots(artifact_key, roots)
}

/// Compare a public-key text against the pinned set by its canonical base64
/// payload line (ignores comment lines / surrounding whitespace).
fn key_in_roots(key: &str, roots: &[String]) -> bool {
    match payload_line(key) {
        Some(target) => roots.iter().any(|r| payload_line(r) == Some(target)),
        None => false,
    }
}

/// Extract a minisign key's base64 payload line (the last non-empty,
/// non-comment line), mirroring [`parse_minisign_pubkey`]'s selection.
fn payload_line(text: &str) -> Option<&str> {
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build a minisign keypair (from `seed`) plus a legacy `Ed` detached
    // signature over `data`; returns `(public_key_text, signature_text)`.
    fn sign(seed: [u8; 32], data: &[u8]) -> (String, String) {
        use base64::{engine::general_purpose::STANDARD, Engine};
        use ed25519_dalek::{Signer, SigningKey};
        let key_id = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let sk = SigningKey::from_bytes(&seed);
        let pk = sk.verifying_key().to_bytes();
        let sig = sk.sign(data).to_bytes();
        let mut pk_raw = b"Ed".to_vec();
        pk_raw.extend_from_slice(&key_id);
        pk_raw.extend_from_slice(&pk);
        let pubkey = format!("untrusted comment: test\n{}", STANDARD.encode(&pk_raw));
        let mut sig_raw = b"Ed".to_vec();
        sig_raw.extend_from_slice(&key_id);
        sig_raw.extend_from_slice(&sig);
        let sig_file = format!(
            "untrusted comment: test sig\n{}\ntrusted comment: t\n{}",
            STANDARD.encode(&sig_raw),
            STANDARD.encode([0u8; 64])
        );
        (pubkey, sig_file)
    }

    #[test]
    fn index_verified_only_against_pinned_root() {
        let index = b"[tools.node]\n# the registry index bytes";
        let (root_pub, sig) = sign([7u8; 32], index);
        let roots = [root_pub];
        // Signed by the pinned root → trusted.
        assert!(index_signed_by_root(index, &sig, &roots));
        // No pinned roots → cannot be trusted.
        assert!(!index_signed_by_root(index, &sig, &[]));
        // A different (attacker) key is pinned → the index's signature does not
        // verify against it → rejected.
        let (attacker_pub, _) = sign([9u8; 32], b"unrelated");
        assert!(!index_signed_by_root(index, &sig, &[attacker_pub]));
        // Tampered index bytes → signature no longer verifies.
        assert!(!index_signed_by_root(b"tampered", &sig, &roots));
    }

    #[test]
    fn artifact_key_trust_rules() {
        let (attacker_pub, _) = sign([3u8; 32], b"x");
        let (root_pub, _) = sign([4u8; 32], b"y");
        let roots = [root_pub.clone()];

        // Unsigned/unverified index + attacker-supplied key → NOT trusted.
        assert!(!artifact_key_is_trusted(&attacker_pub, false, &roots));
        // Verified index → transitive trust of whatever key it carried.
        assert!(artifact_key_is_trusted(&attacker_pub, true, &roots));
        // Unverified index, but the artifact key IS a pinned root → trusted.
        assert!(artifact_key_is_trusted(&root_pub, false, &roots));
    }
}
