# Vanta — Design & Documentation Overview

> **Vanta** is a brand-new, original, cross-platform **package manager + toolchain manager + runtime manager + dev-environment manager** written in Rust: one static binary, one command for everything, deterministic and reproducible, secure by default, offline-friendly, and enterprise-ready. This document is the entry point to the complete implementation plan, split across 33 focused documents under `docs/`, linked below.
>
> *Not a clone of mise, asdf, Homebrew, Nix, uv, pnpm, winget, or scoop — it learns from all of them and copies none. See [33. Prior Art](33-prior-art.md).*

---

## What Vanta is

Installing developer tools today means assembling three to six single-purpose managers — a per-language version manager, a polyglot manager, an OS package manager, a language-native manager, and a reproducibility tool — each with its own commands, config, security posture, and platform reach. Vanta collapses that into one interface, one config format, one content-addressed store, and one reproducibility contract, on Linux, macOS, and Windows.

The mental model is one sentence: **if you need any developer tool, you install it with Vanta.**

```sh
vanta add node@24
vanta add python@3.13
vanta add rust
vanta add terraform
vanta remove node
vanta update
vanta sync          # reproduce exactly from vanta.lock — the post-`git clone` command
vanta doctor
vanta list
```

The ten design pillars (held as constraints, not slogans): **one command for everything · one consistent UX · deterministic & reproducible · extremely fast · secure by default · offline-friendly · atomic operations · cross-platform · human-readable config · zero unnecessary complexity.** See [01. Vision](01-vision.md).

## Quick facts

| Property | Decision |
| --- | --- |
| Name / binary | **Vanta** / `vanta` (alias `vt`) |
| Language / edition / MSRV | Rust · edition 2021 · MSRV = latest stable − 2 |
| Async runtime | Tokio (parallel IO; the work is IO-bound, not latency-critical) |
| HTTP / TLS | reqwest/hyper + **rustls** (no OpenSSL) |
| Hashing / store keys | **BLAKE3** (`blake3-<hex>`) + SHA-256 interop |
| State store | **redb** (embedded, pure-Rust, transactional) |
| Config / lock format | **TOML** — `vanta.toml`, `vanta.lock`, `~/.vanta/config.toml` (no bespoke DSL) |
| Providers | declarative manifests + **WASM (Wasmtime, capability-sandboxed)** hooks |
| Store | content-addressed, immutable, dedup'd, GC'd; atomic **generations** + rollback |
| Activation | shell hook (fast PATH) + shims (universal fallback), one resolution cache |
| Reproducibility | **cross-platform lockfile** (resolves all targets) + hashed artifacts |
| Security | checksums + signatures (TUF-style roles) + SLSA provenance + sandbox, **on by default** |
| License | Apache-2.0 (open core) + a future enterprise edition |
| Workspace | ~20 `vanta-*` crates in one Cargo workspace |

## How to read this plan

The plan is **34 documents**: 20 core documents (`01`–`20`) covering vision through future, and 13 deep-dive references (`21`–`33`). Recommended reading orders:

- **Evaluators:** [01. Vision](01-vision.md) → [33. Prior Art](33-prior-art.md) → [19. Milestones](19-milestones.md) → [20. Future](20-future.md).
- **Architects:** [01](01-vision.md) → [02. Architecture](02-architecture.md) → [09. Store](09-store.md) → [06. Resolution](06-resolution.md) → [24. ADRs](24-architecture-decision-records.md) → [21. Threat Model](21-threat-model.md).
- **Engineers about to build:** [02](02-architecture.md) → [03. Repository](03-repository.md) → your subsystem (`04`–`17`) → [25. Errors](25-error-and-exit-code-catalog.md)/[28. Testing](28-testing.md) → [19. Milestones](19-milestones.md).
- **Operators / enterprise:** [05. Configuration](05-configuration.md) → [11. Reproducibility](11-reproducibility.md) → [14. Enterprise](14-enterprise.md) → [13. Offline](13-offline.md) → [15. Security](15-security.md).

## Table of contents

### Core (01–20)

