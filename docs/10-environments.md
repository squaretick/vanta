# 10. Environments & Activation

> How a set of resolved tools becomes the binaries on a user's `PATH`, and how the right versions appear automatically as the user moves between directories. This document specifies global vs project environments, the shell-hook fast path and its per-directory resolution cache, the shim dispatcher fallback, per-shell integration, the ephemeral and subshell execution modes, environment-variable injection, and workspace composition. Owned by `vanta-env` and `vanta-shim`.

**Contents**

- [Environments](#environments)
- [Global vs project scope](#global-vs-project-scope)
- [Automatic version switching](#automatic-version-switching)
- [The shim dispatcher](#the-shim-dispatcher)
- [Why a hybrid](#why-a-hybrid)
- [Shell integration](#shell-integration)
- [Execution modes](#execution-modes)
- [Environment variables](#environment-variables)
- [Workspaces](#workspaces)
- [Failure modes and doctor](#failure-modes-and-doctor)
- [Cross-references](#cross-references)

---

## Environments

An **environment** is the composed set of tools active in a context, exposed as a single bin directory on `PATH`. It is a cheap *view* over the content-addressed store (a directory of links), identified by an `env-id` that is a hash of the merged, resolved tool set. Two contexts that resolve to the same tools share one `env-id` and one view.

```
merged config (global < workspace < project < env < flags)
        │  resolve (lock-authoritative)            envs/<env-id>/bin/
        ▼                                            ├── node     ─► store/blake3-aa…/bin/node
  { node:24.6.0, python:3.13.4, pnpm:9.7.0 }  ─────► ├── python   ─► store/blake3-bb…/bin/python
        │  env-id = hash(resolved set)               └── pnpm     ─► store/blake3-cc…/bin/pnpm
        ▼
  activation puts envs/<env-id>/bin on PATH
```

Building a view is `[7 Link]` of the lifecycle ([08. Installation](08-installation.md)); activation is `[Activate]`. The view is rebuilt only when the resolved set changes; otherwise it is reused.

## Global vs project scope

- **Project environment** — derived from the nearest `vanta.toml` walking up from the current directory, merged over the global tools. Active whenever the user is inside that project tree.
- **Global environment** — derived from `~/.vanta/config.toml`'s `[tools]`. Active everywhere a project does not override a tool. This is where a user keeps ubiquitous CLIs (`ripgrep`, `gh`).

Scope is **inferred** (canon §3): commands run inside a project default to project scope; elsewhere to global. `--global`/`--project` override. A project tool shadows a global tool of the same name; `vanta which` always shows which scope and store path won.

## Automatic version switching

The default mechanism is a **shell hook** that makes the correct environment active as the user `cd`s around — fast, transparent, and with no `vanta` process on the hot path.

```
                ┌──────────────── on prompt / chpwd ────────────────┐
   user cd's    │  1. find nearest vanta.toml up the tree (cheap fs walk)
   into a dir ─►│  2. compute config-hash of the merged manifest+config
                │  3. look up resolution cache (redb) by config-hash
                │       hit  ─► env-id known ─► if != active, swap PATH  (sub-ms)
                │       miss ─► spawn `vanta` to resolve (lock-authoritative),
                │              populate cache, build view if needed, swap PATH
                └────────────────────────────────────────────────────┘
   on leaving the tree ─► restore the prior (global) PATH segment
```

- **The per-directory resolution cache** (redb `resolution_cache`, keyed by the **config hash**) is the speed trick: once a directory's tools are resolved, the warm path is a single keyed read and a `PATH` string swap — **well under 1 ms**, no subprocess. A change to `vanta.toml` changes the config hash and invalidates the entry, so staleness is impossible.
- **PATH injection, not shims, on the interactive path.** The hook prepends `envs/<env-id>/bin` to `PATH` (and removes the previous env's segment), so tool invocations are direct `exec`s of real binaries with zero per-call indirection. This is the mise-style approach, and it is why interactive performance matches a hand-managed `PATH`.
- The hook **never** blocks the prompt on the network: a cold resolve uses the lock (no registry call); only an explicit `add`/`update` re-resolves against the registry.

## The shim dispatcher

Not every context runs the shell hook — IDEs, `cron`, `make`, editor language servers, and `cmd.exe` invoke binaries directly. For those, Vanta installs **shims**: `~/.vanta/bin` (always on `PATH`) holds one tiny `vanta-shim` binary, hardlinked (or, on Windows, exposed as a launcher `.exe`) under each tool name.

```
   IDE / cron / make runs `node`
        │  ~/.vanta/bin/node  ==  vanta-shim (by name)
        ▼
   vanta-shim:  read cwd → find nearest vanta.toml → resolution cache (redb) → store path
        │  (same per-directory cache as the hook; no full config parse)
        ▼
   execve(store/blake3-aa…/bin/node, argv)        # replaces the process; ~sub-ms overhead
```

`vanta-shim` is a separate, minimal crate (no resolver, no HTTP, no WASM) precisely so its cold start is negligible ([03. Repository](03-repository.md), [16. Performance](16-performance.md)). It reads the *same* per-directory resolution cache the hook uses, so the two mechanisms never disagree. If the cache is cold, the shim does a lock-authoritative resolve once and populates it.

## Why a hybrid

mise defaults to PATH-injection (fast, but only where the hook runs); asdf uses shims (work everywhere, but add per-call overhead and historically were slow). Vanta refuses the trade-off ([ADR-0011](24-architecture-decision-records.md)):

| | Shell hook (default) | Shims (fallback) |
| --- | --- | --- |
| Where it works | interactive shells with the hook | everywhere (IDE, cron, scripts, `cmd.exe`) |
| Per-call overhead | none (direct `exec`) | ~sub-ms (one `execve` after a cached lookup) |
| Transparency | `PATH` shows real store paths | `which` shows the shim; `vanta which` shows the real path |
| Source of truth | per-directory resolution cache | the same cache |

Both are always present: the hook gives speed and transparency where it runs; shims guarantee correctness everywhere else. There is one resolution cache, so there is one answer to "what version is `node` here?"

## Shell integration

Activation is installed by evaluating Vanta's generated hook in the shell's startup file:

```sh
# bash / zsh — ~/.bashrc, ~/.zshrc
eval "$(vanta activate bash)"      # or: zsh
```
```fish
# fish — ~/.config/fish/config.fish
vanta activate fish | source
```
```powershell
# PowerShell — $PROFILE
Invoke-Expression (& vanta activate pwsh)
```
```nu
# Nushell — config.nu
vanta activate nu | save -f ~/.vanta/hook.nu   # then `source ~/.vanta/hook.nu`
```

- Supported: **bash, zsh, fish, PowerShell, Nushell, Elvish**. `cmd.exe` has no hook mechanism and uses **shims only**.
- `vanta activate <shell>` prints the hook; it is a pure function of the shell and does not touch the network or state, so it is safe in startup files.
- The hook is small and self-contained: a `chpwd`/prompt callback that does the cache lookup and `PATH` swap described above, plus deactivation logic that cleanly removes Vanta's `PATH` segment when leaving a project (no `PATH` pollution accumulates).
- `vanta doctor` verifies the hook is installed and functioning and prints the exact line to add if not.

## Execution modes

| Command | Builds | Semantics |
| --- | --- | --- |
| `vanta exec -- <cmd>` | current env | run `<cmd>` with the active environment's tools on `PATH` (CI/script-friendly; no shell hook needed) |
| `vanta run <task\|tool> [args]` | current env | run a defined `[tasks]` entry, or a tool/binary in the env ([05. Configuration](05-configuration.md)) |
| `vanta x <tool> [args]` | ephemeral env | fetch (if needed), verify, cache, and run `<tool>` **without adding it** to any manifest — npx/pipx for everything, backed by the same verified store |
| `vanta shell <tool>@<ver> …` | ephemeral subshell | start a subshell with the given versions active; exit to restore |

`vanta x ruff check` resolves `ruff` against the registry, materializes it into the store (so a second `vanta x ruff` is instant), runs it, and leaves the manifest untouched. Ephemeral runs are GC-eligible like any store entry but can be pinned. This is how Vanta replaces `npx`, `uvx`, `pipx run`, and `pkgx` with one verb over a verified store.

## Environment variables

A project's `[env]` table injects variables when its environment is active:

```toml
[env]
NODE_ENV = "development"
DATABASE_URL = "postgres://localhost/app_dev"
PATH = { prepend = ["./node_modules/.bin", "./bin"] }
```

- Static `KEY = "value"` pairs are exported on activation and unset on deactivation.
- `PATH.prepend` adds project-relative directories *after* the tool bin view, so a project's `node_modules/.bin` is found but never shadows the managed tool versions.
- `[env]` injection is a **trusted** capability: a manifest using `[env]` requires `vanta trust` before the variables take effect ([05. Configuration](05-configuration.md#config-trust)). Until trusted, tools still activate but env injection is inert.
- Precedence follows canon §6; an `[env]` value can be overridden by a real shell environment variable or a CLI flag.

## Workspaces

In a workspace ([05. Configuration](05-configuration.md#workspaces)), entering a member directory composes the member's tools over the inherited root tools into one `env-id`. The whole repo shares **one root `vanta.lock`**, so every member's environment is reproducible from a single file. Different members may pin different versions of the same tool; both coexist in the store and the active one is decided by cwd via the same nearest-manifest walk.

## Failure modes and doctor

| Symptom | Cause | Resolution |
| --- | --- | --- |
| Wrong/old version active | shell hook not installed | `vanta doctor` detects it; add `eval "$(vanta activate …)"`; `VTA-ENV-0001` |
| IDE uses system version | IDE bypasses the hook | shims cover it; point the IDE at `vanta which <tool>` |
| `cd` is slow | cold resolution cache or huge tree | first entry resolves then caches; subsequent are sub-ms |
| Stale environment after editing `vanta.toml` | (should not happen) cache keyed by config hash | edit changes the hash → cache miss → re-resolve |
| `PATH` accumulates Vanta segments | broken/duplicated hook install | `vanta doctor` flags duplicate hooks; deactivation removes only Vanta's segment |
| Tool runs but env vars missing | manifest not trusted | `vanta trust`; `VTA-ENV-*` notes the untrusted `[env]` |

`vanta doctor` checks: hook presence per shell, `~/.vanta/bin` on `PATH` and ahead of conflicting managers, shim integrity, resolution-cache health, and `env-id`↔generation consistency, each mapped to a `VTA-ENV-*` code with a printed fix.

## Cross-references

- [02. Architecture](02-architecture.md) — `[7 Link]`/`[Activate]` in the lifecycle and the resolution-cache fast path.
- [06. Resolution](06-resolution.md) — how cwd + merged config become a resolved tool set.
- [09. Store](09-store.md) — the store entries the env view links to and the link strategies.
- [05. Configuration](05-configuration.md) — manifest precedence, `[env]`, and config trust.
- [16. Performance](16-performance.md) — the sub-ms activation and shim-dispatch budgets.
- [17. Cross-platform](17-cross-platform.md) — per-shell hooks and the Windows launcher-shim model.
- [25. Error & Exit-code Catalog](25-error-and-exit-code-catalog.md) — the `VTA-ENV-*` registry.
