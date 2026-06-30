//! Hermetic end-to-end test of the install pipeline against a fake HTTP server
//! (`docs/28-testing.md`): fetch → verify(sha256) → extract(strip) → publish →
//! link → generation, with no real network.

use std::collections::HashMap;
use vanta_core::{Artifact, Checksum, StoreKey};
use vanta_install::Engine;
use vanta_security::Policy;
use vanta_test::{make_targz, serve, sha256_hex};

#[test]
fn installs_and_links_from_fake_server() {
    let tar = make_targz("tool-1.0.0", &[("bin/tool", b"#!/bin/sh\necho ok\n")]);
    let sha = sha256_hex(&tar);
    let mut files = HashMap::new();
    files.insert("/tool.tar.gz".to_string(), tar);
    let port = serve(files);

    let home = std::env::temp_dir().join(format!("vanta-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let engine = Engine::open(&home).unwrap();

    let artifact = Artifact {
        url: format!("http://127.0.0.1:{port}/tool.tar.gz"),
        mirrors: vec![],
        archive: "tar.gz".to_string(),
        size: None,
        checksum: Checksum {
            algo: "sha256".to_string(),
            value: sha,
        },
        signature: None,
        signature_key: None,
        bin: vec!["bin/tool".to_string()],
        strip: 1,
        store_key: None,
    };

    let key = engine.install_artifact("tool", "1.0.0", &artifact).unwrap();
    assert!(engine.store().has(&key));
    assert!(engine.store().verify_entry(&key).unwrap());
    // Executable linked onto ~/.vanta/bin.
    assert!(home.join("bin").join("tool").exists());

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn rejects_bad_checksum_fail_closed() {
    let tar = make_targz("tool-1.0.0", &[("bin/tool", b"x")]);
    let mut files = HashMap::new();
    files.insert("/tool.tar.gz".to_string(), tar);
    let port = serve(files);

    let home = std::env::temp_dir().join(format!("vanta-e2e-bad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let engine = Engine::open(&home).unwrap();

    let artifact = Artifact {
        url: format!("http://127.0.0.1:{port}/tool.tar.gz"),
        mirrors: vec![],
        archive: "tar.gz".to_string(),
        size: None,
        checksum: Checksum {
            algo: "sha256".to_string(),
            value: "deadbeef".to_string(),
        },
        signature: None,
        signature_key: None,
        bin: vec!["bin/tool".to_string()],
        strip: 1,
        store_key: None,
    };
    assert!(engine.install_artifact("tool", "1.0.0", &artifact).is_err());
    let _ = std::fs::remove_dir_all(&home);
}

// H2: when the policy requires a signature, an unsigned artifact is rejected
// before any materialization (fail-closed).
#[test]
fn missing_signature_under_required_policy_errors() {
    let tar = make_targz("tool-1.0.0", &[("bin/tool", b"ok")]);
    let sha = sha256_hex(&tar);
    let mut files = HashMap::new();
    files.insert("/t.tar.gz".to_string(), tar);
    let port = serve(files);

    let home = std::env::temp_dir().join(format!("vanta-e2e-reqsig-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let policy = Policy {
        require_signature: true,
        ..Default::default()
    };
    let engine = Engine::open_with_policy(&home, policy).unwrap();
    let artifact = Artifact {
        url: format!("http://127.0.0.1:{port}/t.tar.gz"),
        mirrors: vec![],
        archive: "tar.gz".to_string(),
        size: None,
        checksum: Checksum {
            algo: "sha256".to_string(),
            value: sha,
        },
        signature: None,
        signature_key: None,
        bin: vec!["bin/tool".to_string()],
        strip: 1,
        store_key: None,
    };
    let err = engine
        .install_artifact("tool", "1.0.0", &artifact)
        .unwrap_err();
    assert_eq!(err.area, vanta_core::Area::Vrf);
    let _ = std::fs::remove_dir_all(&home);
}

// H4: a store hit whose contents no longer match its key is not trusted — the
// engine drops the poisoned entry and re-fetches + re-verifies.
#[test]
fn store_hit_reverifies_and_recovers_from_corruption() {
    let tar = make_targz("tool-1.0.0", &[("bin/tool", b"#!/bin/sh\necho ok\n")]);
    let sha = sha256_hex(&tar);
    let mut files = HashMap::new();
    files.insert("/t.tar.gz".to_string(), tar);
    let port = serve(files);

    let home = std::env::temp_dir().join(format!("vanta-e2e-hit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let engine = Engine::open(&home).unwrap();
    let mut artifact = Artifact {
        url: format!("http://127.0.0.1:{port}/t.tar.gz"),
        mirrors: vec![],
        archive: "tar.gz".to_string(),
        size: None,
        checksum: Checksum {
            algo: "sha256".to_string(),
            value: sha,
        },
        signature: None,
        signature_key: None,
        bin: vec!["bin/tool".to_string()],
        strip: 1,
        store_key: None,
    };
    // First install populates the store.
    let key = engine.install_artifact("tool", "1.0.0", &artifact).unwrap();

    // Corrupt the store entry in place.
    let entry = engine.store().entry_path(&key);
    let _ = vanta_store::ensure_writable(&entry);
    std::fs::write(entry.join("bin").join("tool"), b"TAMPERED").unwrap();
    assert!(!engine.store().verify_entry(&key).unwrap());

    // Re-install with the lock naming the (now corrupted) key. The engine must
    // NOT trust the hit; it re-downloads and the entry verifies clean again.
    artifact.store_key = Some(key.clone());
    let key2 = engine.install_artifact("tool", "1.0.0", &artifact).unwrap();
    assert_eq!(key, key2);
    assert!(engine.store().verify_entry(&key2).unwrap());
    let _ = std::fs::remove_dir_all(&home);
}

// H3: a bundle whose payload does not hash to its claimed `blake3-<hash>` dir
// name is rejected, and the store is left unchanged (no poisoned entry).
#[test]
fn restore_rejects_content_mismatch_without_poisoning_store() {
    use flate2::write::GzEncoder;
    use flate2::Compression;

    let fake_key = format!("blake3-{}", "a".repeat(64));
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    // KEYS lists the fake key.
    let keys = fake_key.clone();
    let mut kh = tar::Header::new_gnu();
    kh.set_size(keys.len() as u64);
    kh.set_mode(0o644);
    kh.set_cksum();
    builder
        .append_data(&mut kh, "KEYS", keys.as_bytes())
        .unwrap();
    // A dir named after the fake key whose contents will NOT hash to it.
    let payload = b"not the real content";
    let mut fh = tar::Header::new_gnu();
    fh.set_size(payload.len() as u64);
    fh.set_mode(0o644);
    fh.set_cksum();
    builder
        .append_data(&mut fh, format!("{fake_key}/file"), &payload[..])
        .unwrap();
    let bundle_bytes = builder.into_inner().unwrap().finish().unwrap();

    let home = std::env::temp_dir().join(format!("vanta-e2e-restore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let engine = Engine::open(&home).unwrap();
    let bundle_path = home.join("evil.vbundle");
    std::fs::write(&bundle_path, &bundle_bytes).unwrap();

    let result = engine.restore(&bundle_path);
    assert!(result.is_err());
    // The store must be untouched.
    let sk = StoreKey::new(fake_key).unwrap();
    assert!(!engine.store().has(&sk));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn installs_with_valid_signature() {
    let tar = make_targz("tool-1.0.0", &[("bin/tool", b"signed-ok")]);
    let sha = sha256_hex(&tar);
    let (pubkey, sig) = vanta_test::minisign_sign([9u8; 32], &tar);
    let mut files = HashMap::new();
    files.insert("/t.tar.gz".to_string(), tar);
    let port = serve(files);

    let home = std::env::temp_dir().join(format!("vanta-e2e-sig-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let engine = Engine::open(&home).unwrap();
    let artifact = Artifact {
        url: format!("http://127.0.0.1:{port}/t.tar.gz"),
        mirrors: vec![],
        archive: "tar.gz".to_string(),
        size: None,
        checksum: Checksum {
            algo: "sha256".to_string(),
            value: sha,
        },
        signature: Some(sig),
        signature_key: Some(pubkey),
        bin: vec!["bin/tool".to_string()],
        strip: 1,
        store_key: None,
    };
    let key = engine.install_artifact("tool", "1.0.0", &artifact).unwrap();
    assert!(engine.store().has(&key));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn rejects_signature_from_wrong_key() {
    let tar = make_targz("tool-1.0.0", &[("bin/tool", b"signed")]);
    let sha = sha256_hex(&tar);
    let (_, sig) = vanta_test::minisign_sign([1u8; 32], &tar); // signed with key A
    let (other_key, _) = vanta_test::minisign_sign([2u8; 32], b"unrelated"); // key B's pubkey
    let mut files = HashMap::new();
    files.insert("/t.tar.gz".to_string(), tar);
    let port = serve(files);

    let home = std::env::temp_dir().join(format!("vanta-e2e-sigbad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let engine = Engine::open(&home).unwrap();
    let artifact = Artifact {
        url: format!("http://127.0.0.1:{port}/t.tar.gz"),
        mirrors: vec![],
        archive: "tar.gz".to_string(),
        size: None,
        checksum: Checksum {
            algo: "sha256".to_string(),
            value: sha,
        },
        signature: Some(sig),
        signature_key: Some(other_key),
        bin: vec!["bin/tool".to_string()],
        strip: 1,
        store_key: None,
    };
    assert!(engine.install_artifact("tool", "1.0.0", &artifact).is_err());
    let _ = std::fs::remove_dir_all(&home);
}
