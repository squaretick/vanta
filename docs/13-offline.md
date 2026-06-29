# 13. Offline, Mirrors & Air-gapped

> Vanta is designed to work without the network once content is fetched, and to move whole environments into disconnected and air-gapped settings. This document specifies offline mode, mirror configuration and fallback, the portable bundle format (`vanta bundle`/`vanta restore`), internalizing the registry for air-gapped sites, corporate proxy support, and why content addressing makes all of this robust and verifiable.

**Contents**

- [Offline mode](#offline-mode)
- [Mirrors](#mirrors)
- [Air-gapped bundles](#air-gapped-bundles)
- [Internalizing the registry](#internalizing-the-registry)
- [Proxies and corporate networks](#proxies-and-corporate-networks)
- [The content-addressed advantage](#the-content-addressed-advantage)
- [Workflows](#workflows)
- [Failure modes and trade-offs](#failure-modes-and-trade-offs)
- [Cross-references](#cross-references)

---

## Offline mode

Offline operation is first-class, not a degraded fallback (pillar 6, canon §2). With `--offline` (or `[settings] offline = true`), Vanta performs no network IO and serves everything from the store and caches.

| Operation | Offline behavior |
| --- | --- |
| `vanta sync` | installs from the store + download cache; a cache miss is a clean `VTA-NET-0002` ("offline and not cached") |
| `vanta x <tool>` | runs if the tool is cached/materialized; otherwise `VTA-NET-0002` |
| activation / `cd` switching | always works (resolution is lock-authoritative and cached; the hook never needs the network) |
| `vanta which/list/info` (cached) | works from cached metadata |
| `vanta add`/`update` | requires resolution metadata; works if the registry slice is cached or mirrored, else fails cleanly |

The guarantee: anything already resolved and fetched works offline; anything not is a *clear, specific* error rather than a hang. Because resolution is read from `vanta.lock` (not the live registry) for `sync`, **a committed lock plus a warm store reproduces an environment with zero network access** — the basis for CI caching and air-gapped use.

## Mirrors

A mirror is an alternate base URL for registry metadata and/or artifacts. Mirrors are configured globally or per project and tried in priority order before the canonical source:

```toml
# ~/.vanta/config.toml or vanta.toml [settings]
[settings]
mirror = "https://vanta-mirror.corp.internal"     # shorthand: one mirror for everything

[[settings.mirrors]]                                # explicit, ordered, scoped
url = "https://eu-mirror.corp.internal"
priority = 10
scope = "artifacts"                                 # artifacts | metadata | both

[[settings.mirrors]]
url = "https://registry.vanta.dev"                  # canonical fallback
priority = 100
```

- **Fallback ordering.** Fetch tries mirrors by ascending priority, falling through on failure (timeout, 5xx, 404) to the next, finally the canonical source unless `--offline`. A successful fetch is verified against the lock hash regardless of which mirror served it — **a mirror cannot serve bad bytes undetected** ([15. Security](15-security.md)).
- **Metadata vs artifacts** can be mirrored independently (`scope`), so a site can mirror large artifacts locally while still reaching the canonical signed metadata, or mirror both.
- **Regional mirrors** simply have different priorities per location.

Because every fetched artifact is hash-verified, mirrors are a *performance/availability* feature with no trust cost: an untrusted or compromised mirror can cause a verification failure but never a silent substitution.

## Air-gapped bundles

For machines with no path to any mirror, Vanta produces a **portable bundle** containing everything a project (or an explicit tool set) needs, which is then imported on the disconnected machine.

```
# on a connected machine
vanta bundle --out app-tools.vbundle              # everything vanta.lock requires
vanta bundle --out node.vbundle node@24 python@3.13   # an explicit set
vanta bundle --platform linux/x86_64/gnu --out linux.vbundle   # one platform's slice

# transfer app-tools.vbundle across the air gap (USB, artifact store, etc.)

# on the air-gapped machine
vanta restore app-tools.vbundle                   # imports store entries + metadata + keys
vanta sync --offline                              # now fully reproducible, no network
```

**Bundle format** — a single archive (tar) containing:

```
app-tools.vbundle
├── manifest.toml          # what's inside: tools, versions, platforms, the source lock
├── store/                 # the content-addressed store entries (already materialized)
│   ├── blake3-aa3f…/      #   verifiable by re-hash on import
│   └── blake3-7b21…/
├── metadata/              # the registry/provider metadata slice needed to resolve these
├── keys/                  # the public signing keys needed to verify (not private keys)
└── bundle.sig             # the bundle's own signature
```

- **Integrity.** The bundle is signed, and every store entry inside is content-addressed — `vanta restore` re-hashes each entry against its key and verifies the bundle signature before importing. A tampered bundle fails closed (`VTA-VRF-*`).
- **Dedup on import.** Restore is idempotent: entries already present are skipped (same key = same bytes), so overlapping bundles cost nothing extra and re-importing is safe.
- **Selectable platforms.** `--platform` slices a bundle to one OS/arch to keep it small, or omit it to carry all locked targets for a mixed-OS air-gapped fleet.
- Bundles are the secure, open-core answer to Chocolatey's commercial "internalizer" ([33. Prior Art](33-prior-art.md)).

## Internalizing the registry

A connected build host mirrors the official registry index, provider set, and chosen artifacts into an internal location once; air-gapped or restricted machines then point at it.

```
# on the connected gateway, periodically
vanta registry mirror --to /srv/vanta-mirror \
    --include "node,python,go,rust,terraform" --platforms all
# serve /srv/vanta-mirror over HTTPS internally, set it as a mirror or a private registry
```

- The internal copy preserves the **signed metadata** (TUF-style roles, [15. Security](15-security.md)), so internalization does not weaken trust — clients still verify signatures and provenance against pinned keys.
- An enterprise can pin the org to a **vetted snapshot** (a fixed `registry_revision`), so internal resolution is itself reproducible and reviewed ([14. Enterprise](14-enterprise.md)).
- Combined with private registries, an organization can run entirely disconnected from the public internet while keeping Vanta's verification and reproducibility intact.

## Proxies and corporate networks

- Standard `HTTP_PROXY`/`HTTPS_PROXY`/`NO_PROXY` are honored; an explicit proxy can be set via `[settings] proxy`.
- Custom CA bundles are supported (`[settings] ca_bundle` / system trust store) for TLS-inspecting corporate proxies, without disabling verification.
- Authenticated mirrors/registries use the credential model in [14. Enterprise](14-enterprise.md) (tokens, OIDC, keychain).

## The content-addressed advantage

Offline and air-gapped operation are robust *because* the store and caches are content-addressed:

- **Verifiable transport.** Any bundle, mirror, or USB stick can be untrusted; the hash and signature prove the bytes, so moving content through hostile channels is safe.
- **Idempotent import.** Re-running `restore`/`sync` never duplicates or corrupts; keys are content, not locations.
- **Dedup across bundles.** Overlapping tool sets share entries automatically.
- **Reproducible disconnection.** A lock + a restored store = the same environment as online, byte for byte ([11. Reproducibility](11-reproducibility.md)).

## Workflows

```
# CI builds a bundle as an artifact for an air-gapped deploy stage
vanta sync --frozen
vanta bundle --out tools-${GIT_SHA}.vbundle

# air-gapped runner
vanta restore tools-${GIT_SHA}.vbundle
vanta sync --offline --frozen          # reproduce exactly, no network, fail if drift
vanta exec -- ./build.sh
```

```
# laptop before a flight
vanta sync                              # warm the store for the project
# ...offline...
vanta x ruff check                      # works: ruff was cached
vanta add some-new-tool                 # fails cleanly (VTA-NET-0002) — expected offline
```

## Failure modes and trade-offs

| Scenario | Behavior |
| --- | --- |
| Offline and artifact not cached | `VTA-NET-0002`, names the missing tool/platform; suggests `vanta bundle`/mirror |
| Mirror serves wrong bytes | hash verification fails (`VTA-VRF-0001`); falls through to next mirror/source |
| Bundle tampered | signature/hash check fails on `restore`; nothing imported |
| Air-gapped `add` of an un-mirrored tool | clean failure; the tool must be added to a bundle/mirror first |
| Proxy with TLS inspection | works with a configured CA bundle; verification stays on |

- **Trade-off: bundles can be large.** A full multi-platform bundle carries real artifacts; `--platform`/`--include` scope it, and dedup keeps incremental bundles small.
- **Trade-off: offline `add` is limited.** Adding a *new* tool needs its resolution metadata; air-gapped sites pre-mirror or pre-bundle the tools they expect. This is inherent to disconnection, made explicit rather than silently failing.

## Cross-references

- [09. Store](09-store.md) — the content-addressed store and download cache that back offline mode.
- [11. Reproducibility](11-reproducibility.md) — lock-driven, network-free reproduction.
- [15. Security](15-security.md) — signed metadata and hash verification that make untrusted transport safe.
- [14. Enterprise](14-enterprise.md) — private registries, internalization, and authenticated mirrors.
- [26. Registry & Metadata Reference](26-registry-and-metadata-reference.md) — the signed metadata roles preserved by internalization.
- [18. Developer Experience](18-developer-experience.md) — CI caching of `~/.vanta/store`.
