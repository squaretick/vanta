# 23. Data & State Model

> The on-disk truth: the redb `state.db` schema, what is authoritative versus rebuildable, the full directory layout, the concurrency and locking protocol, the exact shape of a generation record, how secrets are stored at rest, state-schema versioning and migration, and backup/recovery. This is the reference behind [02. Architecture](02-architecture.md) and [09. Store](09-store.md).

**Contents**

- [Two stores: bytes and metadata](#two-stores-bytes-and-metadata)
- [The redb schema](#the-redb-schema)
- [Authoritative vs rebuildable state](#authoritative-vs-rebuildable-state)
- [On-disk layout](#on-disk-layout)
- [Generation records](#generation-records)
- [Concurrency and locking](#concurrency-and-locking)
- [Secret handling](#secret-handling)
- [Schema versioning and migration](#schema-versioning-and-migration)
- [Backup and recovery](#backup-and-recovery)
- [Cross-references](#cross-references)

---

## Two stores: bytes and metadata

Vanta's persistent state is split by nature ([02. Architecture](02-architecture.md#state-caches-and-persistence)):

- **Bytes** — materialized tool entries and cached downloads — live as **content-addressed files** under `store/` and `cache/`. They are large, immutable, dedup-able, GC-able, and verifiable by re-hash. A database is the wrong home for them.
- **Structured metadata and indexes** — the store index, generation history, GC roots, caches, and trust — live in a **single transactional embedded database**, `state.db` (redb). They are small, queried, mutated transactionally, and must stay internally consistent.

redb is chosen ([ADR-0008](24-architecture-decision-records.md)) because it is pure-Rust (no C dependency, preserving the single-static-binary goal), transactional (ACID), and memory-mapped (fast reads), with a single-writer/multi-reader model that fits Vanta's access pattern. SQLite (C dependency) and sled (stability/maintenance concerns) were rejected.

## The redb schema

`state.db` holds these tables (redb tables are typed key→value maps; values are serialized records):

| Table | Key | Value | Purpose |
| --- | --- | --- | --- |
| `meta` | `"schema_version"`, … | integer / blob | DB schema version and global markers (drives migration) |
| `store_index` | `StoreKey` (`blake3-…`) | `StoreEntryMeta { tool, version, platform, size, sha256, created_at, recipe_hash }` | catalog of materialized store entries |
| `tool_index` | `(tool, version, platform)` | `StoreKey` | reverse lookup: which store key backs a tool@version |
| `generations` | `GenId` (sortable) | `GenerationRecord` (see below) | immutable generation history |
| `gen_current` | `EnvId` | `GenId` | the active generation per environment (the `current` pointer) |
| `gc_roots` | root id | `Root { kind: pin\|in-progress\|retained, ref }` | explicit GC roots beyond current/retained generations |
| `resolution_cache` | `config_hash` (or `(request, registry_rev)`) | `Resolution` / `EnvId` | the activation/shim fast-path cache; invalidated by config change |
| `registry_cache` | `(registry, path)` | `{ body_ref, etag, fetched_at, ttl }` | cached registry/provider metadata with validators |
| `trust` | key id / path hash | `TrustRecord { pinned_key \| config_trust, scope, added_at }` | pinned signing keys and config trust-on-first-use records |
| `target_set` | `EnvId` / project | `list<platform-token>` | the platforms this project locks for |

`resolution_cache` and `registry_cache` are caches (rebuildable); the rest is index/history. Records are serialized with serde; `#[non_exhaustive]` record types allow forward-compatible additions guarded by `schema_version`.

## Authoritative vs rebuildable state

A clear split bounds the blast radius of state loss:

| State | Authoritative source | Rebuildable from |
| --- | --- | --- |
| Store entries (`store/`) | the files themselves (content-addressed) | re-fetch from `vanta.lock` |
| `store_index` / `tool_index` | redb | scan `store/` + re-hash |
| `resolution_cache` / `registry_cache` | redb (cache) | re-resolve / re-fetch |
| `gen_current` (active pointer) | redb | the committed `vanta.lock` (current = lock) |
| **`generations` history** | **redb (only home)** | not rebuildable — see backup |
| `gc_roots` (pins) | redb | not rebuildable (user intent) |
| `trust` (pinned keys, config trust) | redb + OS keychain | partly (pinned keys ship with Vanta; config trust must be re-granted) |

The only state that is *not* reconstructible from `store/` + `vanta.lock` is **generation history** and **explicit pins/trust**. Everything else — including the current working environment — survives total loss of `state.db` via `vanta doctor --repair` + `vanta sync`. This is why local state loss is recoverable, never catastrophic ([09. Store](09-store.md#integrity-and-repair)).

## On-disk layout

The full tree (canon §7, expanded with the data-model view):

```
~/.vanta/                       ($VANTA_HOME; %LOCALAPPDATA%\Vanta on Windows; one filesystem)
├── bin/                        # on PATH: vanta, vt, vanta-shim (hardlinked per tool name)
├── store/                      # content-addressed immutable entries (bytes; authoritative)
│   └── blake3-<hex>/
├── envs/                       # composed environment views (link farms; derived)
│   └── <env-id>/bin/
├── generations/                # JSON mirror of generation records (human-inspectable; redb authoritative)
│   ├── <gen-id>.json
│   └── current -> <gen-id>
├── cache/
│   ├── downloads/              # content-addressed download cache (bytes; rebuildable)
│   └── registry/              # cached metadata bodies (rebuildable)
├── registries/                # cached/cloned registry indexes
├── trust/                      # pinned public keys + trust material (keychain holds secrets)
├── state.db                    # redb (metadata/index/history; see schema)
├── state.db.lock               # advisory global lock file
├── audit.log                   # append-only audit (JSON lines) — enterprise
└── config.toml                 # global config + global [tools]
```

File formats: TOML for `vanta.toml`/`vanta.lock`/`config.toml`; JSON-lines for `audit.log`; the redb binary format for `state.db`; raw content-addressed bytes for `store/` and `cache/`.

## Generation records

A generation is a complete, hashable description of an environment at a point in time ([12. Updates & Rollback](12-updates.md)):

```
GenerationRecord {
  id:           GenId,                     // sortable, monotonically increasing
  parent:       Option<GenId>,             // the generation this superseded
  env_id:       EnvId,                     // which environment (project/global)
  created_at:   timestamp,                 // metadata only — never hashed
  command:      String,                    // e.g. "vanta update node"
  actor:        Option<String>,            // user/CI identity (for audit)
  tools:        Map<ToolName, StoreKey>,   // the resolved, materialized set
  manifest_hash:String,                    // hash of the vanta.toml that produced it
  lock_hash:    String,                    // hash of the vanta.lock that produced it
  reason:       Reason,                    // add | remove | update | sync | rollback | restore
}
```

The record references store keys (not copies), so a generation is cheap and a rollback is a `gen_current` swap to an existing record. The `lock_hash` lets any generation be reconstructed by `vanta sync` against its recorded lock even after its store entries were GC'd.

## Concurrency and locking

Multiple `vanta` processes (two terminals, a hook firing during a manual command, parallel CI) coordinate via a layered locking scheme ([08. Installation](08-installation.md#concurrency)):

| Lock | Scope | Held during | Timeout |
| --- | --- | --- | --- |
| redb write txn | metadata | any state mutation (single-writer) | redb-internal |
| `state.db.lock` (global advisory) | pointer-class ops | generation append, `gen_current` swap, GC sweep | 30 s |
| per-key advisory lock | one `StoreKey` | materializing that entry (single-flight) | 120 s |

- **Reads never block.** redb gives lock-free multi-reader snapshots; activation/shim lookups never wait on a writer.
- **Single-flight materialization.** Before building `blake3-X`, a process takes the per-key lock; losers wait then observe the published entry. Because the key is the content hash, the winner is irrelevant.
- **Correctness without the lock.** Even if locking failed, two producers of `X` stage into distinct temp dirs and rename onto the same path with identical bytes — the per-key lock is an optimization (avoid duplicate downloads), not a correctness requirement.
- **Timeouts surface the holder.** A lock wait that exceeds its timeout fails with a `VTA-STORE-*` error naming the holding pid, rather than hanging.

## Secret handling

Registry credentials and similar secrets are **never** stored in plaintext config or the lock ([15. Security](15-security.md), [14. Enterprise](14-enterprise.md)):

| Platform | Backend |
| --- | --- |
| macOS | Keychain Services |
| Windows | Credential Manager (DPAPI) |
| Linux | Secret Service (libsecret) when available; otherwise a `0600` file under `trust/` with a clear warning |

- Tokens are scoped per registry host and retrieved on demand; they are redacted from logs, `--json` output, and error messages.
- The `trust` redb table stores *references* and non-secret trust material (pinned public keys, config-trust hashes); actual secrets live in the OS keychain.
- `vanta registry login`/`logout` manage credential lifecycle; `vanta trust` manages pinned keys and config trust.

## Schema versioning and migration

- The `meta` table records `schema_version`. On startup, Vanta compares it to the binary's expected version.
- A **newer binary** on an **older DB** runs a forward migration (additive table/field changes, backfills) inside a transaction, bumping `schema_version`; a backup of `state.db` is taken first.
- A **newer DB** on an **older binary** is refused with a clear `VTA-STORE-*` message ("state written by a newer Vanta; upgrade") rather than risking corruption.
- Migrations are tested with fixtures of each prior schema version ([28. Testing](28-testing.md)).
- Because most state is rebuildable, a migration can, in the worst case, fall back to "rebuild derived tables from `store/` + locks," preserving only history/pins/trust explicitly.

## Backup and recovery

- **What to back up.** For full fidelity, back up `state.db` (history/pins/trust) and `config.toml`. The `store/` and `cache/` are large and reconstructible, so they need not be backed up.
- **Recovery from lost `state.db`.** `vanta doctor --repair` rebuilds `store_index`/`tool_index` by scanning and re-hashing `store/`, and reconstructs the active environment from `vanta.lock`. Generation history and pins are lost (unless backed up); the current environment is fully restored.
- **Recovery from a corrupt store entry.** Re-hash detects it; it is re-fetched from the lock and re-materialized ([09. Store](09-store.md#integrity-and-repair)).
- **Recovery from an empty `~/.vanta`.** `git clone && vanta sync` reconstructs a correct, verified environment from scratch — the strongest guarantee, owed to content addressing + a committed lock.

## Cross-references

- [02. Architecture](02-architecture.md) — the bytes/metadata split and the failure/atomicity model.
- [09. Store](09-store.md) — the content-addressed store these indexes catalog, and GC.
- [08. Installation](08-installation.md) — the commit transaction and the locking protocol in action.
- [12. Updates & Rollback](12-updates.md) — generation records and rollback semantics.
- [15. Security](15-security.md) — secret handling and trust.
- [25. Error & Exit-code Catalog](25-error-and-exit-code-catalog.md) — `VTA-STORE-*` codes for state/locking failures.
