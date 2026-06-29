# 29. Public APIs

> Vanta's stable surfaces and their guarantees: the CLI plus the `--json` output schema and exit codes (the primary public API), the provider ABI, the embeddable Rust library API, and the on-disk formats. This document states what is stable, how each surface is versioned, and the deprecation and compatibility policy, so integrators (CI, editors, embedders, provider authors) can depend on Vanta safely.

**Contents**

- [Stability tiers](#stability-tiers)
- [The CLI and JSON surface](#the-cli-and-json-surface)
- [The provider ABI](#the-provider-abi)
- [The embeddable Rust API](#the-embeddable-rust-api)
- [The on-disk formats](#the-on-disk-formats)
- [Deprecation and compatibility policy](#deprecation-and-compatibility-policy)
- [Experimental features](#experimental-features)
- [Cross-references](#cross-references)

---

## Stability tiers

| Tier | Surface | Audience | Versioning |
| --- | --- | --- | --- |
| 1 | CLI commands + flags + `--json` schema + exit codes | everyone, scripts, CI, editors | binary SemVer; `--json` has `schema_version` |
| 2 | Provider ABI (WIT world) | provider authors | independent WIT SemVer ([22](22-provider-sdk.md)) |
| 3 | Embeddable Rust API (`vanta-core` + a `vanta` library facade) | embedders | crate SemVer (`vanta-sdk`/facade only) |
| 4 | On-disk formats (`vanta.toml`, `vanta.lock`, registry index) | tools reading the files | format `version`/`lock_version` integers |

Everything else — the internal `vanta-*` crates, the redb schema, internal traits — is **not** a public API and may change without notice ([03. Repository](03-repository.md#versioning-policy)). The four tiers above are versioned independently so a change to one never forces a break in another ([ADR-0023](24-architecture-decision-records.md)).

## The CLI and JSON surface

The CLI is the surface almost everyone integrates against, so it is treated as an API:

- **Commands and flags** (canon §5, [04. CLI](04-cli.md)) are stable within a binary major; additions are minor, removals/renames are major with a deprecation period.
- **Exit codes** (canon §13, [25. Error Catalog](25-error-and-exit-code-catalog.md)) are stable forever — CI can branch on them (e.g. `5` = retryable network, `6` = fatal verification) without parsing text.
- **`--json`** output is a stable, versioned schema with a `schema_version`. Human (default) output may change freely; `--json` may not, within a major. Editors and CI must use `--json`, never scrape human text.

Representative `--json` outputs:

```json
// vanta list --json
{ "schema_version": 1, "ok": true,
  "tools": [
    { "name": "node", "version": "24.6.0", "scope": "project",
      "store_key": "blake3-aa3f…", "path": "/Users/x/.vanta/store/blake3-aa3f…/bin/node" },
    { "name": "ripgrep", "version": "14.1.0", "scope": "global", "store_key": "blake3-9d2e…", "path": "…" }
  ] }
```

```json
// vanta add node@24 --json  (result)
{ "schema_version": 1, "ok": true,
  "added": [{ "name": "node", "request": "24", "version": "24.6.0" }],
  "generation": "0009", "lock_changed": true }
```

The error shape is documented in [25. Error Catalog](25-error-and-exit-code-catalog.md); its `code`/`area`/`exit`/`context` fields are stable.

## The provider ABI

The provider WIT world (`vanta:provider@MAJOR.MINOR.PATCH`) is the contract for sandboxed provider hooks ([22. Provider SDK](22-provider-sdk.md)). It is versioned independently; the host supports a documented range of ABI majors, a provider declares the ABI it targets, and providers keep working across host upgrades within a major. This is the most carefully guarded contract because third parties build against it.

## The embeddable Rust API

Vanta's engine is usable as a library (e.g. a build tool or IDE backend embedding tool management). The stable embedding surface is `vanta-core`'s vocabulary/traits plus a thin `vanta` library facade; the other `vanta-*` crates are internal.

Sketch of the facade (signatures, not implementation):

```rust
pub struct Vanta { /* opaque; built from a VantaConfig */ }

impl Vanta {
    pub fn open(home: Option<&Path>) -> Result<Self, VtaError>;

    /// Resolve a request to a pinned Resolution (no install).
    pub async fn resolve(&self, req: &Request, targets: &[Platform]) -> Result<Resolution, VtaError>;

    /// Install resolutions, producing a new generation (the [3..8] lifecycle).
    pub async fn install(&self, res: &[Resolution], scope: Scope) -> Result<Generation, VtaError>;

    /// Reconcile a directory to its manifest+lock (`vanta sync`).
    pub async fn sync(&self, dir: &Path, opts: SyncOpts) -> Result<Generation, VtaError>;

    /// The composed environment (tool -> resolved binary) for a directory.
    pub fn env_for(&self, dir: &Path) -> Result<Environment, VtaError>;

    /// The resolved absolute path of a tool in a directory's environment.
    pub fn which(&self, tool: &str, dir: &Path) -> Result<PathBuf, VtaError>;

    pub fn generations(&self, env: &EnvId) -> Result<Vec<GenerationRecord>, VtaError>;
    pub fn rollback(&self, env: &EnvId, to: Option<GenId>) -> Result<Generation, VtaError>;
}
```

The core trait seams an embedder or extension can implement are defined in `vanta-core` and described in [02. Architecture](02-architecture.md#dependency-injection-and-extension-seams): `Provider`, `Backend`, `CacheStore`, `SignatureVerifier`, `LinkStrategy`. Embedders depend on `vanta` (facade) + `vanta-core` only; both follow crate SemVer with the deprecation policy below.

## The on-disk formats

`vanta.toml`, `vanta.lock`, and the registry index are formats other tools may read/write ([27](27-config-reference.md), [31](31-lockfile-and-manifest-reference.md), [26](26-registry-and-metadata-reference.md)). Each carries an integer format version. They are TOML (manifest/lock) and the documented registry schema, with stable, canonical serialization so external tooling can parse and diff them reliably.

## Deprecation and compatibility policy

- **Additions** (new commands, flags, JSON fields, optional config keys, ABI minor) are backward-compatible and ship in minor releases.
- **Removals/renames** require a major bump and a deprecation period: the old form keeps working and emits a warning for ≥ 1 minor release before removal.
- **Forward/backward formats:** a newer binary reads older `vanta.toml`/`vanta.lock`/index; an older binary refuses a newer format with a clear `VTA-LOCK-0002`/`VTA-CFG-*`/`VTA-REG-0002` ("written by a newer Vanta; upgrade") rather than misinterpreting it ([31](31-lockfile-and-manifest-reference.md), [25](25-error-and-exit-code-catalog.md)).
- **Provider ABI:** within a major, providers keep working; a new major has a migration window during which the host supports both.
- **MSRV** changes are a documented minor event ([03. Repository](03-repository.md)).

## Experimental features

Unstable features are gated behind `[settings] experimental.*` (or `--experimental-<name>`), are excluded from the stability guarantees, may change or be removed, and print a one-time notice when first used. Graduating an experimental feature to stable is a minor release that removes the gate ([27. Configuration Reference](27-config-reference.md)).

## Cross-references

- [04. CLI](04-cli.md) — the command/flag surface and `--json` behavior.
- [25. Error & Exit-code Catalog](25-error-and-exit-code-catalog.md) — exit codes and the JSON error schema.
- [22. Provider SDK](22-provider-sdk.md) — the provider ABI (Tier 2) in detail.
- [31. Lockfile & Manifest Reference](31-lockfile-and-manifest-reference.md) & [26. Registry Reference](26-registry-and-metadata-reference.md) — the on-disk formats (Tier 4).
- [02. Architecture](02-architecture.md) — the core traits an embedder implements.
- [03. Repository](03-repository.md) — independent versioning and which crates are public.
