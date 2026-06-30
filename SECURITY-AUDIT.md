# Vanta Security Audit — Supply-Chain Focus

Date: 2026-06-30. Scope read in full: `vanta-security` (sign.rs, lib.rs), `vanta-net`,
`vanta-store`, `vanta-install`, `vanta-registry`, `vanta-resolve`, `vanta-provider`,
`vanta-lock`, `vanta-env`, `vanta-shim`, `vanta-config`, `vanta-core`, plus a workspace-wide
grep sweep for `unsafe`/`unwrap`/`panic`/`Command`/`http`.

**Memory safety:** every crate `#![forbid(unsafe_code)]` — zero `unsafe`.
**No panics on untrusted input:** all `unwrap`/`expect`/`panic!` are in `#[cfg(test)]`/`mod fuzz`.

### Verdict on "verified, fail-closed"
The cryptographic primitives (`minisign_verify`, `verify_file`) are individually correct and
fail-closed. **But the trust chain that feeds them is broken: the signing key, the signature,
and the checksum all originate from the same unauthenticated registry document. The claim does
NOT hold end-to-end.**

---

## CRITICAL

### C1 — Registry is the sole trust root but is unauthenticated, and the signing key ships inside it
`crates/vanta-cli/src/lib.rs:937-949` (`load_registry`, accepts `http://` at :939),
`crates/vanta-resolve/src/lib.rs:64-65`, `crates/vanta-registry/src/lib.rs:37,63`

`load_registry` fetches `$VANTA_REGISTRY` with no signature/TUF verification of the index.
`vanta-resolve` then reads `artifact.signature` AND `artifact.signature_key` from that same
registry document.

**Attack:** an attacker controlling the registry response (MITM on a plaintext URL, a malicious
`$VANTA_REGISTRY`, a compromised mirror) supplies the artifact URL, checksum, detached signature,
**and the public key that verifies it**. They generate their own keypair, sign malicious bytes,
publish their pubkey as `public_key`. `install_artifact` recomputes the checksum (matches), verifies
the signature against the attacker's own key (passes), installs+links arbitrary code onto PATH.
Signature verification provides zero protection here.

**Fix:** pin a small set of root keys out-of-band (compiled-in or in user-owned trusted config,
never from the fetched index); require the registry index itself to be signed by a pinned key;
check each per-artifact `public_key` against a pinned trust anchor, not accept it from the index.

---

## HIGH

### H2 — Signature verification is optional; `Policy.require_signature` is dead code
`crates/vanta-install/src/lib.rs:85`; `Policy` in `crates/vanta-security/src/lib.rs:83-130`;
`Settings.verify` in `crates/vanta-config/src/model.rs:107`

Verification runs only `if let (Some(sig), Some(key_text)) = (&artifact.signature, &artifact.signature_key)`.
If the registry omits either, install proceeds on checksum alone. `Policy`/`Settings.verify` are
defined and unit-tested but **never instantiated or consulted** in install/CLI (grep-confirmed).

**Fix:** thread `Policy`/`Settings.verify` into `install_artifact`; when a signature is required,
a missing signature must be a hard error (fail-closed).

### H3 — `restore()` poisons the content-addressed store (publish-before-verify, no rollback)
`crates/vanta-install/src/lib.rs:226-240`

The staged dir is `fs::rename`d into the canonical store path *first*, then `verify_entry` is
checked; on mismatch it returns `Err` but the bad entry is left at `dst`. `verify_entry` is not
re-run on the install path.

**Attack:** a crafted `.vbundle` contains a dir `blake3-<validhash>` whose contents don't hash to
`<validhash>`. `vanta restore` renames it into the store, reports an error — but the malicious
entry persists. The next `add/sync` pinned to that `store_key` hits the short-circuit (H4) and
links the attacker's binaries → persistent store poisoning → code execution.

**Fix:** verify the staged tree in staging *before* moving into the store; never publish on failure
(verify into a temp name, rename only on success).

### H4 — Store-hit short-circuit trusts the lockfile's `store_key` without re-verifying
`crates/vanta-install/src/lib.rs:60-65`; `store.has` is a mere `is_dir` check (`vanta-store/src/lib.rs:56`)

If `artifact.store_key` is present and `store.has(key)`, bins are linked and a generation recorded
with no `verify_entry` and no download. Combined with H3 (or any prior tampering), unverified
content reaches PATH. The lockfile is attacker-influenceable (committed in a cloned repo).

