# 26. Registry & Package Metadata Reference

> The exhaustive schema reference for the registry: the index entry, the provider manifest, the artifact descriptor, version metadata, the canonical platform tokens, and the signed-role (TUF-style) metadata model. This is the data contract that providers, the registry, and the resolver all agree on; the authoring guide is [22. Provider SDK](22-provider-sdk.md) and the conceptual model is [07. Providers](07-providers.md).

**Contents**

- [Overview of the data model](#overview-of-the-data-model)
- [Registry index entry](#registry-index-entry)
- [Provider manifest schema](#provider-manifest-schema)
- [Artifact descriptor schema](#artifact-descriptor-schema)
- [Version metadata](#version-metadata)
- [Platform tokens](#platform-tokens)
- [Signed-metadata roles](#signed-metadata-roles)
- [Format versioning](#format-versioning)
- [Worked examples](#worked-examples)
- [Cross-references](#cross-references)

---

## Overview of the data model

```
Registry  ──has many──►  Index Entry (one per tool)  ──refers to──►  Provider (manifest)
   │                                                                      │
   │ signed by roles (root/registry/snapshot/timestamp)                   │ produces, per version+platform
   ▼                                                                      ▼
 snapshot (a consistent registry_revision)                         Artifact Descriptor (url, checksum, sig, layout, bin)
```

The **index** answers "what tools exist and which provider serves each." A **provider** answers "what versions exist and, per platform, what is the artifact." An **artifact descriptor** is the concrete, verifiable thing fetched. Everything is covered by **signed roles** so the whole chain is verifiable ([15. Security](15-security.md)).

## Registry index entry

One entry per tool in the registry index.

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `name` | string | yes | canonical tool name (e.g. `node`) |
| `aliases` | list<string> | no | alternate names (e.g. `nodejs`) |
| `summary` | string | yes | one-line description (shown by `vanta search`/`info`) |
| `homepage` | string (url) | no | project homepage |
| `provider` | string | yes | provider id serving this tool (e.g. `official/node`) |
| `categories` | list<string> | no | e.g. `runtime`, `cli`, `toolchain` |
| `platforms` | list<token> | yes | platform tokens this tool supports |
| `license` | string (SPDX) | no | the tool's license (for policy/reporting) |
| `publisher` | string | yes | verified publisher identity |
| `latest` | string | yes | newest stable version (a cache hint; provider is authoritative) |
| `deprecated` | bool / string | no | true or a message if the tool is deprecated |

## Provider manifest schema

The provider definition (TOML). Authoring guide in [22. Provider SDK](22-provider-sdk.md); this is the exhaustive field table.

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `id` | string | yes | provider id, `namespace/name` (e.g. `official/node`) |
| `tool` / `tools` | string / list | yes | the tool(s) served |
| `provider_version` | int | yes | provider's own version (independent of the tool) |
| `abi` | semver | yes | the provider WIT ABI targeted (e.g. `1.0.0`) |
| `version_source` | enum | yes | `github-releases` \| `url-list` \| `language-registry` \| `wasm` |
| `repo` | string | if github | `owner/repo` for `github-releases` |
| `version_comparator` | enum | no | `semver` (default) \| `calver` \| `wasm` |
| `channels` | table | no | named channels → selection rule (`lts`, `stable`, `nightly`) |
| `allow_hosts` | list<pattern> | if wasm http | host allow-list for the sandboxed `http-get` |
| `wasm` | string (ref) | if any `wasm` | content reference to the signed WASM component |
| `[artifacts.<token>]` | table | yes | one artifact descriptor per supported platform token |
| `deps` | list<dep> | no | declared tool dependencies (`name` + constraint) |
| `signature` | object | yes | provider's own signature (signer/key ref) |

## Artifact descriptor schema

The per-version, per-platform concrete artifact. Returned declaratively (`[artifacts.<token>]` with `{version}` templating) or by a `resolve` hook.

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `url` | string (template) | yes | primary download URL; supports `{version}`/`{os}`/`{arch}`/`{ext}` |
| `mirrors` | list<string> | no | additional URLs tried in order ([13](13-offline.md)) |
| `archive` | enum | yes | `tar.gz` \| `tar.xz` \| `tar.zst` \| `tar.bz2` \| `zip` \| `raw` |
| `size` | int (bytes) | no | expected size (sanity/progress) |
| `checksum` | object | yes | `{ algo: sha256\|blake3, value }` or `{ sha256-url }` or `{ wasm }` |
| `signature` | object | no | `{ type: minisign\|cosign, value, key_ref }` |
| `provenance` | object | no | SLSA provenance reference + level |
| `layout` | object | no | `{ strip: int, subdir: string, modes: … }` |
| `bin` | list<string> | yes | executables to expose (paths relative to the laid-out tree; Windows forms allowed) |
| `env` | table | no | env vars the tool needs when active |
| `store_key` | string | (in lock) | the resolved `blake3-…` (written into `vanta.lock`, not the provider) |

`store_key` is computed by Vanta and recorded in the lock ([31. Lock Reference](31-lockfile-and-manifest-reference.md)); a provider does not supply it.

## Version metadata

Per available version, the provider/registry may expose:

| Field | Type | Description |
| --- | --- | --- |
| `version` | string | the version string |
| `channel` | string | the channel it belongs to (`stable`/`lts`/`nightly`) |
| `lts` | bool | whether it is a long-term-support release |
| `released` | date | release date (for `info`/advisories) |
| `yanked` | bool / string | true or a reason if withdrawn (resolution warns/refuses per policy) |
| `min_vanta` | semver | minimum Vanta version required to install it |
| `advisory` | list<ref> | linked security advisories |

## Platform tokens

The canonical `os/arch[/libc]` identifiers used everywhere (locks, artifact maps, store/cache keys) — see [17. Cross-platform](17-cross-platform.md):

| Token | OS | Arch | libc |
| --- | --- | --- | --- |
| `linux/x86_64/gnu` | Linux | x86-64 | glibc |
| `linux/x86_64/musl` | Linux | x86-64 | musl |
| `linux/aarch64/gnu` | Linux | ARM64 | glibc |
| `linux/aarch64/musl` | Linux | ARM64 | musl |
| `macos/aarch64` | macOS | ARM64 | — |
| `macos/x86_64` | macOS | x86-64 | — |
| `windows/x86_64` | Windows | x86-64 | MSVC |
| `windows/aarch64` | Windows | ARM64 | MSVC |

Tokens are lowercase, slash-separated, and stable; new tokens are additive.

## Signed-metadata roles

A TUF-style role model bounds key compromise and prevents rollback/freeze ([15. Security](15-security.md), [21. Threat Model](21-threat-model.md)):

| Role | Signs | Key handling |
| --- | --- | --- |
| `root` | the set/keys of all roles (delegations) | offline, threshold M-of-N, long expiry, rarely used; the trust anchor pinned by clients |
| `registry` (targets) | the index entries + provider references | the working authority for "what exists/where" |
| `snapshot` | the consistent set of metadata at a `registry_revision` | prevents mix-and-match and metadata rollback |
| `timestamp` | the freshness of the current snapshot | short expiry; bounds staleness, defeats freeze |

Clients ship pinned `root` keys (rotatable via a root-signed update), verify the chain to whatever metadata they consume, and reject anything unsigned, wrongly-signed, expired, or inconsistent. `registry_revision` (the snapshot identity) is recorded in `vanta.lock` so resolution is reproducible at a point in time ([11. Reproducibility](11-reproducibility.md)).

## Format versioning

- The index and metadata carry a schema version; the client negotiates and the registry may serve multiple versions during a transition ([03. Repository](03-repository.md#versioning-policy)).
- The provider `abi` field is the WIT world version; the host supports a documented range ([22. Provider SDK](22-provider-sdk.md)).
- New fields are additive and optional; a breaking change is a new schema version with a migration window.

## Worked examples

A registry index entry:

```toml
[[tool]]
name = "ripgrep"
aliases = ["rg"]
summary = "Recursively search directories for a regex pattern, fast."
homepage = "https://github.com/BurntSushi/ripgrep"
provider = "official/ripgrep"
categories = ["cli", "search"]
platforms = ["linux/x86_64/gnu", "macos/aarch64", "windows/x86_64"]
license = "MIT"
publisher = "vanta-official"
latest = "14.1.0"
```

A provider manifest with one declarative artifact:

```toml
id = "official/ripgrep"
tool = "ripgrep"
provider_version = 1
abi = "1.0.0"
version_source = "github-releases"
repo = "BurntSushi/ripgrep"
version_comparator = "semver"
signature = { type = "minisign", key_ref = "vanta-official@1" }

[artifacts."macos/aarch64"]
url = "https://github.com/BurntSushi/ripgrep/releases/download/{version}/ripgrep-{version}-aarch64-apple-darwin.tar.gz"
archive = "tar.gz"
checksum = { sha256-url = "{url}.sha256" }
layout = { strip = 1 }
bin = ["rg"]
```

The resolved artifact as written into `vanta.lock` (store_key added by Vanta):

```toml
[tool.platform."macos/aarch64"]
store_key = "blake3-9d2e…"
url = "https://github.com/BurntSushi/ripgrep/releases/download/14.1.0/ripgrep-14.1.0-aarch64-apple-darwin.tar.gz"
size = 2398512
sha256 = "7c0e…"
signature = "minisign:RWQf…"
bin = ["rg"]
```

## Cross-references

- [07. Providers](07-providers.md) — the conceptual registry/provider/backend model.
- [22. Provider SDK](22-provider-sdk.md) — authoring providers against these schemas + the WIT ABI.
- [31. Lockfile & Manifest Reference](31-lockfile-and-manifest-reference.md) — how artifact descriptors are pinned into the lock.
- [15. Security](15-security.md) & [21. Threat Model](21-threat-model.md) — the signed-role trust model.
- [17. Cross-platform](17-cross-platform.md) — the platform tokens.
- [06. Resolution](06-resolution.md) — how version metadata drives `[2 Resolve]`.
