# 19. Milestones

> The execution plan: Vanta broken into nine sequenced phases (P0–P8), each with objectives, deliverables mapped to the crate set, a lines-of-code budget, a duration estimate, testing requirements, risks, dependencies, and binary acceptance criteria — enough for a team to plan sprints and track progress against the architecture in [02. Architecture](02-architecture.md) and [03. Repository](03-repository.md).

**Contents**

- [How to read this](#how-to-read-this)
- [Phase timeline](#phase-timeline)
- [Phase 0 — Foundations](#phase-0--foundations)
- [Phase 1 — Config, lock & state core](#phase-1--config-lock--state-core)
- [Phase 2 — Store, net & first install](#phase-2--store-net--first-install)
- [Phase 3 — Resolution, registry & providers](#phase-3--resolution-registry--providers)
- [Phase 4 — Environments, activation & CLI](#phase-4--environments-activation--cli)
- [Phase 5 — Reproducibility & cross-platform lock](#phase-5--reproducibility--cross-platform-lock)
- [Phase 6 — Security & supply chain](#phase-6--security--supply-chain)
- [Phase 7 — Offline, enterprise & migration](#phase-7--offline-enterprise--migration)
- [Phase 8 — 1.0 hardening](#phase-8--10-hardening)
- [Estimates summary](#estimates-summary)
- [Cross-cutting risks](#cross-cutting-risks)
- [Cross-references](#cross-references)

---

## How to read this

- **LOC** is order-of-magnitude Rust LOC excluding tests/docs (which add ~60–100%); it sizes effort, not a target.
- **Durations** assume a small senior team (~3–5 engineers); phases overlap where dependencies allow.
- **Acceptance criteria** are binary and testable — a phase is done only when all are met and its testing requirements pass in CI.
- Deliverables name the canon §14 crates they land in.

## Phase timeline

```
P0 Foundations ─► P1 Config/Lock/State ─► P2 Store/Net/Install ─► P3 Resolve/Registry/Providers ─► P4 Env/Activation/CLI
                                                                            │                              │
                                                                            └──────────────► P5 Reproducibility/X-plat lock
                                                                                                          │
                                              P6 Security/Supply chain ─► P7 Offline/Enterprise/Migration ─► P8 1.0 hardening
   ◀──────────── ~Year 1 (P0–P4, public 0.x) ───────────►◀──────────────── ~Year 2 (P5–P8, → 1.0) ────────────────►
```

Critical path: P0→P1→P2→P3→P4 (the end-to-end install + activation path). P5 hardens reproducibility once resolution exists; P6 can begin in parallel after P3 (verification hooks into the install gate); P7 builds on P5/P6; P8 is the freeze.

---

## Phase 0 — Foundations

- **Objectives:** the workspace, core vocabulary, platform layer, and CI so all later work plugs in cleanly.
- **Deliverables:** Cargo workspace + all crate stubs; `vanta-core` (`Request`/`Resolution`/`Artifact`/`StoreKey`/`Generation`, the trait seams, the `VtaError` taxonomy skeleton); `vanta-platform` (os/arch/libc detection, path math, link `probe`s, shell detection); CI matrix (fmt/clippy/deny/test/doc, all platforms, MSRV); `xtask`.
- **LOC:** ~3k. **Duration:** ~3–4 weeks. **Dependencies:** none.
- **Testing:** CI green on all platforms; trait/doc-tests compile; `cargo deny` passes; platform detection unit tests on each OS.
- **Risks:** over-designing core traits early (mitigate: `#[non_exhaustive]`, iterate).
- **Acceptance:** `cargo build/test/clippy/doc` pass on the full matrix; the layering check enforces the DAG; a "hello" `vanta --version` runs on all three OSes.

## Phase 1 — Config, lock & state core

- **Objectives:** the data backbone — manifests, lockfile, and persistent state.
- **Deliverables:** `vanta-config` (TOML model for `vanta.toml`/`config.toml`, precedence/merge, span-accurate diagnostics); `vanta-lock` (lock model, canonical serialization, manifest↔lock reconcile); `vanta-state` (redb tables: `store_index`, `generations`, `gc_roots`, `resolution_cache`, `registry_cache`, `trust`; transactions; locking); `vanta config`/`vanta validate`.
- **LOC:** ~8k. **Duration:** ~6–8 weeks. **Dependencies:** P0.
- **Testing:** parser unit + **property** (round-trip) + **fuzz** (TOML manifest/lock); golden diagnostics; redb concurrency tests; canonical-serialization stability tests.
- **Risks:** config/diagnostic quality is make-or-break (mitigate: invest in spans early, dogfood); state-schema churn (mitigate: a `meta` version table from day one).
- **Acceptance:** every [05. Configuration](05-configuration.md) example parses/validates; invalid configs yield `VTA-CFG-*` with correct spans; concurrent redb access is correct under a stress test.

## Phase 2 — Store, net & first install

- **Objectives:** install one prebuilt tool end-to-end from a direct URL — the first traversal of `[3 Plan]`..`[8 Commit]`.
- **Deliverables:** `vanta-store` (CAS, canonical hashing, atomic publish, link strategies, GC skeleton); `vanta-net` (parallel/resumable downloads over rustls, mirrors, retries); `vanta-install` (the pipeline + transactions + generations); a minimal verification gate (checksums); `vanta add <url-or-direct-tool>`, `vanta gc`.
- **LOC:** ~10k. **Duration:** ~8–10 weeks. **Dependencies:** P1.
- **Testing:** integration via `vanta-test` (fake artifact server, temp `$VANTA_HOME`); **fuzz** archive extraction (tar/zip/xz/zstd, zip-slip); atomic-publish crash-injection tests; resumable-download tests; GC mark-sweep correctness; **reproducibility** test (same artifact → same store key across runners).
- **Risks:** atomicity/crash-safety subtleties (mitigate: crash-injection tests from the start); archive-extraction security (mitigate: fuzzing, path-traversal tests).
- **Acceptance:** `vanta add` of a direct-URL tool installs, verifies checksum, materializes atomically, and is re-runnable idempotently; killing mid-install never corrupts the store; `vanta gc` reclaims only unreachable entries.

## Phase 3 — Resolution, registry & providers

- **Objectives:** resolve real tools from a registry — `vanta add node@24` works.
- **Deliverables:** `vanta-resolve` (request grammar, version ordering, constraints, dependency DAG); `vanta-registry` (signed index model, fetch/cache, search); `vanta-provider` (declarative provider model + a set of built-in providers covering the headline tools; the Wasmtime host stub for WASM hooks); `vanta search`/`info`/`outdated`.
- **LOC:** ~12k. **Duration:** ~10–12 weeks. **Dependencies:** P2.
- **Testing:** resolution property tests (ordering totality, constraint satisfaction); fake-registry integration; provider golden-resolution tests; dependency-DAG conflict tests; cache TTL/ETag tests.
- **Risks:** provider model scope creep (mitigate: declarative-first, defer WASM to P6 polish); version-ordering edge cases (mitigate: per-provider comparators + tests).
- **Acceptance:** `vanta add node@24`/`python@3.13`/`terraform@latest` resolve from the registry, install, and lock; `vanta outdated` is correct under constraints.

## Phase 4 — Environments, activation & CLI

- **Objectives:** automatic per-directory versions and a complete CLI — the daily experience. **Public 0.x after this phase.**
- **Deliverables:** `vanta-env` (env composition, the resolution-cache fast path, hook generation per shell); `vanta-shim` (the dispatcher); `vanta-diag` (`doctor`); the full `vanta-cli` (every canon §5 command), `--json` output; `vanta activate`/`exec`/`run`/`x`/`shell`/`which`.
- **LOC:** ~12k. **Duration:** ~10–12 weeks. **Dependencies:** P3.
- **Testing:** activation latency benchmarks (sub-ms warm); shim dispatch tests on all OSes (incl. Windows launcher shims, `cmd.exe`); per-shell hook tests; `--json` schema tests; `doctor` check coverage.
- **Risks:** shell-integration breadth (mitigate: a shared hook core + per-shell adapters + tests); Windows activation (mitigate: shims-first on Windows).
- **Acceptance:** `cd` switches versions sub-ms warm; shims resolve correct versions in non-hooked contexts on all OSes; 0.x install→add→switch→rollback works end-to-end; perf budgets met ([16. Performance](16-performance.md)).

> **Milestone: public 0.x release** — a genuinely useful single-machine tool manager. Begins external feedback and dogfooding.

## Phase 5 — Reproducibility & cross-platform lock

- **Objectives:** lock-driven reproduction across operating systems.
- **Deliverables:** resolve-for-all-targets in `vanta-resolve`; full cross-platform `vanta.lock` (`vanta-lock`); `vanta sync` (+`--frozen`/`--offline`), `vanta lock`, `vanta target`; generations/rollback/GC hardened ([12. Updates](12-updates.md)).
- **LOC:** ~9k. **Duration:** ~8–10 weeks. **Dependencies:** P3, P4.
- **Testing:** cross-platform reproducibility tests (one lock → identical store keys on mac/Linux/Windows runners); `--frozen` drift tests; rollback/generation invariant property tests; merge-conflict-resolution tests for the lock.
- **Risks:** non-deterministic artifacts undermining repro (mitigate: prebuilt-first, hash-pin inputs, flag non-reproducible providers).
- **Acceptance:** a single committed lock reproduces byte-identical environments on all three OSes; `vanta sync --frozen` fails on drift; rollback is instant and correct.

## Phase 6 — Security & supply chain

- **Objectives:** the full secure-by-default model.
- **Deliverables:** `vanta-security` (signature verifiers minisign/cosign; TUF-style signed-metadata roles + rotation; SLSA provenance verification; trust DB; SBOM); the **Wasmtime provider sandbox** (capabilities, fuel/epoch) productionized in `vanta-provider`; sandboxed source builds; config trust-on-first-use; `vanta trust`/`sbom`/`provenance`.
- **LOC:** ~12k. **Duration:** ~10–12 weeks. **Dependencies:** P3 (providers), P5 (lock pins signers).
- **Testing:** signature/checksum failure paths fail-closed; metadata rollback/freeze tests; **provider sandbox escape attempts**; fuel/epoch limit tests; zip-slip/path-traversal; trust-prompt tests; SBOM correctness.
- **Risks:** sandbox correctness (mitigate: adversarial tests, capability-deny-by-default); key-management design (mitigate: TUF-modeled roles, threshold root).
- **Acceptance:** unsigned/tampered artifacts are refused by default; a malicious WASM provider cannot exceed granted capabilities or hang the host; metadata rollback is detected; SBOM matches installed reality.

## Phase 7 — Offline, enterprise & migration

- **Objectives:** disconnected and organizational deployment, and easy adoption.
- **Deliverables:** offline mode + mirrors + `vanta bundle`/`restore` + `vanta registry mirror` ([13. Offline](13-offline.md)); private registries + auth (tokens/OIDC/keychain) + signed policy enforcement + audit/license reporting ([14. Enterprise](14-enterprise.md)); `vanta-migrate` importers (mise/asdf/nvm/fnm/pyenv/rbenv/volta/brew/scoop/pkgx) ([30. Migration](30-migration.md)).
- **LOC:** ~12k. **Duration:** ~10–12 weeks. **Dependencies:** P5, P6.
- **Testing:** air-gapped bundle round-trip (build→transfer→restore→sync offline); mirror fallback + bad-mirror verification tests; private-registry auth tests; policy-enforcement tests; migration fidelity tests vs real foreign files.
- **Risks:** bundle/format stability (mitigate: version it); policy distribution security (mitigate: signed policy).
- **Acceptance:** a project reproduces fully air-gapped from a bundle; org policy denies out-of-policy installs at resolution; `vanta migrate` converts representative real-world setups with a fidelity report.

## Phase 8 — 1.0 hardening

- **Objectives:** the hardening to call it 1.0.
- **Deliverables:** continuous fuzzing sweep; performance budgets met and published; an **external security audit**; format/ABI **freeze** (manifest, lock, provider WIT, registry index); complete docs and migration guides; the official registry's signing pipeline productionized ([32. Release Engineering](32-release-engineering.md)).
- **LOC:** ~8k (hardening/polish). **Duration:** ~10–14 weeks. **Dependencies:** P5–P7.
- **Testing:** full cross-platform e2e on real tools; long-duration soak (repeated add/update/gc/rollback); fuzz corpus expansion; audit-issue remediation; reproducibility CI across all targets.
- **Risks:** 1.0 stability commitment (mitigate: freeze + deprecation policy); late audit findings (mitigate: continuous security review from P6).
- **Acceptance:** all perf budgets met and published; security audit issues resolved; formats/ABI frozen with a deprecation policy; cross-platform reproducibility proven in CI → **tag 1.0**.

## Estimates summary

| Phase | Focus | Core LOC | Duration |
| --- | --- | --- | --- |
| P0 | Foundations | ~3k | 3–4 wk |
| P1 | Config/Lock/State | ~8k | 6–8 wk |
| P2 | Store/Net/Install | ~10k | 8–10 wk |
| P3 | Resolve/Registry/Providers | ~12k | 10–12 wk |
| P4 | Env/Activation/CLI | ~12k | 10–12 wk |
| P5 | Reproducibility/X-platform lock | ~9k | 8–10 wk |
| P6 | Security/Supply chain | ~12k | 10–12 wk |
| P7 | Offline/Enterprise/Migration | ~12k | 10–12 wk |
| P8 | 1.0 hardening | ~8k | 10–14 wk |
| **Total** | core ~**86k** Rust LOC (+ tests/docs) | | ~**Year 1**: P0–P4 (0.x); **Year 2**: P5–P8 (1.0) |

These align with the [01. Vision](01-vision.md) roadmap (Year 1 single-machine excellence; Year 2 reproducibility/security/enterprise to 1.0).

## Cross-cutting risks

- **Scope discipline.** "One command for everything" invites endless provider scope; hold the 1.0 line on the *engine*, and let the *registry/providers* grow independently after 1.0 ([07. Providers](07-providers.md)).
- **Correctness under concurrency.** The store/lock/generation model is powerful but subtle; property/fuzz/crash-injection/soak from P2 onward, not at the end ([28. Testing](28-testing.md)).
- **Security debt.** Continuous security review and the threat model ([21. Threat Model](21-threat-model.md)) from P6, not a single pre-1.0 audit.
- **Performance regressions.** The CI perf-gate ([16. Performance](16-performance.md)) runs from P4 so regressions are caught per-PR.
- **Format/ABI stability.** Manifest/lock format stabilizes earliest (P1/P5); provider ABI and registry format freeze at 1.0; all versioned independently ([03. Repository](03-repository.md)).

## Cross-references

- [02. Architecture](02-architecture.md) — the system these phases build.
- [03. Repository](03-repository.md) — the crates each phase delivers and the CI/release gates.
- [28. Testing](28-testing.md) — the test classes named as phase requirements.
- [16. Performance](16-performance.md) — the budgets that gate P4 onward.
- [01. Vision](01-vision.md) — the multi-year roadmap these phases execute.
- [20. Future](20-future.md) — what comes after 1.0.
