# 06. Resolution & Version Management

> Resolution is the deterministic function that turns a human Request (`node@24`,
> `terraform@latest`, `rust`) into a concrete, lockable Resolution: an exact version, the
> provider that serves it, and a per-platform set of verified artifact descriptors. This
> document specifies the request grammar, the `[2 Resolve]` algorithm, version ordering and
> comparators, declared-dependency resolution, scope and automatic version switching, the
> re-resolution rules that preserve reproducibility, cross-platform resolution, and every
> resolution failure mode mapped to a `VTA-RES-*` code. It is implemented by the
> `vanta-resolve` crate over `vanta-registry`, `vanta-provider`, and `vanta-lock`.

**Contents**

- [Request grammar](#request-grammar)
- [Resolution algorithm: stage [2 Resolve]](#resolution-algorithm-stage-2-resolve)
- [Version ordering and comparators](#version-ordering-and-comparators)
- [Dependency resolution](#dependency-resolution)
- [Scope and version selection](#scope-and-version-selection)
- [Automatic version switching](#automatic-version-switching)
- [Re-resolution triggers and lock authority](#re-resolution-triggers-and-lock-authority)
- [Cross-platform resolution](#cross-platform-resolution)
- [Failure modes](#failure-modes)
- [Rejected alternatives](#rejected-alternatives)
- [Cross-references](#cross-references)

---

## Request grammar

A **Request** is what the user asks for, written `name@version` where `@version` is optional.
The `name` selects a tool (or one of its aliases) in some registry; the `version` part is a
**version specifier** parsed into a `VersionSpec`. The grammar is intentionally small: it must
be writable by hand in a shell and in `vanta.toml`, unambiguous, and identical across all tools.

```
request    := name [ "@" spec ]
name       := tool-name | alias                 ; e.g. node, py (alias), ripgrep
spec       := exact | prefix | range | channel | "system"
exact      := <version literal accepted by the provider comparator>   ; 24.3.0, 1.21.6
prefix     := <dotted numeric prefix>                                  ; 24, 24.3
range      := <SemVer range expression>                                ; ^24, ~24.3, ">=20 <24", 20.x
channel    := "latest" | "lts" | <provider-declared channel>          ; stable, nightly, beta
```

### Parsing rules

The specifier is classified by the following ordered rules. The first matching rule wins; this
ordering is the canonical ambiguity resolution and is part of the spec.

| Order | If the specifier… | Classified as | Example |
|------|--------------------|---------------|---------|
| 1 | is empty (no `@`) | **Scope/lock default**, else `latest` | `node` |
| 2 | equals `system` | `VersionSpec::System` | `python@system` |
| 3 | equals `latest` or `lts` | `VersionSpec::Channel` (reserved) | `go@lts` |
| 4 | equals a provider-declared channel name | `VersionSpec::Channel` | `rust@nightly` |
| 5 | contains any range operator `^ ~ > < = * x` or whitespace/comma | `VersionSpec::Range` | `node@^24`, `node@">=20 <24"` |
| 6 | is a complete version literal the comparator fully parses | `VersionSpec::Exact` | `node@24.3.0` |
| 7 | is a dotted numeric prefix of a version | `VersionSpec::Prefix` | `node@24`, `node@24.3` |
| 8 | otherwise | parse error `VTA-RES-0001` | `node@twenty` |

Notes on the ordering:

- **Prefix vs. exact (rules 6–7).** A bare `24` is a *prefix*, not the exact version `24.0.0`,
  because almost no user means "the literal build 24.0.0" when they type `24`. `24` resolves to
  the newest stable version whose first component is `24`. A user who truly wants a single build
  pins the full literal (`24.0.0`) or uses the lock.
- **Range detection (rule 5)** is purely syntactic: the presence of a range token. `24.x` and
  `20.*` are ranges; `24` is a prefix. We deliberately keep `24` out of the range grammar so the
  common case has the simplest spelling.
- **Channels (rules 3–4)** are *names*, never versions. `latest` and `lts` are reserved across
  all tools; additional channel names are declared by the provider (see
  [version ordering](#version-ordering-and-comparators)). Requesting a channel a tool does not
  define is `VTA-RES-0004`.
- **`system` (rule 2)** is a sentinel: it instructs the resolver to bind to a tool already on the
  host (`/usr/bin/python3`, a system `git`), recording *that it is system-provided and at what
  observed version* in the lock, but not materializing it into the store. It is the explicit
  escape hatch from management; everything else is managed.

### Examples

| Request | `VersionSpec` | Resolves to (illustrative) |
|---------|---------------|----------------------------|
| `node` (in a project) | lock value, else `latest` | `node@24.3.0` from `vanta.lock` |
| `node@24` | Prefix `24` | newest stable `24.*.*`, e.g. `24.3.0` |
| `node@24.3` | Prefix `24.3` | newest stable `24.3.*`, e.g. `24.3.0` |
| `node@24.3.0` | Exact | exactly `24.3.0` (or fail `VTA-RES-0002`) |
| `node@^24` | Range | newest `>=24.0.0 <25.0.0` |
| `node@">=20 <24"` | Range | newest `>=20.0.0 <24.0.0` |
| `node@lts` | Channel `lts` | newest version flagged `lts=true` |
| `terraform@latest` | Channel `latest` | newest non-prerelease |
| `rust@nightly` | Channel `nightly` | head of the `nightly` channel |
| `python@system` | System | the host `python3`, recorded as `system` |

`VersionSpec` and `Request` are defined in `vanta-core` and parsed in `vanta-resolve`:

```rust
pub struct Request {
    pub name: String,           // as typed; alias resolution happens in [2 Resolve]
    pub spec: VersionSpec,
    pub scope: Scope,           // Project(path) | Global, inferred or forced
    pub targets: Vec<Platform>, // platforms to resolve for (see cross-platform resolution)
}

pub enum VersionSpec {
    Default,                    // no @ given: defer to lock/scope, else latest
    Exact(Version),             // 24.3.0
    Prefix(Vec<u64>),           // [24] or [24, 3]
    Range(VersionReq),          // ^24, >=20 <24, 20.x
    Channel(String),            // "latest" | "lts" | provider-declared
    System,                     // bind to a host-provided tool
}
```

## Resolution algorithm: stage [2 Resolve]

`[2 Resolve]` is the second stage of the canonical lifecycle (canon §4). Its contract: **given a
Request, the registry revision, and the comparator/dependency declarations of the involved
providers, produce exactly one `Resolution` (or one deterministic error).** No I/O performed
here mutates state; resolution is a pure function of its inputs plus read-only registry/provider
metadata, which makes it cacheable and reproducible.

```
                      ┌──────────────────────────────────────────────────────────┐
 [1 Request] ───────► │ [2 Resolve]                                               │
  name@spec, scope,   │                                                           │
  target platforms    │  (a) name → provider    (registry lookup, alias, precedence)
                      │  (b) candidates ← provider.list-versions()                │
                      │  (c) filter: comparator-valid, prerelease policy,         │
                      │              yanked, min-vanta-version                     │
                      │  (d) select: apply spec → set of satisfying versions      │
                      │  (e) pick:   max under provider comparator                 │
                      │  (f) per platform p in targets:                           │
                      │        artifact[p] ← provider.resolve(version, p)         │
                      │  (g) expand declared deps → DAG → consistent set          │
                      │  (h) pin: version + provider id + manifest rev + artifacts │
                      └──────────────────────────────────────────────────────────┘
                                              │
                                              ▼
                                   Resolution ──► [3 Plan]
```

Step by step:

1. **(a) Name → provider.** The tool name (or alias) is looked up in the merged registry index.
   Registries are consulted in precedence order (see [scope](#scope-and-version-selection) and
   [07. Providers & Registry](07-providers.md)). If a bare name matches tools served by more
   than one provider across registries with no precedence winner, resolution fails with
   `VTA-RES-0003` (ambiguous) listing the candidates and the disambiguating `registry/name`
   syntax. If the name matches nothing, the registry layer returns `VTA-REG-0003` (not found).
2. **(b) Candidate versions.** The provider yields its full version list. For declarative
   providers this is computed from the version source (e.g. GitHub release tags) and cached; for
   providers with a WASM hook it is `list-versions()` run in the sandbox (see
   [22. Provider SDK & ABI](22-provider-sdk.md)). Candidates are cached in
   `~/.vanta/cache/registry/` keyed by provider id + manifest revision.
3. **(c) Filter.** Drop versions the comparator cannot parse (provider bug → surfaced as
   `VTA-RES-0005` only if it would otherwise be selected), versions excluded by the provider's
   `prerelease` policy unless the spec explicitly opts in, **yanked** versions (unless the exact
   yanked version is named *and* `--allow-yanked` is set, else `VTA-RES-0010`), and versions whose
   `min-vanta-version` exceeds the running binary (`VTA-RES-0011`).
4. **(d) Select.** Apply the `VersionSpec` to the surviving set: `Exact` keeps the equal version;
   `Prefix` keeps versions sharing the dotted prefix; `Range` keeps versions satisfying the
   `VersionReq`; `Channel` keeps versions tagged for that channel; `System` short-circuits to the
   host binding. The empty result is `VTA-RES-0002` ("no version satisfies").
5. **(e) Pick.** Take the **maximum** surviving version under the provider's comparator. Ordering
   is total (ties are impossible because version strings are unique per tool), so the pick is
   deterministic.
6. **(f) Per-platform artifacts.** For every target platform, ask the provider to map the chosen
   version to an artifact descriptor (URLs/mirrors, archive kind, checksum source, signature
   source, layout, bin entries, env). A platform with no artifact is `VTA-RES-0008` unless the
   provider marks that platform optional.
7. **(g) Dependencies.** Expand declared tool dependencies into a DAG and select a consistent set
   (see [dependency resolution](#dependency-resolution)).
8. **(h) Pin.** Emit a `Resolution` recording the exact version, provider id, the **manifest
   revision** that produced it, and the per-platform artifact descriptors. This is the unit
   written to `vanta.lock` and consumed by `[3 Plan]`.

```rust
pub struct Resolution {
    pub name: String,
    pub version: Version,
    pub provider: ProviderId,        // e.g. "official.nodejs.node"
    pub provider_rev: ManifestRev,   // content hash of the provider manifest used
    pub artifacts: BTreeMap<Platform, ArtifactDescriptor>, // one per target platform
    pub deps: Vec<ResolvedDep>,      // resolved, pinned dependency edges
    pub source: SourceKind,          // Channel/Range/Exact/System provenance for audit
}

pub trait Resolver {
    fn resolve(&self, req: &Request, ctx: &ResolveCtx) -> Result<Resolution, ResolveError>;
    fn resolve_all(&self, reqs: &[Request], ctx: &ResolveCtx)
        -> Result<ResolutionSet, ResolveError>; // batched, shares the dep DAG
}
```

**Determinism guarantee.** Two resolutions of the same `Request` against the same *registry
revision* and the same lock yield byte-identical `Resolution`s on any machine, because every
input is content-addressed: the registry index is signed and snapshotted (a monotonic revision),
provider manifests are pinned by content hash, comparators are pure, and the pick is `max` over a
total order. This is the foundation of [11. Reproducibility & Lockfiles](11-reproducibility.md).

## Version ordering and comparators

A **comparator** is declared by the provider; the resolver never assumes a global versioning
scheme. The comparator defines how a version string parses, how two versions order, what counts
as a prerelease, and whether a `VersionReq` is satisfied.

```rust
pub trait Comparator: Send + Sync {
    fn parse(&self, s: &str) -> Result<Version, ParseError>;
    fn cmp(&self, a: &Version, b: &Version) -> std::cmp::Ordering; // total order
    fn is_prerelease(&self, v: &Version) -> bool;
    fn satisfies(&self, v: &Version, req: &VersionReq) -> bool;
}
```

| Comparator | Declared as | Ordering | Used by (examples) |
|------------|-------------|----------|--------------------|
| **SemVer** (default) | `comparator = "semver"` | `major.minor.patch` with prerelease rules (RFC: `1.0.0-rc.1 < 1.0.0`) | node, go (mapped), terraform, ripgrep |
| **CalVer** | `comparator = "calver"` | numeric date components, e.g. `2024.11` | tools versioned by date |
| **Lexical** | `comparator = "lexical"` | natural/lexical with numeric segment awareness | odd upstreams without SemVer |
| **Custom** | `comparator = "custom"` + WASM hook | provider-supplied `cmp`/`parse` via the sandboxed hook | rust channels, exotic schemes |

`semver` is the default because the large majority of managed tools publish SemVer or a SemVer
prefix; we map near-SemVer schemes (e.g. Go's `go1.21.6`) to SemVer in the provider rather than
inventing a comparator. `custom` is the escape hatch: the comparator runs as part of the
provider's WASM hook (`compare(a, b) -> ordering`) under the same sandbox and fuel limits as the
rest of the hook, so a buggy comparator cannot diverge resolution non-deterministically — it is
pure and bounded. A version that the chosen comparator fails to parse *and* that would be selected
is `VTA-RES-0005`; unparseable versions that are never selected are silently dropped.

### Prerelease handling

Each provider declares a `prerelease` policy: `exclude` (default), `include`, or `channel-only`.

- **`exclude`** — prereleases never appear in `latest`, prefix, or range results. They are only
  reachable by an exact pin (`node@25.0.0-rc.1`).
- **`include`** — prereleases participate in ordering and may be selected by ranges/`latest`.
- **`channel-only`** — prereleases are reachable solely via a named channel (e.g. `rust@beta`).

A range or channel may opt in locally: `node@^25.0.0-0` follows SemVer's rule that a prerelease
lower bound admits prereleases of that version line. The default `exclude` implements the
principle "the simplest path is the safe path": `vanta add node` never silently installs a
release candidate.

### Defining `latest` and `lts`

`latest` and `lts` are **reserved channels** with provider-supplied definitions:

- **`latest`** defaults to "the maximum version under the comparator after the `prerelease`
  filter." A provider may override it (e.g. point `latest` at a `stable` channel head).
- **`lts`** has no universal meaning, so it is **data**, not policy: the registry's version
  metadata carries an `lts: bool` flag per version (and optionally an `lts_codename`), and `lts`
  resolves to the maximum version with `lts = true`. A tool with no LTS line that is asked for
  `lts` fails with `VTA-RES-0004`. This keeps Node's even-major LTS, Java's LTS majors, and Go's
  "two latest" support model expressible without bespoke per-tool code.

## Dependency resolution

Most tools are **independent leaves**: `ripgrep`, `terraform`, and `node` depend on nothing Vanta
manages. A minority declare dependencies on *other Vanta tools* (e.g. a JVM-based CLI that needs
`java`, a Node-based CLI that needs `node`). Providers declare these as edges:

```toml
[[deps]]
tool    = "java"
version = ">=17"        # a VersionSpec in the dependency's own grammar
```

The resolver builds a directed graph from the requested roots, expands each provider's declared
deps, and selects a consistent set:

```
   terraform@1.9.5  (leaf)            some-cli@2.0.0
                                           │ deps: java >=17
                                           ▼
                                     java  ── selected: 21.0.4  (max satisfying >=17, also
                                           ◄── another-cli deps: java ^21  satisfies both)
```

Algorithm (in `vanta-resolve`):

1. Topologically expand the DAG from the requested roots (depth-first, memoized per
   `name → already-resolved version`).
2. For each tool, intersect all incoming `VersionReq`s. If the intersection is empty, emit
   `VTA-RES-0006` (conflicting dependency constraints) with the conflicting edges and their
   originating providers.
3. Pick the maximum version satisfying the intersection (same `[2 Resolve]` machinery, applied to
   the dependency rather than a user request).
4. Detect cycles during expansion; a cycle is `VTA-RES-0007`.

**One version per tool per resolution.** Within a single environment a tool resolves to exactly
one version even if reached by several paths (the intersection rule). Different *environments* may
hold different versions; the store keeps them side by side. This is sufficient because Vanta tools
are coarse-grained executables selected on `PATH`, not fine-grained libraries linked into one
address space — there is no "diamond dependency in a shared closure" to satisfy.

### Why Vanta does not run a full SAT/PubGrub solver

A distribution package manager (apt, dnf) or a language package manager (Cargo, npm) must solve a
**shared-closure** problem: hundreds of interdependent libraries must co-exist in one build/link
graph, so version selection is NP-hard and warrants SAT/PubGrub. Vanta's problem is different:

| Property | Distro/library manager | Vanta |
|----------|------------------------|-------|
| Unit of management | library in a shared closure | independent tool executable |
| Co-existence of versions | usually one global version | many versions side by side in the store |
| Typical graph | dense, deep, conflicting | shallow, mostly leaves |
| Failure on conflict | must backtrack to find *some* solution | report the conflict; the user resolves it |

Because tools are versioned independently and installed side by side, the only real constraint is
*per-tool intersection*, which is linear, explainable, and fast. We therefore use the simple DAG
+ intersection algorithm above and **reject** importing a SAT/PubGrub engine: it would add a large
dependency and opaque backtracking ("why did it pick X?") to solve a problem we do not have. If a
future provider ecosystem grows genuinely conflicting deep graphs, the `Resolver` trait lets us
slot a PubGrub backend behind it without changing the request/lock model — but that is explicitly
out of scope until the data demands it (see [24. ADRs](24-architecture-decision-records.md)).

## Scope and version selection

**Scope** decides *which manifest* a resolution is recorded in and *which version is active* in a
context. Canon §3 defines two scopes:

- **Project** — the nearest `vanta.toml` walking up from the cwd. `vanta add` here writes to that
  `vanta.toml` and its sibling `vanta.lock`.
- **Global** — `~/.vanta/config.toml`'s `[tools]` table, active everywhere a project does not
  override.

Scope is **inferred** (inside a project → project; otherwise → global) and overridable with
`--project` / `--global`. The selection precedence for "what version is active for tool T in
directory D" is, low → high (canon §6):

```
built-in default  <  global [tools]  <  project vanta.toml (nearest, walking up)
                  <  [env]/per-dir overrides  <  VANTA_* env vars  <  CLI flags
```

Project manifests **merge over** global rather than replacing it: a project that pins only `node`
still inherits the user's global `ripgrep`. Multiple project manifests up the tree merge nearest
-wins (a `vanta.toml` in `repo/services/api/` overrides one in `repo/`). Because every selected
version is materialized as its own immutable store entry (`blake3-<hex>`), there is never an
install-time conflict between, say, `node@20` in one project and `node@24` in another; they are
distinct entries linked into distinct environments (see [09. Store](09-store.md)).

## Automatic version switching

Resolution determines *which version wins for a given cwd*; the *mechanism* that exposes it on
`PATH` (shell hook + shims) is specified in [10. Environments & Activation](10-environments.md).
The selection semantics, owned here, are:

1. From the cwd, walk up to the filesystem root collecting every `vanta.toml`.
2. Merge them nearest-wins, then merge the result over the global `[tools]` table.
3. For each tool in the merged set, the active version is its locked resolution
   (lock-authoritative; see below). The composed environment is the union of these versions'
   store entries.
4. A tool requested as `system` resolves to the host binary discovered on the underlying `PATH`
   at activation time and is **not** shadowed by a managed version; if no system binary is found,
   activation reports `VTA-RES-0009`.
5. **`system` fallback for unmanaged tools:** a tool present on the host but not declared in any
   scope is left untouched — Vanta only manages what is declared, so existing `git`/`cc` keep
   working.

Activation reads a per-directory **resolution cache** keyed by the hash of the merged config, so
the steady-state cost of `cd` is a single hash compare and a `PATH` swap (sub-millisecond, canon
§11). The cache is invalidated by any of the [re-resolution triggers](#re-resolution-triggers-and-lock-authority).

## Re-resolution triggers and lock authority

The lock is **authoritative for reads**. Everyday operations (`sync`, `run`, `x`, `exec`,
activation, `which`) never re-resolve: they read the pinned `Resolution` from `vanta.lock` and act
on it. Re-resolution — running `[2 Resolve]` against live registry state — happens only on:

| Trigger | What re-resolves | Lock effect |
|---------|------------------|-------------|
| `vanta add <tool>[@spec]` | the added tool (+ its deps) | new/updated lock entry |
| `vanta update [tool ...]` (`up`) | the named tools, or all, within their manifest constraints | bumped entries |
| `vanta use <tool>@<ver>` | that tool (sugar over `add`) | updated entry |
| any command with `--refresh` | refreshes registry metadata, then re-resolves the targeted requests | possibly bumped |
| editing `vanta.toml` then `vanta sync` | tools whose constraint changed or that are missing from the lock | reconciled |

`vanta sync` is the reconciliation verb: it makes the machine match `vanta.toml` + `vanta.lock`
*exactly*, re-resolving only entries the manifest added or whose constraint the lock no longer
satisfies, and otherwise trusting the lock. This split — **re-resolve on explicit intent,
read-lock otherwise** — is what makes a `git clone && vanta sync` reproduce a teammate's
environment byte-for-byte while still letting `vanta update` move forward deliberately. See
[11. Reproducibility & Lockfiles](11-reproducibility.md) and [12. Updates & Rollback](12-updates.md).

If the lock and manifest disagree in a way that cannot be honored without re-resolution (e.g. the
manifest now says `^25` but the lock pins `24.3.0`), a read-only command reports the drift and
points to `vanta sync`/`vanta update` rather than silently re-resolving (`VTA-LOCK-*`, catalogued
in [25. Error & Exit-code Catalog](25-error-and-exit-code-catalog.md)).

## Cross-platform resolution

Resolution is performed for **all declared target platforms**, not just the host, so that
`vanta.lock` is portable: a macOS developer and a Linux CI runner share one lock and one set of
pins. The target set comes from `[settings] platforms` in `vanta.toml` (defaulting to the host
platform plus any platforms already present in the lock) or `--platform` flags.

```
            request: node@24
            targets: [ linux/x86_64/gnu, macos/aarch64, windows/x86_64 ]
                                  │
              pick version 24.3.0 (one version for all platforms)
                                  │
        ┌─────────────────┬───────┴───────────┬────────────────────┐
        ▼                 ▼                   ▼                    ▼
 linux/x86_64/gnu   macos/aarch64        windows/x86_64      (each gets its own
 artifact: .tar.xz  artifact: .tar.gz    artifact: .zip       URL + hash + sig)
```

The **version** is chosen once (a tool is the same version everywhere); only the **artifacts**
differ per platform. If the chosen version lacks an artifact for a *declared* platform, resolution
fails with `VTA-RES-0008` rather than silently picking a different version on that platform —
divergent versions across a team would defeat the purpose of the shared lock. Platform identifier
tokens (`linux/x86_64/gnu`, `macos/aarch64`, `windows/x86_64`, …) are canonicalized in
[26. Registry & Package Metadata Reference](26-registry-and-metadata-reference.md) and consumed
via `vanta-platform`.

## Failure modes

Every resolution failure is a typed `ResolveError` carrying a `VTA-RES-*` code, the offending
request, and an actionable remedy. Resolution failures exit with code `4` (canon §13).

| Code | Condition | Typical cause | Remedy surfaced |
|------|-----------|---------------|-----------------|
| `VTA-RES-0001` | Unparseable version specifier | typo (`node@twenty`) | show grammar + nearest valid form |
| `VTA-RES-0002` | No version satisfies the constraint | over-tight range, future version | list nearest available versions |
| `VTA-RES-0003` | Ambiguous tool name across providers/registries | name served by 2+ registries | print `registry/name` candidates |
| `VTA-RES-0004` | Channel/`lts` not defined for tool | `go@lts` where Go declares none | list the tool's channels |
| `VTA-RES-0005` | Comparator rejected a would-be-selected version | provider/upstream version anomaly | report provider id + version |
| `VTA-RES-0006` | Conflicting dependency constraints (empty intersection) | two providers demand incompatible dep ranges | print the conflicting edges |
| `VTA-RES-0007` | Dependency cycle | mis-declared provider deps | print the cycle path |
| `VTA-RES-0008` | No artifact for a declared target platform | upstream ships no build for that triple | drop platform or pin a version that has it |
| `VTA-RES-0009` | `system` tool requested but absent/unusable | no host binary on `PATH` | install it or pick a managed version |
| `VTA-RES-0010` | Selected version is yanked | upstream pulled the release | choose another or `--allow-yanked` |
| `VTA-RES-0011` | Version requires a newer Vanta (`min-vanta-version`) | old binary, new package metadata | `vanta self update` |
| `VTA-RES-0012` | Offline and no cached candidates for the request | `--offline`/no network, cold cache | go online once, or `vanta restore` a bundle |

**Provider unreachable → offline cache.** When the registry or a provider's version source cannot
be reached, the resolver does **not** fail immediately. It transparently falls back to the cached
registry/provider metadata in `~/.vanta/cache/registry/` and resolves from it, annotating the
result as "resolved from cache (offline)". The underlying transport error is `VTA-NET-0001`
(network/offline, exit code `5`), surfaced only if the cache also cannot satisfy the request — in
which case the user-facing resolution error is `VTA-RES-0012`. This layering means a fully cached
project resolves and installs with the network down; only genuinely-new requests need the network.
Mirror and retry behavior for the fetch stages is specified in
[13. Offline, Mirrors & Air-gapped](13-offline.md) and [07. Providers & Registry](07-providers.md).

## Rejected alternatives

- **Treat a bare `24` as exact `24.0.0`.** Rejected: it surprises users (they mean "the 24 line")
  and would make `vanta add node@24` brittle the moment `24.0.1` ships. Prefix semantics match
  intent; full pins remain available.
- **A single global version comparator (SemVer everywhere).** Rejected: real tools use CalVer and
  ad-hoc schemes. A provider-declared comparator keeps the resolver simple while covering reality;
  `semver` remains the default so the common case needs no configuration.
- **Full SAT/PubGrub dependency solving.** Rejected for the reasons in
  [dependency resolution](#dependency-resolution): Vanta manages independently-versioned,
  side-by-side tools, not a shared library closure, so per-tool intersection suffices and stays
  explainable. The `Resolver` trait keeps the door open if the data ever changes.
- **Re-resolve on every command for "freshness."** Rejected: it would make every `cd` and every
  CI run non-deterministic and network-bound, violating reproducibility and the sub-millisecond
  activation target. Re-resolution is gated to explicit intent; the lock is authoritative.
- **Per-platform version selection (pick whatever each OS has).** Rejected: it silently diverges a
  team's environment. We pick one version for all declared platforms and fail loudly
  (`VTA-RES-0008`) if a platform cannot provide it.

## Cross-references

- [04. CLI & Command Design](04-cli.md) — how `add`/`update`/`use`/`sync` trigger resolution.
- [05. Configuration & Manifests](05-configuration.md) — `vanta.toml` tool requests, scope, and `[settings] platforms`.
- [07. Providers & Registry](07-providers.md) — where candidate versions and artifacts come from, and registry precedence.
- [10. Environments & Activation](10-environments.md) — the mechanism that exposes the resolved version on `PATH`.
- [11. Reproducibility & Lockfiles](11-reproducibility.md) — how `vanta.lock` pins resolutions and guarantees byte-identical results.
- [12. Updates & Rollback](12-updates.md) — the `update`/`outdated`/`rollback` flows over re-resolution.
- [22. Provider SDK & ABI](22-provider-sdk.md) — `list-versions`/`resolve`/comparator hooks used by `[2 Resolve]`.
- [25. Error & Exit-code Catalog](25-error-and-exit-code-catalog.md) — the full `VTA-RES-*` and `VTA-LOCK-*` registry.
