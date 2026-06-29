# 28. Testing, Fuzzing & Benchmarks

> The strategy that makes Vanta trustworthy: the testing pyramid, property tests for the trickiest invariants, continuous fuzzing of every parser and the archive extractor, a hermetic integration harness, cross-platform reproducibility tests, adversarial security tests, end-to-end flows, the benchmark suite and CI perf-gate, and the release gates a build must clear. Correctness under concurrency and against hostile input is treated as a first-class requirement, designed in from Phase 2 ([19. Milestones](19-milestones.md)).

**Contents**

- [The testing pyramid](#the-testing-pyramid)
- [Property tests](#property-tests)
- [Fuzzing](#fuzzing)
- [The integration harness](#the-integration-harness)
- [Reproducibility tests](#reproducibility-tests)
- [Security tests](#security-tests)
- [End-to-end tests](#end-to-end-tests)
- [Cross-platform matrix](#cross-platform-matrix)
- [Benchmarks and the perf-gate](#benchmarks-and-the-perf-gate)
- [Release gates](#release-gates)
- [Cross-references](#cross-references)

---

## The testing pyramid

| Class | Covers | Tooling | When it runs |
| --- | --- | --- | --- |
| Unit | per-crate logic | `cargo test` | every PR |
| Property | invariants over generated inputs | `proptest` | every PR |
| Fuzz | parsers + archive extraction (hostile input) | `cargo-fuzz`/libFuzzer + `arbitrary` | smoke per PR; continuous nightly |
| Integration | full lifecycle against fakes | `vanta-test` harness | every PR |
| Reproducibility | byte-identical store keys across runners | harness + multi-runner | every PR (cross-platform job) |
| Security | fail-closed verification, sandbox, traversal | targeted suites | every PR |
| End-to-end | real `vanta` flows, real network | gated job | nightly / pre-release |
| Performance | latency/throughput budgets | `xtask bench` | every PR (perf-gate) |
| Soak | repeated add/update/gc/rollback over time | scripted | pre-release |

Coverage target: ≥ 85% line coverage on `vanta-core` and the domain crates; **100% of `VTA-*` error codes exercised** (the code registry generates a test that fails if a code is never produced — [25. Error Catalog](25-error-and-exit-code-catalog.md)).

## Property tests

The subtle invariants get generative tests rather than hand-picked cases:

- **Version ordering is total and consistent** — for any generated version set + comparator, sorting is a total order; `latest`/prefix/range selection always picks the maximum satisfying version ([06. Resolution](06-resolution.md)).
- **Manifest↔lock reconcile** — round-trips: a manifest resolved to a lock and re-read yields the same resolution; `add`/`remove`/`update` mutate both consistently ([11](11-reproducibility.md), [31](31-lockfile-and-manifest-reference.md)).
- **Store canonicalization** — the same logical content (paths/modes permuted, timestamps varied) always hashes to the same store key; different content never collides in tests ([09. Store](09-store.md#hashing-and-canonicalization)).
- **Generation/rollback invariants** — for any sequence of mutations, `current` always points at a valid generation, rollback is its inverse, and GC never deletes a reachable entry ([12](12-updates.md)).
- **TOML round-trip** — parse→serialize→parse is identity for canonical forms; diffs are minimal.

## Fuzzing

Every component that consumes untrusted bytes is fuzzed:

| Target | Why |
| --- | --- |
| `vanta.toml` / `config.toml` parser | hostile/malformed manifests must never panic |
| `vanta.lock` parser | tampered locks must fail cleanly, not crash |
| version-request parser | adversarial version strings |
| provider-manifest parser | untrusted community providers |
| registry-index parser | hostile registry responses |
| **archive extractor** (tar/zip/gz/xz/zstd/bzip2) | **path traversal / zip-slip / decompression bombs** — the highest-risk surface ([08. Installation](08-installation.md), [21. Threat Model](21-threat-model.md)) |

Fuzzers use `arbitrary` for structure-aware inputs, run a smoke pass per PR and continuously on a nightly fuzzing host, and every crash is committed as a regression seed. The archive extractor is fuzzed with both random bytes and malicious-by-construction archives (entries escaping the root, symlink tricks, huge expansion ratios).

## The integration harness

`vanta-test` makes the full lifecycle runnable hermetically — no real network, deterministic, on every platform:

- **A fake registry** serving signed index/provider metadata from fixtures.
- **Fake providers** (declarative and a WASM sample) exercising resolution paths.
- **A local artifact HTTP server** serving fixture archives with controllable checksums/signatures, latency, range support, and failures (to test resume, mirror fallback, and verification).
- **A temp `$VANTA_HOME`** per test so the store/state/caches are isolated and disposable.

This lets a test do `vanta add tool@x` → assert store entry, lock contents, generation, and env view — fast and offline, which is what makes the suite runnable in CI on all three OSes.

## Reproducibility tests

The headline guarantee is tested directly: the same lock, materialized on different runners and operating systems, must produce **byte-identical store keys** for the same artifact. The cross-platform CI job builds a fixture environment on macOS, Linux (gnu and musl), and Windows runners and asserts key equality, plus that `vanta sync --frozen` is a no-op when already in sync and fails on injected drift ([11. Reproducibility](11-reproducibility.md)).

## Security tests

Adversarial by design ([15. Security](15-security.md), [21. Threat Model](21-threat-model.md)):

- **Fail-closed verification** — a tampered checksum or bad/missing signature aborts and quarantines; nothing reaches the store; the right `VTA-VRF-*` is produced.
- **Provider sandbox escape attempts** — a hostile WASM provider that tries filesystem/network/env access, infinite loops (fuel), or wall-time exhaustion (epoch) is contained and reported `VTA-PROV-0001`.
- **Path traversal / zip-slip** — archives crafted to write outside the staging dir are rejected.
- **Metadata rollback/freeze** — serving an old snapshot is detected by the snapshot/timestamp roles.
- **Trust gating** — env/tasks from an untrusted manifest do not execute; `--no-verify` is blocked under policy.
- **Secret redaction** — tokens never appear in logs, `--json`, or error output.

## End-to-end tests

A gated job (real network, nightly/pre-release) runs genuine flows on each OS: install Vanta, `vanta add node@X`/`python@Y`, `vanta x <tool>`, `vanta sync`, switch directories (activation), `vanta update`, `vanta rollback`, `vanta gc`. These catch upstream/registry reality that fakes cannot, but are not PR gates (network variance).

## Cross-platform matrix

CI runs unit/property/integration/reproducibility/security on: Linux x86_64 (gnu, musl), Linux aarch64, macOS aarch64, macOS x86_64, Windows x86_64 (plus a Windows-aarch64 and an Alpine/musl job), on Rust stable + MSRV ([03. Repository](03-repository.md)). Platform-specific behavior (links, quarantine removal, long paths, PATH mutation, per-shell hooks, shim launchers) has OS-targeted tests.

## Benchmarks and the perf-gate

`xtask bench` measures the budgeted operations against the targets in [16. Performance](16-performance.md) on fixed fixtures via the fake registry/artifact server (hermetic, network-independent):

- cold start, warm activation, shim dispatch, cold/warm `add`, `sync`.
- Reported as p50/p95/p99 distributions.
- A **CI perf-gate** runs on a reference runner per PR and **fails a regression beyond the configured threshold** on any budgeted operation, so performance cannot silently rot ([19. Milestones](19-milestones.md): from Phase 4).
- Cross-tool comparisons (vs mise/asdf/uv) run with published, fair methodology (same machine, cold/warm separated, no cherry-picking).

## Release gates

A release ships only when all of the following pass:

1. fmt + clippy (`-D warnings`) + `cargo-deny` + `cargo-audit`.
2. Unit + property + integration green on the full cross-platform matrix (stable + MSRV).
3. Reproducibility tests pass (identical keys across OSes).
4. Security suite green; fuzz smoke clean; no open critical fuzz crashes.
5. Coverage ≥ target; all `VTA-*` codes exercised.
6. Perf-gate within budget.
7. For 1.0: an external security audit's findings resolved and formats/ABI frozen.

## Cross-references

- [03. Repository](03-repository.md) — CI/CD, coverage targets, and the build matrix.
- [16. Performance](16-performance.md) — the budgets the perf-gate enforces.
- [15. Security](15-security.md) & [21. Threat Model](21-threat-model.md) — what the security suite asserts.
- [08. Installation](08-installation.md) & [09. Store](09-store.md) — extraction fuzzing and canonicalization tests.
- [11. Reproducibility](11-reproducibility.md) — the cross-platform reproducibility guarantee under test.
- [19. Milestones](19-milestones.md) — when each test class comes online.
