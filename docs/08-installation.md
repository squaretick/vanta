# 08. Installation Engine

> The engine that turns resolutions into a committed environment. This document specifies the six install stages (`[3 Plan]`..`[8 Commit]` of canon §4) in depth: planning against the store, parallel resumable fetching, the fail-closed verification gate, materialization (unpack or sandboxed build) with atomic publish, environment linking, and the transactional commit that produces a new generation. It also covers concurrency, partial-failure recovery, and post-install hooks. Owned by `vanta-install`.

**Contents**

- [Overview](#overview)
- [Stage 3 — Plan](#stage-3--plan)
- [Stage 4 — Fetch](#stage-4--fetch)
- [Stage 5 — Verify](#stage-5--verify)
- [Stage 6 — Materialize](#stage-6--materialize)
- [Stage 7 — Link](#stage-7--link)
- [Stage 8 — Commit](#stage-8--commit)
- [Transactions and atomicity](#transactions-and-atomicity)
- [Concurrency](#concurrency)
- [Post-install hooks](#post-install-hooks)
- [Failure scenarios](#failure-scenarios)
- [Cross-references](#cross-references)

---

## Overview

The install engine consumes a `Resolution` set (from `[2 Resolve]`, see [06. Resolution](06-resolution.md)) and produces a new generation. It is the orchestration crate that drives the domain crates; it owns no policy of its own beyond sequencing and transaction boundaries.

```
 Resolution set ─►[3 Plan]─►[4 Fetch]─►[5 Verify]─►[6 Materialize]─►[7 Link]─►[8 Commit]─► new generation
                     │          │           │             │             │          │
                  store_index  CAS      FAIL-CLOSED   store/.tmp→     envs/<id>   redb txn +
                  diff         downloads  (exit 6)    rename (RO)     staged view lock/manifest
                     │          │           │             │             │          │
                  ─ skip if all "have" ─►───────────────────────────────────────►─┘ (store hit fast path)
```

Two properties shape everything below: **the store is content-addressed** (so a needed entry that already exists is free — the fast path skips fetch/verify/materialize), and **the atomicity boundary is `[8 Commit]`** (so any failure before commit leaves the prior environment, manifest, and lock byte-for-byte intact). The store and link mechanics are specified in [09. Store](09-store.md); this document is the engine that uses them.

## Stage 3 — Plan

Planning converts resolutions into a concrete, minimal unit of work.

1. **Compute target store keys.** For each resolved `tool@version` on each target platform, derive the expected `StoreKey` (`blake3-<hex>`) from the resolution's recipe (provider id + version + artifact hash + layout). The key is known *before* fetching because the lock records the artifact hash.
2. **Diff against the store.** Query `state.db`'s `store_index` (and stat the store path) to partition keys into `have` (already materialized) and `need`.
3. **Build the work DAG.** Most tools are independent leaves. Where a provider declares dependencies (e.g. a CLI needs a JVM), order `need` topologically so a dependency materializes before its dependent's post-install. Conflicts were already rejected at resolution; planning assumes a consistent set.
4. **Short-circuit.** If `need` is empty, jump straight to `[7 Link]` — this is the common `vanta sync` and repeated-`add` case and is dominated by a redb read plus cheap links.

The plan is reported to the user (`+ node 24.6.0`, `= python 3.13.4 (cached)`) and, with `--dry-run`, printed without executing.

## Stage 4 — Fetch

Fetching downloads artifacts into the **content-addressed download cache** (`cache/downloads/`), never directly into the store. Owned by `vanta-net` writing through `vanta-store`'s cache.

- **Parallel.** Artifacts download concurrently, bounded by a global semaphore (default `jobs = min(num_cpus, 8)`, overridable via `[settings] jobs` or `--jobs`). Per-host connection limits prevent hammering a single origin.
- **Resumable.** Downloads use HTTP range requests; a partial blob from a previous run resumes from its byte offset. A killed `vanta` never forces a full re-download.
- **Mirror-aware.** The fetch plan carries the primary URL plus configured mirrors ([13. Offline](13-offline.md)); on failure the engine falls back in priority order before erroring.
- **Retry/backoff.** Transient failures (timeouts, 5xx, connection resets) retry with capped exponential backoff and jitter; a permanent failure (404, exhausted mirrors) ends the stage with `VTA-NET-*` (exit 5).
- **Streaming + bounded memory.** Bytes stream to disk and are hashed incrementally; the whole artifact is never held in RAM ([16. Performance](16-performance.md)).
- **Offline.** With `--offline`/`[settings] offline`, fetch is skipped for anything already cached; a cache miss is a clean `VTA-NET-0002` ("offline and not cached").

The cache is itself content-addressed (keyed by the published checksum), so two projects needing the same artifact download it once, and a re-fetch of identical bytes is a no-op.

## Stage 5 — Verify

Verification is the **security gate** and is **fail-closed**: nothing proceeds to materialization until it passes. Owned by `vanta-security` (full model in [15. Security](15-security.md)).

For each fetched artifact:

1. **Checksum.** Compute SHA-256 (to match the upstream-published checksum) and BLAKE3 (internal); compare to the values pinned in `vanta.lock` / the signed registry metadata. Any mismatch ⇒ abort the whole command (`VTA-VRF-0001`, exit 6) and **quarantine** the offending cache blob (move aside, do not serve).
2. **Signature.** Verify the artifact's signature (minisign/Ed25519 for the official registry; cosign/sigstore where the publisher provides it) against the pinned trust keys (`~/.vanta/trust/`). A missing-but-required or invalid signature ⇒ `VTA-VRF-0002`.
3. **Provenance.** Where SLSA provenance is available, verify it meets the configured level. `VTA-VRF-0004` on failure.
4. **Trust.** A third-party registry/provider not yet trusted ⇒ `VTA-VRF-0003` (exit 9), prompting `vanta trust`.

`--no-verify` downgrades verification to a loud warning and can be **forbidden by org policy**; it never silently disables checks. Because verification happens before materialization, a corrupted or tampered artifact is always a clean failure, never a poisoned store.

## Stage 6 — Materialize

Materialization turns verified bytes into an immutable store entry. Owned by `vanta-store`.

- **Unpack (the common path).** Detect and extract the archive (`tar`, `zip`, `gzip`, `xz`, `zstd`, `bzip2`) per the provider's layout (`strip` components, subdir, bin paths). Extraction is hardened against path traversal / zip-slip (every entry path is validated to stay within the staging dir — a fuzzed surface, see [28. Testing](28-testing.md)).
- **Build (the exception).** When a provider declares a source build (no prebuilt for the platform), the build runs in a **sandbox**: a restricted filesystem view and **network off after fetch** (all inputs were fetched and verified in `[4]`/`[5]`). Prebuilt-binary-first is policy ([ADR-0022](24-architecture-decision-records.md)); source builds are opt-in and can be forbidden by enterprise policy.
- **Canonicalize.** Normalize the tree for hashing (sorted paths, normalized modes, timestamps stripped) so the same content hashes identically on every machine/OS ([09. Store](09-store.md#hashing-and-canonicalization)).
- **Atomic publish.** Stage into `store/.tmp-<rand>/`, fsync, then `rename` onto `store/blake3-<hex>/`, and mark the entry read-only. A crash leaves only a discardable temp dir; a concurrent producer of the same key converges idempotently (single-flight + identical bytes).

After publish, `store_index` will be updated as part of the commit transaction (not here — materialize produces bytes, commit records them).

## Stage 7 — Link

Linking composes the **environment view** — the per-environment `envs/<env-id>/bin` directory that will go on `PATH`. Owned by `vanta-env` using `vanta-platform` link primitives.

- For each tool in the target environment, create links from the env's bin dir to the tool's executables in the store, using the cheapest mechanism the filesystem/OS supports: **reflink → hardlink → symlink → copy** ([09. Store](09-store.md#link-strategies), [17. Cross-platform](17-cross-platform.md)).
- Linking is done into a **staged** view (`envs/.tmp-<rand>/`) so the live environment is untouched until commit.
- This stage is cheap (a handful of links), which is why activation and environment switching are fast and why many tool versions can coexist without disk cost.

## Stage 8 — Commit

Commit is the **only** stage that makes a mutation visible, and it is atomic. It writes the generation record and the lock/manifest, then swaps pointers.

```
[8 Commit]   (ATOMICITY BOUNDARY)
  1. begin redb write transaction
  2. insert generations[new]; update store_index, tool_index      (current still = old)
  3. write vanta.lock.tmp (+ vanta.toml.tmp for `add`/`remove`) → fsync → rename onto final paths
  4. set gen_current[env] = new ; COMMIT redb txn (fsync)         ◄── linearization point
  5. rename envs/.tmp-<rand> → envs/<env-id> ; update on-disk `current` mirror
```

- The redb commit (step 4) is the linearization point: before it, a concurrent reader sees the old generation; after it, the new one. Steps 3 and 5 are made crash-safe by temp-then-rename and are reconciled on next start if interrupted ([transactions](#transactions-and-atomicity)).
- For `add`/`remove`, the manifest edit is part of the same temp-then-rename batch so the manifest and lock never disagree on success.
- The new generation references the materialized store keys; rollback to any prior generation is a later pointer swap that installs nothing ([12. Updates & Rollback](12-updates.md)).

## Transactions and atomicity

The engine treats an install as all-or-nothing up to the commit barrier, built on four primitives (shared with [02. Architecture](02-architecture.md#failure-and-atomicity-model)):

1. **Temp-then-rename** for every durable write (store entries, lock, manifest, env view, pointer).
2. **Fail-closed verification** so unverified bytes never reach the store.
3. **The commit barrier** at `[8]`, with the redb transaction as the linearization point.
4. **Reconciliation on next start.** Each invocation runs a fast pass that sweeps orphan `store/.tmp-*` and `envs/.tmp-*` dirs, and detects a lock/manifest written but redb `current` not advanced (crash between steps 3 and 4) — re-committing idempotently (same inputs → same content) or offering to revert.

Idempotence: re-running the same `vanta add`/`sync` after any failure converges on the same result, because store keys are content-derived and the plan re-diffs against what already exists.

## Concurrency

Two `vanta` processes can run at once (two terminals, a hook firing during a manual `add`, CI parallelism). Correctness comes from content-addressing plus locking ([23. Data & State Model](23-data-and-state-model.md#concurrency-and-locking)):

- **Single-flight per store key.** Before materializing `blake3-X`, a process takes an advisory per-key lock; losers wait and then observe the published entry. Because the key is the content hash, who wins is irrelevant.
- **Atomic-rename safety net.** Even without the lock, two producers of key `X` stage into distinct temp dirs and rename onto the same path; the bytes are identical, so the outcome is correct regardless of ordering. The lock is an optimization (avoid duplicate downloads), not a correctness requirement.
- **Global lock for pointer-class steps.** The generation append, `current` swap, and GC sweep take a short-held global advisory lock; redb's single-writer transaction serializes the record write. Lock waits time out (30 s global, 120 s per-key) with a `VTA-STORE-*` error naming the holder's pid.

Within a process, fetch/verify/unpack run in parallel under the `jobs` semaphore, with CPU-bound unpack/hash on a blocking pool so the async reactor is never stalled.

## Post-install hooks

Some tools need a post-install step (e.g. generating a default config, compiling bytecode caches). Providers may declare a `post_install` hook, which runs:

- **Sandboxed**, with the same capability restrictions as provider WASM hooks ([22. Provider SDK](22-provider-sdk.md)) — no ambient network, a scoped filesystem view limited to the new store entry and a scratch dir.
- **Deterministically** where it affects store content (its output is part of the materialized, hashed entry); non-deterministic side effects are disallowed.
- Hooks can be globally disabled (`[settings] run_hooks = false`) or forbidden by policy.

This is the controlled replacement for the arbitrary `postinstall`/`PKGBUILD`/PowerShell scripts that other ecosystems run with full user privilege ([33. Prior Art](33-prior-art.md)).

## Failure scenarios

| Failure | Stage | Code (exit) | Behavior |
| --- | --- | --- | --- |
| Manifest/request invalid | pre-`[3]` | `VTA-CFG-*` (3) | nothing touched |
| No artifact for the platform | `[3]` | `VTA-RES-0005` (4) | plan aborts; prior env intact |
| Download fails / mirrors exhausted | `[4]` | `VTA-NET-0001` (5) | partial blob kept for resume; store untouched |
| Offline and not cached | `[4]` | `VTA-NET-0002` (5) | clean offline failure |
| Checksum mismatch | `[5]` | `VTA-VRF-0001` (6) | blob quarantined; command aborts |
| Signature invalid/missing | `[5]` | `VTA-VRF-0002` (6) | command aborts |
| Untrusted registry/provider | `[5]` | `VTA-VRF-0003` (9) | prompts `vanta trust` |
| Archive corrupt / zip-slip | `[6]` | `VTA-INST-0001` (7) | only a discardable temp dir exists |
| Sandboxed build failed | `[6]` | `VTA-INST-0002` (7) | temp discarded; store untouched |
| Disk full / IO error | `[6]`/`[8]` | `VTA-STORE-0002` (7) | temp discarded; commit not reached |
| Lock/manifest write race | `[8]` | `VTA-LOCK-0001` (7) | reconciled on next run; atomic either way |

In every row before `[8 Commit]`, the user's working environment is unchanged. The strongest guarantee: because `vanta.lock` is in VCS and the store is content-addressed, the correct environment is always reachable via `vanta sync` even from an empty `~/.vanta`.

## Cross-references

- [02. Architecture](02-architecture.md) — the full lifecycle and the store-centric model this engine drives.
- [06. Resolution](06-resolution.md) — where the `Resolution` set this engine consumes comes from.
- [09. Store](09-store.md) — content addressing, atomic publish, link strategies, and GC.
- [15. Security](15-security.md) — the `[5 Verify]` gate, signatures, provenance, and sandboxing.
- [12. Updates & Rollback](12-updates.md) — generations and rolling back a bad install.
- [16. Performance](16-performance.md) — fetch/unpack parallelism, the `jobs` default, and the store fast path.
- [23. Data & State Model](23-data-and-state-model.md) — the redb transaction and the locking protocol.