**Fix:** call `verify_entry` on store hits before linking, or guarantee store entries are immutable
and were verified at insert (H3 shows they aren't).

---

## MEDIUM

- **M5 — Archive symlink/hardlink targets unchecked; setuid preserved.** `crates/vanta-install/src/lib.rs:310-352` (`extract_targz`). Classic zip-slip names ARE rejected, but per-entry `entry.unpack` bypasses tar's symlink-ancestry guard and never inspects symlink *targets*; `set_preserve_permissions(true)` keeps setuid/setgid. Fix: reject symlink/hardlink entries with absolute/`..` targets; re-check realpath stays under dest; strip setuid/setgid.
- **M6 — No TLS enforcement / `https_only` / redirect cap.** `crates/vanta-net/src/lib.rs:27-34`. `http://` accepted, `https→http` downgrade redirects followed. Fix: reject `http` scheme, `https_only(true)`, cap + forbid scheme-downgrade on redirects.
- **M7 — `StoreKey::new` validates prefix only, not hex charset/length.** `crates/vanta-core/src/types.rs:21-31` accepts `blake3-../../etc`; flows into `entry_path().join(key)`, `staging.join(key)`, shim `store.join(key)`. Latent path traversal. Fix: validate suffix is exactly lowercase hex of expected length.
- **M8 — No download/decompression size cap.** `vanta-net/src/lib.rs:111` (`io::copy`), `vanta-install/src/lib.rs:313` (`GzDecoder` unbounded). `PlatformChecksum.size` never enforced → decompression-bomb DoS. Fix: enforce declared `size` as a hard ceiling on downloaded + decompressed bytes.
- **M9 — `vanta trust` TOFU list written but never enforced; tasks run via `sh -c`.** `crates/vanta-cli/src/lib.rs:707-734` (records hashes, only `--list` reads them), `:529`/`:871` (`cmd_run` → `shell_command` → `sh -c <cmd>`, no trust gate). `cd` into hostile repo + `vanta run build` = arbitrary shell. Fix: consult the trust list before executing tasks / syncing an untrusted manifest, or prompt on first use.

---

## LOW

- **L10 — WASM sandbox caps fuel but not memory; provider modules unsigned.** `crates/vanta-provider/src/wasm.rs:20-25,32`. `memory.grow`→~4 GB OOM; `Module::new` has no signature check. Latent (`run_i32` has no callers yet). Fix: `Store::limiter` memory ceiling; verify modules against a pinned key before instantiation.
- **L11 — `download_any` reuses one `.part`/dest across mirrors.** `vanta-net/src/lib.rs:63-74`. Resume can concatenate bytes from two hosts (caught by checksum). Fix: per-URL part file or truncate on mirror switch.
- **L12 — Shim path built via `join(key)`** (`vanta-shim/src/main.rs:51`) shares M7's non-validation; locally controlled, low risk.

---

## Audited but clean

- No `unsafe` anywhere (`#![forbid(unsafe_code)]` workspace-wide).
- `sign.rs` minisign/Ed25519: length-checked (74-byte sig, 42-byte key), key_id pinned to provided key, legacy `Ed` vs prehashed `ED` (BLAKE2b-512) handled, fail-closed on every path. Sound *given a trusted key* (problem is provenance, C1).
- Checksum gate (`vanta-security/src/lib.rs:57-78`): streamed, unknown algorithm rejected, constant-form hex compare, fail-closed.
- `hash_tree` (`vanta-store/src/hash.rs`): canonical sorted traversal, length-prefixed, exec-bit normalized, symlinks hashed by target, deterministic.
- `publish_tree`: atomic stage→rename, dedup, read-only; reachability-based `gc`.
- Lock parser proptest-fuzzed, rejects newer `lock_version`; config uses `deny_unknown_fields`; lone runtime index guarded by `is_empty`.

---

## Remediation notes (implemented 2026-06-30)

All findings were addressed; status and the trust model implemented are below.
Every crate retains `#![forbid(unsafe_code)]`.

### Trust model (C1)

A minimal but real **pinned-root** model now backs registry trust. New module
`vanta-security/src/trust.rs`:

- **Pinned roots** come from two out-of-band sources, never the fetched index:
  a compiled-in constant `COMPILED_IN_ROOT_KEYS` (currently an intentionally
  **empty placeholder** — maintainer follow-up: replace with the real Vanta
  release root key(s)) and the user-owned `~/.vanta/trust/roots.toml`
  (`keys = ["<minisign pubkey text>", ...]`).
- `load_registry` (vanta-cli) now: (1) rejects plaintext `http://` registry URLs
  unless `VANTA_INSECURE_REGISTRY=1` (documented dangerous); (2) downloads the
  index (size-capped) and a detached signature companion `<url>.minisig`;
  (3) verifies the index against a pinned root via
  `trust::index_signed_by_root` **before** parsing/trusting entries — an
  unsigned/unverifiable network index is **refused** (fail-closed) unless the
  insecure opt-in is set; (4) marks the parsed `Registry` `index_verified` and
  carries the pinned root set.
- `vanta-resolve` only propagates a per-artifact `signature_key` if the index was
  verified against a pinned root (transitive trust) **or** that key is itself
  pinned (`trust::artifact_key_is_trusted`); otherwise the key is dropped and the
  artifact is treated as unsigned (and refused under a signature-requiring
  policy). A local-file `$VANTA_REGISTRY` is treated as user-owned/trusted.

Because the compiled-in root set is an empty placeholder, **signed network
registries will not verify until a maintainer adds the real root key** (or an
operator adds one to `roots.toml`). This is deliberate fail-closed posture; there
is no default network registry, so default/builtin usage is unaffected.

### Per-finding status

| Finding | Status | Key files |
|---|---|---|
| C1 | Fixed | `vanta-security/src/trust.rs`, `vanta-cli` `load_registry`, `vanta-resolve/src/lib.rs`, `vanta-registry/src/lib.rs` |
| H2 | Fixed — `Policy` threaded into `Engine`/`install_artifact`; CLI maps `settings.verify` | `vanta-install/src/lib.rs`, `vanta-cli` `install_policy` |
| H3 | Fixed — staged subtree hashed and matched to its key before rename; store untouched on failure | `vanta-install/src/lib.rs` `restore` |
| H4 | Fixed — store hits re-verified (`verify_entry`); poisoned entry dropped + re-fetched | `vanta-install/src/lib.rs`, `vanta-store` `remove_entry` |
| M5 | Fixed — symlink/hardlink targets rejected if absolute/`..`; realpath kept under staging; setuid/setgid/sticky stripped | `vanta-install/src/lib.rs` `extract_targz` |
| M6 | Fixed — plaintext http rejected (loopback exception for dev/test); custom redirect policy caps redirects and forbids https→http downgrade; insecure opt-in. (`https_only(true)` deliberately not used — it would block loopback; TLS enforced per-request instead.) | `vanta-net/src/lib.rs` |
| M7 | Fixed — `StoreKey` suffix must be exactly 64 lowercase hex chars | `vanta-core/src/types.rs` |
| M8 | Fixed — download capped at declared `size`; decompression capped via `LimitReader` (`DEFAULT_MAX_DECOMPRESSED`, configurable) | `vanta-net/src/lib.rs`, `vanta-install/src/lib.rs` |
| M9 | Fixed — `[tasks]` execution and `sync` gated on the TOFU trust list; refuse non-interactive / prompt interactive; `VANTA_ASSUME_TRUST=1` opt-in | `vanta-cli/src/lib.rs` |
| L10 | Partial — `Store::limiter` memory ceiling added (256 MiB); module signature verification left as a `TODO` (no provider-signing key infrastructure yet) | `vanta-provider/src/wasm.rs` |
| L11 | Fixed — `download_any` truncates the `.part` file on each mirror switch | `vanta-net/src/lib.rs` |
| L12 | Fixed — covered by M7; shim now validates the key via `StoreKey::new` before joining the store path | `vanta-shim/src/main.rs` |

### New env flags / config

- `VANTA_INSECURE_REGISTRY=1` — dangerous: allow http and skip pinned-root index
  verification.
- `VANTA_ASSUME_TRUST=1` — approve untrusted manifests non-interactively (CI).
- `roots.toml` schema under `~/.vanta/trust/`: `keys = ["<minisign pubkey>", ...]`.
- `settings.verify = "require"` (synonyms: `required`/`signature`/`strict`) now
  enforces signatures at install time (default unchanged = checksum-gated).

### Maintainer follow-ups

- Replace the empty `COMPILED_IN_ROOT_KEYS` placeholder with the real release
  root key before shipping signed registries.
- L10: add a provider-module signing key and verify modules before
  `Module::new` (TODO in `wasm.rs`).
