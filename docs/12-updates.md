# 12. Updates & Rollback

> How tools move forward safely and back instantly. This document specifies the update strategy (constraint-respecting, lock-rewriting), channels, the atomic generation model that makes every mutation reversible, `vanta rollback` and generation history, `vanta self update` for the binary itself, update-safety guards, and post-update pruning. Built on the store and generation machinery in [09. Store](09-store.md) and [02. Architecture](02-architecture.md).

**Contents**

- [Update strategy](#update-strategy)
- [Channels](#channels)
- [Atomicity of updates](#atomicity-of-updates)
- [Rollback and generations](#rollback-and-generations)
- [Updating Vanta itself](#updating-vanta-itself)
- [Update safety](#update-safety)
- [Pruning](#pruning)
- [Failure scenarios and trade-offs](#failure-scenarios-and-trade-offs)
- [Cross-references](#cross-references)

---

## Update strategy

`vanta update [tool …]` moves tools to the newest version **allowed by the manifest constraint**, re-resolves, and rewrites `vanta.lock`. It never silently changes the constraint in `vanta.toml`.

- `vanta update` with no argument updates every tool in scope within its constraint.
- `vanta update node` updates only `node`.
- The constraint governs how far it moves: `node = "24"` updates within `24.x`; `node = "^24"` allows `24` and above per SemVer; `node = "24.6.0"` is pinned and `update` is a no-op. To cross a constraint boundary the user edits the manifest (or `vanta add node@25`, which both sets the constraint and updates the lock).
- `vanta outdated` previews what *could* update (current → latest-allowed → latest-available) without changing anything:

```
$ vanta outdated
tool        current   latest (allowed)   latest (available)
node        24.4.1    24.6.0             25.1.0      (constraint "24")
terraform   1.9.2     1.9.8              1.10.0      (constraint "1.9")
ripgrep     14.1.0    14.1.0             14.1.0      up to date
```

Re-resolution happens for **all target platforms** (like `add`), so an update keeps the cross-platform lock complete ([11. Reproducibility](11-reproducibility.md)).

## Channels

Version requests may name a channel rather than a number (canon §10), and `update` follows it:

| Request | Meaning | `update` behavior |
| --- | --- | --- |
| `"24"` / `"24.6"` | newest matching prefix | newest within the prefix |
| `"latest"` | newest stable | newest stable at update time |
| `"lts"` | newest long-term-support line (provider-defined) | newest within the current LTS |
| `"stable"` / `"nightly"` (e.g. rust) | named channel (provider-defined) | newest on that channel |
| `"24.6.0"` | exact pin | no-op |

Channels are defined per provider ([07. Providers](07-providers.md)); `latest`/`lts`/`nightly` mean what the provider declares, and the resolved exact version is always written to the lock so a channel request is still reproducible at a point in time. Pinning (`"24.6.0"`) is the way to opt out of any movement.

## Atomicity of updates

An update is a normal traversal of the install lifecycle ([08. Installation](08-installation.md)) and is therefore atomic by construction:

```
vanta update node
  [2 Resolve]  re-resolve node within constraint → 24.6.0 (all targets)
  [3-6]        build/fetch the new store entry (the old entry stays present)
  [7 Link]     stage a new environment view
  [8 Commit]   append generation N+1, swap `current`     ◄── the only visible change
               (generation N — with the old node — is retained)
```

Until `[8 Commit]`, the running environment uses the old version; the new version is fully materialized and verified before anything switches. A failure at any earlier stage leaves the user on the old, working version with no partial state. The new and old store entries coexist (content-addressed dedup), so the switch — and a later rollback — costs only a pointer swap.

## Rollback and generations

Every mutation (`add`, `remove`, `update`, `sync`, `restore`) appends an immutable **generation** — a complete, hashable description of an environment (env → tool → store key) plus the command and lock/manifest hashes that produced it. The active generation is a pointer; rollback flips it.

```
$ vanta generations
gen  when                 command                   change
0009 2026-06-29 14:02     vanta update node          node 24.4.1 → 24.6.0   (current)
0008 2026-06-27 09:15     vanta add terraform@1.9    + terraform 1.9.2
0007 2026-06-20 18:40     vanta add python@3.13      + python 3.13.4
...

$ vanta rollback                 # → previous generation (0008)
rolled back to generation 0008 (node 24.4.1). generation 0010 records the rollback.

$ vanta rollback 0007            # → a specific generation
$ vanta rollback --tool node     # roll back just node to its prior version (new generation)
```

- **Rollback is non-destructive and instant:** it appends a new generation that points at already-present store entries and swaps `current`. Nothing is reinstalled; no network is touched.
- **Whole-environment** rollback (default) reverts every tool to that generation's set; **`--tool`** rolls back a single tool by composing a new generation that differs only in that entry.
- Generations are **GC roots** subject to retention (default: last 5 or 30 days, whichever keeps more — canon §9, [09. Store](09-store.md#garbage-collection)). A generation older than retention can be pruned, after which rolling back *to it specifically* requires `vanta sync` against its recorded lock (still reproducible, just not instant). The lock for each generation is retained in history, so even a GC'd generation is reconstructible.
- This is the dnf-history / Nix-generations capability ([33. Prior Art](33-prior-art.md)) delivered in user space with no solver and no DSL.

## Updating Vanta itself

`vanta self update` updates the Vanta binary, distinct from updating managed tools:

```
vanta self update [--channel stable|beta|nightly] [--check]
  1. query the release channel for the newest signed release
  2. download the new binary + its signature + SLSA provenance
  3. verify signature and provenance against pinned release keys      (FAIL CLOSED)
  4. atomically replace ~/.vanta/bin/vanta (stage → rename); keep the prior binary
  5. on a failed post-update self-check, restore the prior binary
```

- The same verification rigor as tool artifacts applies to Vanta's own binary ([32. Release Engineering](32-release-engineering.md)): nothing is swapped in unverified.
- The previous binary is retained so `vanta self update --rollback` restores it.
- Channels (`stable`/`beta`/`nightly`) mirror the release channels; most users stay on `stable`. `--check` reports availability without updating.
- When Vanta was installed by a system package manager (Homebrew/winget/apt), `self update` defers to that manager with a clear message rather than fighting it.

## Update safety

- **Verify-before-swap.** New artifacts are fully verified ([15. Security](15-security.md)) before the generation switch; a bad signature aborts the update with the old environment intact.
- **`--dry-run`** shows the resolved changes and the lock diff without applying.
- **Lock diff preview.** `vanta update` prints a concise diff of `vanta.lock` so the change is reviewable before commit.
- **`--frozen`** (in CI) refuses to update; CI reproduces the committed lock and never drifts ([11. Reproducibility](11-reproducibility.md)).
- **Policy gates.** Enterprise policy can cap versions, require signatures, or forbid channels; an update violating policy fails at resolution ([14. Enterprise](14-enterprise.md)).
- **Guard window (optional).** `[settings] auto_rollback` can revert an update if a configured post-update health check fails, mirroring the safe-reload pattern.

## Pruning

Updates leave the prior versions in the store (they back the retained generations). Disk is reclaimed by `vanta gc`, which removes store entries unreachable from any retained generation or pin ([09. Store](09-store.md#garbage-collection)). Until GC runs, rollback to a recent generation is instant because the old entries are still present — retention is the knob that trades disk for rollback depth.

## Failure scenarios and trade-offs

| Scenario | Behavior |
| --- | --- |
| Update's new artifact fails verification | update aborts at `[5]`; old version stays active; `VTA-VRF-*` |
| Network down mid-update | old version stays active; resume later; `VTA-NET-*` |
| New version is broken at runtime | `vanta rollback` returns to the prior generation instantly |
| Rolled-back-to generation was GC'd | `vanta sync` against its recorded lock reconstructs it |
| `update` would exceed a policy ceiling | fails at resolution with a policy message |

- **Trade-off vs rolling-release managers (brew/pacman).** Vanta never moves a tool you did not ask to move and never leaves you unable to go back; the cost is that "update everything to latest" is an explicit `vanta update`, not an implicit side effect of any command.
- **Trade-off vs immutable-only systems (Nix).** Vanta retains a bounded number of generations rather than all of them by default, trading unbounded history for bounded disk — tunable via retention, and history-of-locks means even pruned generations are reproducible.

## Cross-references

- [02. Architecture](02-architecture.md) — generations, the `current` pointer, and the commit barrier.
- [09. Store](09-store.md) — generations as GC roots and the retention policy.
- [08. Installation](08-installation.md) — the lifecycle an update traverses.
- [11. Reproducibility](11-reproducibility.md) — `--frozen`, lock rewrites, and cross-platform re-resolution.
- [15. Security](15-security.md) — verify-before-swap for both tools and `self update`.
- [32. Release Engineering](32-release-engineering.md) — the signed channels behind `vanta self update`.
- [14. Enterprise](14-enterprise.md) — policy gates on updates and channels.
