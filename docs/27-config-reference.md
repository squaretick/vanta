# 27. Configuration Reference

> The exhaustive, key-by-key reference for `vanta.toml` (the project manifest) and `~/.vanta/config.toml` (global config). Every table, key, type, default, scope, and constraint. The conceptual guide is [05. Configuration](05-configuration.md); the manifest↔lock relationship is [31. Lockfile & Manifest Reference](31-lockfile-and-manifest-reference.md).

**Contents**

- [Conventions](#conventions)
- [Top-level keys](#top-level-keys)
- [`[tools]`](#tools)
- [`[env]`](#env)
- [`[tasks]`](#tasks)
- [`[settings]`](#settings)
- [`[registries]`](#registries)
- [`[workspace]`](#workspace)
- [`[policy]`](#policy)
- [Fully-populated examples](#fully-populated-examples)
- [Cross-references](#cross-references)

---

## Conventions

- **Scope** column: `project` (`vanta.toml`), `global` (`config.toml`), or `both`.
- Defaults are the actual values Vanta uses when the key is absent.
- Precedence (low→high): defaults < global config < workspace root < project < `[env]`/per-dir < `VANTA_*` env < CLI flags ([05. Configuration](05-configuration.md#precedence-and-merging)).

## Top-level keys

| Key | Type | Default | Scope | Description |
| --- | --- | --- | --- | --- |
| `version` | int | `1` | both | manifest/config format version ([31](31-lockfile-and-manifest-reference.md)) |

## `[tools]`

Maps a tool name to a request. Value is a string or an inline table.

| Form | Example | Meaning |
| --- | --- | --- |
| version string | `node = "24"` | request (canon §10): exact / prefix / `latest` / `lts` / channel / range / `system` |
| inline table | `node = { version = "24", … }` | a request plus options |

Inline-table keys:

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `version` | string | required | the version request |
| `channel` | string | provider default | named channel (`stable`/`nightly`/`lts`) |
| `registry` | string | resolution order | force a specific registry by name |
| `provider` | string | registry default | force a specific provider id |
| `os` | list<token-prefix> | all | restrict to platforms (e.g. `["macos","linux"]`) |
| `env` | table | — | tool-scoped env vars set only when this tool is active |
| `optional` | bool | `false` | skip (warn) if no artifact exists for the current platform |

## `[env]`

Environment variables injected when the environment is active (a **trusted** section — requires `vanta trust`, [05](05-configuration.md#config-trust)). Scope: project (and per-member).

| Form | Example | Meaning |
| --- | --- | --- |
| static | `NODE_ENV = "development"` | export `KEY=value` on activation; unset on deactivation |
| path prepend | `PATH = { prepend = ["./bin"] }` | add project-relative dirs after the tool bin view |
| path append | `PATH = { append = ["./extra"] }` | add dirs at the end |
| reference | `CACHE = "${HOME}/.cache/app"` | `${VAR}` expansion from the ambient environment |

## `[tasks]`

The optional minimal task runner ([05](05-configuration.md#the-optional-task-runner)). A **trusted** section. Scope: project.

| Form | Example | Meaning |
| --- | --- | --- |
| string | `build = "cargo build"` | a command run in the activated environment |
| table | `test = { run = "…", description = "…", depends = ["build"] }` | command + metadata + prerequisite tasks |

Table keys: `run` (string, required), `description` (string), `depends` (list<string>, prerequisite tasks; cycles → `VTA-CFG-0010`), `env` (table, task-scoped vars), `cwd` (string, working dir).

## `[settings]`

Behavioral settings. Most are valid in both files; project settings override global.

| Key | Type | Default | Scope | Description |
| --- | --- | --- | --- | --- |
| `auto_install` | bool | `false` | both | on entering a project with missing tools, offer to install (after trust) |
| `verify` | enum | `require` | both | `require` \| `warn` \| `off` (`off` discouraged; policy can forbid) |
| `jobs` | int | `min(cpus, 8)` | both | download/unpack parallelism |
| `link_strategy` | enum | `auto` | both | `auto` \| `reflink` \| `hardlink` \| `symlink` \| `copy` ([09](09-store.md#link-strategies)) |
| `shims` | bool | `true` | global | install/maintain the shim dispatcher |
| `offline` | bool | `false` | both | operate from store/caches only ([13](13-offline.md)) |
| `mirror` | string | — | both | a single mirror base URL (shorthand) |
| `mirrors` | list<table> | `[]` | both | ordered mirrors (`url`, `priority`, `scope`) |
| `proxy` | string | system | both | HTTP(S) proxy override |
| `ca_bundle` | string (path) | system trust | both | custom CA bundle for TLS-inspecting proxies |
| `retain_generations` | int | `5` | both | minimum generations kept by GC ([12](12-updates.md)) |
| `gc_keep_days` | int | `30` | both | also keep generations newer than N days |
| `cache_max_size` | size | unlimited | both | cap on `cache/` size before pruning |
| `targets` | list<token> | sensible default set | both | platforms the lock resolves for ([11](11-reproducibility.md)) |
| `read_foreign_versions` | bool | `true` | both | honor `.tool-versions`/`.nvmrc`/etc. ([05](05-configuration.md#interop-with-foreign-version-files)) |
| `run_hooks` | bool | `true` | both | run provider post-install hooks (sandboxed) |
| `color` | enum | `auto` | both | `auto` \| `always` \| `never` |
| `telemetry` | bool | `false` | both | off by default; opt-in only |
| `auto_rollback` | bool | `false` | both | revert an update if a post-update health check fails |
| `experimental.*` | table | `{}` | both | gated unstable features ([29](29-public-apis.md)) |

## `[registries]`

Named registries that overlay the official one ([07](07-providers.md), [14](14-enterprise.md)). Usually in global/org config; allowed in a project (a project registry is a **trusted** addition).

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `url` | string | required | registry base URL |
| `priority` | int | `100` | resolution order (lower = checked first; official defaults to `100`) |
| `auth` | enum | `none` | `none` \| `token` \| `oidc` \| `netrc` |
| `public_key` | string | — | pinned root key reference for this registry |

Example: `[registries.acme]` with `url`, `priority = 10`, `auth = "oidc"`.

## `[workspace]`

Root-manifest only ([05](05-configuration.md#workspaces)).

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `members` | list<glob> | required | member paths (globs allowed) |
| `inherit` | bool | `true` | members inherit root `[tools]`/`[env]` unless overridden |

## `[policy]`

Org policy (usually a separate signed file referenced by config, but documented here for completeness; enforcement in [14. Enterprise](14-enterprise.md)).

| Key | Type | Description |
| --- | --- | --- |
| `require_signature` | bool | refuse unsigned artifacts |
| `forbid_no_verify` | bool | disallow `--no-verify` |
| `allow_source_builds` | bool | permit sandboxed source builds |
| `min_slsa_level` | int | provenance floor |
| `tools.allow` / `tools.deny` | list | allow/deny tool patterns |
| `versions.<tool>` | constraint | per-tool version floor/ceiling |
| `licenses.allow` / `licenses.deny` | list<SPDX> | license policy |

## Fully-populated examples

A maximal `vanta.toml`:

```toml
version = 1

[tools]
node = "24"
pnpm = "9"
python = { version = "3.13", os = ["macos", "linux"] }
terraform = { version = "1.9", registry = "acme" }
rust = { version = "stable", channel = "stable" }

[env]
NODE_ENV = "development"
DATABASE_URL = "postgres://localhost/app_dev"
PATH = { prepend = ["./node_modules/.bin", "./bin"] }

[tasks]
dev    = "pnpm dev"
test   = { run = "pnpm test", description = "unit tests", depends = ["build"] }
build  = "pnpm build"
deploy = { run = "terraform apply", depends = ["test"] }

[settings]
auto_install = true
verify = "require"
targets = ["macos/aarch64", "linux/x86_64/gnu", "windows/x86_64"]

[registries.acme]
url = "https://vanta.acme.internal"
priority = 10
auth = "oidc"
```

A maximal `~/.vanta/config.toml`:

```toml
version = 1

[tools]                       # global tools, available everywhere unless a project overrides
ripgrep = "14"
gh = "latest"
fd = "10"

[settings]
jobs = 12
verify = "require"
link_strategy = "auto"
retain_generations = 8
gc_keep_days = 45
mirror = "https://vanta-mirror.corp.internal"
telemetry = false

[registries.official]
url = "https://registry.vanta.dev"
priority = 100

[registries.acme]
url = "https://vanta.acme.internal"
priority = 10
auth = "oidc"
public_key = "acme-root@1"
```

## Cross-references

- [05. Configuration](05-configuration.md) — the conceptual guide and precedence/merge rules.
- [31. Lockfile & Manifest Reference](31-lockfile-and-manifest-reference.md) — the lock and the format `version`.
- [04. CLI](04-cli.md) — flags that override these keys and `VANTA_*` env overrides.
- [14. Enterprise](14-enterprise.md) — `[policy]` and `[registries]` in an org context.
- [09. Store](09-store.md) — `link_strategy`, `retain_generations`, `gc_keep_days`.
- [13. Offline](13-offline.md) — `offline`, `mirror`, `mirrors`, `proxy`.
