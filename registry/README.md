# Vanta official tool registry

`registry.toml` is the official, root-signed index that `vanta add <tool>@<version>`
uses by default. `registry.toml.minisig` is its detached minisign signature over
the exact bytes of `registry.toml`, produced by the pinned **registry root key**.

Both files are **generated** — do not hand-edit. Regenerate and re-sign with the
`registry-gen` xtask (below).

## Trust model

The index is the system's trust anchor (see `crates/vanta-security/src/trust.rs`,
audit C1). The CLI:

1. Fetches `registry.toml` and `registry.toml.minisig` over HTTPS.
2. Verifies the signature against the **compiled-in pinned root key**
   (`COMPILED_IN_ROOT_KEYS`) — or a user-pinned root in
   `~/.vanta/trust/roots.toml` — before trusting any entry.
3. Gates every artifact on its `sha256`; the install engine refuses on mismatch.

A local-file index (`VANTA_REGISTRY=/path/to/registry.toml`) is treated as
user-owned and trusted without a signature.

## Per-artifact signatures: intentionally unset

Entries carry **no** per-artifact `signature` / `public_key`. The trust anchor is
the **root-signed index plus the per-artifact `sha256`**:

- The root signature authenticates the whole index (so the checksums cannot be
  tampered with in transit).
- The `sha256` then gates the downloaded bytes (so the artifact cannot be
  swapped).

This is sufficient and avoids re-signing every upstream asset. If you later want
defence-in-depth per-artifact signatures, set `[tools.<name>].public_key` and each
platform's `signature`; the resolver will only propagate that key as trusted when
the index itself was root-verified (transitive trust) or the key is itself pinned.

## Platform tokens

Checksums are keyed by Vanta's canonical platform tokens
(`crates/vanta-core/src/platform.rs`): `linux/x86_64/gnu`, `linux/aarch64/gnu`,
`macos/x86_64`, `macos/aarch64`, `windows/x86_64`. Note that a provider's
`url_template` is rendered from the `{os}`/`{arch}` tokens only (there is no
`{libc}` placeholder) and an entry has one `archive` kind, so a single tool entry
serves one URL pattern + one archive kind. Consequences in the seed set:

- **Linux** uses the statically-linked **musl** build where one exists (it runs on
  any glibc system too), stored under the `…/gnu` token that `Platform::current()`
  reports on Linux.
- **Windows** is omitted for tools whose Windows asset is a `.zip` (node, go,
  ripgrep, fd, uv) or has a different binary layout (python), because the archive
  kind / `bin` paths differ from the unix entry. jq's Windows asset is omitted
  because the raw template cannot express its `.exe` suffix.

## Seed tools

| Tool | Versions | Source of distribution | Checksum source |
|------|----------|------------------------|-----------------|
| node | 22.11.0, 20.18.0 | nodejs.org tarballs | `SHASUMS256.txt` |
| go | 1.23.4, 1.22.10 | go.dev tarballs | `?mode=json` (sha256 + size) |
| ripgrep | 14.1.1 | GitHub release (BurntSushi/ripgrep) | download + sha256 |
| fd | 10.2.0 | GitHub release (sharkdp/fd) | download + sha256 |
| jq | 1.7.1 | GitHub release (jqlang/jq), raw binary | download + sha256 |
| uv | 0.5.11 | GitHub release (astral-sh/uv) | download + sha256 |
| python | 3.12.7, 3.11.10 | astral-sh **python-build-standalone** (`install_only`, release tag `20241016`) | `<asset>.sha256` sidecar |

**python distribution choice:** we use astral-sh/python-build-standalone
`install_only` tarballs. Their filenames embed both the CPython version and a
build-tag (`cpython-3.12.7+20241016-…`), and the release tag is a date; we pin a
single release (`20241016`) so the build-tag is constant, and store the version as
`X.Y.Z+20241016` (valid SemVer build metadata, so `python@3.12` still resolves).

## Regenerating and signing

```sh
# 1. (one-time) mint the root keypair. Writes the UNENCRYPTED secret to <path>
#    and prints the minisign public key. NEVER commit the secret.
cargo run -p xtask -- keygen /secure/offline/vanta-registry-root.key
#    → paste the printed public key into COMPILED_IN_ROOT_KEYS
#      (crates/vanta-security/src/trust.rs) and rebuild.

# 2. regenerate registry.toml from real upstream checksums and sign it.
VANTA_ROOT_KEY=/secure/offline/vanta-registry-root.key \
    cargo run -p xtask -- registry-gen
#    → writes registry/registry.toml and registry/registry.toml.minisig
```

The generator is **resilient and idempotent**: an asset/version/platform that
cannot be resolved is logged and skipped (a per-line `ok`/`skip` log plus an
`included`/`skipped` summary is printed), and output is fully sorted so a re-run
over unchanged upstreams reproduces byte-identical files (and an identical
deterministic Ed25519 signature).

## Maintainer responsibility for the root key

The root **private key** is the registry's sole trust anchor. Whoever holds it can
sign an index that every Vanta install will trust. Therefore:

- **Keep it offline.** It must never be committed, checked into CI logs, or stored
  alongside the repository. The example dev key used to bootstrap this registry
  lives only outside the tree.
- **Restrict access** to the release maintainer(s); store it encrypted at rest.
- **Rotate by appending**, never replacing: add the new public key to
  `COMPILED_IN_ROOT_KEYS` (keeping the old one until every published index is
  re-signed), ship that build, then regenerate + re-sign with the new key.
- **Compromise response:** remove the leaked key from `COMPILED_IN_ROOT_KEYS`,
  cut a release, and re-sign the registry with a fresh root key.
