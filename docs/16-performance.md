# 16. Performance

> Performance is a measured property with explicit budgets, not a vague aspiration. This document states the latency and throughput targets, the techniques that achieve them (no daemon, lazy init, the per-directory resolution cache, content-addressed dedup, reflink/hardlink materialization, parallel IO), the memory and disk strategies, and the benchmark methodology and CI perf-gate that keep them honest.

**Contents**

- [Targets](#targets)
- [Startup optimization](#startup-optimization)
- [Activation latency](#activation-latency)
- [Install performance](#install-performance)
- [Memory](#memory)
- [Disk](#disk)
- [Benchmark methodology](#benchmark-methodology)
- [Trade-offs](#trade-offs)
- [Cross-references](#cross-references)

---

## Targets

These are the budgets the build is held to; the CI perf-gate ([benchmark methodology](#benchmark-methodology)) fails a PR that regresses them.

| Operation | Target | Why achievable |
| --- | --- | --- |
| `vanta` cold start to first output (e.g. `vanta --version`, `vanta which`) | **< 5 ms** | no daemon, single static binary, lazy init, minimal parse |
| Warm per-directory activation (hook on `cd`, env unchanged or cached) | **< 1 ms** | a single redb keyed read + `PATH` string swap; no subprocess |
| Shim dispatch overhead (`vanta-shim` → real binary) | **< 1 ms** | tiny binary, cached resolution, one `execve` |
| `vanta sync` with a warm store (nothing to fetch) | **near-instant** (tens of ms) | store hits + cheap links; no network, no unpack |
| `vanta add <tool>` cold (one prebuilt artifact) | **download-bound** | parallel ranged fetch saturates the link; unpack overlaps |
| Re-`add`/re-`sync` of an installed tool | **< 10 ms** | content-addressed store hit; fetch/verify/materialize skipped |
| `vanta x <cached-tool>` | **shim-class** | already in the store; just resolve-from-cache + exec |

The two budgets that matter most for daily feel are **cold start** and **activation**, because they sit on the `cd`/prompt and shim hot paths; everything else is dominated by the network or by work the user explicitly asked for.

## Startup optimization

`vanta` must be cheap to invoke because shells, hooks, and shims invoke it constantly.

- **No daemon** ([ADR-0012](24-architecture-decision-records.md)). There is no resident process to attach to, no socket, no cache-coherence protocol. The cost model is "open redb + read what you need," which is faster and simpler than IPC for these access patterns ([02. Architecture](02-architecture.md#why-no-daemon)).
- **Lazy subsystem init.** Nothing constructs the resolver, the WASM host, or the HTTP stack unless a command needs them. `vanta which`/`--version` touch almost nothing.
- **mmap'd state.** redb is memory-mapped; reads are page-cache-backed pointer chases, not parse-the-world.
- **No full config/registry parse on the hot path.** Activation and shims read the **per-directory resolution cache** (a single keyed lookup), not the manifest+registry. A full resolve happens only on `add`/`update`/`--refresh`.
- **Static binary, minimal deps for the hot binaries.** `vanta-shim` is a separate crate with no resolver/HTTP/WASM dependencies precisely so its cold start is negligible ([03. Repository](03-repository.md)).

## Activation latency

Per-directory auto-switching is the single most latency-sensitive feature because it runs on every prompt in a hooked shell ([10. Environments](10-environments.md#automatic-version-switching)):

- The shell hook computes the merged-config hash and does **one redb read** of `resolution_cache[config_hash]`. On a hit (the overwhelming common case), it compares the cached `env-id` to the active one and, if different, swaps a `PATH` segment — **sub-millisecond, no subprocess**.
- A cache miss spawns `vanta` once to resolve (lock-authoritative, no network) and populate the cache; subsequent entries are warm.
- The cache is invalidated only by a manifest change (the key is the config hash), so it is never stale and never needs a TTL.
- Shims read the *same* cache, so an IDE/cron invocation pays the same sub-ms lookup plus one `execve`.

This is why Vanta matches mise's PATH-injection speed while also covering asdf's shim contexts — without either one's downside ([33. Prior Art](33-prior-art.md)).

## Install performance

- **Parallel, resumable fetch.** Downloads run concurrently under `jobs = min(num_cpus, 8)` (tunable), each using HTTP range requests so a retry resumes rather than restarts ([08. Installation](08-installation.md#stage-4--fetch)).
- **Content-addressed dedup avoids work.** A needed artifact already in the store or download cache is skipped entirely — the dominant speedup for `sync` and for teams sharing tools ([09. Store](09-store.md#deduplication)). Re-installs are near-free.
- **Reflink/hardlink materialization.** Composing an environment links into the store rather than copying; on CoW/hardlink-capable filesystems this is near-zero-cost regardless of tool size ([09. Store](09-store.md#link-strategies)).
- **Overlapped, parallel extraction.** Verified archives are decompressed on a blocking/rayon pool while other downloads continue; the async reactor is never blocked on CPU-bound unpack or hashing.
- **BLAKE3** keeps the mandatory hashing cheap — verification's cost is small relative to IO, so "secure by default" does not cost meaningful latency.

Intuition vs incumbents: asdf pays per-exec shim and often serial, shell-driven installs; Homebrew pays Ruby startup and git operations. Vanta's Rust binary, parallel IO, dedup, and link-based materialization put it in uv's performance class, generalized to all tools.

## Memory

- **Streaming everywhere.** Downloads and extraction stream with bounded buffers; a multi-hundred-MB artifact is never loaded whole into RAM.
- **Small RSS.** A typical `add`/`sync` holds a few buffers plus the mmap'd index; resident memory is tens of MB, not gigabytes.
- **Bounded concurrency** caps peak memory as a function of `jobs`, not of the number of tools.
- The WASM host (only loaded for providers with hooks) bounds guest memory explicitly ([22. Provider SDK](22-provider-sdk.md)).

## Disk

- **Dedup** means disk grows with *distinct tool versions*, not project count ([09. Store](09-store.md#deduplication)); `vanta store stats` reports logical vs physical (link-aware) usage and the dedup ratio.
- **Reflink/hardlink** views cost only directory entries, so many environments and generations are cheap.
- **GC** reclaims unreferenced entries under a tunable retention policy, trading disk for rollback depth ([12. Updates & Rollback](12-updates.md)).

## Benchmark methodology

Performance claims are reproducible and not cherry-picked:

- **A versioned benchmark harness** (`xtask bench`) measures the budgeted operations (cold start, warm activation, shim dispatch, cold/warm `add`, `sync`) on fixed fixtures using a fake local registry/artifact server so runs are hermetic and network-independent ([28. Testing](28-testing.md)).
- **Reported as distributions** (p50/p95/p99), not single numbers, because tail latency on the `cd` path is what users feel.
- **Fair comparison rules** for cross-tool benchmarks (vs mise/asdf/uv where comparable): same machine, same tools, cold and warm cache reported separately, methodology published, no selective wins.
- **A CI perf-gate** runs the harness on every PR on a reference runner and **fails a regression beyond a set threshold** on any budgeted operation, so performance cannot silently rot.
- Real-world e2e timings (with network) run in a nightly job and are tracked over time but are not gates (network variance).

## Trade-offs

- **Verification vs speed.** Mandatory checksum+signature adds work, kept small by BLAKE3 and by overlapping it with IO; the safety is non-negotiable, so the cost is optimized, not removed.
- **Store indirection vs direct installs.** The content-addressed store adds a link indirection; reflink/hardlink make it free at runtime and `vanta which` keeps it transparent — the dedup/atomicity/rollback payoff is worth it ([09. Store](09-store.md#trade-offs-and-alternatives)).
- **No daemon vs amortized warm state.** A daemon could cache more in memory, but at the cost of coherence, security surface, and cross-platform complexity; on-disk caches (redb + CAS) deliver the warm-path speed without those costs ([02. Architecture](02-architecture.md#why-no-daemon)).

## Cross-references

- [02. Architecture](02-architecture.md) — the no-daemon model and the store fast path.
- [10. Environments](10-environments.md) — the resolution cache behind sub-ms activation and shim dispatch.
- [08. Installation](08-installation.md) — parallel fetch, the `jobs` default, and the store-hit short-circuit.
- [09. Store](09-store.md) — dedup and reflink/hardlink materialization.
- [28. Testing](28-testing.md) — the benchmark harness and the CI perf-gate.
- [03. Repository](03-repository.md) — the minimal `vanta-shim` dependency tree for fast start.
