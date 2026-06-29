# 15. Security & Supply Chain

> Vanta executes code that users download from the internet; security is therefore a primary design property, not a feature. This document specifies the secure-by-default posture, mandatory checksums, the signed-metadata trust model (TUF-style roles), artifact signatures and provenance, the WASM provider sandbox, sandboxed source builds, SBOM generation, config trust-on-first-use, secret handling, network security, registry anti-abuse, and a threat→control matrix. The adversary analysis is in [21. Threat Model](21-threat-model.md); Vanta's own release supply chain is in [32. Release Engineering](32-release-engineering.md).

**Contents**

- [Secure-by-default posture](#secure-by-default-posture)
- [Checksums](#checksums)
- [Signed metadata and trust roles](#signed-metadata-and-trust-roles)
- [Artifact signatures and provenance](#artifact-signatures-and-provenance)
- [The provider sandbox](#the-provider-sandbox)
- [Sandboxed source builds](#sandboxed-source-builds)
- [SBOM and license assurance](#sbom-and-license-assurance)
- [Config trust](#config-trust)
- [Secrets and network](#secrets-and-network)
- [Registry anti-abuse](#registry-anti-abuse)
- [Controls matrix](#controls-matrix)
- [Cross-references](#cross-references)

---

## Secure-by-default posture

Verification is **on by default; you opt out, never in** (pillar 5, [ADR-0017](24-architecture-decision-records.md)). The reasoning: the dominant real-world risks for a tool manager are supply-chain compromise (a substituted or malicious artifact) and arbitrary-code execution during install (the `postinstall`/`PKGBUILD`/PowerShell pattern, [33. Prior Art](33-prior-art.md)). Vanta's defaults assume hostile input at every boundary:

- Every artifact is checksum- and signature-verified before it can enter the store.
- Provider logic runs sandboxed with no ambient authority.
- Source builds run sandboxed with no network after fetch.
- Project configs that inject env or run commands require explicit trust.
- `--no-verify` is a loud, warned escape hatch that **org policy can forbid** ([14. Enterprise](14-enterprise.md)).

The whole verification chain is **fail-closed**: any failure aborts the operation and leaves the store untouched ([08. Installation](08-installation.md#stage-5--verify)).

## Checksums

- Every artifact carries a content hash in the signed registry metadata and in `vanta.lock`. On download, Vanta computes **SHA-256** (matching the upstream-published checksum) and **BLAKE3** (internal, fast) and compares both to the pinned values.
- A mismatch ⇒ `VTA-VRF-0001` (exit 6); the offending download is **quarantined** (moved aside, never served), not deleted, to aid forensics.
- The store path *is* a BLAKE3 hash, so integrity is continuously checkable by re-hashing ([09. Store](09-store.md#integrity-and-repair)).
- Checksums alone defend against corruption and an attacker who can replace bytes but not forge a signature; signatures (below) defend against an attacker who can also alter the metadata.

## Signed metadata and trust roles

The registry index and provider metadata are **signed**, using a role model inspired by TUF (The Update Framework) to limit the blast radius of any single key and to make key rotation safe ([26. Registry Reference](26-registry-and-metadata-reference.md)):

| Role | Signs | Property |
| --- | --- | --- |
| **root** | the set of role keys (delegations) | offline, threshold (M-of-N) keys; rotates the others; rarely used |
| **registry/targets** | the index of tools → providers → versions | the authority for "what exists and where" |
| **snapshot** | a consistent point-in-time view (the `registry_revision`) | prevents mix-and-match / rollback of metadata |
| **timestamp** | freshness of the snapshot | bounds how stale metadata can be; defeats freeze attacks |

- Clients pin the **root** keys (shipped with Vanta, rotatable via the root role) and verify the chain down to the metadata they use. Metadata that is unsigned, signed by an untrusted key, expired, or inconsistent is rejected (`VTA-VRF-*`).
- **Snapshot/timestamp** defend against rollback and freeze attacks (serving an old, vulnerable version as if current).
- Key material lives in `~/.vanta/trust/`; `vanta trust keys` lists pinned keys and `vanta trust rotate` follows a root-signed rotation.

## Artifact signatures and provenance

Beyond metadata signing, the **artifacts themselves** are verified:

- **Signatures.** Official-registry artifacts are signed with **minisign (Ed25519)**; where a publisher provides **cosign/sigstore** signatures, Vanta verifies those against the publisher's identity. A required-but-missing or invalid signature ⇒ `VTA-VRF-0002`.
- **Provenance (SLSA).** Where build provenance is published, Vanta verifies it and can require a minimum SLSA level via policy (`min_slsa_level`); `VTA-VRF-0004` on failure. `vanta provenance <tool>` shows the verified chain (source → build → artifact).
- **Pinning in the lock.** The lock records the signature/key reference per artifact, so reproduction re-verifies the same signer ([11. Reproducibility](11-reproducibility.md)).

## The provider sandbox

Providers describe how to discover and fetch tools; their declarative form (TOML) has no code at all. When a provider needs custom logic, it ships a **WebAssembly** hook run under **Wasmtime** with **capability-based** isolation — the core defense that makes Vanta's extension model safe where asdf/AUR/Chocolatey are not ([ADR-0006](24-architecture-decision-records.md), [ADR-0020](24-architecture-decision-records.md), [22. Provider SDK](22-provider-sdk.md)):

- **No ambient authority.** A WASM provider gets *only* the capabilities the host grants through the WIT interface: a **scoped HTTP GET** (to declared host patterns), **hashing**, and pure computation. It has **no filesystem, no arbitrary network, no environment, no process spawn, no clock/RNG** (determinism).
- **Bounded execution.** Wasmtime fuel and epoch interruption cap CPU and wall time; memory is bounded. A runaway or malicious provider is killed (`VTA-PROV-0001`), it cannot hang or exhaust the host.
- **Determinism.** Hooks must be deterministic (same inputs → same outputs); this is required for cacheable, reproducible resolution and removes a class of attacks that depend on environmental variance.
- **Signed and reviewed.** Providers are signed; community providers go through review before entering the official registry ([32. Release Engineering](32-release-engineering.md)).

A provider, even a malicious one, can at worst return *wrong metadata or a wrong URL* — and the resulting artifact still must pass checksum + signature + provenance against the *lock/metadata*, so a bad provider cannot deliver bad bytes undetected.

## Sandboxed source builds

When no prebuilt artifact exists and a provider declares a source build, the build runs in a sandbox:

- A restricted filesystem view (the verified source + a scratch dir + the declared toolchain; nothing else).
- **Network off after fetch** — all inputs were fetched and verified in `[4]`/`[5]`, so the build cannot reach out.
- Bounded resources; output canonicalized and content-addressed.
- Builds are **prebuilt-first** ([ADR-0022](24-architecture-decision-records.md)) and can be **forbidden by policy** (`allow_source_builds = false`), so an enterprise can run binaries-only.

This converts the AUR/`postinstall` "arbitrary build script as you" model into a contained, policy-governed operation.

## SBOM and license assurance

- `vanta sbom [--format cyclonedx|spdx]` produces a bill of materials for a project or environment: every tool, exact version, artifact hash, source/provider, signer, and license.
- `vanta licenses` reports licenses and flags any outside an allow-list ([14. Enterprise](14-enterprise.md)).
- Because the lock pins hashes and signers, the SBOM is *evidence*: it asserts exactly the bytes present, verifiable independently.

## Config trust

Distinct from artifact trust: a project `vanta.toml` can inject `[env]`, define `[tasks]` (commands that will run), or reference a third-party registry/provider. These are capabilities a malicious repo could abuse, so Vanta applies **trust-on-first-use** ([ADR-0018](24-architecture-decision-records.md), [05. Configuration](05-configuration.md#config-trust)):

- Until a manifest's trusted sections are approved via `vanta trust`, env injection and task execution are inert (tools still resolve/install — those are independently verified).
- Trust is keyed by a content hash of the manifest; editing the trusted sections re-prompts.
- This stops the "clone a repo, it silently runs code / poisons your PATH" class of attack while keeping the safe common case (declaring tools) frictionless.

## Secrets and network

- **Secrets at rest.** Registry credentials live in the OS keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service, with a 0600 file fallback) — never in plaintext config or the lock ([23. Data & State Model](23-data-and-state-model.md#secret-handling)). Secrets are redacted from logs and error output.
- **Transport.** All metadata and artifact fetches are over TLS via **rustls** (no OpenSSL); plaintext HTTP is refused for metadata. Optional certificate/key pinning and custom CA bundles support inspecting corporate proxies without weakening verification ([13. Offline](13-offline.md)).
- **Scoped credentials.** A registry's credential is sent only to that registry's host.

## Registry anti-abuse

- **Verified publishers and namespaces.** Provider/tool names in the official registry are bound to verified publishers; namespaces (e.g. `acme/*`) are reserved, mitigating typosquatting.
- **Name-similarity warnings.** `vanta add` warns on a likely typosquat (a name close to a popular tool) before installing.
- **Yank/advisory.** A compromised version can be yanked and an advisory attached; `vanta` warns or refuses per policy when a locked version is later flagged.

## Controls matrix

| Threat class | Primary control | Doc |
| --- | --- | --- |
| Substituted/corrupted artifact | mandatory SHA-256 + BLAKE3 checks vs signed metadata/lock, fail-closed | [08](08-installation.md#stage-5--verify) |
| Forged metadata | TUF-style signed roles, pinned root keys, threshold | this doc |
| Rollback / freeze of metadata | snapshot + timestamp roles | this doc |
| Malicious provider logic | WASM capability sandbox, fuel/epoch, determinism; signing/review | [22](22-provider-sdk.md) |
| Arbitrary install-time code | no ambient scripts; sandboxed builds; sandboxed hooks; policy off-switch | this doc / [08](08-installation.md) |
| Malicious project config | config trust-on-first-use | [05](05-configuration.md) |
| Compromised mirror | hash/signature verification independent of source; mirror = no trust cost | [13](13-offline.md) |
| Typosquatting | verified publishers, reserved namespaces, similarity warnings | this doc |
| Credential theft | OS keychain, scoped tokens, log redaction | [23](23-data-and-state-model.md) |
| Tampered lock in a PR | hashes re-verified at install; `--frozen` + review | [11](11-reproducibility.md) |
| Compromised Vanta release | signed releases + SLSA, verified self-update | [32](32-release-engineering.md) |

## Cross-references

- [21. Threat Model](21-threat-model.md) — STRIDE, trust boundaries, and abuse cases behind these controls.
- [08. Installation](08-installation.md) — the fail-closed `[5 Verify]` gate in the lifecycle.
- [22. Provider SDK](22-provider-sdk.md) — the WIT capability interface and the sandbox in detail.
- [26. Registry & Metadata Reference](26-registry-and-metadata-reference.md) — the signed-role metadata format.
- [11. Reproducibility](11-reproducibility.md) — hash/signer pinning in the lock.
- [14. Enterprise](14-enterprise.md) — policy that tightens these defaults org-wide.
- [32. Release Engineering](32-release-engineering.md) — Vanta's own signed, reproducible supply chain.
