# 05. Configuration & Manifests

> Vanta's configuration surface: why it is TOML and not a bespoke DSL or YAML, the project manifest `vanta.toml` table by table, the optional minimal task runner, the global config, workspaces, the precedence-and-merge model, interop with foreign version files, the trust-on-first-use model for manifests, and validation diagnostics. The exhaustive key reference is [27. Configuration Reference](27-config-reference.md).

**Contents**

- [Why TOML](#why-toml)
- [The project manifest](#the-project-manifest)
- [The optional task runner](#the-optional-task-runner)
- [Global configuration](#global-configuration)
- [Workspaces](#workspaces)
- [Precedence and merging](#precedence-and-merging)
- [Interop with foreign version files](#interop-with-foreign-version-files)
- [Config trust](#config-trust)
- [Validation and diagnostics](#validation-and-diagnostics)
- [Failure modes and trade-offs](#failure-modes-and-trade-offs)
- [Cross-references](#cross-references)

---

## Why TOML

Configuration is a primary product surface, and pillar 9 (canon §2) requires it be human-readable. Vanta uses **TOML** for all three files (`vanta.toml`, `~/.vanta/config.toml`, and the `vanta.lock` lockfile), and deliberately does **not** invent a configuration language.

The decision and its rejected alternatives (recorded as [ADR-0005](24-architecture-decision-records.md)):

| Option | Verdict | Reasoning |
| --- | --- | --- |
| **TOML** (chosen) | ✅ | Ubiquitous, instantly readable, minimal surprises, serde-native, supports comments, diffs cleanly, and is already what developers expect from `Cargo.toml`/`pyproject.toml`/`.mise.toml`. Pillar 10: it *adds no new thing to learn*. |
| A bespoke DSL | ❌ | A manifest that lists tools and versions has no domain complexity that a DSL would clarify. A new language is a cost (a parser, an editor story, documentation, a learning curve) with no payoff here — it would *add* cognitive load, violating pillars 9–10. (Contrast a tool whose config encodes control flow, where a DSL can pay off; Vanta's does not.) |
| YAML | ❌ | Significant-whitespace footguns, type ambiguities (`no`/`yes`/`on`, sexagesimal, the "Norway problem"), and the annotation/label sprawl that pillar 9 explicitly rejects. |
| JSON | ❌ | No comments, noisy for humans, poor for hand-editing. Fine as a `--json` *output*, not as a hand-authored manifest. |

This is a case where **not** building a DSL is the first-principles answer: the simplest representation that a human can read and a machine can validate is the right one.

## The project manifest

`vanta.toml` lives at a project root and declares the tools (and optionally the environment, tasks, and settings) for that project. The only accepted filename is `vanta.toml`.

**Minimal — the 95% case.** Most manifests are three lines:

```toml
[tools]
node = "24"
python = "3.13"
```

`node = "24"` is a *request* (canon §10): the newest `24.x` allowed, pinned to an exact version + artifact hash in `vanta.lock`. A bare `[tools]` table is all most projects ever need.

**Tool value forms.** A tool maps either to a version string or to an inline table for the rare cases that need more:

```toml
[tools]
node = "24"                       # newest 24.x
python = "3.13.4"                 # exact
go = "latest"                     # newest stable
rust = { version = "stable", channel = "stable" }   # a named channel
terraform = { version = "1.9", registry = "acme" }  # from a specific registry
ripgrep = { version = "14", os = ["macos", "linux"] }  # only on these platforms
```

**Environment, tasks, settings, registries.** A fuller manifest:

```toml
[tools]
node = "24"
pnpm = "9"
terraform = "1.9"

[env]
NODE_ENV = "development"
DATABASE_URL = "postgres://localhost/app_dev"
PATH = { prepend = ["./node_modules/.bin", "./bin"] }   # project-relative PATH additions

[tasks]
dev    = "pnpm dev"
test   = { run = "pnpm test", description = "run the test suite" }
deploy = { run = "terraform apply", depends = ["test"] }

[settings]
auto_install = true               # offer to install missing tools on directory entry
link_strategy = "auto"            # reflink -> hardlink -> symlink -> copy

[registries.acme]
url = "https://vanta.acme.internal"
priority = 10
```

Every table and key is specified exhaustively in [27. Configuration Reference](27-config-reference.md). The manifest carries an optional `version = 1` marker (the manifest format version) so future format changes are unambiguous ([31. Lock & Manifest Reference](31-lockfile-and-manifest-reference.md)).

## The optional task runner

The `[tasks]` table is a **deliberately minimal** convenience: it lets a project keep its tools *and* its everyday dev commands in the same human-readable file, so `git clone && vanta sync && vanta run dev` is the whole onboarding. It is **present but quiet** — a project that does not define tasks never sees it, and it is not a build system (pillar: not a Make/Bazel replacement; see [01. Vision](01-vision.md) non-goals).

Syntax, kept small on purpose:

```toml
[tasks]
build = "cargo build --release"                          # string form
test  = { run = "cargo test", description = "unit tests" }
ci    = { run = "cargo test && cargo clippy", depends = ["build"] }
```

- A task runs inside the project's activated environment (its tools are on `PATH`).
- `depends` declares prerequisite tasks (a small DAG; cycles are a `VTA-CFG-*` error).
- `vanta run <name>` runs a task; `vanta run <tool>` runs a tool. **Disambiguation:** if `<name>` matches a defined task it is a task; otherwise it is treated as a tool/binary in the environment. To force either, `vanta run --task <name>` or `vanta run --tool <name>`.

What it intentionally does **not** have: matrices, conditionals, file-watching, caching, cross-task artifacts. Projects that need those keep using their real build system; Vanta just makes sure that build system's binary is the right version.

## Global configuration

`~/.vanta/config.toml` holds user-global settings and the **global tool set** — tools available everywhere when no project overrides them:

```toml
[tools]
ripgrep = "14"      # always available
gh = "latest"
fd = "10"

[settings]
jobs = 8                  # download/unpack parallelism
verify = "require"       # require | warn | off (off is discouraged; policy can forbid it)
mirror = "https://vanta-mirror.internal"
retain_generations = 5
gc_keep_days = 30
shims = true             # install/maintain the shim dispatcher
color = "auto"
telemetry = false        # off by default; opt-in only

[registries.official]
url = "https://registry.vanta.dev"
priority = 0
```

Settings here are the lowest-priority non-default layer; a project's `vanta.toml` overrides them (see precedence). The full settings list is in [27. Configuration Reference](27-config-reference.md).

## Workspaces

A monorepo or multi-package project uses a **workspace**: a root `vanta.toml` declares members, and each member may have its own `vanta.toml`.

```toml
# repo-root/vanta.toml
[workspace]
members = ["apps/*", "services/api", "tools/cli"]
inherit = true        # members inherit the root [tools]/[env] unless they override

[tools]                # workspace-wide baseline
node = "24"
pnpm = "9"
```

```toml
# repo-root/services/api/vanta.toml
[tools]
go = "1.23"           # this member also needs Go; node/pnpm inherited from root
```

- There is **one `vanta.lock` at the workspace root** pinning every tool any member declares (per platform), so the whole repo reproduces from one file.
- Activation in a member directory composes the member's tools over the inherited root tools (precedence below). Entering `services/api/` puts `go`, `node`, and `pnpm` on `PATH`.
- `inherit = false` makes a member self-contained (no root tools). Members can pin different versions of the same tool; both coexist in the store and the active one depends on cwd.

## Precedence and merging

Configuration is layered. Lower layers are defaults; higher layers win on conflict (canon §6):

```
built-in defaults
  < ~/.vanta/config.toml (global settings + global [tools])
    < workspace root vanta.toml
      < project/member vanta.toml (nearest, walking up from cwd)
        < [env]/per-directory overrides
          < VANTA_* environment variables
            < CLI flags
```

Merging rules:

- **`[tools]`** merges by tool name; the higher layer's version request replaces the lower's. (A project's `node = "20"` overrides the global `node = "24"` *in that project*.)
- **`[env]`** merges by key; `PATH.prepend` lists concatenate (project entries first).
- **`[settings]`** merges by key; the higher layer's value wins.
- **`[registries]`** union by name; priority decides resolution order ([07. Providers](07-providers.md)).

Worked example: global `config.toml` sets `node = "24"`, `jobs = 8`; the workspace root sets `node = "22"`; the member `apps/web/vanta.toml` sets `node = "20"`. Inside `apps/web/`, `node` resolves to `20`; `jobs` is `8` (only the global set it). `vanta which node` in `apps/web/` shows the 20.x store path; in a sibling with no `node` override, the 22.x path.

## Interop with foreign version files

To make adoption frictionless, Vanta **reads** common foreign version files at resolution time even without a `vanta.toml`, so a repository that already pins versions "just works":

| File | Source tool | Interpreted as |
| --- | --- | --- |
| `.tool-versions` | asdf | a `[tools]` table |
| `.mise.toml` | mise | tools + env |
| `.nvmrc`, `.node-version` | nvm/fnm | `node = <version>` |
| `.python-version` | pyenv | `python = <version>` |
| `.ruby-version` | rbenv | `ruby = <version>` |
| `.go-version` | gvm/goenv | `go = <version>` |
| `rust-toolchain.toml` | rustup | `rust = <channel/version>` |

These are **read-only**: Vanta does not modify them. If both a `vanta.toml` and a foreign file exist, `vanta.toml` wins (it is the higher-precedence, Vanta-native source). Foreign files are *not* locked with hashes (they carry no artifact identity), so a project that wants reproducibility runs `vanta migrate` to convert them once ([30. Migration](30-migration.md)). Interop can be disabled with `[settings] read_foreign_versions = false`.

## Config trust

A `vanta.toml` that only declares tools from the official registry is harmless and requires no trust step — installing `node@24` is as safe as the registry's verification makes it. But a manifest can also **inject environment variables**, **define tasks** (commands that will run), or **reference third-party registries/providers**. Those are capabilities that a malicious repository could abuse, so Vanta applies **trust-on-first-use** (the direnv model):

- The first time Vanta sees a manifest that uses `[env]`, `[tasks]`, or a non-official `[registries]` entry, it prompts: `vanta trust` (showing exactly what would be enabled). Until trusted, those sections are inert — tools still resolve and install, but no env is injected and no task can run.
- Trust is recorded per directory keyed by a content hash of the manifest; **editing** the trusted sections re-triggers the prompt.
- `vanta trust` / `vanta trust --revoke` manage the trust database; `--yes` and CI tokens cover non-interactive flows.
- This is distinct from *artifact/registry* trust (signature keys), which is covered in [15. Security](15-security.md).

## Validation and diagnostics

Configuration is validated before it is used, and errors are span-accurate. `vanta-config` keeps byte spans through parsing so a diagnostic points at the offending line and column with a suggested fix:

```
error[VTA-CFG-0007]: unknown tool value type for `node`
  ┌─ vanta.toml:2:8
  │
2 │ node = 24
  │        ^^ expected a version string like "24" or a table, found an integer
  │
help: quote the version — `node = "24"`
```

Validation covers: TOML syntax (`VTA-CFG-0001`), schema (unknown keys, wrong types — `VTA-CFG-0002..`), referential integrity (a `[registries]` name used by a tool exists), and invariants (no task dependency cycle; no two workspace members claim the same path). A manifest that fails validation is rejected whole; the previous good state is never touched ([02. Architecture](02-architecture.md#failure-and-atomicity-model)). The full code list is in [25. Error Catalog](25-error-and-exit-code-catalog.md).

## Failure modes and trade-offs

- **A foreign file and `vanta.toml` disagree.** `vanta.toml` wins; `vanta doctor` warns if a stale foreign file might mislead other tools.
- **A workspace member pins a tool the root forbids by policy.** Policy ([14. Enterprise](14-enterprise.md)) is evaluated at resolution and reported as `VTA-RES-*`/policy-denied before anything installs.
- **Trade-off: TOML cannot express computed config.** That is intentional — dynamism comes from `[env]` and (rarely) tasks, not from templating the manifest. Projects that genuinely need generated config generate the `vanta.toml` from their own tooling; Vanta consumes static, reviewable input.
- **Trade-off: a global tool set can surprise.** Global `[tools]` are convenient but make `PATH` depend on user config; `vanta which`/`vanta doctor` always make the source explicit, and a project can shadow any global tool.

## Cross-references

- [27. Configuration Reference](27-config-reference.md) — every key, type, default, and constraint.
- [04. CLI](04-cli.md) — the commands that read and write the manifest, and flag/precedence interaction.
- [06. Resolution](06-resolution.md) — how a `[tools]` request becomes a locked resolution.
- [10. Environments](10-environments.md) — how the merged config becomes an active `PATH` per directory.
- [11. Reproducibility](11-reproducibility.md) — the `vanta.lock` that pins what the manifest requests.
- [30. Migration](30-migration.md) — converting foreign version files into a `vanta.toml`.
- [15. Security](15-security.md) — artifact/registry trust, distinct from config trust.