| # | Document | In one line |
| --- | --- | --- |
| 01 | [Vision](01-vision.md) | Mission, the fragmentation problem, ten pillars, audiences, non-goals, comparison, roadmap. |
| 02 | [Architecture](02-architecture.md) | The store-centric model, the resolution lifecycle, no daemon, atomicity, extension seams. |
| 03 | [Repository & Engineering](03-repository.md) | The Cargo workspace, every crate, the dependency DAG, standards, CI/CD, versioning. |
| 04 | [CLI & Command Design](04-cli.md) | Command philosophy, the full command/flag reference, output, exit codes, scope inference. |
| 05 | [Configuration & Manifests](05-configuration.md) | `vanta.toml`, the task runner, global config, workspaces, precedence, interop, trust. |
| 06 | [Resolution & Version Management](06-resolution.md) | Request grammar, the resolver, ordering, dependency DAG, auto-switching, cross-platform resolve. |
| 07 | [Providers & Registry](07-providers.md) | The provider model, backends, the registry, plugins, private registries. |
| 08 | [Installation Engine](08-installation.md) | Plan→fetch→verify→materialize→link→commit, parallelism, atomicity, hooks. |
| 09 | [Store, Cache & Storage](09-store.md) | Content addressing, canonical hashing, atomic publish, links, dedup, GC, repair. |
| 10 | [Environments & Activation](10-environments.md) | Global vs project, the shell hook, shims, the resolution cache, `exec`/`run`/`x`. |
| 11 | [Reproducibility & Lockfiles](11-reproducibility.md) | The thesis, the lock, cross-platform locking, `vanta sync`, determinism boundaries. |
| 12 | [Updates & Rollback](12-updates.md) | Update strategy, channels, atomic generations, rollback, self-update, safety. |
| 13 | [Offline, Mirrors & Air-gapped](13-offline.md) | Offline mode, mirrors, bundles, registry internalization, the CAS advantage. |
| 14 | [Enterprise](14-enterprise.md) | Private registries, auth/SSO, policy, team distribution, audit/SBOM, editions. |
| 15 | [Security & Supply Chain](15-security.md) | Checksums, signed roles, provenance, the sandbox, config trust, the controls matrix. |
| 16 | [Performance](16-performance.md) | Targets, startup/activation/install/memory/disk optimization, the perf-gate. |
| 17 | [Cross-platform](17-cross-platform.md) | The abstraction, platform tokens, links per OS, Windows/macOS/Linux specifics. |
| 18 | [Developer Experience](18-developer-experience.md) | First ten minutes, `init`, clone→sync, editors, CI, `vanta x`, `doctor`. |
| 19 | [Milestones](19-milestones.md) | Phases P0–P8: objectives, LOC, duration, deliverables, tests, risks, acceptance. |
| 20 | [Future](20-future.md) | Innovations matured, the ecosystem, team/cloud sharing, the open-core invariant. |

### Deep-dive references (21–33)

| # | Document | In one line |
| --- | --- | --- |
| 21 | [Threat Model](21-threat-model.md) | Trust boundaries, STRIDE, abuse cases, the threat→control matrix, non-mitigations. |
| 22 | [Provider SDK & ABI](22-provider-sdk.md) | The manifest format, the WIT world, the capability sandbox, the guest SDK, testing. |
| 23 | [Data & State Model](23-data-and-state-model.md) | The redb schema, authoritative vs rebuildable state, locking, secrets, recovery. |
| 24 | [Architecture Decision Records](24-architecture-decision-records.md) | 25 ADRs recording the *why* of every major choice. |
| 25 | [Error & Exit-code Catalog](25-error-and-exit-code-catalog.md) | The `VTA-*` taxonomy, per-area code tables, exit codes, the JSON error shape. |
| 26 | [Registry & Package Metadata Reference](26-registry-and-metadata-reference.md) | Index/provider/artifact schemas, platform tokens, the signed-role model. |
| 27 | [Configuration Reference](27-config-reference.md) | Exhaustive key-by-key reference for `vanta.toml` and `config.toml`. |
| 28 | [Testing, Fuzzing & Benchmarks](28-testing.md) | The pyramid, property/fuzz/integration/security tests, the perf-gate, release gates. |
| 29 | [Public APIs](29-public-apis.md) | The CLI/JSON surface, the provider ABI, the embeddable Rust API, compatibility. |
| 30 | [Migration & Import](30-migration.md) | `vanta migrate` importers, interop, fidelity, coexistence, limitations. |
| 31 | [Lockfile & Manifest Format Reference](31-lockfile-and-manifest-reference.md) | The full lock schema, reconcile rules, canonical serialization, merge guidance. |
| 32 | [Release Engineering & Supply Chain](32-release-engineering.md) | Reproducible builds, signing/SBOM/SLSA, channels, self-update, the registry pipeline. |
| 33 | [Prior Art & Ecosystem Analysis](33-prior-art.md) | Deep teardown of 17 incumbents + the synthesis Vanta forms. |

