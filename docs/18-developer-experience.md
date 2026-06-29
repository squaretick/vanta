# 18. Developer Experience

> The first ten minutes and the daily loop. This document is the narrative of using Vanta: one-line install, shell setup, `vanta add`, automatic per-directory versions, `vanta init` auto-detection, the `git clone && vanta sync` onboarding, editor/IDE integration, CI usage, ephemeral `vanta x`, and `vanta doctor`. It ties together the user-facing behavior specified across the CLI, configuration, environment, and reproducibility docs.

**Contents**

- [The first ten minutes](#the-first-ten-minutes)
- [Adopting an existing project](#adopting-an-existing-project)
- [vanta init and auto-detection](#vanta-init-and-auto-detection)
- [The daily loop](#the-daily-loop)
- [Ephemeral tools with vanta x](#ephemeral-tools-with-vanta-x)
- [Editor and IDE integration](#editor-and-ide-integration)
- [CI usage](#ci-usage)
- [vanta doctor](#vanta-doctor)
- [Error experience](#error-experience)
- [Cross-references](#cross-references)

---

## The first ten minutes

Installation is one line and adds two binaries (`vanta`, `vt`) plus the shim directory to `PATH` ([32. Release Engineering](32-release-engineering.md)):

```sh
# Linux / macOS — downloads a signed, verified binary
curl --proto '=https' --tlsv1.2 -fsSL https://get.vanta.dev | sh

# Windows — PowerShell
irm https://get.vanta.dev/install.ps1 | iex
# or:  winget install vanta   |   scoop install vanta
```

Then enable automatic version switching by adding the hook to the shell profile (printed by the installer and by `vanta doctor`):

```sh
echo 'eval "$(vanta activate zsh)"' >> ~/.zshrc && exec zsh
```

Now the core loop works:

```sh
$ vanta add node@24
  resolving node@24 … 24.6.0
  ↓ node 24.6.0 (macos/aarch64)  24 MB  ✓ verified
  ✓ added node 24.6.0  ·  generation 0001
$ node --version
v24.6.0
```

The newcomer has the right Node on `PATH`, verified, in seconds, having read nothing and chosen no backend — pillar 1 and pillar 10 in practice ([01. Vision](01-vision.md)).

## Adopting an existing project

The headline onboarding story is two commands. A repository that commits `vanta.toml` + `vanta.lock` reproduces exactly on any OS:

```sh
$ git clone git@github.com:acme/app.git && cd app
$ vanta sync
  installing 3 tools from vanta.lock (linux/x86_64/gnu) …
  = node 24.6.0   (cached)
  ↓ pnpm 9.7.0    ✓ verified
  ↓ terraform 1.9.8  ✓ verified
  ✓ environment ready  ·  generation 0001
$ node --version && pnpm --version && terraform version
```

No per-tool installers, no version drift, identical on a teammate's Mac, Linux box, or Windows machine ([11. Reproducibility](11-reproducibility.md)). With `[settings] auto_install = true`, simply `cd`-ing into the project offers to run this for you (after a trust prompt if the manifest injects env or defines tasks — [05. Configuration](05-configuration.md#config-trust)).

## vanta init and auto-detection

For a project that does not yet have a `vanta.toml`, `vanta init` proposes one by scanning for the version signals already present:

```sh
$ vanta init
  detected:
    .nvmrc                → node = "20"
    package.json engines  → pnpm = "9"
    go.mod                → go   = "1.23"
    .python-version       → python = "3.12"
  write vanta.toml with these tools? [Y/n]
  ✓ wrote vanta.toml  (run `vanta sync` to install)
```

It reads `.nvmrc`/`.node-version`, `.python-version`, `.ruby-version`, `.go-version`, `rust-toolchain.toml`, `go.mod`, `package.json` `engines`/`packageManager`, `.tool-versions`, and `.mise.toml` ([05. Configuration](05-configuration.md#interop-with-foreign-version-files)). It never overwrites an existing `vanta.toml` without `--force`, and `vanta migrate` does the deeper conversion (locking versions) from a specific foreign manager ([30. Migration](30-migration.md)).

## The daily loop

```sh
vanta add ripgrep            # add a tool to this project (or --global)
vanta remove ripgrep         # remove it
vanta update                 # bump everything within its constraints; rewrites the lock
vanta outdated               # preview what could update
vanta list                   # what's active here, and from which scope
vanta which node             # the exact store path the active node resolves to
vanta rollback               # undo the last change instantly
```

Moving between projects switches tool versions automatically with no command at all — the shell hook does it on `cd`, sub-millisecond ([10. Environments](10-environments.md)).

## Ephemeral tools with vanta x

`vanta x` runs a tool without adding it to any manifest — the npx/uvx/pipx-run experience for everything, backed by the verified store:

```sh
$ vanta x ruff check            # fetch (first time), verify, cache, run
$ vanta x ripgrep@14 "TODO"     # a specific version, one-off
$ vanta x node@22 -e 'console.log(process.version)'
```

The first run fetches and materializes into the store (so the second is instant); nothing is added to `vanta.toml`. This is ideal for try-before-you-add, CI one-offs, and tools you need once ([10. Environments](10-environments.md#execution-modes)).

## Editor and IDE integration

Because Vanta exposes tools as **real binaries on `PATH`** (via the hook for terminals and shims for everything else), most editors "just work" — they find the project's Node/Python/Go without an extension:

- **VS Code / JetBrains** pick up the active versions when launched from a hooked shell; when launched from a launcher that bypasses the shell, the shims in `~/.vanta/bin` still resolve the correct per-directory version.
- For explicit configuration (interpreter paths, SDK locations), `vanta which <tool>` prints the exact path to point the IDE at.
- Editor extensions and other tooling integrate via the **stable `--json` surface** ([29. Public APIs](29-public-apis.md)) rather than scraping human output.

## CI usage

CI is a first-class, reproducible flow:

```yaml
# illustrative CI steps
- run: curl -fsSL https://get.vanta.dev | sh          # install Vanta (pinned version in real use)
- uses: actions/cache@v4                               # cache the content-addressed store
  with:
    path: ~/.vanta/store
    key: vanta-${{ runner.os }}-${{ hashFiles('vanta.lock') }}
- run: vanta sync --frozen                             # reproduce exactly; fail if lock != manifest
- run: vanta exec -- pnpm test                         # run inside the activated environment
- run: vanta x actionlint                              # a one-off tool, no manifest change
```

- `vanta sync --frozen` guarantees the committed lock is used unchanged ([11. Reproducibility](11-reproducibility.md)).
- Caching `~/.vanta/store` keyed by the lock hash makes warm CI near-instant (store hits + links).
- For air-gapped CI, build a bundle and `vanta restore` + `vanta sync --offline` ([13. Offline](13-offline.md)).

## vanta doctor

`vanta doctor` is the single "is everything healthy?" command, and it prints fixes, not just findings:

```sh
$ vanta doctor
  ✓ vanta 0.9.2 on macos/aarch64
  ✓ ~/.vanta/bin is on PATH (ahead of /opt/homebrew/bin)
  ✗ shell hook not detected for zsh
      fix: add  eval "$(vanta activate zsh)"  to ~/.zshrc   [VTA-ENV-0001]
  ✓ store integrity: 41 entries verified
  ✓ state.db healthy · resolution cache warm
  ⚠ vanta.lock has no entry for windows/x86_64 (a teammate target)
      fix: run  vanta target add windows/x86_64 && vanta lock   [VTA-LOCK-0003]
```

It checks installation, PATH ordering against other managers, the shell hook per shell, shim integrity, store integrity, state-DB health, registry reachability, and lock completeness — each finding mapped to a `VTA-*` code with a concrete remedy ([25. Error Catalog](25-error-and-exit-code-catalog.md)). `vanta doctor --repair` fixes what it safely can (rebuild derived state, re-fetch a corrupt entry).

## Error experience

Errors are designed to teach. Every error carries a stable code, the cause, a suggested fix, and a doc link ([25. Error Catalog](25-error-and-exit-code-catalog.md)):

```
error[VTA-RES-0001]: no version of `node` satisfies "^25"
  the newest available is 24.6.0 (constraint allows >=25)
help: relax the constraint, e.g. `vanta add node@24`, or check `vanta info node`
docs: https://vanta.dev/errors/VTA-RES-0001
```

Good defaults plus legible errors mean the common path needs no documentation and the uncommon path explains itself.

## Cross-references

- [04. CLI](04-cli.md) — the full command and flag reference behind these flows.
- [05. Configuration](05-configuration.md) — `vanta.toml`, auto-detection sources, and config trust.
- [10. Environments](10-environments.md) — automatic switching, `vanta x`, and shell integration.
- [11. Reproducibility](11-reproducibility.md) — `vanta sync`/`--frozen` and the clone→sync onboarding.
- [30. Migration](30-migration.md) — converting from an existing manager.
- [25. Error & Exit-code Catalog](25-error-and-exit-code-catalog.md) — the codes `doctor` and errors reference.
- [29. Public APIs](29-public-apis.md) — the `--json` surface editors and CI consume.
