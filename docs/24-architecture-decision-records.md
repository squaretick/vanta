# 24. Architecture Decision Records

> The formal record of *why* Vanta is built the way it is. Each ADR captures a decision's context, the choice, the alternatives weighed, and the consequences. ADRs are immutable once accepted; a reversal is a new ADR that supersedes an old one. They are the durable output of the design and the answer to "why not just do X?"

**Contents**

- [Format and status](#format-and-status)
- [Index](#index)
- [The records](#the-records)
- [Cross-references](#cross-references)

---

## Format and status

Each ADR: **Context** (the forces) → **Decision** → **Alternatives** (and why rejected) → **Consequences** (good and bad). Status ∈ {Proposed, Accepted, Superseded(by N), Deprecated}. All records below are Accepted.

## Index

| # | Title | Status |
| --- | --- | --- |
| 0001 | Rust as the implementation language | Accepted |
| 0002 | A single static binary, zero runtime dependencies | Accepted |
| 0003 | A content-addressed immutable store as the keystone | Accepted |
| 0004 | Atomic generations + pointer-swap rollback | Accepted |
| 0005 | TOML manifests over a bespoke DSL or YAML | Accepted |
| 0006 | Declarative + WASM-sandboxed providers over arbitrary shell | Accepted |
| 0007 | BLAKE3 store keys with SHA-256 interop | Accepted |
| 0008 | redb as the embedded state store | Accepted |
| 0009 | rustls, no OpenSSL | Accepted |
| 0010 | A cross-platform lockfile (resolve for all targets) | Accepted |
| 0011 | Hybrid activation: shell hook + shims | Accepted |
| 0012 | No daemon by default | Accepted |
| 0013 | Tokio for parallel IO; no custom runtime abstraction | Accepted |
| 0014 | clap for the CLI | Accepted |
| 0015 | A Cargo workspace with a layered crate graph | Accepted |
| 0016 | Apache-2.0 + open-core | Accepted |
| 0017 | Secure by default (verification on, opt-out only) | Accepted |
| 0018 | Trust-on-first-use for project configs and third-party registries | Accepted |
| 0019 | A stable error taxonomy (VTA-*) + stable exit codes | Accepted |
| 0020 | Wasmtime component model for the provider sandbox | Accepted |
| 0021 | Ephemeral run (`vanta x`) as a first-class mode | Accepted |
| 0022 | Prebuilt-binary-first; sandboxed source builds as fallback | Accepted |
| 0023 | Independent versioning (binary / format / provider ABI / registry) | Accepted |
| 0024 | An optional, minimal built-in task runner | Accepted |
| 0025 | OS keychain for secrets at rest | Accepted |

## The records

**ADR-0001 — Rust.** *Context:* Vanta executes downloaded code and runs on a latency-sensitive hot path (shims, activation); it must be safe and fast and ship as one binary. *Decision:* implement in Rust. *Alternatives:* Go (GC pauses, larger binaries, weaker zero-cost abstractions — rejected for hot-path latency and single-binary goals), C/C++ (memory unsafety in a security tool — rejected), a scripting language (startup cost, distribution — rejected). *Consequences:* memory safety + C-class performance + easy static binaries; steeper contributor ramp; `unsafe` confined to `vanta-platform` ([03. Repository](03-repository.md)).

**ADR-0002 — Single static binary.** *Context:* distribution and operability are features; every runtime dependency is a failure mode. *Decision:* ship one statically-linked binary (plus the tiny `vanta-shim`) with no runtime deps. *Alternatives:* dynamic linking / interpreter runtime (breaks "works everywhere," adds install friction). *Consequences:* trivial install, identical on every OS; rules out OpenSSL and any C runtime dep, constraining library choices (rustls, redb) — which is desired.

**ADR-0003 — Content-addressed store.** *Context:* we want dedup, integrity, atomic rollback, reproducibility, and safe concurrency. *Decision:* materialize every tool into an immutable directory keyed by a content hash. *Alternatives:* per-tool mutable prefixes (asdf/mise — no dedup, no integrity, rollback = reinstall), a global mutable prefix (Homebrew — not multi-version/reproducible). *Consequences:* the keystone that yields most pillars at once; cost is an indirection and the need for GC ([09. Store](09-store.md)).

**ADR-0004 — Generations + rollback.** *Context:* a package manager that can leave a broken state is a liability. *Decision:* every mutation appends an immutable generation; the active one is a pointer; rollback flips it. *Alternatives:* in-place mutation (unrecoverable), full snapshots (expensive). *Consequences:* instant, non-destructive rollback referencing existing store entries; bounded retention + GC trade disk for history depth ([12. Updates](12-updates.md)).

**ADR-0005 — TOML, not a DSL.** *Context:* config is a primary UX and must be human-readable (pillar 9). *Decision:* TOML for manifest/lock/config; no bespoke language. *Alternatives:* a custom DSL (no domain complexity to justify a language; adds learning cost — rejected, in contrast to tools whose config encodes control flow), YAML (whitespace/type footguns), JSON (no comments). *Consequences:* instantly familiar, diffs cleanly; cannot express computed config — intentional ([05. Configuration](05-configuration.md)).

**ADR-0006 — Declarative + WASM providers.** *Context:* extension must be safe, cross-platform (incl. Windows), and reliable. *Decision:* declarative TOML providers, with sandboxed WASM hooks only where logic is unavoidable. *Alternatives:* arbitrary shell plugins (asdf/AUR/Chocolatey — no Windows, arbitrary-code risk — rejected outright), Lua (one language, still unsandboxed authority). *Consequences:* a safe, language-agnostic, cross-platform extension model; cost is a richer manifest schema + a WASM host ([22. Provider SDK](22-provider-sdk.md)).

**ADR-0007 — BLAKE3 + SHA-256.** *Context:* store keys need a fast, secure hash; upstream artifacts publish SHA-256. *Decision:* BLAKE3 for store keys/internal integrity; also compute SHA-256 for interop. *Alternatives:* SHA-256 only (slower for large trees), SHA-1/MD5 (broken). *Consequences:* fast parallel hashing + ecosystem compatibility; two hashes to compute (cheap relative to IO) ([09. Store](09-store.md)).

**ADR-0008 — redb.** *Context:* need transactional, embedded, pure-Rust metadata storage. *Decision:* redb. *Alternatives:* SQLite (C dependency — breaks single-binary), sled (stability), files-only (no transactions/consistency). *Consequences:* ACID local state with no C dep; smaller ecosystem than SQLite ([23. Data & State Model](23-data-and-state-model.md)).

**ADR-0009 — rustls.** *Context:* single static binary, memory safety. *Decision:* rustls only. *Alternatives:* OpenSSL (C dep, CVE history — breaks single-binary), native-tls (platform variance). *Consequences:* memory-safe TLS, modern defaults; a FIPS-validated provider would be a separate build for regulated use ([15. Security](15-security.md)).

**ADR-0010 — Cross-platform lock.** *Context:* mixed-OS teams must share one reproducible environment. *Decision:* resolve and lock artifacts for *all* declared target platforms, not just the current one. *Alternatives:* per-platform locks (drift, N files), version-only locks (mise/asdf — not byte-reproducible). *Consequences:* one lock reproduces on every OS; resolving for absent platforms needs their metadata (cheap, cached) ([11. Reproducibility](11-reproducibility.md)).

**ADR-0011 — Hook + shim hybrid.** *Context:* activation must be fast *and* work everywhere. *Decision:* a shell hook (PATH injection, default) plus shims (fallback), both reading one resolution cache. *Alternatives:* hook-only (mise — misses IDE/cron/cmd), shim-only (asdf — per-call overhead). *Consequences:* fast interactive switching + universal correctness; two mechanisms to maintain, kept consistent by a single cache ([10. Environments](10-environments.md)).

**ADR-0012 — No daemon.** *Context:* cold start must be <5 ms and the model must be identical and secure across OSes. *Decision:* a short-lived CLI; warm state on disk (redb + CAS), no resident process. *Alternatives:* a daemon (cache coherence, security surface, cross-platform service management — rejected as the default). *Consequences:* simple, secure, identical everywhere; warm-path speed comes from on-disk caches, not memory; an opt-in file-watcher is the one exception ([02. Architecture](02-architecture.md#why-no-daemon)).

**ADR-0013 — Tokio, no runtime abstraction.** *Context:* the work is IO-bound (parallel downloads), not microsecond-latency-bound like a proxy. *Decision:* use Tokio directly. *Alternatives:* a runtime-abstraction layer (unneeded complexity here), thread-per-core io_uring (overkill for a CLI). *Consequences:* parallel IO with a mature ecosystem; no abstraction tax ([03. Repository](03-repository.md)).

**ADR-0014 — clap.** *Context:* a large, discoverable command surface with completions and great help. *Decision:* clap (derive). *Alternatives:* hand-rolled parsing (reinventing help/completions). *Consequences:* consistent UX, shell completions for free; a build-time dependency ([04. CLI](04-cli.md)).

**ADR-0015 — Workspace + layering.** *Context:* enforceable architecture, parallel builds, a minimal-dependency shim. *Decision:* one Cargo workspace, layered crates, a DAG with `vanta-core`/`vanta-platform` at the leaves; `vanta-shim` depends on the minimum. *Alternatives:* mega-crate (no layering, slow), many repos (coordination tax). *Consequences:* enforced seams, fast shim start, parallel CI ([03. Repository](03-repository.md)).

**ADR-0016 — Apache-2.0 + open-core.** *Context:* broad adoption + a durable project, without rug-pulls. *Decision:* Apache-2.0 core (with the enterprise needs *in* the core), a separate commercial edition for fleet-scale convenience. *Alternatives:* MIT (no patent grant), AGPL/SSPL core (chills adoption), freemium-gated core (the incumbent anti-pattern). *Consequences:* adoption + patent protection; a published open/closed line to hold ([14. Enterprise](14-enterprise.md), [20. Future](20-future.md)).

**ADR-0017 — Secure by default.** *Context:* supply-chain compromise and arbitrary install code are the top real risks. *Decision:* verification (checksum+signature) on by default; `--no-verify` warns and is policy-forbiddable. *Alternatives:* opt-in verification (unsafe defaults). *Consequences:* safe out of the box; occasional friction when a tool genuinely lacks signatures, made explicit ([15. Security](15-security.md)).

**ADR-0018 — Trust-on-first-use.** *Context:* a cloned repo's manifest can inject env or run tasks. *Decision:* env/task/third-party-registry sections are inert until `vanta trust`. *Alternatives:* trust all configs (direnv's original footgun), trust nothing (breaks the feature). *Consequences:* blocks "clone runs code" while keeping the safe common case frictionless ([05. Configuration](05-configuration.md)).

**ADR-0019 — Error taxonomy.** *Context:* operators need stable, searchable, scriptable errors. *Decision:* `VTA-<AREA>-<NNNN>` codes + stable exit codes, generated from a registry. *Alternatives:* free-text errors (unsearchable, unstable). *Consequences:* documentable, scriptable diagnostics; discipline to assign/maintain codes ([25. Error Catalog](25-error-and-exit-code-catalog.md)).

**ADR-0020 — Wasmtime component model.** *Context:* provider hooks need a safe, language-agnostic, capability-scoped sandbox. *Decision:* Wasmtime + the component model + a WIT world. *Alternatives:* native dlopen (no sandbox), a custom interpreter (cost), Lua (one language, weak isolation). *Consequences:* strong isolation + fuel/epoch limits + multi-language authoring; an ABI to version and marshaling overhead (off the hot path) ([22. Provider SDK](22-provider-sdk.md)).

**ADR-0021 — Ephemeral run.** *Context:* developers want npx/pipx/uvx for any tool. *Decision:* `vanta x` runs a tool without adding it, backed by the verified store. *Alternatives:* require `add` first (friction), a separate cache (duplication). *Consequences:* one verb replaces npx/uvx/pipx-run/pkgx, with verification and dedup; ephemeral entries are GC-eligible ([10. Environments](10-environments.md)).

**ADR-0022 — Prebuilt-first.** *Context:* source builds are slow and a security/reproducibility hazard. *Decision:* prefer prebuilt binaries; treat source builds as a sandboxed, policy-gated exception. *Alternatives:* build-from-source-first (AUR/`cargo install` — slow, unsafe), prebuilt-only (limits coverage). *Consequences:* fast, reproducible installs by default; the long tail still reachable via sandboxed builds ([08. Installation](08-installation.md)).

**ADR-0023 — Independent versioning.** *Context:* upgrading the binary must not break configs or providers. *Decision:* version the binary (SemVer), the manifest/lock format, the provider ABI (WIT), and the registry index independently. *Alternatives:* one version for all (forced lockstep upgrades). *Consequences:* smooth upgrades; more compatibility matrices to test ([03. Repository](03-repository.md)).

**ADR-0024 — Optional task runner.** *Context:* keeping tools and dev commands in one file is convenient, but Vanta is not a build system. *Decision:* a deliberately minimal `[tasks]` table, present but quiet. *Alternatives:* no task runner (a gap users fill with Make/scripts anyway), a full build system (scope creep, violates non-goals). *Consequences:* one-file onboarding; a hard line against growing into Bazel ([05. Configuration](05-configuration.md)).

**ADR-0025 — OS keychain for secrets.** *Context:* registry credentials must not sit in plaintext. *Decision:* store secrets in the OS keychain (Keychain/Credential Manager/Secret Service), with a guarded file fallback. *Alternatives:* plaintext config/lock (unsafe), a custom encrypted store (key-management burden). *Consequences:* OS-grade secret protection, log redaction; a per-OS integration to maintain ([23. Data & State Model](23-data-and-state-model.md)).

## Cross-references

- [02. Architecture](02-architecture.md) — decisions 0003, 0004, 0011, 0012 in practice.
- [05. Configuration](05-configuration.md) — 0005, 0018, 0024.
- [09. Store](09-store.md) & [23. Data & State Model](23-data-and-state-model.md) — 0003, 0007, 0008.
- [15. Security](15-security.md) & [22. Provider SDK](22-provider-sdk.md) — 0006, 0017, 0020.
- [11. Reproducibility](11-reproducibility.md) — 0010, 0022.
- [03. Repository](03-repository.md) — 0001, 0002, 0015, 0023.
