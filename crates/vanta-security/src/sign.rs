//! Minisign (Ed25519) signature verification (`docs/15-security.md`).
//!
//! Supports both legacy (`Ed`, signs the file) and prehashed (`ED`, signs the
//! BLAKE2b-512 of the file) minisign signatures. Verification is the
//! security-relevant operation; key management/distribution is wired with the
//! registry (`docs/26-registry-and-metadata-reference.md`).

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use blake2::{Blake2b512, Digest};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use vanta_core::{Area, VtaError, VtaResult};

/// A parsed minisign public key.
pub struct MinisignKey {
    pub key_id: [u8; 8],
    vk: VerifyingKey,
}

/// Parse a minisign public key (the file is a comment line + a base64 line).
pub fn parse_minisign_pubkey(text: &str) -> VtaResult<MinisignKey> {
    let b64 = text
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
        .ok_or_else(|| err("empty public key"))?;
    let raw = decode(b64)?;
    if raw.len() != 42 {
        return Err(err("public key has wrong length"));
    }
    // raw = algo[2] ("Ed") + key_id[8] + ed25519_pk[32]
    let mut key_id = [0u8; 8];
    key_id.copy_from_slice(&raw[2..10]);
    let mut pk = [0u8; 32];
    pk.copy_from_slice(&raw[10..42]);
    let vk = VerifyingKey::from_bytes(&pk).map_err(|e| err(&format!("bad public key: {e}")))?;
    Ok(MinisignKey { key_id, vk })
}

/// Verify a `.minisig` signature over `data` against `key`. `VTA-VRF-0002` on failure.
pub fn minisign_verify(data: &[u8], sig_file: &str, key: &MinisignKey) -> VtaResult<()> {
    // The first base64 line is the signature; a later one is the global signature.
    let sig_b64 = sig_file
        .lines()
        .map(str::trim)
        .find(|l| {
            !l.is_empty()
                && !l.starts_with("untrusted comment:")
                && !l.starts_with("trusted comment:")
        })
        .ok_or_else(|| err("no signature line"))?;
    let raw = decode(sig_b64)?;
    if raw.len() != 74 {
        return Err(err("signature has wrong length"));
    }
    // raw = algo[2] + key_id[8] + sig[64]
    let algo = &raw[0..2];
    if raw[2..10] != key.key_id {
        return Err(err("signature key id does not match the trusted key"));
    }
    let mut sig_bytes = [0u8; 64];
    sig_bytes.copy_from_slice(&raw[10..74]);
    let signature = Signature::from_bytes(&sig_bytes);

    let result = if algo == b"ED" {
        // Prehashed: the signature covers BLAKE2b-512(data).
        let mut hasher = Blake2b512::new();
        hasher.update(data);
        let digest = hasher.finalize();
        key.vk.verify(digest.as_slice(), &signature)
    } else {
        key.vk.verify(data, &signature)
    };
    result.map_err(|_| err("signature verification failed"))
}

fn decode(s: &str) -> VtaResult<Vec<u8>> {
    STANDARD
        .decode(s.trim())
        .map_err(|e| err(&format!("base64 decode: {e}")))
}

fn err(msg: &str) -> VtaError {
    VtaError::new(Area::Vrf, 2, msg.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    /// Build a minisign pubkey string and a `.minisig` for `data` (legacy `Ed`).
    fn make(seed: [u8; 32], key_id: [u8; 8], data: &[u8]) -> (String, String) {
        let sk = SigningKey::from_bytes(&seed);
        let pk = sk.verifying_key().to_bytes();
        let sig = sk.sign(data).to_bytes();

        let mut pk_raw = Vec::new();
        pk_raw.extend_from_slice(b"Ed");
        pk_raw.extend_from_slice(&key_id);
        pk_raw.extend_from_slice(&pk);
        let pubkey = format!("untrusted comment: test\n{}", STANDARD.encode(&pk_raw));

        let mut sig_raw = Vec::new();
        sig_raw.extend_from_slice(b"Ed");
        sig_raw.extend_from_slice(&key_id);
        sig_raw.extend_from_slice(&sig);
        let sig_file = format!(
            "untrusted comment: sig\n{}\ntrusted comment: t\n{}",
            STANDARD.encode(&sig_raw),
            STANDARD.encode([0u8; 64])
        );
        (pubkey, sig_file)
    }

    #[test]
    fn verifies_valid_signature() {
        let (pubkey, sig) = make([7u8; 32], [1, 2, 3, 4, 5, 6, 7, 8], b"hello world");
        let key = parse_minisign_pubkey(&pubkey).unwrap();
        assert!(minisign_verify(b"hello world", &sig, &key).is_ok());
    }

    #[test]
    fn rejects_tampered_data() {
        let (pubkey, sig) = make([7u8; 32], [1, 2, 3, 4, 5, 6, 7, 8], b"hello world");
        let key = parse_minisign_pubkey(&pubkey).unwrap();
        let err = minisign_verify(b"HELLO WORLD", &sig, &key).unwrap_err();
        assert_eq!(err.area, Area::Vrf);
    }

    #[test]
    fn rejects_wrong_key_id() {
        let (pubkey, _) = make([7u8; 32], [9, 9, 9, 9, 9, 9, 9, 9], b"data");
        let (_, sig) = make([7u8; 32], [1, 1, 1, 1, 1, 1, 1, 1], b"data");
        let key = parse_minisign_pubkey(&pubkey).unwrap();
        assert!(minisign_verify(b"data", &sig, &key).is_err());
    }
}
