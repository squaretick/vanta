# 03. Repository & Engineering

> The Cargo workspace that implements Vanta: every crate and its job, the layered dependency graph that keeps the design honest, shared-dependency discipline, coding standards, naming conventions, the testing pyramid in brief, the CI/CD matrix, and the independent-versioning policy. This is the first document an engineering team reads before working in the codebase.

**Contents**

- [Workspace layout](#workspace-layout)
- [Crate catalog](#crate-catalog)
- [Dependency graph](#dependency-graph)
- [Shared dependencies](#shared-dependencies)
- [Coding standards](#coding-standards)
- [Naming conventions](#naming-conventions)
- [Testing strategy](#testing-strategy)
- [CI/CD architecture](#cicd-architecture)
- [Versioning policy](#versioning-policy)
- [Cross-references](#cross-references)

---

## Workspace layout

Vanta is one Cargo workspace. Two binary crates (`vanta` and the tiny `vanta-shim`), a set of library crates each owning one subsystem, an `xtask` for dev automation, and the `docs`/`examples` trees.

```
vanta/
├── Cargo.toml                # [workspace] + [workspace.dependencies]
├── rust-toolchain.toml       # pinned toolchain (reproducible builds)
├── crates/
│   ├── vanta/                # bin: `vanta` + `vt`; CLI wiring; supervisor/engine assembly
│   ├── vanta-shim/           # bin: the shim dispatcher (separate, minimal deps, fast start)
│   ├── vanta-core/           # vocabulary: Request, Resolution, Artifact, StoreKey, Generation, traits, errors
│   ├── vanta-platform/       # OS/arch, paths, links (reflink/hardlink/symlink/copy), shells, exe handling
│   ├── vanta-config/         # vanta.toml + config.toml model, parsing, schema, span diagnostics, precedence
│   ├── vanta-lock/           # lockfile model (read/write/verify), manifest<->lock reconcile
│   ├── vanta-resolve/        # resolver: request->resolution, version ordering, constraints, dependency DAG
│   ├── vanta-registry/       # registry index, distribution, search, metadata, caching
│   ├── vanta-provider/       # provider model + builtin providers + Wasmtime host for WASM hooks
│   ├── vanta-net/            # http (rustls), parallel/resumable downloads, mirrors, retries, proxy, auth
│   ├── vanta-store/          # content-addressed store, atomic publish, links, dedup, integrity, layout
│   ├── vanta-install/        # install engine: plan->fetch->verify->materialize->link->commit; generations
│   ├── vanta-env/            # environment composition + activation logic + shell hook generation
│   ├── vanta-security/       # checksums, signatures, trust db, SBOM, provenance, sandboxing policy
│   ├── vanta-state/          # redb state DB: store index, gc roots, generation history, caches
│   ├── vanta-diag/           # doctor checks, diagnostics rendering, error-code registry
│   ├── vanta-cli/            # command implementations (library behind the `vanta` bin; testable)
│   ├── vanta-sdk/            # provider-author SDK (guest side, for WASM providers)
│   ├── vanta-migrate/        # importers: mise/asdf/nvm/fnm/pyenv/rbenv/volta/brew/scoop/pkgx
│   └── vanta-test/           # test harness, fakes (fake registry/provider/upstream), conformance utils (dev)
├── xtask/                    # dev automation: code/doc gen, dist, bench, registry tooling
├── docs/                     # this plan
└── examples/                 # example manifests + provider definitions
```

Rationale for a workspace over a single mega-crate: parallel compilation, enforceable layering (a crate cannot depend on something not in its `Cargo.toml`), and the ability to keep `vanta-shim`'s dependency tree minimal so its cold start is as fast as possible (it must not pull in the resolver, the WASM host, or the HTTP stack). Rationale over many repositories: atomic cross-cutting changes, one CI, no version-coordination tax.

## Crate catalog

Layers, from the leaves up: **shared** → **infra** → **domain** → **orchestration** → **cli** → **bin**, plus **tooling** crates consumed off the main path.

| Crate | Layer | Purpose | Key dependencies |
| --- | --- | --- | --- |
| `vanta-core` | shared | Core vocabulary (`Request`, `Resolution`, `Artifact`, `StoreKey`, `Generation`), the trait seams (`Provider`, `Backend`, `CacheStore`, `SignatureVerifier`, `LinkStrategy`), and the `VtaError` taxonomy | serde, thiserror, semver |
| `vanta-platform` | infra | OS/arch/libc detection, path math, link primitives, shell + executable handling, dirs | (minimal; libc/winapi via crates) |
| `vanta-state` | infra | redb tables: `store_index`, `generations`, `gc_roots`, `resolution_cache`, `registry_cache`, `trust`; transactions; locking | redb, serde |
| `vanta-net` | infra | HTTP over rustls; parallel, ranged, resumable downloads; mirrors; retries; proxy; auth | reqwest/hyper, rustls, tokio |
| `vanta-config` | domain | `vanta.toml`/`config.toml` typed model; parse; precedence/merge; span-accurate diagnostics | toml, serde, vanta-core |
| `vanta-lock` | domain | `vanta.lock` model; read/write/verify; manifest↔lock reconcile; canonical serialization | toml, serde, vanta-core |
| `vanta-resolve` | domain | Request → Resolution; version ordering; constraint satisfaction; dependency DAG | semver, vanta-registry, vanta-provider |
| `vanta-registry` | domain | Registry index model + distribution + search + metadata + caching | vanta-net, vanta-state, vanta-security |
| `vanta-provider` | domain | Provider model; built-in providers; Wasmtime host (WIT) for WASM hooks; capability sandbox | wasmtime, vanta-net, vanta-security |
| `vanta-store` | domain | Content-addressed store; atomic publish; link strategies; dedup; integrity; GC; layout | blake3, sha2, tar/zip/zstd/xz/flate2, vanta-platform |
| `vanta-security` | domain | Checksums; signature verifiers (minisign/cosign); trust DB; SBOM; provenance; sandbox policy | blake3, sha2, ed25519/minisign, sigstore, vanta-state |
| `vanta-install` | orchestration | The install engine: plan→fetch→verify→materialize→link→commit; transactions; generations | all domain + infra crates |
| `vanta-env` | orchestration | Environment composition; activation; per-shell hook generation; the resolution-cache fast path | vanta-store, vanta-state, vanta-platform |
| `vanta-diag` | orchestration | `doctor` checks; diagnostics rendering; the error-code registry | vanta-core, vanta-state, vanta-store |
| `vanta-cli` | cli | Every subcommand implemented as a testable library function; argv parsing; output/`--json` | clap, all orchestration crates |
| `vanta` | bin | Process entry for `vanta`/`vt`; assembles the `Engine`; runs the supervisor | vanta-cli |
| `vanta-shim` | bin | The shim dispatcher; resolves cwd→version from cache and `exec`s the real binary | vanta-state, vanta-platform (only) |
| `vanta-sdk` | tooling | Guest-side SDK for authoring WASM providers (the WIT bindings + helpers) | (guest target; wit-bindgen) |
| `vanta-migrate` | tooling | Importers from foreign managers to `vanta.toml` | vanta-config, vanta-core |
| `vanta-test` | tooling (dev) | Fakes (registry/provider/upstream HTTP), a temp-`$VANTA_HOME` harness, conformance helpers | (dev-dependency only) |

## Dependency graph

Arrows mean "depends on." The graph is a DAG with `vanta-core`/`vanta-platform` at the root and the binaries at the top. The hard rule: **domain crates never depend on `vanta-cli` or the binaries**, and **`vanta-shim` depends on the minimum** (no resolver, no HTTP, no WASM) so it starts cold in well under a millisecond.

```
                         vanta (bin)            vanta-shim (bin)
                              │                       │
                         vanta-cli                    │
              ┌──────────────┼───────────────┐        │
              ▼              ▼                ▼        │
        vanta-install   vanta-env        vanta-diag   │
        ┌───┬───┬───┬────┴───┐  └────┬──────┘         │
        ▼   ▼   ▼   ▼        ▼       ▼                │
  vanta-resolve  vanta-store  vanta-registry  vanta-provider
        │            │            │                │
        └── vanta-lock           vanta-security ────┘
                     │            │
                  vanta-net   vanta-state  vanta-config
                     │            │            │
                     └────────────┼────────────┘
                                  ▼
                       vanta-core   vanta-platform   (leaves)
                                  ▲        ▲
                          vanta-shim ──────┘ (only these two)
```

Layering is enforced in CI by a dependency-direction check (an `xtask` lint or `cargo-deny`'s ban rules): a PR that makes a domain crate depend on `vanta-cli`, or that adds a cycle, fails the build.

## Shared dependencies

All third-party versions are declared once in `[workspace.dependencies]` and inherited (`tokio.workspace = true`) so versions never drift across crates. The load-bearing external dependencies and why each was chosen (decisions recorded in [24. ADRs](24-architecture-decision-records.md)):

| Dependency | Used for | Why this one |
| --- | --- | --- |
| `tokio` | async runtime for parallel IO | mature, ecosystem fit; IO-bound work, not latency-critical |
| `reqwest` / `hyper` + `rustls` | HTTP + TLS | **no OpenSSL** → single static binary; range/HTTP-2 support |
| `redb` | embedded state DB | pure-Rust, transactional, single-writer/multi-reader; no C dep (vs SQLite/sled) |
| `toml` + `serde` | manifest/lock/config | human-readable, ubiquitous, serde-native |
| `clap` | CLI parsing | derive API, completions, help quality |
| `wasmtime` | provider sandbox | component model + WIT; capability-based isolation |
| `blake3` + `sha2` | hashing | BLAKE3 for store keys (fast, parallel); SHA-256 for upstream-checksum interop |
| `tar`/`zip`/`flate2`/`zstd`/`xz2`/`bzip2` | archive extraction | pure-Rust where possible; the common artifact formats |
| `ed25519-dalek`/`minisign` + sigstore | signatures | Ed25519 minisign for the registry; cosign/sigstore verification for artifacts |
| `semver` | version ordering | the default comparator; providers may override |

## Coding standards

- **`#![forbid(unsafe_code)]` by default.** A crate may opt into a narrowly-scoped `unsafe` allow-list only with written justification (e.g. a platform syscall in `vanta-platform` for reflink/hardlink), confined behind a safe wrapper and covered by tests. No `unsafe` on a data path that handles untrusted bytes outside of audited, fuzzed parsers.
- **No panics on non-bug paths.** Every fallible operation returns `Result<_, VtaError>` carrying a stable `VTA-<AREA>-<NNNN>` code. `unwrap`/`expect`/`panic!` are reserved for provable invariants and are denied by clippy on library crates. A panic that does escape is caught at the top level, logged, and mapped to exit code 1 / `VTA-INT-*`.
- **Errors:** `thiserror`-style enums inside libraries; `anyhow`-style context only at the binary/CLI top level. Every error variant maps to a catalog code ([25. Error Catalog](25-error-and-exit-code-catalog.md)).
- **Async hygiene.** No blocking calls inside async fns; CPU-bound work (decompression, hashing, large copies) runs on `spawn_blocking`/rayon. Every `await` that holds a lock is reviewed for cancellation safety.
- **Public API discipline.** Anything `pub` in `vanta-core`/`vanta-sdk` has a doc comment and, where it is an example-worthy API, a doc-test. Public enums/structs that may grow are `#[non_exhaustive]`.
- **Determinism.** Code that affects store keys or lock output must be deterministic: sorted iteration, normalized paths/modes, no wall-clock or RNG in hashed material (timestamps are recorded as metadata, never hashed). This is enforced by reproducibility tests ([28. Testing](28-testing.md)).
- **Formatting/lints.** `cargo fmt` and `cargo clippy -D warnings` are gates; `cargo-deny` enforces license/advisory/ban policy.

## Naming conventions

- **Crates:** `vanta-<area>`, lower-kebab. **Binaries:** `vanta` (alias `vt`) and `vanta-shim`.
- **Types:** `CamelCase`; trait names are nouns/capabilities (`Provider`, `CacheStore`); error enums end in `Error` (`ResolveError`) with the top-level `VtaError`; config types end in `Config` (`ToolConfig`).
- **Functions/vars:** `snake_case`; constructors `new`/`with_*`/`from_*`; builders return `Self`.
- **Error codes:** `VTA-<AREA>-<NNNN>` with the canonical area set (CFG/RES/REG/PROV/NET/VRF/STORE/INST/ENV/LOCK/SYS/INT).
- **Store keys:** `blake3-<lowercase-hex>`. **Env ids / generation ids:** opaque, sortable.
- **CLI:** verbs are short and intent-named (`add`/`remove`/`update`/`sync`); flags follow the canon global set.

## Testing strategy

Summarised here; specified in full in [28. Testing](28-testing.md).

- **Unit + property** tests in every crate (version ordering totality, manifest↔lock round-trips, store canonicalization).
- **Fuzzing** of every parser and the archive extractor (TOML, version requests, provider manifests, tar/zip/xz/zstd — path-traversal/zip-slip).
- **Integration** via `vanta-test`: a fake registry + fake providers + a local artifact server, a temp `$VANTA_HOME`, so the full lifecycle runs hermetically and offline in CI.
- **Reproducibility** tests: the same lock on different runners produces byte-identical store keys.
- **Cross-platform** runs on the full CI matrix; **e2e** with real network in a gated/nightly job.
- Coverage target ≥ 85% on `vanta-core` and the domain crates; 100% of error codes exercised; a release is gated on green.

## CI/CD architecture

```
PR  ─► fmt · clippy -Dwarnings · cargo-deny · unit/prop tests · build matrix · doc-test · MSRV check · perf-diff
main ─► full integration + cross-platform e2e + fuzz smoke + coverage gate
tag  ─► reproducible release build · sign + SBOM + SLSA · publish artifacts/installers (see doc 32)
```

- **Matrix:** Linux (x86_64, aarch64; gnu + musl), macOS (aarch64, x86_64), Windows (x86_64, aarch64); Rust stable + MSRV.
- **Caching:** `sccache`/registry cache for fast PRs; per-crate incremental builds.
- **Quality gates:** clippy/fmt/deny pass; coverage must not drop; the perf-diff job flags a >X% regression on the benchmark suite ([16. Performance](16-performance.md)).
- **Supply chain:** `cargo-deny` (licenses/advisories/bans), `cargo-audit`, dependency review on PRs; release builds are reproducible and attested ([32. Release Engineering](32-release-engineering.md)).

## Versioning policy

Four surfaces are versioned **independently** so an upgrade of one never silently breaks another (canon §14):

| Surface | Scheme | Compatibility guarantee |
| --- | --- | --- |
| The `vanta` binary | SemVer | breaking CLI/`--json` changes only on a major bump; deprecations warned for ≥1 minor |
| Manifest/lock format | `version` / `lock_version` integers in the files | a newer binary reads older formats; an older binary refuses a newer format with a clear `VTA-LOCK-*`/`VTA-CFG-*` message ([31. Lock Reference](31-lockfile-and-manifest-reference.md)) |
| Provider ABI | WIT world SemVer | the host supports a documented range of ABI majors; providers keep working within a major ([22. Provider SDK](22-provider-sdk.md)) |
| Registry index format | versioned schema | the client negotiates; the registry may serve multiple schema versions ([26. Registry Reference](26-registry-and-metadata-reference.md)) |

- **Pre-1.0 (0.x):** breaking changes are allowed at minor bumps but documented with migrations and confined to scheduled windows; the **manifest/lock format is stabilized first** so early adopters' committed files keep working.
- **MSRV:** latest stable minus two; an MSRV bump is a documented minor-version event.
- Only `vanta-sdk` (and any later-exposed public library facade) carries a Rust API stability guarantee; all other `vanta-*` crates are internal and may change freely.

## Cross-references

- [02. Architecture](02-architecture.md) — the subsystem map these crates implement and the trait seams in `vanta-core`.
- [22. Provider SDK](22-provider-sdk.md) — `vanta-sdk` and the independently-versioned provider ABI.
- [28. Testing](28-testing.md) — the full testing pyramid summarised here.
- [16. Performance](16-performance.md) — the CI perf-gate and the `vanta-shim` cold-start budget.
- [24. ADRs](24-architecture-decision-records.md) — the rationale for redb, rustls, the workspace, and BLAKE3.
- [32. Release Engineering](32-release-engineering.md) — how tagged releases become signed, reproducible artifacts.
- [25. Error & Exit-code Catalog](25-error-and-exit-code-catalog.md) — the `VTA-*` taxonomy referenced by the coding standards.