## Document synopses

**01 Vision** — The thesis (collapse the fragmented toolbox into one interface), the ten pillars as constraints, three concentric audiences, explicit non-goals (not a language dep manager, not a system PM, not a build system), an honest comparison, and the five-year arc.

**02 Architecture** — The spine. Vanta as a short-lived CLI (no daemon), the content-addressed store as the keystone, the eight-stage resolution lifecycle (`[1 Request]`..`[8 Commit]` + `[Activate]`), the atomicity boundary at commit, the trait seams, and the failure/recovery model.

**03 Repository & Engineering** — The ~20-crate workspace with a layered DAG (`vanta-shim` minimal for fast start), the crate catalog, coding standards (`forbid(unsafe)` by default), CI matrix, and independent versioning of binary / format / provider ABI / registry.

**04 CLI** — One verb per intent, inferred scope, CI-friendly defaults; the full command and flag reference, `--json`, exit codes, and the scope-inference algorithm.

**05 Configuration** — Why TOML over a DSL/YAML; `vanta.toml` table by table, the minimal task runner, global config, workspaces, precedence/merge, foreign-file interop, and config trust-on-first-use.

**06 Resolution** — The request grammar, the deterministic resolver, provider-declared ordering, the dependency DAG (and why not a SAT solver), global-vs-project resolution, and cross-platform resolve.

**07 Providers** — Declarative-first providers with sandboxed WASM hooks (not arbitrary shell), the backend abstraction, the signed registry, plugins, and private registries.

**08 Installation Engine** — The six install stages in depth, parallel/resumable fetch, the fail-closed verify gate, atomic publish, transactions/generations, concurrency, and sandboxed hooks.

**09 Store** — Content addressing and why; canonical, cross-platform hashing; the storage layout; atomic publish; reflink→hardlink→symlink→copy; dedup; the caches; GC; and integrity repair.

**10 Environments & Activation** — Global vs project; the shell-hook fast path and the per-directory resolution cache; the shim dispatcher; the hook+shim hybrid; per-shell integration; and `exec`/`run`/`x`/`shell`.

**11 Reproducibility** — The thesis and its four guarantees; the lockfile; cross-platform locking (one lock, every OS); `vanta sync`/`--frozen`; determinism boundaries; and team/CI workflows.

**12 Updates & Rollback** — Constraint-respecting updates, channels, atomic generation swaps, instant rollback, generation history, verified `self update`, and safety guards.

**13 Offline, Mirrors & Air-gapped** — Offline mode, mirror fallback (verified, so no trust cost), `vanta bundle`/`restore`, registry internalization, and the content-addressed advantage.

**14 Enterprise** — Private registries overlaying the official one, auth/SSO, signed policy enforcement, team distribution, audit/SBOM/provenance, fleet management, and the open-core boundary.

**15 Security & Supply Chain** — Secure-by-default; mandatory checksums; TUF-style signed roles; artifact signatures + provenance; the WASM capability sandbox; sandboxed builds; SBOM; config trust; the controls matrix.

**16 Performance** — Budgets (<5 ms cold start, <1 ms warm activation/shim) and how they're met (no daemon, the resolution cache, dedup, reflink/hardlink, parallel IO), with the CI perf-gate.

