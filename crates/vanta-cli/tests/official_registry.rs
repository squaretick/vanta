//! Integration tests for the generated, root-signed official registry
//! (`registry/registry.toml` + `.minisig`). These prove the on-disk index is
//! both authentic (signed by the compiled-in pinned root) and usable (parses
//! and resolves a tool for the running platform).

use std::path::PathBuf;
use vanta_core::{Platform, Request};
use vanta_registry::Registry;
use vanta_resolve::{artifact_for, Resolver};
use vanta_security::trust::{index_signed_by_root, COMPILED_IN_ROOT_KEYS};

/// `<repo>/registry`.
fn registry_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("registry")
}

#[test]
fn official_index_is_signed_by_pinned_root() {
    let dir = registry_dir();
    let index = std::fs::read(dir.join("registry.toml")).expect("read registry.toml");
    let sig = std::fs::read_to_string(dir.join("registry.toml.minisig"))
        .expect("read registry.toml.minisig");
    let roots: Vec<String> = COMPILED_IN_ROOT_KEYS
        .iter()
        .map(|s| s.to_string())
        .collect();

    assert!(
        !roots.is_empty(),
        "COMPILED_IN_ROOT_KEYS must be baked with the release root key"
    );
    assert!(
        index_signed_by_root(&index, &sig, &roots),
        "the generated .minisig must verify against the compiled-in root key"
    );
    // A one-byte change to the index must invalidate the signature (sanity).
    let mut tampered = index.clone();
    tampered.push(b'\n');
    assert!(!index_signed_by_root(&tampered, &sig, &roots));
}

#[test]
fn official_index_parses_and_resolves_for_current_platform() {
    let dir = registry_dir();
    let src = std::fs::read_to_string(dir.join("registry.toml")).expect("read registry.toml");
    let registry = Registry::from_toml(&src).expect("registry.toml must parse");
    assert!(registry.tool("jq").is_some(), "jq must be present");

    // jq covers all non-windows platforms, so it resolves on whatever host runs
    // the test suite.
    let resolver = Resolver::new(&registry);
    let platform = Platform::current();
    let resolution = resolver
        .resolve(
            &Request::parse("jq@1.7.1").expect("parse request"),
            &[platform],
        )
        .expect("jq@1.7.1 must resolve for the current platform");
    let artifact = artifact_for(&resolution, &platform).expect("jq artifact for current platform");

    assert_eq!(artifact.checksum.algo, "sha256");
    assert_eq!(artifact.checksum.value.len(), 64, "sha256 hex digest");
    assert!(artifact.url.starts_with("https://github.com/jqlang/jq/"));
}
