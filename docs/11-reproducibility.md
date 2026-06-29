# 11. Reproducibility & Lockfiles

> Reproducibility is the property that turns Vanta from a convenience into infrastructure. This document specifies the reproducibility thesis and what guarantees it, the `vanta.lock` lockfile, **cross-platform locking** (resolving for every target platform so a mixed-OS team shares one lock), `vanta sync` semantics, the precise determinism guarantees and their boundaries, lock verification, and the team/CI workflows that depend on all of it. The exhaustive field reference is [31. Lockfile & Manifest Reference](31-lockfile-and-manifest-reference.md).

**Contents**

- [The reproducibility thesis](#the-reproducibility-thesis)
- [The lockfile](#the-lockfile)
- [Cross-platform locking](#cross-platform-locking)
- [vanta sync](#vanta-sync)
- [Determinism guarantees and boundaries](#determinism-guarantees-and-boundaries)
- [Lock verification](#lock-verification)
- [Team and CI workflows](#team-and-ci-workflows)
- [Comparison](#comparison)
- [Cross-references](#cross-references)

---

## The reproducibility thesis

> Given the same committed `vanta.lock`, Vanta materializes a **byte-for-byte identical** set of tools on any machine and any supported operating system.

This is the property every incumbent half-implements (canon §15, [33. Prior Art](33-prior-art.md)): version managers pin a version string but not the bytes; OS package managers are rolling; Nix achieves it but at the cost of a language and Unix-only reach. Vanta makes full reproducibility the default, achieved by the composition of four mechanisms already specified in other docs:

1. **Exact version pinning** — a request like `node = "24"` resolves once to an exact version recorded in the lock ([06. Resolution](06-resolution.md)).
2. **Artifact-hash pinning** — the lock records the *content hash* (SHA-256 + BLAKE3) of the exact artifact per platform, not just a URL. A changed or substituted artifact fails verification ([15. Security](15-security.md)).
3. **Provider pinning** — the lock records which provider (and provider version) produced the resolution, so the *mapping* from version to artifact is fixed too.
4. **Content-addressed materialization** — the store key is derived from canonicalized content, so identical artifacts produce identical store entries everywhere ([09. Store](09-store.md#hashing-and-canonicalization)).

Pin the version, the bytes, the mapping, and the materialization, and "works on my machine" stops being possible to violate by accident.

## The lockfile

`vanta.lock` is a human-readable TOML file, committed to version control alongside `vanta.toml`. It is the reproducibility contract; `vanta.toml` says *what you want*, `vanta.lock` says *exactly what that resolved to*.

A representative excerpt (full schema in [31. Lock Reference](31-lockfile-and-manifest-reference.md)):

```toml
lock_version = 1
generated_by = "vanta 0.9.2"
registry_revision = "2026-06-20T11:04:00Z#a91f…"   # the registry snapshot resolution used
targets = ["macos/aarch64", "linux/x86_64/gnu", "windows/x86_64"]

[[tool]]
name = "node"
request = "24"                      # the constraint from vanta.toml
version = "24.6.0"                  # the exact resolved version
provider = "official/node@3"        # provider id + provider version

  [tool.platform."macos/aarch64"]
  store_key = "blake3-aa3f9c…"
  url = "https://registry.vanta.dev/node/24.6.0/node-darwin-arm64.tar.xz"
  size = 24117248
  sha256 = "5f2c…"
  signature = "minisign:RWQf…"
  bin = ["bin/node", "bin/npx"]

  [tool.platform."linux/x86_64/gnu"]
  store_key = "blake3-7b21…"
  url = "https://registry.vanta.dev/node/24.6.0/node-linux-x64.tar.xz"
  size = 26214400
  sha256 = "9a14…"
  signature = "minisign:RWQf…"
  bin = ["bin/node", "bin/npx"]

  [tool.platform."windows/x86_64"]
  store_key = "blake3-c4d8…"
  url = "https://registry.vanta.dev/node/24.6.0/node-win-x64.zip"
  size = 28311552
  sha256 = "b0f3…"
  signature = "minisign:RWQf…"
  bin = ["node.exe", "npx.cmd"]
```

Key points: the lock pins **per-platform** store keys and hashes; it records the **registry revision** it resolved against (so a re-resolve can be reproduced); and it is serialized canonically (stable key order, normalized formatting) so it diffs cleanly and merges sanely in VCS.

## Cross-platform locking

This is a signature feature (canon §16). When Vanta resolves, it resolves for **all declared target platforms at once**, not just the machine doing the resolving. A developer on an Apple-Silicon Mac running `vanta add node@24` produces lock entries for macOS *and* Linux *and* Windows (whatever `targets` declares), by reading each platform's artifact metadata from the registry.

```
vanta add node@24   (run on macos/aarch64)
        │ resolve for every target in `targets`
        ▼
  node 24.6.0 ─► macos/aarch64   : artifact + sha256 + store_key
              ─► linux/x86_64/gnu: artifact + sha256 + store_key   ◄─ resolved even though
              ─► windows/x86_64  : artifact + sha256 + store_key      we're not on these
```

- **Targets** default to a sensible set (the common desktop/CI platforms) and are configurable per project (`[settings] targets` / `vanta target add linux/x86_64/musl`). A team adds exactly the platforms its members and CI use.
- The consequence: a Linux teammate and a Windows teammate run `vanta sync` against the *same* committed lock and each gets the right, verified artifact for their OS — **one lock, one reproducible environment, three operating systems**. This is what mise/asdf cannot do (no hashes, single-platform) and what uv does for Python (universal lock) generalized to all tools.
- Resolving for absent platforms only needs *metadata* (versions + artifact descriptors), which is cheap and cached; no foreign-platform artifact is downloaded until that platform actually syncs.

## vanta sync

`vanta sync` is the reconcile-to-declared-state verb — the command that makes a machine match `vanta.toml` + `vanta.lock` exactly. It is what you run after `git clone`, after pulling a lock change, and in CI.

```
vanta sync [--frozen] [--offline] [--platform <p>]
  1. read vanta.toml + vanta.lock (+ merged config)
  2. for this platform: compute the set of store keys the lock requires
  3. install any missing entries (plan→fetch→verify→materialize, see doc 08)
  4. compose the environment view; swap to a new generation
  5. (default) prune env entries not declared; store entries are left to `vanta gc`
```

- **Idempotent.** Running `sync` when already in sync is a few redb reads and does nothing else.
- **`--frozen` (CI mode).** Fails if the lock would need to change to satisfy the manifest (i.e. the manifest and lock are out of agreement). This guarantees CI uses exactly the committed lock and never silently re-resolves — the analog of `npm ci`, `cargo --locked`, `uv sync --frozen`.
- **`--offline`.** Installs entirely from the store and caches; a missing artifact is a clean `VTA-NET-0002` rather than a network call ([13. Offline](13-offline.md)).
- `sync` never changes versions; only `add`/`update` re-resolve and rewrite the lock.

## Determinism guarantees and boundaries

Vanta is explicit about what it does and does not guarantee:

**Guaranteed:**
- For a **prebuilt artifact**, byte-identical reproduction across machines/OSes — the hash in the lock is verified on download, and the canonicalized store entry is identical everywhere.
- Identical **resolution** given the same lock (resolution is read from the lock, not recomputed) and given the same registry revision when re-resolving.
- Identical **environment composition** — the same generation yields the same `PATH` view.

**Bounded (and mitigated):**
- For a **source build**, byte-identical reproduction holds only if the build itself is deterministic. Vanta cannot make an upstream build reproducible. Mitigations: **prebuilt-binary-first** policy ([ADR-0022](24-architecture-decision-records.md)), hash-pinning of all build *inputs*, sandboxed builds (network off after fetch) to remove ambient nondeterminism, and flagging providers whose builds are not reproducible so policy can forbid them.
- **Registry drift** is contained by recording `registry_revision`; a re-resolve at the same revision is reproducible, and the lock (not the live registry) governs `sync`.
- **Platform coverage** is bounded by `targets`: a platform not in the lock has no entry and `sync` on it errors clearly (`VTA-LOCK-0003`, "no locked entry for this platform"), prompting `vanta add`/`vanta target add` to extend the lock.

## Lock verification

The lock is checked, not trusted blindly:

- **Drift detection.** If `vanta.toml` changed but the lock was not updated (a new tool, a changed constraint that the lock no longer satisfies), commands report `VTA-LOCK-0001` and `--frozen` fails. `vanta lock` / `vanta update` regenerate it deliberately.
- **Tamper detection.** Every artifact is verified against the lock's hash at install; a mismatch (`VTA-VRF-0001`) means the artifact or the lock was altered.
- **Completeness.** `vanta lock --check` verifies every declared tool has an entry for every declared target, with a hash and signature, and that store keys are internally consistent.
- **Merge safety.** Canonical serialization keeps lock diffs minimal and ordered; a VCS merge conflict in `vanta.lock` is resolved by `vanta lock` (re-deriving from the manifest at the recorded revision), not by hand-editing hashes ([31. Lock Reference](31-lockfile-and-manifest-reference.md)).

## Team and CI workflows

```
# author a change
vanta add terraform@1.9      # resolves for all targets, writes vanta.toml + vanta.lock
git commit vanta.toml vanta.lock

# teammate (any OS) reproduces exactly
git pull
vanta sync                   # installs the locked artifacts for their platform

# CI — reproducible and strict
vanta sync --frozen          # fails if lock != manifest; uses only committed pins
# cache key: hash(vanta.lock) → cache ~/.vanta/store across runs

# deliberate upgrade
vanta update terraform       # re-resolves within the constraint, rewrites the lock
git commit vanta.lock
```

Onboarding a new engineer is `git clone && vanta sync` — no per-tool installation, no version drift, identical on macOS, Linux, and Windows. CI caches `~/.vanta/store` keyed by the lock hash so warm runs are near-instant ([18. Developer Experience](18-developer-experience.md)).

## Comparison

| Capability | Vanta | mise/asdf | uv | Nix/Flox |
| --- | --- | --- | --- | --- |
| Pins exact version | ✅ | ✅ | ✅ | ✅ |
| Pins artifact hash | ✅ | ❌ | ✅ | ✅ |
| One lock covers all OSes | ✅ | ❌ (per-platform/none) | ✅ (Python) | ✅ |
| Reproduces without a DSL | ✅ | ✅ (but weak) | ✅ | ❌ (Nix language) |
| Native Windows | ✅ | ⚠️/❌ | ✅ | ❌ |
| `--frozen` CI mode | ✅ | ⚠️ | ✅ | ✅ |

Vanta delivers Nix-grade practical reproducibility for tools — hashed, cross-platform, verified — with a TOML lock anyone can read and no functional language to learn, and it is the only option in the table that is reproducible *and* cross-platform including native Windows.

## Cross-references

- [31. Lockfile & Manifest Reference](31-lockfile-and-manifest-reference.md) — every lock field and the canonical serialization rules.
- [06. Resolution](06-resolution.md) — how a request becomes the locked resolution, and resolving for all targets.
- [09. Store](09-store.md) — canonical hashing that makes materialization reproducible.
- [08. Installation](08-installation.md) — `vanta sync`'s install path and the verification gate.
- [15. Security](15-security.md) — artifact-hash and signature verification behind the lock.
- [13. Offline](13-offline.md) — `--offline` sync and air-gapped reproduction.
- [18. Developer Experience](18-developer-experience.md) — the clone→sync onboarding and CI caching.
