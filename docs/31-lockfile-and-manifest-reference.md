# 31. Lockfile & Manifest Format Reference

> The exhaustive format reference for `vanta.lock` and its relationship to `vanta.toml`: the full lock schema, a complete multi-tool multi-platform example, the manifest↔lock reconcile rules, format versioning and compatibility, the canonical serialization rules that keep locks diffable, and VCS merge-conflict guidance. The conceptual model is [11. Reproducibility](11-reproducibility.md); the manifest keys are in [27. Configuration Reference](27-config-reference.md).

**Contents**

- [The two files](#the-two-files)
- [Lockfile top-level schema](#lockfile-top-level-schema)
- [Locked tool entry schema](#locked-tool-entry-schema)
- [Complete example](#complete-example)
- [Manifest to lock reconcile rules](#manifest-to-lock-reconcile-rules)
- [Format versioning](#format-versioning)
- [Canonical serialization](#canonical-serialization)
- [Merge-conflict guidance](#merge-conflict-guidance)
- [Cross-references](#cross-references)

---

## The two files

`vanta.toml` (the manifest) says **what you want**; `vanta.lock` (the lockfile) says **exactly what that resolved to**, per platform, with hashes. Both are TOML, both are committed to VCS. The manifest is hand-edited; the lock is machine-generated and should not be hand-edited (regenerate it with `vanta lock`/`add`/`update`). The manifest is covered in [27. Configuration Reference](27-config-reference.md); this document is the lock plus the relationship between them.

## Lockfile top-level schema

| Key | Type | Description |
| --- | --- | --- |
| `lock_version` | int | the lock format version (independent of the binary version) |
| `generated_by` | string | the Vanta version that wrote it (informational) |
| `targets` | list<token> | the platform tokens this lock resolves for ([17](17-cross-platform.md)) |
| `registry_revision` | string | the registry snapshot identity resolution used (reproducible re-resolve) |
| `[[tool]]` | array of tables | one locked entry per tool (below) |

## Locked tool entry schema

Each `[[tool]]` records the resolution plus a per-platform artifact pin.

| Key | Type | Description |
| --- | --- | --- |
| `name` | string | the tool name |
| `request` | string | the constraint from `vanta.toml` (e.g. `"24"`) |
| `version` | string | the exact resolved version (e.g. `"24.6.0"`) |
| `provider` | string | provider id + provider version (e.g. `"official/node@3"`) |
| `channel` | string | the channel resolved against, if any |
| `deps` | list<string> | resolved tool dependencies (names@versions), if any |
| `[tool.platform."<token>"]` | table | the per-platform artifact pin |

Per-platform table:

| Key | Type | Description |
| --- | --- | --- |
| `store_key` | string | `blake3-<hex>` — the content-addressed store identity |
| `url` | string | the resolved artifact URL |
| `mirrors` | list<string> | optional additional URLs |
| `size` | int | artifact size in bytes |
| `sha256` | string | the upstream checksum (verified on download) |
| `blake3` | string | the internal checksum |
| `signature` | string | signature reference (e.g. `"minisign:RWQf…"`) |
| `provenance` | string | optional SLSA provenance reference + level |
| `bin` | list<string> | executables exposed (platform-appropriate forms) |
| `layout` | table | strip/subdir used during materialization |

## Complete example

A `vanta.lock` for three tools across three platforms:

```toml
lock_version = 1
generated_by = "vanta 0.9.2"
targets = ["macos/aarch64", "linux/x86_64/gnu", "windows/x86_64"]
registry_revision = "2026-06-20T11:04:00Z#a91f3c…"

[[tool]]
name = "node"
request = "24"
version = "24.6.0"
provider = "official/node@3"

  [tool.platform."macos/aarch64"]
  store_key = "blake3-aa3f9c12…"
  url = "https://registry.vanta.dev/node/24.6.0/node-darwin-arm64.tar.xz"
  size = 24117248
  sha256 = "5f2c1b…"
  blake3 = "aa3f9c12…"
  signature = "minisign:RWQf9K…"
  bin = ["bin/node", "bin/npx"]
  layout = { strip = 1 }

  [tool.platform."linux/x86_64/gnu"]
  store_key = "blake3-7b21d4…"
  url = "https://registry.vanta.dev/node/24.6.0/node-linux-x64.tar.xz"
  size = 26214400
  sha256 = "9a14ef…"
  blake3 = "7b21d4…"
  signature = "minisign:RWQf9K…"
  bin = ["bin/node", "bin/npx"]
  layout = { strip = 1 }

  [tool.platform."windows/x86_64"]
  store_key = "blake3-c4d80a…"
  url = "https://registry.vanta.dev/node/24.6.0/node-win-x64.zip"
  size = 28311552
  sha256 = "b0f3aa…"
  blake3 = "c4d80a…"
  signature = "minisign:RWQf9K…"
  bin = ["node.exe", "npx.cmd"]
  layout = { strip = 1 }

[[tool]]
name = "ripgrep"
request = "14"
version = "14.1.0"
provider = "official/ripgrep@1"

  [tool.platform."macos/aarch64"]
  store_key = "blake3-9d2e77…"
  url = "https://github.com/BurntSushi/ripgrep/releases/download/14.1.0/ripgrep-14.1.0-aarch64-apple-darwin.tar.gz"
  size = 2398512
  sha256 = "7c0e2a…"
  blake3 = "9d2e77…"
  signature = "minisign:RWQf9K…"
  bin = ["rg"]
  layout = { strip = 1 }
  # ... linux/windows entries elided for brevity

[[tool]]
name = "terraform"
request = "1.9"
version = "1.9.8"
provider = "official/terraform@2"
# ... per-platform entries
```

## Manifest to lock reconcile rules

The lock is derived from the manifest; these rules govern when it changes:

| Operation | Manifest (`vanta.toml`) | Lock (`vanta.lock`) |
| --- | --- | --- |
| `vanta add t@c` | sets/updates `[tools] t = c` | resolves `t` for all `targets`, adds/updates its `[[tool]]` |
| `vanta remove t` | removes `t` from `[tools]` | removes its `[[tool]]` |
| `vanta update [t]` | unchanged (constraint kept) | re-resolves within the constraint, rewrites affected entries |
| `vanta lock` | unchanged | regenerates the lock to satisfy the manifest at `registry_revision` |
| `vanta sync` | unchanged | unchanged (authoritative read); installs to match it |
| `vanta sync --frozen` | unchanged | must already satisfy the manifest, else fail (`VTA-LOCK-0001`) |
| `vanta target add <p>` | (records target) | adds per-platform entries for `<p>` to every tool |

The invariant: after any mutating command, **the manifest and lock agree** (the lock satisfies every manifest constraint for every target), and on success both are written atomically together ([08. Installation](08-installation.md#stage-8--commit)). `sync` never silently re-resolves; only `add`/`update`/`lock` do.

## Format versioning

- `lock_version` (and the manifest's `version`) are integers independent of the binary version ([ADR-0023](24-architecture-decision-records.md)).
- A **newer binary** reads an **older** `lock_version` directly (it knows all prior versions).
- An **older binary** facing a **newer** `lock_version` refuses with `VTA-LOCK-0002` ("lock written by a newer Vanta; upgrade") rather than misreading it ([25. Error Catalog](25-error-and-exit-code-catalog.md)).
- New fields are additive and optional within a `lock_version`; a breaking change increments it with a documented migration ([29. Public APIs](29-public-apis.md)).

## Canonical serialization

The lock is serialized canonically so it diffs cleanly and merges sanely:

- **Stable ordering** — tools sorted by name; platform tables sorted by token; keys within a table in a fixed order.
- **Normalized formatting** — consistent indentation, quoting, and number formatting; no trailing whitespace.
- **Deterministic** — the same resolution always serializes identically (no timestamps in the body beyond `registry_revision`/`generated_by`), so re-running `vanta lock` with no changes produces a zero-diff file.

This makes a `vanta.lock` change in a PR a small, reviewable diff (a version bump shows as a few changed lines, not a reshuffle) — important for the lock-tampering defenses in [21. Threat Model](21-threat-model.md).

## Merge-conflict guidance

Because the lock is generated, a VCS merge conflict is resolved by **regenerating, not hand-editing**:

```sh
# after a conflict in vanta.lock
git checkout --theirs vanta.lock     # or --ours; pick a base
vanta lock                            # re-derive from the merged vanta.toml at registry_revision
git add vanta.lock
```

`vanta lock` re-derives a consistent lock from the (merged) manifest, so two branches that each added a tool reconcile cleanly. Hand-editing hashes is never necessary and is discouraged (a wrong hash fails verification at install anyway). Canonical serialization keeps most merges conflict-free in the first place by minimizing churn.

## Cross-references

- [11. Reproducibility](11-reproducibility.md) — the conceptual model and cross-platform locking.
- [27. Configuration Reference](27-config-reference.md) — the `vanta.toml` manifest keys.
- [26. Registry & Metadata Reference](26-registry-and-metadata-reference.md) — the artifact descriptors pinned here.
- [08. Installation](08-installation.md) — the atomic commit that writes manifest + lock together.
- [29. Public APIs](29-public-apis.md) — the format as a stable on-disk surface.
- [21. Threat Model](21-threat-model.md) — lock-tampering defenses and reviewable diffs.
