# 09. Store, Cache & Storage

> The content-addressed store is Vanta's architectural keystone. This document specifies how store keys are computed and why content addressing is used, the canonicalization that makes hashes reproducible across machines and operating systems, the on-disk layout, atomic publication, the reflink→hardlink→symlink→copy link strategy, deduplication, the caches, garbage collection, and integrity repair. Owned by `vanta-store` (state index in [23. Data & State Model](23-data-and-state-model.md)).

**Contents**

- [Why content addressing](#why-content-addressing)
- [Hashing and canonicalization](#hashing-and-canonicalization)
- [Storage layout](#storage-layout)
- [Atomic publish](#atomic-publish)
- [Link strategies](#link-strategies)
- [Deduplication](#deduplication)
- [Caches](#caches)
- [Garbage collection](#garbage-collection)
- [Integrity and repair](#integrity-and-repair)
- [Disk-usage accounting](#disk-usage-accounting)
- [Trade-offs and alternatives](#trade-offs-and-alternatives)
- [Cross-references](#cross-references)

---

## Why content addressing

Every tool Vanta installs is materialized into an **immutable, content-addressed directory** under `~/.vanta/store/`, keyed by a hash of its content. This single decision ([ADR-0003](24-architecture-decision-records.md)) yields, as direct consequences, almost every pillar in canon §2:

| Property | Why content addressing gives it |
| --- | --- |
| **Integrity** | The path *is* the checksum. A store entry that hashes to its own key is provably the bytes that were verified at install. |
| **Deduplication** | Two requests resolving to identical bytes produce the same key and share one directory — across tool versions, projects, and generations. |
| **Reproducibility** | A generation is a set of content hashes; the same lock on any machine materializes the same keys ([11. Reproducibility](11-reproducibility.md)). |
| **Safe concurrency** | Two processes producing the same key converge on the same path; the publish `rename` is idempotent. There is never a "which copy is correct" race. |
| **Atomic rollback** | Switching environments/generations is repointing links at already-present entries; nothing is reinstalled ([12. Updates & Rollback](12-updates.md)). |
| **Trivial, safe GC** | Liveness is reachability from roots; an unreferenced entry is provably safe to delete. |

The store is **append-mostly and never mutated**. Entries are created and (eventually) garbage-collected; they are never edited in place. The environment a user sees is a cheap *view* over the store, not a copy.

## Hashing and canonicalization

- **Algorithm.** Store keys use **BLAKE3** — fast, parallelizable, and cryptographically strong — rendered as `blake3-<lowercase-hex>`. **SHA-256** is also computed for every artifact to match upstream-published checksums and for ecosystems/policies that require it. ([ADR-0007](24-architecture-decision-records.md).)
- **What is hashed.** For a downloaded artifact, the store key derives from the canonicalized *materialized tree*. For built or composed entries, the key derives from the **recipe** — provider id + exact version + input artifact hash(es) + layout parameters — so identical inputs yield identical keys even when a build is involved (the build itself must be deterministic for this to hold; see trade-offs).
- **Canonicalization (the reproducibility crux).** Hashing a directory tree is made platform-independent by normalizing before hashing:
  - entries are walked in a **stable sorted order** (byte-wise path sort);
  - **modes are normalized** to a canonical set (executable bit preserved; other permission noise dropped);
  - **timestamps are excluded** from the hash (recorded as metadata, never hashed);
  - **symlinks** are hashed by target string, not by following them;
  - path separators and case are normalized to a canonical form so a tree hashes identically on case-insensitive (macOS/Windows) and case-sensitive (Linux) filesystems.

  This is what lets a macOS and a Linux machine agree that a given tool artifact materializes to the same logical content where the bytes truly are the same, and is exercised by the reproducibility test suite ([28. Testing](28-testing.md)).

## Storage layout

Everything lives under `$VANTA_HOME` (default `~/.vanta`; `%LOCALAPPDATA%\Vanta` on Windows), one predictable root:

```
~/.vanta/
├── bin/                       # ON THE USER'S PATH: vanta, vt, and the shim dispatcher
│   ├── vanta                  #   (the real binary)
│   ├── vt -> vanta            #   (alias)
│   ├── node -> vanta-shim     #   (hardlink/launcher: shim dispatched by name)
│   └── python -> vanta-shim   #   ...one per shimmed tool
├── store/                     # content-addressed, immutable
│   ├── blake3-aa3f…/          #   node 24.6.0 (linux/x86_64)  — read-only tree
│   ├── blake3-bb71…/          #   python 3.13.4
│   └── .tmp-7f2c…/            #   transient staging dir (swept on next run)
├── envs/                      # composed per-environment views (link farms)
│   └── 4e9c…/                 #   env-id → bin/ links into store entries
│       └── bin/{node,python,…}
├── generations/               # immutable generation records + `current` pointer mirror
│   ├── 0007.json  0008.json   #   (canonical store is redb; these mirror for inspectability)
│   └── current -> 0008
├── cache/
│   ├── downloads/             # CAS of fetched artifacts (pre-materialize, resumable)
│   │   └── sha256-….part      #   in-progress download
│   └── registry/              # cached registry + provider metadata (TTL/ETag)
├── registries/                # cached/cloned registry indexes (official + private)
├── trust/                     # trust DB + pinned public keys
├── state.db                   # redb: store_index, generations, gc_roots, caches, trust
└── config.toml                # global config + global [tools]
```

`state.db` (redb) is the authoritative index and history; the JSON files under `generations/` are a human-inspectable mirror, not the source of truth. The split between **bytes** (content-addressed files) and **metadata** (redb) is deliberate ([02. Architecture](02-architecture.md#state-caches-and-persistence)).

## Atomic publish

Publishing a store entry is crash-safe by construction:

```
1. mkdir store/.tmp-<rand>/                 # unique staging dir
2. extract/build into it; canonicalize
3. fsync the staged tree
4. rename store/.tmp-<rand>/ -> store/blake3-<hex>/   # atomic within the filesystem
5. set the entry read-only (chmod / clear write bits / Windows ACL)
```

- `rename` over a same-filesystem path is atomic: a reader sees either no entry or the complete entry, never a partial one.
- If step 4's target already exists (a concurrent producer won), the loser discards its temp dir — the bytes are identical, so correctness holds either way.
- Making the entry read-only prevents accidental mutation of a content-addressed path; the integrity checker relies on it.
- A crash before step 4 leaves only `store/.tmp-<rand>/`, swept by the next invocation's reconciliation pass.

`$VANTA_HOME` is required to be on a single filesystem so renames are atomic; `vanta doctor` warns if `store/` and `cache/downloads/` straddle filesystems (which would turn the cache→store publish into a copy and break atomicity).

## Link strategies

An environment view is built by linking a store entry's executables into `envs/<env-id>/bin`. Vanta uses the cheapest mechanism the underlying filesystem and OS support, probed in order:

| Strategy | Cost | When usable | Notes |
| --- | --- | --- | --- |
| **Reflink** (CoW) | near-zero, copy-on-write | Btrfs, XFS, APFS, ReFS | best of both worlds: independent file, no data copy |
| **Hardlink** | near-zero | same filesystem, link support | shares inodes; the default on most setups |
| **Symlink** | near-zero | Unix always; Windows with privilege/Dev Mode | a pointer; some tools resolve real paths (handled by also exposing canonical store paths) |
| **Copy** | full bytes | last resort (cross-fs, no link support) | always correct; used only when nothing cheaper works |

`vanta-platform` exposes a `LinkStrategy` per kind with a fast `probe`; `vanta-env`'s `LinkResolver` walks the ordered list and uses the first that succeeds (canon §9). On **Windows**, where unprivileged symlinks are not guaranteed, the order is reflink (ReFS) → hardlink → copy, and the *shim dispatcher* is exposed as a real `.exe` launcher rather than a symlink ([17. Cross-platform](17-cross-platform.md)). The chosen strategy is configurable (`[settings] link_strategy`) but defaults to `auto`.

## Deduplication

Because the key is the content hash, identical content is stored once:

- **Across versions of one tool** — shared files between, say, two patch releases of Node need not duplicate (when the materialized subtrees hash equal; whole-entry dedup is guaranteed, sub-file dedup is a future optimization).
- **Across projects** — every project needing `node 24.6.0` references the same store entry; the second project's install is just a link.
- **Across generations** — rolling forward and back reuses entries; only the pointer and view change.

The practical effect mirrors pnpm and uv: a machine with dozens of projects holds one copy of each distinct tool artifact, not one per project. Disk grows with *distinct* tool versions, not with project count. The dedup ratio is reported by `vanta cache stats` and `vanta store stats`.

## Caches

Two caches sit in front of the store, both validated:

- **Download cache (`cache/downloads/`)** — a content-addressed cache of *fetched artifacts* before materialization. Keyed by the published checksum, so re-fetching identical bytes is free and a second project sharing an artifact downloads it once. Partial downloads (`*.part`) are resumable. This cache is freely GC-able (re-fetchable from the lock).
- **Registry/metadata cache (`cache/registry/`)** — cached registry index entries and provider metadata, with **TTL** and **ETag/`If-None-Match`** conditional revalidation so `vanta` is fast when warm and correct when stale. `--refresh` forces revalidation; `--offline` uses whatever is cached and fails cleanly on a miss.

Both caches are derived state: deleting them only forces re-fetching, never data loss. Cache size limits and pruning are configurable (`vanta cache prune`, `[settings] cache_max_size`).

## Garbage collection

Liveness is **reachability from GC roots** (tracing GC, chosen over reference counting because tracing cannot leak from a missed decrement and makes "is this safe to delete?" a provable global property). Roots are:

- the **active generation** for every environment (`current` pointers);
- **retained generations** — default **the last 5 generations or anything newer than 30 days, whichever retains more** (canon §9; `[settings] retain_generations`, `gc_keep_days`);
- **pinned** entries (`vanta pin <tool>@<ver>`), e.g. a base toolset kept for offline use;
- **in-progress install markers** (so a concurrent install isn't swept mid-flight).

`vanta gc` runs mark-and-sweep under the global store lock:

```
vanta gc [--dry-run] [--keep-days N] [--aggressive]
  mark:  walk roots → set of reachable store keys
  sweep: for each store entry not reachable → delete; remove from store_index
  also:  prune cache/downloads and quarantined blobs; remove dangling env views
```

GC is **safe and atomic**: it deletes only provably-unreachable entries, holds the global lock so it cannot race a commit, and `--dry-run` reports exactly what would be freed. An interrupted GC is harmless (deletes are individually safe; the index is reconciled on next run).

## Integrity and repair

Because entries are content-addressed and read-only, corruption is detectable by re-hashing:

- `vanta doctor` (and `vanta store verify`) re-hashes store entries and compares to their keys; a mismatch flags corruption (disk error, external tampering) as `VTA-STORE-0001`.
- **Self-healing:** a corrupt or missing entry that is referenced by the lock is re-fetched and re-materialized (`vanta sync` / `vanta doctor --repair`), because the lock carries the artifact identity.
- Derived state is rebuildable: `vanta doctor --repair` can reconstruct `store_index` and `tool_index` by scanning `store/`, and reconstruct the active environment from `vanta.lock`. Generation *history* is the only non-derivable state; its loss degrades rollback depth but never the current environment.

## Disk-usage accounting

`vanta store stats` / `vanta cache stats` report logical vs physical size (accounting for reflink/hardlink sharing), per-tool and per-generation usage, the dedup ratio, reclaimable space (what `vanta gc` would free), and cache sizes. This makes the store's cost legible and gives operators the data to tune retention.

## Trade-offs and alternatives

- **Indirection.** Entries live in the store and the environment links to them, rather than tools living "where you installed them." This is the cost of dedup/atomicity/rollback; `vanta which` always reveals the real path, and reflink/hardlink make the indirection free at runtime.
- **GC is required.** Unlike a mutable prefix, an immutable store accumulates entries and needs GC. This is a feature (safe rollback depends on retaining old entries) with a bounded cost (retention policy + `vanta gc`).
- **Single-filesystem requirement** for atomic rename. Acceptable and checked by `doctor`.
- **Versus per-tool prefixes (asdf/mise: `~/.../installs/<tool>/<version>`).** That model is simpler to inspect but cannot dedupe, cannot guarantee content integrity, makes "the active set" implicit, and turns rollback into reinstallation. Vanta accepts the store's indirection to gain those properties.
- **Versus `/nix/store`.** Vanta's store is the same core idea (content-addressed, immutable, GC-by-reachability, generations) but keyed off pragmatic artifact/recipe hashes rather than full functional derivations, and without a build language — Nix's guarantees for the common case at a fraction of the complexity ([33. Prior Art](33-prior-art.md)).
- **Build reproducibility limit.** For prebuilt artifacts, byte-identical reproduction is guaranteed by hash. For *source builds*, Vanta can only guarantee reproduction if the build is deterministic; the mitigation is prebuilt-first and hash-pinning of build inputs, with non-reproducible builds flagged.

## Cross-references

- [02. Architecture](02-architecture.md) — the store-centric model and where the store sits in the lifecycle.
- [08. Installation](08-installation.md) — the engine that fetches, verifies, and publishes store entries.
- [11. Reproducibility](11-reproducibility.md) — how canonical hashing underwrites the cross-platform lock.
- [12. Updates & Rollback](12-updates.md) — generations as GC roots and the retention policy.
- [17. Cross-platform](17-cross-platform.md) — link-strategy differences and the single-filesystem requirement per OS.
- [23. Data & State Model](23-data-and-state-model.md) — `store_index`, `gc_roots`, and the locking protocol.
- [15. Security](15-security.md) — verification that gates entry into the store and integrity guarantees.