**17 Cross-platform** — The abstraction layer, the canonical platform tokens, links per OS, and the Windows (no symlink reliance, launcher shims, Authenticode), macOS (quarantine/notarization, Rosetta), and Linux (glibc/musl, no root) specifics.

**18 Developer Experience** — The first ten minutes, `vanta init` auto-detection, the `git clone && vanta sync` onboarding, editor/IDE integration, CI usage, `vanta x`, and `vanta doctor`.

**19 Milestones** — Nine phases (P0 foundations → P8 1.0 hardening) each with objectives, LOC/duration, deliverables mapped to crates, tests, risks, and acceptance criteria; Year-1 (0.x) / Year-2 (1.0) split.

**20 Future** — Sub-file dedup, a community provider ecosystem, team/cloud environment sharing, an optional fleet control plane, reproducible source builds, editor integration, AI diagnostics — all additive, open-core preserved.

**21 Threat Model** — Assets and trust boundaries, STRIDE per boundary, concrete abuse cases with residual risk, the threat→control matrix, and explicit out-of-scope items.

**22 Provider SDK & ABI** — The declarative manifest (with a complete example), the WIT world, the capability sandbox, the `vanta-sdk` guest SDK, signing/publishing, and provider testing.

**23 Data & State Model** — The redb schema table by table, authoritative vs rebuildable state, the on-disk layout, the locking protocol, generation records, secret handling, and recovery.

**24 ADRs** — 25 immutable decision records (Rust, content-addressed store, TOML-not-DSL, WASM providers, BLAKE3, redb, no daemon, cross-platform lock, hybrid activation, …) with context, alternatives, and consequences.

**25 Error & Exit-code Catalog** — The `VTA-<AREA>-<NNNN>` scheme, per-area code tables, the message style, stable exit codes, the JSON error shape, and the doctor-check mapping.

**26 Registry & Metadata Reference** — Exhaustive schemas for the index entry, provider manifest, artifact descriptor, and version metadata; platform tokens; and the signed-role model.

**27 Configuration Reference** — Every key for `vanta.toml` and `config.toml`: type, default, scope, constraint — with fully-populated examples.

**28 Testing** — The pyramid; property tests for ordering/reconcile/canonicalization; fuzzing every parser and the archive extractor; the hermetic harness; reproducibility and security tests; the perf-gate; release gates.

**29 Public APIs** — The four stability tiers (CLI/JSON, provider ABI, embeddable Rust, on-disk formats), the JSON schema, the library facade, and the deprecation/compatibility policy.

**30 Migration & Import** — `vanta migrate` from mise/asdf/nvm/pyenv/brew/scoop/pkgx and more, the fidelity model, read-only interop, the safe workflow, coexistence, and honest limitations.

**31 Lockfile & Manifest Reference** — The complete `vanta.lock` schema with a multi-platform example, the manifest↔lock reconcile rules, format versioning, canonical serialization, and merge guidance.

**32 Release Engineering & Supply Chain** — Reproducible hermetic builds, the target matrix, signing + provenance + SBOM, channels, distribution, verified self-update, the registry signing pipeline, and key management.

**33 Prior Art & Ecosystem Analysis** — The deep, honest teardown of mise, asdf, pkgx, Cargo, npm, pnpm, uv, pipx, Homebrew, apt, dnf, pacman, winget, scoop, Chocolatey, Nix, and Flox — across fourteen dimensions each — and the unified model Vanta forms.

## How this plan is structured

The plan is anchored to a single canonical set of decisions — names, the content-addressed store model, the resolution lifecycle stage names, the crate list, the `VTA-*` error scheme, the command set, and the config/lock formats — so terminology and interfaces line up across all 34 files. Core documents (01–20) map to the required topic areas; the deep-dive references (21–33) add the threat model, the provider/registry/error/config catalogs, testing, migration, the formats, release engineering, and the competitive analysis a team needs to actually build, operate, and harden Vanta. Cross-references at the foot of every document let you navigate by concept.

This documentation is the reference for the implementation, which lives as a Rust workspace under [`crates/`](../crates). Start from [03. Repository](03-repository.md) for the crate layout and [19. Milestones](19-milestones.md) for the roadmap.
