# 04. CLI & Command Design

> Vanta presents a single command-line surface for every developer tool. This document defines the command philosophy (one verb per intent, inferred scope, sane defaults, CI-friendly behaviour), the complete command reference, the global flag set, output and error presentation, exit codes, environment-variable overrides and precedence, and the scope-inference algorithm. It is the contract between the user and the `vanta` binary; every other subsystem is reached through the verbs defined here.

**Contents**

- [Command Philosophy](#command-philosophy)
- [Global Flags](#global-flags)
- [Command Reference](#command-reference)
- [The Core Flow](#the-core-flow)
- [Scope Inference](#scope-inference)
- [Output Design](#output-design)
- [Error Presentation](#error-presentation)
- [Exit Codes](#exit-codes)
- [Environment & Precedence](#environment--precedence)
- [Interactivity, Confirmation & Trust](#interactivity-confirmation--trust)

---

## Command Philosophy

Vanta's CLI is the most visible expression of pillar 10, *zero unnecessary complexity*, and its guiding rule: **prefer designs that remove user decisions over designs that add configuration.** The command set is built from five concrete commitments.

1. **One verb per intent.** The same verb works regardless of whether the tool is a runtime, a toolchain, a CLI, or a service. `vanta add node` and `vanta add terraform` and `vanta add ripgrep` traverse the identical [resolution lifecycle](02-architecture.md) — the user never learns a per-ecosystem dialect. There is no `vanta install-runtime` versus `vanta install-cli` split; the [provider](07-providers.md) decides the backend, not the user.
2. **Inferred scope.** Almost no command needs an explicit `--global` or `--project` flag. Vanta determines scope from context: inside a project (a directory tree containing `vanta.toml`) operations are project-scoped; elsewhere they are global. The [scope-inference algorithm](#scope-inference) is deterministic and overridable, but the override is rarely typed.
3. **Sensible defaults over flags.** Every flag has a default chosen so that the bare command does the correct thing for the common case: `add` activates, `sync` reconciles exactly, downloads run in parallel, artifacts are verified. Flags exist to *opt out* of safety or to reach power behaviour — they are never required to reach the happy path.
4. **Non-interactive and CI-friendly.** Every command runs unattended. `--yes` accepts all confirmations, `--json` emits a stable machine schema, and [exit codes](#exit-codes) are stable and categorical so a pipeline can branch on them. No command blocks on a prompt when stdin is not a TTY unless a hard trust gate requires it (and even that is satisfiable ahead of time with `vanta trust`).
5. **Progressive disclosure.** The simple surface (`add`, `remove`, `update`, `sync`, `list`, `doctor`) covers ninety per cent of use. Power flags and maintenance verbs (`gc`, `rollback`, `bundle`, `registry`, `cache`) exist but stay out of the way: they are absent from the default `--help` summary's first screen and surface only when the user goes looking.

The result is that a new user types `vanta add node@24` and it works, while a release engineer can drive the same binary deterministically from a script with `--json --yes --offline`.

## Global Flags

These flags are accepted by every command (subject to relevance) and parsed uniformly by the `clap`-derived front end in the [`vanta`](03-repository.md) binary. Per-command flags are documented in the [command reference](#command-reference).

| Flag | Alias | Argument | Default | Effect |
|------|-------|----------|---------|--------|
| `--global` | `-g` | — | inferred | Force global scope (`~/.vanta`), ignoring any nearby `vanta.toml`. |
| `--project` | — | — | inferred | Force project scope; create `vanta.toml` in `--cwd` if none exists. |
| `--yes` | `-y` | — | `false` | Assume "yes" for every confirmation; never prompt. Implied when stdin is not a TTY for non-trust prompts. |
| `--json` | — | — | `false` | Emit the stable machine schema on stdout; suppress human chrome. See [Output Design](#output-design). |
| `--offline` | — | — | `false` | Never touch the network; fail closed (`VTA-NET-*`, exit `5`) if an artifact or metadata is not cached. |
| `--no-verify` | — | — | `false` | Skip checksum/signature gates. Discouraged; prints a loud warning (`VTA-VRF-0001`) on every use. |
| `--refresh` | — | — | `false` | Bypass the registry metadata TTL and re-resolve from upstream. |
| `--quiet` | `-q` | — | `false` | Suppress all non-error output; progress bars off. |
| `--verbose` | `-v` | — | `0` | Increase verbosity; repeatable (`-vv`, `-vvv`) for debug and trace. |
| `--color` | — | `auto\|always\|never` | `auto` | Colour policy. `auto` colours only an interactive TTY; honours `NO_COLOR`. |
| `--cwd` | — | `<dir>` | process cwd | Run as if invoked from `<dir>`; affects scope inference and activation. |
| `--config` | — | `<file>` | `~/.vanta/config.toml` | Use an alternate global config file. |

Conflicting scope flags (`--global` with `--project`) are a usage error (exit `2`). `--quiet` and `--verbose` may be combined: errors are always emitted, verbosity only affects diagnostics.

## Command Reference

Commands are grouped by how often a user reaches for them. Aliases are first-class: `vt` is the short binary alias (identical to `vanta`), and per-command aliases (`rm`, `up`, `ls`, `gen`, `x`) are interchangeable with their long forms.

### Primary commands

The everyday surface. These six cover the majority of all invocations.

| Command | Purpose | Key flags | Example |
|---------|---------|-----------|---------|
| `add <tool>[@<ver>] …` | Resolve, install, activate; write `[tools]` to the manifest and pin in `vanta.lock`. Scope inferred. | `--global`, `--dev`, `--no-activate`, `--dry-run` | `vanta add node@24 ripgrep` |
| `remove <tool> …` (`rm`) | Remove tools from the scope's manifest; update the lock; new generation. | `--global`, `--gc` | `vanta rm terraform` |
| `update [tool …]` (`up`) | Re-resolve to the newest version each constraint allows; update the lock. Bare form updates all. | `--global`, `--latest`, `--dry-run` | `vanta up node` |
| `sync` | Reconcile the machine to `vanta.toml` + `vanta.lock` exactly: install what is missing, prune what is extra. The reproducibility verb; run after `git clone`. | `--frozen`, `--prune`, `--dry-run` | `vanta sync` |
| `list` (`ls`) | Show tools active in the scope, their versions, and source. | `--global`, `--all`, `--outdated`, `--json` | `vanta ls --outdated` |
| `doctor` | Diagnose install health: PATH, shell hook, store integrity, registry reachability, orphaned generations. | `--fix`, `--json` | `vanta doctor` |

Notes. `add --dev` records a tool under a development-only group that `sync` may skip in production with `--no-dev` (mirrors `[tools]` versus a dev group; see [05. Configuration](05-configuration.md)). `add --dry-run` runs stages `[1 Request]` through `[3 Plan]` and prints the plan without fetching. `sync --frozen` forbids any lock change and fails (`VTA-LOCK-*`) if the manifest and lock disagree — the canonical CI assertion that the lock is current.

### Everyday commands

Reached often once a project is set up: running tools, inspecting versions, ad-hoc execution.

| Command | Purpose | Key flags | Example |
|---------|---------|-----------|---------|
| `run <name> [args …]` | If `<name>` is a `[tasks]` entry, run the task; otherwise run the tool `<name>` (from the active/locked version, fetching ephemerally if absent). | `--global` | `vanta run test` |
| `x <name> [args …]` | Always an ephemeral *tool* run, never a task: fetch-on-demand, verify, cache, execute — `npx`/`pipx` semantics for any tool. | `@<ver>`, `--from <provider>` | `vanta x ruff check` |
| `exec -- <cmd …>` | Run an arbitrary command inside the activated environment (composed `PATH`) without launching a login shell. | `--global` | `vanta exec -- make build` |
| `which <tool>` | Print the resolved absolute path the active scope would execute. | `--global`, `--json` | `vanta which node` |
| `use <tool>@<ver>` | Pin/select a version for the current scope. Thin sugar over `add` that always writes the explicit version. | `--global` | `vanta use python@3.13` |
| `shell <tool>@<ver> …` | Start an ephemeral subshell with the given versions active; exit restores the prior environment. | `--global` | `vanta shell node@20 python@3.11` |
| `search <query>` | Search configured registries for tools matching `<query>`. | `--json`, `--limit` | `vanta search postgres` |
| `info <tool>` | Show metadata: provider, backend, available versions, current resolution, dependencies. | `--versions`, `--json` | `vanta info node` |
| `outdated` | List tools that could move under current constraints, with current → candidate versions. | `--global`, `--json` | `vanta outdated` |

The `run`/`x` split removes the only genuine ambiguity in the surface: `run` is convenient and task-aware; `x` is unambiguous and always means "fetch and run this tool now." A name collision between a task and a tool is resolved in favour of the task under `run`, and `x` is documented as the escape hatch.

### Lifecycle & maintenance commands

Lower-frequency operations: project setup, storage hygiene, generations, offline workflows, trust, registries, migration, configuration, shell integration, and self-management.

| Command | Purpose | Key flags | Example |
|---------|---------|-----------|---------|
| `init` | Create a `vanta.toml`, auto-detecting tools from the project (lockfiles, `.tool-versions`, language files). | `--from <tool>`, `--force` | `vanta init` |
| `gc` | Garbage-collect unreferenced store entries via mark-and-sweep from roots. Safe and atomic. | `--dry-run`, `--keep-days <n>` | `vanta gc --dry-run` |
| `rollback [gen]` | Revert the active scope to a prior generation by pointer swap (no fetches). Bare form reverts one step. | `--global` | `vanta rollback 7` |
| `generations` (`gen`) | List generations for the scope with timestamps and diffs; mark the current. | `--global`, `--json` | `vanta gen` |
| `bundle [--out <file>]` | Produce a portable, verified archive of resolutions + artifacts for air-gapped transfer. | `--out`, `--platform`, `--all` | `vanta bundle --out api.vbundle` |
| `restore <file>` | Install from a bundle without network access; verifies every artifact. | `--yes` | `vanta restore api.vbundle` |
| `cache <subcmd>` | Inspect/prune caches: `cache info`, `cache prune`, `cache verify`, `cache path`. | `--prune`, `--json` | `vanta cache prune` |
| `trust [path\|provider]` | Manage trust for configs, providers, registries, and keys: `trust`, `trust --list`, `trust --revoke`. | `--list`, `--revoke` | `vanta trust .` |
| `registry <subcmd>` | Manage registries: `registry add`, `registry list`, `registry refresh`, `registry remove`. | `--priority`, `--auth` | `vanta registry add corp https://reg.acme.dev` |
| `migrate [from]` | Import tools/versions from another manager (mise, asdf, nvm, fnm, pyenv, rbenv, volta, brew, scoop, pkgx). | `--dry-run`, `--write` | `vanta migrate mise` |
| `config <subcmd>` | Show/edit configuration: `config get`, `config set`, `config edit`, `config path`. | `--global`, `--json` | `vanta config get jobs` |
| `activate <shell>` | Print the shell hook to stdout for `eval`. Shells: bash, zsh, fish, PowerShell, Nushell, Elvish. | — | `eval "$(vanta activate zsh)"` |
| `completions <shell>` | Print shell completion script for the given shell. | — | `vanta completions fish` |
| `self update` | Update the `vanta`/`vt` binary itself; verified with cosign + SLSA provenance. | `--check`, `--version` | `vanta self update` |
| `self uninstall` | Remove the binary and optionally `~/.vanta`; reverses shell integration. | `--purge` | `vanta self uninstall` |

## The Core Flow

A realistic session illustrating the day-to-day loop. Output is the default human-readable form on an interactive TTY (colour and progress rendered here as plain text).

```console
$ cd ~/work/api
$ vanta add node@24
  Resolve   node@24 → 24.4.1   (provider official/node · backend curated)
  Fetch     node-v24.4.1-linux-x64.tar.xz   28.3 MiB  ▕████████████████▏ 41.2 MiB/s
  Verify    sha256 ✓   minisign ✓
  Material  store/blake3-9f2a…c71   (reflink · 312 files)
  Link      envs/api  +node +npm +npx +corepack
  Commit    generation 7 → 8
✓ added node@24.4.1 in 1.84s · PATH updated for ~/work/api

$ vanta x ruff check
  Resolve   ruff@latest → 0.6.9   (provider official/ruff · backend github)
  Fetch     ruff-x86_64-unknown-linux-gnu.tar.gz   9.1 MiB  ▕████████████████▏
  Verify    sha256 ✓
  Material  store/blake3-4c8e…1a0   (ephemeral · not added to vanta.toml)
All checks passed!

$ vanta sync
  Lock      vanta.lock   4 tools · 3 platforms   ✓ current
  Plan      4 tools · 2 present · 2 to fetch
  Fetch     python-3.13.1-linux-x64.tar.zst   ▕████████████████▏
  Fetch     terraform_1.9.5_linux_amd64.zip   ▕████████████████▏
  Verify    4/4 ✓
  Link      envs/api  node python terraform ripgrep
  Commit    generation 8 → 9
✓ environment reproduced from lock in 6.21s · 4 tools active

$ vanta rollback
  Current   generation 9   (2026-06-29 14:02:11)
  Target    generation 8   (2026-06-29 13:55:40)   ⟵ rollback
  Diff      −python@3.13.1  −terraform@1.9.5
✓ rolled back to generation 8 (pointer swap · 0 fetches) in 12 ms
```

Two properties to note. First, `vanta x ruff` neither edited `vanta.toml` nor created a generation — it executed from the verified store cache and exited. Second, `vanta rollback` performed no I/O beyond a pointer swap, because every prior generation's store entries are still present (until `vanta gc`); rollback is therefore effectively instantaneous, in line with pillar 7, *atomic operations*.

## Scope Inference

Scope is decided once, early, before any resolution. The algorithm:

```
fn infer_scope(cwd, flags) -> Scope:
    if flags.global  && flags.project: error(VTA-CFG-0002, exit 2)   # conflicting
    if flags.global:  return Global
    if flags.project: return Project(cwd, create_if_missing = true)

    # Walk up from --cwd (or the process cwd) to the filesystem root,
    # stopping at the first directory that contains a `vanta.toml`.
    for dir in ancestors(cwd):
        if exists(dir / "vanta.toml"):
            return Project(dir)

    return Global
```

The walk stops at the nearest `vanta.toml`; nested manifests are not merged across the project boundary (workspace composition is a separate, opt-in mechanism — see [05. Configuration](05-configuration.md#workspaces)). Worked examples:

| Invocation | cwd contains/under `vanta.toml`? | Flags | Resulting scope |
|------------|-----------------------------------|-------|-----------------|
| `vanta add node` | yes (`~/work/api/vanta.toml`) | none | Project `~/work/api` |
| `vanta add node` | no | none | Global `~/.vanta` |
| `vanta add node -g` | yes | `--global` | Global (override) |
| `vanta add node --project` | no | `--project` | Project (creates `./vanta.toml`) |
| `vanta add node --cwd ~/work/api` | depends on target | `--cwd` | Project/Global per target dir |
| `vanta ls` (subdir of project) | yes (ancestor) | none | Project (nearest ancestor manifest) |

This design means the *common* command — `vanta add <tool>` typed inside a repository — requires no flags and does the locally-correct thing, while a deliberate global install is one short flag away. It directly serves the "remove user decisions" rule: the user states intent (`add node`), not bookkeeping (where to record it).

## Output Design

Vanta produces two distinct output contracts, selected by `--json` and by TTY detection.

| Aspect | Human (default) | `--json` |
|--------|-----------------|----------|
| Audience | Interactive terminal | Scripts, CI, editors, other tools |
| Stream | Status on stderr, results on stdout | A single JSON document/stream on stdout |
| Colour | Per `--color` (auto: TTY only) | Never |
| Progress bars | Yes, on a TTY; redraw in place | Never; structured progress events if `--verbose` |
| Stability | May change between releases | **Stable schema — a public API** |
| Verbosity | Concise; one line per lifecycle stage | Full structured records |

Defaults are TTY-aware. When stdout is a pipe, colour and progress bars are suppressed automatically even without `--color never`, and `--yes` is implied for non-trust confirmations so piped invocations do not hang. `--quiet` reduces human output to nothing but errors; `--verbose` adds per-stage timing, resolved URLs, store keys, and (at `-vvv`) wire-level traces.

The `--json` schema is versioned and treated as a compatibility surface: fields are added, never removed or repurposed within a major version, and every record carries a `schema` discriminator. The full schema — record shapes for `add`, `list`, `outdated`, `doctor`, errors, and progress events — is specified in [29. Public APIs](29-public-apis.md). A representative `vanta which --json`:

```json
{ "schema": "vanta.which/1", "tool": "node", "version": "24.4.1",
  "path": "/home/u/.vanta/envs/api/bin/node", "scope": "project",
  "store_key": "blake3-9f2a...c71", "provider": "official/node" }
```

## Error Presentation

Every failure carries a stable error code of the form **`VTA-<AREA>-<NNNN>`** (areas: `CFG`, `RES`, `REG`, `PROV`, `NET`, `VRF`, `STORE`, `INST`, `ENV`, `LOCK`, `SYS`, `INT`). A good Vanta error states four things: the **problem**, the **cause**, a **suggested fix**, and a **documentation link**. When the error originates in a manifest, it is rendered with a source span pointing at the offending key.

```console
$ vanta sync
error[VTA-RES-0007]: no version of `node` satisfies `@25`
  ┌─ vanta.toml:3:8
  │
3 │ node = "25"
  │        ^^^^ requested here
  │
  = cause: the official/node provider lists 0 versions matching `25`;
           the newest published version is 24.4.1
  = help:  use `node = "24"` for the latest 24.x line, or `node = "latest"`
  = docs:  https://vanta.dev/errors/VTA-RES-0007
```

Errors are emitted on stderr in both modes; under `--json` they are a structured record (`{"schema":"vanta.error/1","code":"VTA-RES-0007","message":…,"span":…,"help":…}`) and the process still exits with the matching categorical [exit code](#exit-codes). The complete catalogue — every code, its meaning, and its remediation — lives in [25. Error & Exit-code Catalog](25-error-and-exit-code-catalog.md). The span-accurate rendering for configuration errors (`VTA-CFG-*`) is produced by the diagnostics layer in [`vanta-config`](03-repository.md) and detailed in [05. Configuration](05-configuration.md#validation--diagnostics).

## Exit Codes

Exit codes are stable and categorical so callers can branch without parsing text. They map onto the error areas: a script can treat anything `>= 1` as failure, or switch on the specific class.

| Code | Meaning | Typical error areas |
|------|---------|---------------------|
| `0` | Success | — |
| `1` | Generic failure (uncategorised) | `INT` |
| `2` | Usage / CLI error (bad flags, conflicting scope) | — |
| `3` | Configuration / manifest invalid | `CFG` |
| `4` | Resolution failed (no satisfying version, conflict) | `RES` |
| `5` | Network / offline (unreachable, or `--offline` miss) | `NET`, `REG` |
| `6` | Verification / security failure (checksum, signature, provenance) | `VRF` |
| `7` | Store / I/O failure | `STORE`, `INST` |
| `8` | Not found (unknown tool, registry, generation) | `REG`, `PROV` |
| `9` | Trust required (untrusted config/registry/provider) | `VRF` (trust gate) |

Determinism note: a single invocation maps to exactly one exit code — the most specific applicable class. Verification failures (`6`) and trust gates (`9`) always take precedence over generic failure so that supply-chain problems are never masked.

## Environment & Precedence

`VANTA_HOME` relocates the entire data root (default `~/.vanta`; `%LOCALAPPDATA%\Vanta` on Windows). Most `[settings]` keys also have a `VANTA_*` environment override, useful for CI and one-shot runs. The standard `NO_COLOR` convention is honoured. The complete list is normative in [27. Configuration Reference](27-config-reference.md#environment-variable-overrides); the commonly used subset:

| Variable | Overrides | Example |
|----------|-----------|---------|
| `VANTA_HOME` | data root path | `/opt/vanta` |
| `VANTA_CONFIG` | global config file path | `/etc/vanta/config.toml` |
| `VANTA_OFFLINE` | `settings.offline` | `1` |
| `VANTA_JOBS` | `settings.jobs` (parallelism) | `8` |
| `VANTA_VERIFY` | `settings.verify` | `false` (discouraged) |
| `VANTA_MIRROR` | `settings.mirror` URL | `https://mirror.acme.dev` |
| `VANTA_COLOR` | `settings.color` policy | `never` |
| `VANTA_LINK_STRATEGY` | `settings.link_strategy` | `hardlink` |
| `VANTA_LOG` | diagnostic log level | `debug` |
| `NO_COLOR` | disables colour (any value) | `1` |

Configuration precedence runs low to high (later wins), matching canon §6:

```
built-in defaults
  < global config (~/.vanta/config.toml)
    < project vanta.toml (nearest, walking up)
      < [env] / per-directory overrides
        < environment variables (VANTA_*)
          < CLI flags
```

Thus a CLI flag always wins, an environment variable beats any file, and a project manifest beats the global config — but a project's own `[env]` block can still layer directory-specific values on top of its `[settings]`. The merge algorithm and a worked conflict resolution are in [05. Configuration](05-configuration.md#precedence--merging).

## Interactivity, Confirmation & Trust

Vanta prompts in exactly two situations, and both are bypassable for automation.

1. **Destructive or large operations.** `remove`, `gc`, `self uninstall --purge`, and any plan that would prune more than it adds present a one-line confirmation summarising the diff. `--yes`/`-y` accepts it; a non-TTY stdin implies `--yes` for these. Example:

   ```console
   $ vanta gc
   This will delete 6 store entries (1.9 GiB) unreferenced by any retained generation.
   Proceed? [y/N]
   ```

2. **Trust gates (TOFU).** A new or changed `vanta.toml` that injects `[env]`, defines `[tasks]`, or references a non-official registry/provider is *untrusted* until approved, following a direnv-style trust-on-first-use model. Until trusted, Vanta will resolve and install tools from the official registry but will **not** apply the file's `[env]` or run its tasks, and it exits `9` (`VTA-VRF-*`, "trust required") if such an action is requested non-interactively.

   ```console
   $ cd ~/work/api
   vanta: this project's vanta.toml sets environment variables and defines tasks.
          review:  vanta config show --project
          trust:   vanta trust .
   (env and tasks are inactive until trusted)
   $ vanta trust .
   ✓ trusted ~/work/api/vanta.toml (fingerprint blake3-2b9e…). Re-prompts if it changes.
   ```

   Trust is recorded by content fingerprint in `~/.vanta/trust/`, so editing the file re-arms the prompt. Tool installs from the official registry never require per-project trust; only env injection, task definitions, and custom registries/providers do. The complete trust model — fingerprinting, revocation, key pinning, and the relationship to provider/registry signing — is specified in [15. Security & Supply Chain](15-security.md).

## Cross-references

- [02. Architecture](02-architecture.md) — the resolution lifecycle `[1 Request]`..`[8 Commit]` every command drives.
- [05. Configuration](05-configuration.md) — `vanta.toml` and `config.toml` semantics, precedence merging, and trust that the CLI reads.
- [27. Configuration Reference](27-config-reference.md) — exhaustive key and environment-variable tables backing `config get/set` and the override table here.
- [25. Error & Exit-code Catalog](25-error-and-exit-code-catalog.md) — the full `VTA-<AREA>-<NNNN>` registry and remediation for the codes surfaced by the CLI.
- [29. Public APIs](29-public-apis.md) — the stable `--json` schema the CLI emits as a machine contract.
- [15. Security & Supply Chain](15-security.md) — the trust-on-first-use model and signature gates behind confirmation and `--no-verify`.
- [10. Environments & Activation](10-environments.md) — how `activate`, `exec`, `shell`, and `which` compose and expose the environment on `PATH`.
