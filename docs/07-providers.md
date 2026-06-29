# 07. Providers & Registry

> A **provider** is the declarative definition of how to discover, fetch, verify, lay out, and
> expose one tool (or family). A **registry** is the signed, content-addressed index that says
> which tools exist and which provider serves each. Together they are Vanta's answer to "where
> does software come from," abstracted so the user never names a backend, a URL, or a shell
> script — they type `vanta add <tool>`. This document specifies the provider model
> (declarative-first, WASM for the hard cases), the five backends, resolution order across
> registries, the registry index format and its incremental distribution, tool metadata, the
> plugin/publishing model, private registries, versioning/compatibility, and the security
> posture. Implemented by `vanta-registry` and `vanta-provider`.

**Contents**

- [The provider model](#the-provider-model)
- [Backends](#backends)
- [Resolution order across registries and providers](#resolution-order-across-registries-and-providers)
- [Registry architecture](#registry-architecture)
- [Tool and package metadata](#tool-and-package-metadata)
- [Plugin architecture](#plugin-architecture)
- [Private registries](#private-registries)
- [Provider and registry versioning](#provider-and-registry-versioning)
- [Security posture](#security-posture)
- [Trade-offs and failure modes](#trade-offs-and-failure-modes)
- [Cross-references](#cross-references)

---

## The provider model

A provider answers a fixed set of questions about one tool:

```
list-versions()                  → which versions exist?
resolve(version, platform)       → which artifact (url, archive, checksum, sig, layout, bins, env)?
verify(version, platform, bytes) → is this artifact authentic? (beyond the declared checksum/sig)
layout / bin-paths / env / deps  → how is it materialized and exposed?
```

Vanta is **declarative-first**: the common case is a TOML manifest with a URL template and a
checksum source — no code at all. When a tool's discovery or resolution logic cannot be expressed
declaratively (custom version comparators, paginated or computed release listings, irregular asset
naming, content negotiation), the provider attaches a **sandboxed WASM hook** that implements the
same four functions against a capability-scoped host ABI. The manifest format is specified in
[22. Provider SDK & ABI](22-provider-sdk.md); the exhaustive field reference is in
[26. Registry & Package Metadata Reference](26-registry-and-metadata-reference.md).

```
            ┌──────────────────────────────────────────────┐
 Request    │ Provider (one tool/family)                    │
 node@24 ──►│  manifest.toml  ── declarative path (default) │──► ArtifactDescriptor(s)
            │      └─ optional hook.wasm ── escape hatch     │     per platform
            └──────────────────────────────────────────────┘
                         │ uses
                         ▼
            ┌──────────────────────────────────────────────┐
            │ Backend (how bytes are discovered + fetched)  │
            │  registry | github-releases | direct-url |    │
            │  language-registry | os-package-manager       │
            └──────────────────────────────────────────────┘
```

### Why declarative-first + WASM, not arbitrary shell (asdf/mise)

asdf and mise plugins are arbitrary shell scripts (`bin/list-all`, `bin/install`) executed with
the user's full privileges. Vanta rejects that model for four concrete reasons:

| Concern | asdf/mise shell plugin | Vanta provider |
|---------|------------------------|----------------|
| **Security** | runs with full user rights; can read `~/.ssh`, exfiltrate, persist | declarative manifest does nothing; WASM hook is sandboxed with no ambient fs/net/env (see [22](22-provider-sdk.md)) |
| **Reliability** | depends on host `curl`/`grep`/`jq`/`sed` versions; breaks subtly | manifest is data; hook is a deterministic, fuel-bounded function |
| **Cross-platform** | bash does not exist on Windows; plugins are POSIX-only | manifest + WASM run identically on Linux/macOS/Windows |
| **Reviewability** | a script is hard to audit and re-audit on update | a manifest diffs cleanly; a hook is signed and capability-scoped |

The cost is expressiveness: a manifest cannot do *anything*, and a hook can only do what the host
ABI permits. We accept this deliberately — the overwhelming majority of tools fit the declarative
path, and the few that do not are exactly the ones where a reviewable, sandboxed hook is most
valuable. This is canon §16 innovation #4: "declarative, WASM-sandboxed providers."

## Backends

A **backend** is *how a provider discovers versions and fetches bytes*. The **user never selects a
backend** — the provider declares one in `[source] backend = "…"`. Vanta ships five:

| Backend | What it is | Good for | Version discovery | Trust / verification story |
|---------|-----------|----------|-------------------|----------------------------|
| **`registry`** | Curated artifacts referenced directly by the signed registry index (Vanta-hosted or mirrored) | first-party, high-traffic tools needing strong guarantees | from the index's version metadata | strongest: hashes + signatures live in TUF-signed targets metadata; provenance (SLSA) attached |
| **`github-releases`** | GitHub Releases assets for a `owner/repo` | the large class of CLIs released as GitHub assets | release tags via the GitHub API (cached, conditional GET) | `sha256sums`/`.sig` assets when published; GitHub artifact attestations when present; else checksum-on-first-fetch + TOFU |
| **`direct-url`** | A templated URL to a binary/archive (ubi-style) | upstreams with stable download URLs but no API | enumerated by the manifest's known versions or a hook | checksum source declared in the manifest (sidecar file or pinned hash); signature if upstream publishes one |
| **`language-registry`** | A language package index (npm, PyPI, crates.io, RubyGems) used to fetch a *tool* (not a library graph) | CLIs distributed as language packages (`prettier`, `ruff` wheels) | the index's version API | the index's own integrity (npm `integrity`, PyPI hashes); optional sigstore where the ecosystem provides it |
| **`os-package-manager`** | Delegates to a system package manager (apt/dnf/brew/winget) — **last resort** | tools only distributed as OS packages | the OS manager's metadata | the OS manager's signing (apt keys, etc.); Vanta records what was requested but cannot content-address the result |

Design notes:

- **`registry` is the gold path** for tools where Vanta (or an enterprise mirror) curates and signs
  artifacts: it gives content-addressed, signed, provenance-backed installs that resolve and verify
  offline. Most "blessed" toolchains (node, go, python builds, rust) are served this way.
- **`os-package-manager` is explicitly last resort** because its results are not content-addressed
  or reproducible — the same request yields different bytes across hosts and time. It exists for
  the long tail (system libraries, GUI deps) where no portable artifact exists. Such installs are
  flagged non-reproducible in the lock and excluded from `vanta bundle` byte-for-byte guarantees.
- Backends compose with verification uniformly: whatever the backend, the resulting
  `ArtifactDescriptor` carries a checksum (required) and an optional signature/provenance, which
  `[5 Verify]` enforces per the trust policy in [15. Security & Supply Chain](15-security.md).

## Resolution order across registries and providers

Vanta abstracts *where software comes from* behind a single lookup. A name resolves to a provider
through an ordered list of registries:

```
 name "node"
     │
     ▼
 for each registry in precedence order (highest first):
     ├─ enterprise/private registries     (configured)   ── precedence: highest
     ├─ pinned/overridden providers        (vanta.toml [providers])
     ├─ community registry                 (opt-in)
     └─ official Vanta registry            (default)      ── precedence: lowest
            │
            ▼
     first registry that defines the name → its provider serves the request
```

Rules:

- **Precedence is highest-first; the first match wins.** A private/enterprise registry can shadow
  the official one (e.g. an internal `node` build) without renaming the tool.
- **Aliases** are resolved within a registry before cross-registry comparison.
- **Ambiguity** only arises when two registries at the *same* precedence define the same name; that
  is `VTA-RES-0003`, and Vanta requires the explicit `registry/name` form to disambiguate.
- A name found in no registry is `VTA-REG-0003`.
- Per-tool provider overrides in `vanta.toml` (`[providers]`) sit above community/official but
  below enterprise policy, letting a project pin an exact provider/manifest revision for
  reproducibility while still honoring org-level shadowing.

This ordering is the registry-layer half of `[2 Resolve]` step (a) in
[06. Resolution & Version Management](06-resolution.md).

## Registry architecture

The registry is an **index** plus the **signed metadata** that makes it trustworthy and
incrementally updatable. It is *not* a live API Vanta queries per request; it is a content-
addressed dataset Vanta syncs and then reads locally.

### What the index contains

The index is the catalog: for every tool, its canonical name, aliases, description, homepage,
license, categories, supported platforms, the **provider reference** (provider id + the content
hash of its manifest), and pointers to that tool's version metadata (which versions exist, their
channels/LTS flags, yank status, `min-vanta-version`). The full per-tool schema is the subject of
[26. Registry & Package Metadata Reference](26-registry-and-metadata-reference.md); an overview is
in [Tool and package metadata](#tool-and-package-metadata) below.

### Format and distribution

The index is published as a set of **content-addressed metadata blobs** (`blake3-<hex>`) plus a
small signed chain of role metadata (TUF-like: `root` → `registry` → `snapshot` → `targets`; see
[26](26-registry-and-metadata-reference.md) and [15. Security & Supply Chain](15-security.md)). A
client never trusts an individual file in isolation: it verifies the signed `snapshot`, which names
the exact content hashes of every other metadata file, preventing mix-and-match and rollback.

```
 registry root (https://registry.vanta.dev)
   ├─ 1.root.json            signed by offline root keys (rotated rarely)
   ├─ registry.json          signed by the registry role: list of namespaces/shards
   ├─ snapshot.json          signed: maps every metadata path → {hash, len, version}
   └─ targets/
        ├─ index/<shard>.cbor.zst     content-addressed catalog shards
        └─ tools/<name>.cbor.zst      per-tool version + provider metadata
```

**Incremental, conditional fetch.** Sync is bandwidth-minimal:

1. Conditional GET on `snapshot.json` using stored `ETag`/`If-None-Match` (and
   `If-Modified-Since`). A `304 Not Modified` ends the sync in one round trip.
2. On change, diff the new `snapshot` against the cached one: only blobs whose content hash
   changed are refetched (**delta update**). Shards keep the changed set small even as the catalog
   grows to tens of thousands of tools.
3. Each fetched blob is verified against the hash named in the snapshot before it replaces the
   cached copy; replacement is atomic (stage + rename), mirroring the store's atomicity rules
   (canon §9).
4. The snapshot's monotonic version is the **registry revision** that makes resolution
   reproducible ([06](06-resolution.md)); a snapshot whose version regresses is a rollback attack
   and is refused with `VTA-REG-0004`.

**Local caching and search.** Synced metadata lives under `~/.vanta/registries/<id>/` (verified,
authoritative copy) with a derived search index in `~/.vanta/cache/registry/`. `vanta search
<query>` and `vanta info <tool>` read these locally — search works offline against the last sync.
Encoding is CBOR + Zstd for compact, fast-to-parse blobs; the human-facing surfaces (`info`,
`search --json`) render from the same data.

| Concern | Decision | Why |
|---------|----------|-----|
| On-wire format | CBOR + Zstd, content-addressed | compact, deterministic, dedup-friendly |
| Freshness | conditional GET + signed snapshot version | one round trip when unchanged; rollback-proof |
| Update size | per-shard delta by content hash | scales to large catalogs |
| Offline | last verified sync is authoritative | `search`/`info`/resolution work offline |
| Trust | TUF-like signed roles, content addressing | no single file is trusted alone |

## Tool and package metadata

Per tool, the registry stores the catalog fields below (overview; exact types, requiredness, and
nested schemas are in [26. Registry & Package Metadata Reference](26-registry-and-metadata-reference.md)):

| Field | Meaning |
|-------|---------|
| `name` | canonical tool name (`node`) |
| `aliases` | alternate names (`nodejs`) |
| `description` | one-line summary for `search`/`info` |
| `homepage` | upstream URL |
| `provider` | provider id + manifest content hash that serves this tool |
| `platforms` | platform triples with available artifacts |
| `license` | SPDX expression |
| `categories` | taxonomy tags (`runtime`, `cli`, `iac`) for search/browse |
| `versions` | pointer to version metadata (versions, channels, `lts`, `yanked`, `min-vanta-version`) |

This is deliberately a *catalog*, not the artifact data itself: artifact descriptors
(URLs/mirrors, checksums, signatures, layout) are produced by the provider at `[2 Resolve]` /
`[5 Verify]` time and pinned into `vanta.lock`, so the index stays small and the heavy, per-version
detail is fetched lazily and cached.

## Plugin architecture

A **plugin is a provider**: a declarative manifest plus an optional WASM hook. There is no second
extension mechanism — everything that teaches Vanta about a new tool is a provider.

```
 provider package (signed)
   ├─ manifest.toml         declarative spec (id, tool, source backend, artifact, layout, bins…)
   ├─ hook.wasm (optional)  implements the WIT provider world (list-versions/resolve/verify/cmp)
   └─ signature             cosign/minisign over the package digest
```

### Official vs. community registries

- The **official registry** (default, lowest precedence) ships curated, reviewed, signed
  providers maintained by the Vanta project.
- The **community registry** is opt-in (`vanta registry add community`). Community providers are
  reviewed and signed before publication but carry a distinct trust namespace so a user/enterprise
  can enable or disable them wholesale.

### Publishing a community provider

```
 author writes manifest.toml (+ hook.wasm)        # vanta-sdk, see doc 22
        │  vanta provider test ./my-provider      # golden resolutions, sandbox check
        ▼
 submit to community registry (PR / `vanta provider publish`)
        │  automated checks: schema valid, ABI compatible, hook capability audit,
        │  golden resolutions reproduce, license present
        ▼
 human review ──► signed by the community `targets` role ──► appears in next snapshot
```

Every provider is **signed**; the registry's `targets` role signs the provider package digest, and
the snapshot pins it. Installing a tool whose provider signature does not verify against a trusted
key is `VTA-PROV-0005`; a manifest that fails schema validation is `VTA-PROV-0001`. The hook ABI
and capability rules are specified in [22. Provider SDK & ABI](22-provider-sdk.md).

## Private registries

Enterprises run private registries to serve internal tools, mirror the public catalog, and pin
approved versions. Detail is in [14. Enterprise & Teams](14-enterprise.md); the model:

```bash
vanta registry add acme https://registry.acme.internal --priority 100
vanta registry list
vanta trust key acme ./acme-root.pub      # establish the root of trust (TOFU + explicit)
```

- **Precedence** is set per registry (`--priority`); private registries are typically configured
  highest so they shadow public providers (an internal `node` build wins).
- **Auth** is delegated to `vanta-net`: bearer tokens, basic auth, or mTLS, sourced from the OS
  keychain or `VANTA_*` env vars, never written to disk in plaintext (see [15](15-security.md)).
- A private registry uses the **same** signed-role format as the public one, so the integrity and
  rollback guarantees are identical; the enterprise controls its own `root` keys.

## Provider and registry versioning

Three version axes evolve independently so one can change without forcing a flag day:

| Axis | Versioned by | Compatibility rule |
|------|-------------|--------------------|
| **Registry index format** | `format_version` in `registry.json` | Vanta refuses an unknown major (`VTA-REG-0002`); minors are additive |
| **Provider manifest schema** | `manifest_version` in each manifest | Vanta refuses an unknown major (`VTA-PROV-0001`); minors are additive |
| **Provider hook ABI** | the WIT world version (e.g. `vanta:provider@0.1.0`) | host accepts a documented range of world versions; out-of-range is `VTA-PROV-0004` |

The provider package records `min-vanta-version`; resolution drops versions/providers that need a
newer binary (`VTA-RES-0011`) and tells the user to `vanta self update`. Because resolutions pin
the provider manifest's content hash, an upstream manifest change never silently alters a locked
environment — it is picked up only on an explicit re-resolution ([06](06-resolution.md)).

## Security posture

Overview only; the full model is [15. Security & Supply Chain](15-security.md) and the sandbox/ABI
is [22. Provider SDK & ABI](22-provider-sdk.md):

- **Sandbox.** WASM hooks run in Wasmtime with **no ambient authority** — no filesystem, no
  sockets, no environment. Their only host imports are a capability-scoped `http-get`/`http-head`
  (restricted to the hosts the manifest declares), `hash`, and `log`. CPU is bounded by fuel and
  wall-clock by epoch interruption; a runaway or trapping hook is `VTA-PROV-0002`; an attempt to
  reach an undeclared host is a capability violation `VTA-PROV-0003`.
- **Signing.** Registry metadata is signed via TUF-like roles; providers are signed; artifacts are
  verified by checksum (always) and signature/provenance (when published) using
  sigstore/cosign or minisign. Vanta's own releases ship cosign signatures + SLSA provenance
  (canon §8).
- **Trust.** New or changed registries, providers, and project configs are untrusted until
  `vanta trust` (direnv-style TOFU), so pulling a repo cannot silently run a new provider.

## Trade-offs and failure modes

- **Declarative ceiling.** Some upstreams have genuinely messy release processes; a manifest cannot
  model them and a hook must be written. We accept the extra authoring cost to keep the default
  path safe and data-only.
- **Backend coupling to upstream stability.** `github-releases` and `direct-url` depend on
  upstream URL/tag stability; when an upstream renames assets, the provider manifest must be
  updated and re-signed. The signed-manifest + content-hash pin means old locks keep working from
  cache/mirrors even after upstream churn.
- **`os-package-manager` is not reproducible.** Documented and flagged; it is a compatibility
  bridge, not a guarantee.
- **Registry availability.** A registry outage degrades to the last verified local sync
  (resolution/search continue offline); only genuinely-new lookups need the network.

Failure-mode catalog (full registry in [25. Error & Exit-code Catalog](25-error-and-exit-code-catalog.md)):

| Code | Condition | Exit class |
|------|-----------|------------|
| `VTA-REG-0001` | Registry metadata signature invalid / untrusted key | security (`6`) |
| `VTA-REG-0002` | Registry index format major unsupported | resolution (`4`) |
| `VTA-REG-0003` | Tool not found in any registry | resolution (`4`) |
| `VTA-REG-0004` | Snapshot rollback / freshness violation | security (`6`) |
| `VTA-PROV-0001` | Provider manifest fails schema validation | resolution (`4`) |
| `VTA-PROV-0002` | WASM hook trapped or exhausted fuel/epoch | provider (`7`) |
| `VTA-PROV-0003` | Hook attempted a disallowed (undeclared) capability | security (`6`) |
| `VTA-PROV-0004` | Provider hook ABI (WIT world) out of supported range | provider (`7`) |
| `VTA-PROV-0005` | Provider package signature invalid / untrusted | security (`6`) |
| `VTA-NET-0001` | Registry/provider source unreachable (→ offline cache) | network (`5`) |

## Cross-references

- [06. Resolution & Version Management](06-resolution.md) — how providers feed candidate versions and artifacts into `[2 Resolve]`.
- [09. Store](09-store.md) — how the artifacts a provider resolves are materialized and content-addressed.
- [14. Enterprise & Teams](14-enterprise.md) — private registries, auth, mirrors, and org policy in depth.
- [15. Security & Supply Chain](15-security.md) — signing, TUF-like roles, trust DB, and verification policy.
- [22. Provider SDK & ABI](22-provider-sdk.md) — provider manifest format and the sandboxed WASM hook ABI.
- [26. Registry & Package Metadata Reference](26-registry-and-metadata-reference.md) — exhaustive index/metadata schemas and platform tokens.
- [25. Error & Exit-code Catalog](25-error-and-exit-code-catalog.md) — the full `VTA-REG-*` / `VTA-PROV-*` registry.
