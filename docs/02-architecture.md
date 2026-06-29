# 02. Architecture

> Vanta is a single statically-linked binary that runs as a short-lived CLI process: no daemon, no resident state, no background services for core operation. This document is the architectural spine. It defines the subsystem map, the content-addressed store-centric model that is the system's keystone, the eight-stage resolution lifecycle that every mutating command flows through, the process and concurrency model, the failure and atomicity guarantees, and the extension seams (core traits plus WASM providers) that keep the design open without compromising those guarantees.

**Contents**

- [System overview](#system-overview)
- [Subsystem map](#subsystem-map)
- [Anatomy of a vanta add](#anatomy-of-a-vanta-add)
- [The store-centric model](#the-store-centric-model)
- [The resolution lifecycle](#the-resolution-lifecycle)
- [Process and concurrency model](#process-and-concurrency-model)
- [State, caches and persistence](#state-caches-and-persistence)
- [Generations and rollback](#generations-and-rollback)
- [Activation](#activation)
- [Failure and atomicity model](#failure-and-atomicity-model)
- [Extension seams and dependency injection](#extension-seams-and-dependency-injection)
- [Internal API stability tiers](#internal-api-stability-tiers)
- [Cross-references](#cross-references)

---

## System overview

Vanta is one binary (`vanta`, aliased `vt`) that is invoked, does one unit of work, and exits.
There is **no daemon** in the core architecture. Every command — `vanta add`, `vanta sync`,
`vanta run`, the shell-hook fast path, the `vanta-shim` dispatcher — is a process that starts
cold, reads the minimal state it needs, performs IO-bound work in parallel on a Tokio runtime,
commits atomically, and terminates.

The architecture is organized around a small number of invariants that hold for every command:

1. **The store is the source of truth for bytes.** Everything Vanta installs is an immutable,
   content-addressed entry under `~/.vanta/store/`. Entries are never mutated in place.
2. **The environment is a cheap composed view**, not a copy. Activating a tool is composing a
   `PATH` view over store entries, not installing into a shared prefix.
3. **Every mutation is a new generation.** State moves forward by appending an immutable
   generation record and swapping a pointer. Rollback is a pointer swap, never a re-install.
4. **The atomicity boundary is `[8 Commit]`.** Any failure before commit leaves the prior
   generation, manifest, and lockfile byte-for-byte untouched.
5. **Verification is fail-closed.** Bytes do not enter the store until checksum and signature
   gates pass; `--no-verify` is the only escape and it warns loudly.

### Why no daemon

A resident daemon was considered and rejected as the *default* model. The trade-off:

| Concern | Daemon model | Short-lived CLI (chosen) |
| --- | --- | --- |
| Cold start | Amortized after first start | Must be < 5 ms every invocation |
| Crash blast radius | Daemon crash stalls all clients; restart/supervision needed | A crash affects one command; the next invocation is clean |
| State coherence | In-memory caches can diverge from disk; needs invalidation protocol | Disk (redb + store) is the only truth; read fresh each time |
| Security surface | Long-lived listening process, privilege questions, IPC attack surface | No listening socket, no ambient process |
| Cross-platform | Service management differs sharply (systemd / launchd / Windows services) | Identical model everywhere |
| Multi-user / CI | Per-user daemons, socket permissions, container lifetimes | Trivially correct; nothing to start |

The premise that pushes other tools toward daemons — *avoiding repeated work* — is solved here by
on-disk caches instead of memory: the redb resolution cache makes warm activation sub-millisecond
(see [Activation](#activation)) and the CAS download cache makes re-fetches free. We pay a few
hundred microseconds to open redb and `mmap` the relevant tables; we do not pay daemon complexity.

The **one** exception is explicitly opt-in: a background file-watcher for auto-`sync` on manifest
changes. It is never required for correctness and is off by default. Core operation needs no
resident process. This decision is recorded as an ADR; see
[24. ADRs](24-architecture-decision-records.md).

## Subsystem map

The crate workspace (canon §14) maps one-to-one onto subsystems. `vanta-core` and
`vanta-platform` are leaves with no internal dependencies; the `vanta` binary depends on
everything; there are no cycles.

```
                                  +------------------------------------------+
   user / shell / IDE  ─────────► |  vanta  (bin)  ·  vt  ·  supervisor       |
                                  |  vanta-cli  (command impls, testable lib) |
                                  +---------------------+--------------------+
                                                        │
        ┌───────────────────────────────────────────────┼───────────────────────────────────────────────┐
        ▼                       ▼                         ▼                        ▼                        ▼
 +-------------+        +---------------+         +----------------+       +----------------+      +----------------+
 | vanta-config|        | vanta-resolve |         |  vanta-install |       |   vanta-env    |      |   vanta-diag   |
 | manifest +  |        | request ->    │◄──────► |  plan->fetch-> │◄────► | compose view + │      | doctor checks  |
 | config model|        | resolution    │         │ verify->mat->  │       | activation +   │      | error registry │
 | precedence  |        | version order │         │ link->commit   │       | shell hooks    │      +--------+-------+
 +------+------+        | dep DAG       │         | transactions/  │       +-------+--------+               │
        │               +---+-------+---+         | generations    │               │                       │
        │                   │       │             +---+--------+---+               │                       │
        ▼                   ▼       ▼                 │        │                   ▼                       ▼
 +-------------+     +-----------+ +-----------+       │        │           +-------------+        +----------------+
 | vanta-lock  |     |vanta-     | |vanta-     │       │        │           | vanta-shim  |        |  vanta-migrate │
 | lock model  |     |registry   | |provider   │       │        │           | dispatcher  │        |  importers     │
 | reconcile   |     |index+meta | |+ wasmtime │◄──────┘        │           | (separate   │        +----------------+
 +-------------+     |cache/srch | |host (WIT) │                │           |  tiny bin)  │
                     +-----+-----+ +-----+-----+                │           +-------------+
                           │             │                      │
                           ▼             ▼                      ▼
                     +-----------+ +-----------+         +----------------+       +----------------+
                     | vanta-net | | vanta-    |         |   vanta-store  |◄─────►|   vanta-state  |
                     | rustls    | | security  |         | CAS, atomic    |       | redb: index,   |
                     | parallel  | | checksum, |         | publish, links,│       | gc roots, gens,│
                     | resumable | | sig, trust│         | dedup, GC,     |       | resolution &   |
                     | mirrors   | | sandbox   |         | integrity      │       | registry cache │
                     +-----+-----+ +-----+-----+         +-------+--------+       +-------+--------+
                           │             │                       │                       │
                           └─────────────┴───────────────────────┴───────────────────────┘
                                                  │
                                          +---------------+
                                          | vanta-platform|
                                          | OS/arch, paths|
                                          | links, shells |
                                          | exe handling  |
                                          +---------------+
```

Responsibilities (authoritative crate-to-job mapping):

| Subsystem (crate) | Owns | Key inputs → outputs |
| --- | --- | --- |
| `vanta-core` | Vocabulary types (`Request`, `Resolution`, `Artifact`, `StoreKey`, `Generation`), core traits, error taxonomy | — (leaf) |
| `vanta-platform` | OS/arch detection, path math, link primitives, shell + executable handling | — (leaf) |
| `vanta-config` | `vanta.toml` + `config.toml` model, parsing, precedence/merge, span diagnostics | files → typed config |
| `vanta-lock` | `vanta.lock` model, manifest↔lock reconcile | manifest+resolution → lock |
| `vanta-resolve` | Request → Resolution; version ordering; constraints; dependency DAG | request+registry → resolution(s) |
| `vanta-registry` | Registry index, search, metadata, caching | query → provider id + index |
| `vanta-provider` | Provider model, built-in providers, Wasmtime host for WASM hooks | version → artifact descriptors |
| `vanta-net` | HTTP (rustls), parallel/resumable downloads, mirrors, retries, auth | fetch plan → bytes in CAS |
| `vanta-security` | Checksums, signatures, trust DB, provenance, sandbox policy | artifact → verified / rejected |
| `vanta-store` | Content-addressed store, atomic publish, links, dedup, integrity, layout | bytes → store entry |
| `vanta-install` | Install engine: plan → fetch → verify → materialize → link → commit; transactions & generations | resolution → new generation |
| `vanta-env` | Environment composition, activation logic, shell-hook generation | generation → PATH view |
| `vanta-state` | redb state DB: store index, gc roots, generation history, resolution/registry caches | reads/writes persistent state |
| `vanta-diag` | `doctor` checks, diagnostics rendering, error-code registry | system → diagnosis |
| `vanta-cli` | Command implementations (library behind the `vanta` bin) | argv → orchestration |
| `vanta-shim` | The shim dispatcher binary | argv+cwd → exec real binary |
| `vanta-sdk` | Provider-author SDK (guest side, WASM) | (consumed by providers) |
| `vanta-migrate` | Importers (mise/asdf/nvm/pyenv/brew/scoop/pkgx/...) | foreign config → `vanta.toml` |

## Anatomy of a vanta add

This is the canonical data flow. The example is `vanta add node@24` invoked inside a project
directory (scope inferred to **project**). Stage labels are the normative names from canon §4.

```
$ vanta add node@24
 │
 ▼
[1 Request]  vanta-cli + vanta-config + vanta-core
   parse "node@24" → Request{ name:node, ver:^24, scope:Project(<root>), targets:[host, +locked platforms] }
   load nearest vanta.toml (+ merged config). Untrusted-config gate if it injects env/tasks.
 │
 ▼
[2 Resolve]  vanta-resolve  ⟵ vanta-registry ⟵ vanta-provider (⟵ vanta-net for cold metadata)
   registry lookup: node → provider "official/node"
   provider.versions() → ordering → pick exact 24.6.0
   provider.artifacts(24.6.0) → per-platform ArtifactDesc{ url(s), sha256, sig, layout, bin, env }
   resolve declared deps (usually none) → DAG → Resolution set
 │
 ▼
[3 Plan]  vanta-install  ⟵ vanta-state (store_index)
   compute target StoreKeys; diff against store_index
   PLAN = { need: [blake3-… (node 24.6.0)], have: [...] }   ← if "have" is complete, jump to [7]
 │
 ▼
[4 Fetch]  vanta-net → vanta-store (cache/downloads, CAS)         ┐ parallel across artifacts
   range/resume, mirror-aware, retry w/ backoff; bytes land in     │ single-flight per content hash
   cache/downloads/<sha256|blake3>  (pre-materialize CAS)          ┘
 │
 ▼
[5 Verify]  vanta-security                                          (FAIL CLOSED → exit 6, VTA-VRF-*)
   sha256 == locked/published?  signature (minisign/cosign) valid against trusted keys?
   provenance/SLSA gate where present. Reject ⇒ nothing enters the store.
 │
 ▼
[6 Materialize]  vanta-store (⟵ vanta-provider layout, archive crates)
   unpack/build into store/.tmp-<rand>/ → fsync → atomic rename → store/blake3-<hex>/ (read-only)
 │
 ▼
[7 Link]  vanta-env  ⟵ vanta-platform (reflink→hardlink→symlink→copy)
   compose envs/<env-id>/bin view over store entries; build per-tool bin links
 │
 ▼
[8 Commit]  vanta-install + vanta-state + vanta-lock     ◄── ATOMICITY BOUNDARY
   redb write txn: append generation record, update store_index/tool_index, set current pointer
   write vanta.lock (+ vanta.toml [tools] edit) via temp→fsync→rename
 │
 ▼
[Activate]  vanta-env (hook) / vanta-shim (shims)
   shell hook swaps PATH to envs/<env-id>/bin on next prompt; shims resolve cwd→version on exec
```

The dashed jump from `[3 Plan]` to `[7 Link]` is the common case for an already-installed tool:
the store entry exists, so fetch/verify/materialize are skipped and the command is dominated by a
redb read plus a cheap link/commit. This is why repeated `add`/`sync` is fast.

## The store-centric model

The store-centric model is the architectural keystone; almost every guarantee in the pillar set
(canon §2) is a consequence of it.

```
   REQUEST                 RESOLUTION                STORE (immutable, CAS)         ENVIRONMENT (cheap view)
  node@24      ──►   node 24.6.0 / official  ──►   store/blake3-aa…/   (node 24.6.0)   envs/<id>/bin/node ─┐
  python@3.13  ──►   python 3.13.4 / official ─►   store/blake3-bb…/   (python 3.13.4) envs/<id>/bin/python │ PATH
  terraform    ──►   terraform 1.9.5 / direct ─►   store/blake3-cc…/   (terraform 1.9) envs/<id>/bin/terraform ┘
                                                          ▲
                                                          │  shared across projects & generations (dedup)
                                          generations/  ──┘  gen N → {node:aa, python:bb, terraform:cc}
                                          current ───────────► gen N    (pointer swap = rollback)
```

The three layers and why each property falls out:

- **Store entry** — an immutable directory keyed by `blake3-<hex>` holding one materialized
  `tool@version`. Because the key is the content hash, two requests that resolve to identical
  bytes share one entry. This is the source of **deduplication**, **integrity** (the path *is*
  the checksum), and **safe concurrent install** (two processes producing the same key converge
  on the same path; the rename is idempotent).
- **Environment** — a directory of links (`envs/<env-id>/`) composing store entries onto a single
  `PATH` root. It is a *view*, costing a handful of links, not a copy. Switching environments is
  switching which view is on `PATH`. This is the source of **cheap activation** and the ability
  to have **many versions coexist**.
- **Generation** — an immutable record mapping an environment to a set of store keys at a point in
  time, plus the manifest/lock hashes that produced it. The `current` pointer selects the active
  generation. This is the source of **atomicity** (commit = append + pointer swap), **instant
  rollback** (flip the pointer), and **reproducibility** (a generation is a complete, hashable
  description of an environment).

Compare the alternative most version managers use — installing into mutable per-tool prefixes
(`~/.asdf/installs/<tool>/<version>`). That model cannot dedupe identical files, cannot offer a
content-integrity guarantee, makes "the set of tools active right now" implicit and unsnapshot­able,
and turns rollback into re-installation. The store-centric model buys Nix-grade safety without the
Nix language; the cost is an indirection (entries live in the store, the environment links to
them) and a need for garbage collection, both detailed in [09. Store](09-store.md).

## The resolution lifecycle

Every mutating command (`add`, `remove`, `update`, `sync`, `rollback`, `restore`) is expressed as
a traversal of the canon §4 lifecycle. Read-only commands (`which`, `list`, `info`) execute a
prefix and stop. The table fixes ownership, IO, and failure behavior per stage.

| Stage | Owning crate(s) | Input → Output | May fail with | State on failure |
| --- | --- | --- | --- | --- |
| `[1 Request]` | `vanta-cli`, `vanta-config`, `vanta-core` | argv/manifest → `Request` (+ scope, targets) | `VTA-CFG-*` (bad manifest), exit 2 (usage), `VTA-*` trust gate (exit 9) | nothing touched |
| `[2 Resolve]` | `vanta-resolve` ← `vanta-registry`, `vanta-provider` | `Request` → `Resolution` set (exact ver + provider + per-platform `ArtifactDesc` + deps) | `VTA-RES-*` (unsatisfiable/conflict, exit 4), `VTA-REG-*`, `VTA-PROV-*`, `VTA-NET-*` (cold meta) | nothing touched |
| `[3 Plan]` | `vanta-install` ← `vanta-state` | `Resolution` → install plan (`need`/`have` store keys) | `VTA-STORE-*`, `VTA-INT-*` | nothing touched |
| `[4 Fetch]` | `vanta-net` → `vanta-store` (CAS downloads) | `ArtifactDesc` → bytes in `cache/downloads/` | `VTA-NET-*` (exit 5) | partial downloads kept for resume; store untouched |
| `[5 Verify]` | `vanta-security` | bytes → verified bytes | `VTA-VRF-*` (exit 6) | rejected bytes never enter store; cache entry quarantined |
| `[6 Materialize]` | `vanta-store` ← `vanta-provider`, archive crates | verified bytes → `store/blake3-<hex>/` | `VTA-STORE-*`, `VTA-INST-*` (exit 7) | only a discardable `store/.tmp-<rand>/` exists |
| `[7 Link]` | `vanta-env` ← `vanta-platform` | store entries → `envs/<env-id>/` staged view | `VTA-ENV-*`, `VTA-STORE-*` | staged view in temp; live env untouched |
| `[8 Commit]` | `vanta-install`, `vanta-state`, `vanta-lock` | staged view → new generation + lock/manifest | `VTA-LOCK-*`, `VTA-STORE-*` | **atomic**: either fully applied or fully not |
| `[Activate]` | `vanta-env` (hook) / `vanta-shim` (shims) | generation → `PATH` exposure | `VTA-ENV-*` | prior PATH/shims still valid |

Stage notes:

- **`[2 Resolve]`** is the only stage that consults the registry/providers and is therefore the
  one that re-runs only on `add`/`update`/`--refresh`. Otherwise `vanta.lock` is authoritative and
  resolution is read from it (canon §10). The resolver builds a DAG, applies provider-declared
  version ordering, and produces a *total, deterministic* resolution given registry state + lock.
- **`[4 Fetch]`** writes into the content-addressed download cache, not the store; it is parallel,
  resumable, and mirror-aware. Re-running after a network failure resumes from byte offsets.
- **`[5 Verify]`** is the security gate and is fail-closed: a hash or signature mismatch aborts the
  whole command with exit 6 and quarantines the offending cache blob. Nothing partially verified
  proceeds. See [15. Security](15-security.md).
- **`[6 Materialize]` and `[8 Commit]`** are the two stages that touch durable shared state, and
  both are built on temp-then-rename (detailed in [Failure and atomicity model](#failure-and-atomicity-model)).
- The **atomicity boundary is `[8 Commit]`**: any failure in `[1]`..`[7]` leaves the previous
  generation, `vanta.toml`, and `vanta.lock` byte-for-byte unchanged. The user's working
  environment is never left half-mutated.

## Process and concurrency model

Within a single `vanta` invocation, work is parallel and IO-bound, scheduled on a Tokio
multi-threaded runtime (canon §8). Across invocations, correctness is enforced by file locking and
the idempotence of content-addressing.

**Intra-process parallelism**

```
[2 Resolve] ─┬─► provider.versions(node)     ┐
             ├─► provider.versions(python)   │  concurrent metadata fetch (bounded)
             └─► provider.versions(rust)     ┘
[4 Fetch]   ─┬─► download node artifact   (resumable, N-way ranged)   ┐
             ├─► download python artifact                              │  global semaphore caps
             └─► download rust artifact                                ┘  concurrency (default 8)
[6 Mat.]    ─┬─► unpack node   → store/.tmp → rename                   ┐  CPU-bound unpack on
             ├─► unpack python → store/.tmp → rename                   │  spawn_blocking pool
             └─► unpack rust   → store/.tmp → rename                   ┘
```

Network concurrency, per-host connection limits, and unpack parallelism are bounded by semaphores
(defaults in [16. Performance](16-performance.md)). Decompression and hashing run on
`spawn_blocking`/rayon workers so they do not stall the async reactor.

**Inter-process coordination**

Two `vanta` processes may run simultaneously (a developer in two terminals; a hook firing during a
manual `add`; CI parallelism). Coordination uses three mechanisms:

1. **Single-flight on content hash.** Before fetching/materializing key `blake3-X`, a process takes
   a per-key lock (`store/.lock-blake3-X` advisory file lock). The winner materializes; losers wait
   then observe the published entry. Because the key is the content hash, the result is identical
   regardless of who wins — there is no "which copy is right" question.
2. **Atomic rename publish.** Even without the per-key lock, two processes producing key `X`
   stage into distinct `store/.tmp-<rand>/` dirs and `rename` onto the same final path. `rename`
   over an existing directory is resolved by keeping one and discarding the loser's temp; the
   final bytes are identical, so the outcome is correct either way. The lock is an optimization
   (avoid duplicate downloads), not a correctness requirement.
3. **Global store lock for pointer-class operations.** The `current` pointer swap, generation
   append, and GC sweep take a short-held global advisory lock (`store/.lock`). redb itself is
   single-writer/multi-reader, so the generation-record write transaction is serialized by redb;
   the global lock additionally serializes the on-disk pointer/view swap with GC.

Lock acquisition has a default timeout of 30 s for the global lock and 120 s for per-key
single-flight (long downloads), after which the command fails with a `VTA-STORE-*` code naming the
holding pid. Details and the exact lock files are in
[23. Data & State Model](23-data-and-state-model.md#concurrency-and-locking).

## State, caches and persistence

Persistent state lives in two places under `~/.vanta/`: the **content-addressed store/caches**
(raw files) and **`state.db`** (redb). The full schema is the subject of
[23. Data & State Model](23-data-and-state-model.md); the architectural summary:

| Store | Backed by | Holds | Rebuildable? |
| --- | --- | --- | --- |
| `store/` | CAS dirs | immutable materialized tool@version entries | yes, by re-fetch from lock |
| `cache/downloads/` | CAS files | pre-materialize artifact blobs (resumable) | yes, by re-fetch |
| `cache/registry/` | files + redb meta | registry/provider metadata with TTL/ETag | yes, by re-fetch |
| `state.db` `store_index` | redb | StoreKey → entry metadata | yes, by scanning `store/` |
| `state.db` `generations` | redb | generation history records | no (authoritative history) |
| `state.db` `gc_roots` | redb | pins + in-progress markers | partly (pins authoritative) |
| `state.db` `resolution_cache` | redb | (request+registry-rev) → resolution | yes, by re-resolve |
| `state.db` `trust` | redb / keychain | pinned keys; secrets via OS keychain | partly (pins authoritative) |

The split is deliberate: **bytes** belong in content-addressed files (cheap, dedup-able,
GC-able, verifiable by re-hash); **structured metadata and indexes** belong in a transactional
embedded DB (redb — pure Rust, no C dependency, single-writer/multi-reader). We rejected SQLite
(C dependency, against the single-static-binary pillar) and sled (unmaintained, larger on-disk
footprint). Caches use validators (ETag/`If-None-Match`, TTL) so offline and `--refresh` behave
predictably; see [09. Store](09-store.md#caches).

## Generations and rollback

A **generation** is an immutable snapshot of an environment: `env_id → { tool → store key }`
plus the manifest hash and lock hash that produced it, a monotonic id, a parent id, a timestamp,
a reason, and the originating command (full field list in
[23. Data & State Model](23-data-and-state-model.md#generation-records)).

```
generations:   gen 7 ──parent── gen 8 ──parent── gen 9        current ─► gen 9
                  ▲                                  │
   vanta rollback 7  (or `vanta rollback` = previous)│  vanta add … → appends gen 10, current ─► 10
                  └──────── current ─► gen 7 ◄────────┘  (gen 9 retained; rollback is non-destructive)
```

- **Commit** appends a new generation and swaps `current` — the atomic step of `[8 Commit]`.
- **Rollback** (`vanta rollback [gen]`) swaps `current` to an existing generation; it installs
  nothing because every referenced store entry already exists (or is re-fetchable from the
  generation's pinned store keys). It is therefore near-instant and cannot half-apply.
- Generations are GC roots; retention defaults to the **last 5 generations or 30 days, whichever
  retains more** (canon §9). Older generations are collectable by `vanta gc`.

This is an overview. Rollback UX, retention policy, and the `vanta generations` listing are
specified in [12. Updates & Rollback](12-updates.md); the storage of generation records is in
[23. Data & State Model](23-data-and-state-model.md).

## Activation

Activation is how an environment's tools reach the user's `PATH`. Vanta uses a deliberate
**hook + shims hybrid** (canon §11), and both read the same cached resolutions so there is no
second source of truth.

```
                         per-directory resolution cache (redb resolution_cache, keyed by config hash)
                                       ▲                                   ▲
        interactive shell             │                                   │            non-interactive
   ┌──────────────────────┐   on prompt/cd, find nearest        on exec, resolve cwd→version  ┌────────────┐
   │  shell hook (zsh,…)  │──► vanta.toml, if env changed ──┐    └──────────────────────────► │ vanta-shim │
   └──────────────────────┘   swap PATH → envs/<id>/bin     │                                  └─────┬──────┘
                              (sub-ms when cache warm)       │                                        │ exec
                                                             ▼                                        ▼
                                                    envs/<env-id>/bin  ──────────────────────►  store/blake3-…/bin/<tool>
```

- **Hook (default):** a tiny shell function injected by `eval "$(vanta activate zsh)"` that, on
  prompt or `cd`, finds the nearest `vanta.toml` and, only if the resolved environment changed,
  swaps `PATH` to that environment's bin view. A per-directory resolution cache keyed by the
  config hash makes the warm path sub-millisecond (no `vanta` process spawned on the hot path).
- **Shims (fallback):** `~/.vanta/bin` holds one `vanta-shim` dispatcher, hardlinked under each
  tool name. When invoked outside an interactive shell (IDE, cron, `make`), it resolves the right
  version for its cwd from the cached resolution and `exec`s the real binary. This guarantees
  correctness everywhere, even where no hook runs.

The hybrid is intentional: the hook gives speed and transparency in interactive shells; shims give
universal correctness. Supported shells: bash, zsh, fish, PowerShell, Nushell, Elvish; `cmd.exe`
via shims only. Full mechanism, cache invalidation, and the activation state machine are in
[10. Environments & Activation](10-environments.md).

## Failure and atomicity model

Vanta assumes processes can be killed and disks can lie. The model is built from four primitives.

**1. Temp-then-rename for every durable write.** Materialization stages into
`store/.tmp-<rand>/`, fsyncs, then `rename`s onto `store/blake3-<hex>/`. Lock/manifest writes go to
`<file>.tmp-<rand>`, fsync, then `rename`. `rename` within a filesystem is atomic, so a reader
sees either the old state or the new state, never a torn one. Store entries are made read-only
after publish so an accidental write cannot corrupt a content-addressed path.

**2. Fail-closed verification.** No bytes enter the store before `[5 Verify]` passes. A mismatch
aborts the command (exit 6, `VTA-VRF-*`) and quarantines the offending download. This makes a
corrupted or tampered artifact a clean failure, not a poisoned store.

**3. The commit barrier.** `[8 Commit]` is the only stage that makes a mutation visible. Its steps
(redb write txn for the generation + index; temp→rename for lock/manifest; pointer/view swap) are
ordered so the redb commit is the linearization point. Concretely:

```
[8 Commit]
  1. begin redb write txn
  2. insert generations[new]; update store_index, tool_index; (current still = old)
  3. write vanta.lock.tmp + vanta.toml.tmp  → fsync → rename onto final paths
  4. set gen_current[env] = new ; COMMIT redb txn (fsync)   ◄── linearization point
  5. rename envs/<id> staged view; update on-disk `current` mirror
```

**4. Crash recovery on next start.** Each invocation runs a fast reconciliation pass:

| Crash window | Symptom on disk | Recovery |
| --- | --- | --- |
| During `[4 Fetch]` | partial blob in `cache/downloads/` | resumed via range request, or re-fetched; hash gate still applies |
| During `[6 Materialize]` | orphan `store/.tmp-<rand>/` | swept (deleted) on next run; no entry was published |
| Between step 3 and 4 above | new lock/toml on disk, redb `current` still old | `doctor`/`sync` detects `lock_hash` ≠ current gen; idempotently re-commits (same inputs → same content) or offers to revert the file |
| After redb commit, before step 5 | generation committed, env view stale | env view rebuilt from the committed generation (deterministic) |
| redb file corruption | redb open error (`VTA-STORE-*`) | `vanta doctor --repair` rebuilds derived tables from `store/` + locks; history may be lost but current env is reconstructible from `vanta.lock` |

The strong guarantee that underwrites all of this: because `vanta.lock` is committed to VCS and
the store is content-addressed, a correct environment is always reachable via `vanta sync` even
from an empty `~/.vanta`. Local state loss is recoverable, never catastrophic. Details:
[23. Data & State Model](23-data-and-state-model.md#backup-restore-and-recovery).

## Extension seams and dependency injection

Vanta is extensible through a fixed set of core traits defined in `vanta-core`, with concrete
implementations registered in an engine. The **primary, supported** extension point is **WASM
providers** (declarative manifest + sandboxed Wasmtime hook); the trait seams below are the
internal plug points those providers and the built-ins flow through. This keeps the security
model intact: third parties extend Vanta by writing a sandboxed provider, not by linking into the
core or running arbitrary shell (contrast asdf/mise plugins; see [33. Prior Art](33-prior-art.md)).

```rust
// vanta-core — the core trait seams (signatures, not implementations).
// Async methods are IO-bound; the engine drives them on Tokio.

/// A Provider enumerates versions of a tool and maps a concrete version to
/// per-platform artifact descriptors. Built-in providers implement this
/// natively; WASM providers implement the guest-side WIT world and are
/// adapted to this trait by the Wasmtime host in `vanta-provider`.
#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &ProviderId;                      // e.g. "official/node"
    fn ordering(&self) -> VersionOrdering;            // SemVer (default) | CalVer | Custom

    async fn versions(&self, tool: &ToolName, cx: &ProviderCx)
        -> Result<Vec<Version>, ProviderError>;

    async fn artifacts(&self, tool: &ToolName, ver: &Version, cx: &ProviderCx)
        -> Result<PlatformMap<ArtifactDesc>, ProviderError>;

    async fn dependencies(&self, tool: &ToolName, ver: &Version, cx: &ProviderCx)
        -> Result<Vec<Dep>, ProviderError> { Ok(Vec::new()) }   // usually empty
}

/// A Backend is *how* a provider fetches. Selected by the provider, never the
/// user. Produces a concrete, mirror-aware fetch plan that vanta-net executes.
#[async_trait]
pub trait Backend: Send + Sync {
    fn kind(&self) -> BackendKind;   // Registry | GitHub | DirectUrl | LangRegistry | OsPkg
    async fn fetch_plan(&self, desc: &ArtifactDesc, cx: &NetCx)
        -> Result<FetchPlan, BackendError>;
}

/// Content-addressed blob cache (downloads) + validated metadata cache.
#[async_trait]
pub trait CacheStore: Send + Sync {
    async fn get_blob(&self, h: &ContentHash) -> Result<Option<BlobHandle>, CacheError>;
    async fn put_blob(&self, src: TempBlob)   -> Result<ContentHash, CacheError>; // atomic publish
    async fn get_meta(&self, k: &CacheKey)    -> Result<Option<CachedMeta>, CacheError>;
    async fn put_meta(&self, k: &CacheKey, v: CachedMeta, ttl: Duration)
        -> Result<(), CacheError>;
}

/// Verifies one signature/checksum scheme. The security policy fans an
/// artifact out to all applicable verifiers and enforces the required quorum.
pub trait SignatureVerifier: Send + Sync {
    fn scheme(&self) -> SigScheme;   // Minisign | Cosign | Sha256Sum
    fn verify(&self, m: &SignedMaterial, trust: &TrustStore)
        -> Result<Verified, VerifyError>;
}

/// Materializes one file from a store entry into an env view using the
/// cheapest mechanism the filesystem/OS supports. `probe` is fail-fast.
pub trait LinkStrategy: Send + Sync {
    fn kind(&self) -> LinkKind;                       // Reflink | Hardlink | Symlink | Copy
    fn probe(&self, src: &Path, dst_dir: &Path) -> bool;
    fn link(&self, src: &Path, dst: &Path) -> io::Result<()>;
}

/// The engine wires implementations together (dependency injection). Tests
/// inject fakes (fake registry/provider/upstream) from `vanta-test`.
pub struct Engine {
    providers: ProviderSet,                 // ProviderId -> Arc<dyn Provider>
    backends:  BackendTable,                // BackendKind -> Arc<dyn Backend>
    cache:     Arc<dyn CacheStore>,
    verifiers: Vec<Arc<dyn SignatureVerifier>>,   // tried per applicable scheme
    links:     LinkResolver,                // ordered Vec<Arc<dyn LinkStrategy>>: reflink→…→copy
}
```

The `LinkResolver` walks its ordered strategies and uses the first whose `probe` succeeds for the
target filesystem/OS, giving the reflink → hardlink → symlink → copy fallback (canon §9). The
`SignatureVerifier` set lets minisign, cosign, and plain checksum coexist; the security policy
decides the required quorum. Injecting fakes for all of these is how the test harness
(`vanta-test`) runs the full lifecycle hermetically; see [28. Testing](28-testing.md).

## Internal API stability tiers

Vanta versions four surfaces independently (canon §14) so internal refactors never break users
and provider authors get a stable contract.

| Tier | Surface | Stability contract | Versioned by |
| --- | --- | --- | --- |
| Public CLI | commands, flags, exit codes, `--json` shapes | SemVer; breaking changes only on a major | binary SemVer |
| File formats | `vanta.toml`, `vanta.lock` | explicit `version` / `lock_version`; forward-compatible reads | manifest/lock format version |
| Provider ABI | WIT world for WASM providers | SemVer on the WIT world; host supports a range | provider ABI (WIT) version |
| Registry format | registry index + metadata schema | versioned schema; client negotiates | registry index format |
| Internal crates | `vanta-*` library APIs | **no stability guarantee**; may change any release | not separately versioned |

The `vanta-sdk` crate is the *only* crate intended for third-party consumption (provider authors);
its surface tracks the provider ABI. All other `vanta-*` crates are internal implementation detail
and may change freely. The public Rust API surface, if any is later exposed, is documented in
[29. Public APIs](29-public-apis.md); the provider ABI is specified in
[22. Provider SDK & ABI](22-provider-sdk.md).

## Cross-references

- [01. Vision](01-vision.md) — the design pillars and signature innovations this architecture realizes.
- [06. Resolution](06-resolution.md) — `[2 Resolve]` in depth: version grammar, ordering, the dependency DAG.
- [07. Providers](07-providers.md) — the `Provider`/`Backend` model, registries, and the Wasmtime host.
- [08. Installation](08-installation.md) — the install engine driving `[3 Plan]`..`[8 Commit]`.
- [09. Store](09-store.md) — content addressing, atomic publish, link strategies, and GC.
- [10. Environments](10-environments.md) — `[Activate]`: the hook + shims hybrid and the resolution cache.
- [12. Updates & Rollback](12-updates.md) — generation retention and the rollback workflow.
- [23. Data & State Model](23-data-and-state-model.md) — the redb schema, locking, and recovery.
- [24. ADRs](24-architecture-decision-records.md) — the no-daemon, content-addressing, and redb decisions.
