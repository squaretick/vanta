//! Hermetic end-to-end test of the install pipeline against a fake HTTP server
//! (`docs/28-testing.md`): fetch → verify(sha256) → extract(strip) → publish →
//! link → generation, with no real network.

use std::collections::HashMap;
use vanta_core::{Artifact, Checksum};
use vanta_install::Engine;
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
