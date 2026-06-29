# 30. Migration & Import

> How a user or team moves to Vanta from an existing manager with minimal friction: `vanta migrate` importers and their fidelity, read-only interop with foreign version files, the safe migration workflow, per-tool notes and honest limitations, coexistence during transition, and removing the old tool afterward. Owned by `vanta-migrate`.

**Contents**

- [Importers](#importers)
- [Read-only interop](#read-only-interop)
- [The migration workflow](#the-migration-workflow)
- [Fidelity model](#fidelity-model)
- [Per-tool notes](#per-tool-notes)
- [Coexistence and uninstalling the old tool](#coexistence-and-uninstalling-the-old-tool)
- [Failure modes](#failure-modes)
- [Cross-references](#cross-references)

---

## Importers

`vanta migrate [from]` converts an existing setup into a `vanta.toml` (and then a locked `vanta.lock` via `vanta sync`). It auto-detects the source if `from` is omitted.

| Source | Reads | Maps to |
| --- | --- | --- |
| `mise` | `.mise.toml`, `~/.config/mise` | `[tools]`, `[env]`, and (best-effort) `[tasks]` |
| `asdf` | `.tool-versions`, plugin list | `[tools]` |
| `nvm` / `fnm` | `.nvmrc`, `.node-version` | `node = <version>` |
| `pyenv` | `.python-version` | `python = <version>` |
| `rbenv` | `.ruby-version` | `ruby = <version>` |
| `goenv`/`gvm` | `.go-version`, `go.mod` toolchain | `go = <version>` |
| `rustup` | `rust-toolchain.toml` | `rust = <channel/version>` |
| Volta | `package.json` `volta` field | `node`/`pnpm`/`yarn = <version>` |
| Homebrew | `Brewfile` | CLI formulae → `[tools]` (casks/GUI skipped) |
| Scoop | `scoop export` JSON | apps → `[tools]` (where a provider exists) |
| pkgx | project deps / `dev` markers | `[tools]` |

`vanta migrate --report` prints exactly what mapped, what was approximated, and what could not be mapped, before writing anything.

## Read-only interop

Even without migrating, Vanta **reads** common foreign version files at resolution time, so a repository that already pins versions works immediately ([05. Configuration](05-configuration.md#interop-with-foreign-version-files)): `.tool-versions`, `.mise.toml`, `.nvmrc`, `.node-version`, `.python-version`, `.ruby-version`, `.go-version`, `rust-toolchain.toml`. These are never modified. If a `vanta.toml` is also present, it wins. Foreign files carry no artifact identity, so they are not locked with hashes; converting via `vanta migrate` is what adds reproducibility. Interop is toggled by `[settings] read_foreign_versions`.

## The migration workflow

```
$ vanta migrate            # auto-detect; or `vanta migrate asdf`
  detected asdf (.tool-versions) with: node 20.11.0, python 3.12.2, terraform 1.7.5
  proposed vanta.toml:
    [tools]
    node = "20.11.0"
    python = "3.12.2"
    terraform = "1.7.5"
  could not map: 1 asdf plugin "weird-internal-tool" (no Vanta provider; see report)
  write vanta.toml? [Y/n] y
  ✓ wrote vanta.toml   ·   next: `vanta sync` to install + lock

$ vanta sync               # resolve for all targets, install, write vanta.lock
$ vanta doctor             # verify PATH ordering vs the old manager
```

The steps are always: **detect → preview (with the report of unmapped items) → write `vanta.toml` → `vanta sync` → verify → (optionally) remove the old tool**. Nothing is installed or changed until you accept the preview, and the old manager is untouched until you choose to remove it.

## Fidelity model

Each mapping is labeled so expectations are clear:

| Fidelity | Meaning | Example |
| --- | --- | --- |
| **exact** | maps directly and reproducibly | `.nvmrc` `20.11.0` → `node = "20.11.0"` |
| **approximate** | maps but semantics differ slightly | a mise `latest` alias → `node = "latest"` (now lock-pinned on sync) |
| **manual** | needs a human decision | a mise task with shell-specific logic → review before adding to `[tasks]` |
| **dropped** | no equivalent; reported, not silently lost | a Homebrew cask (GUI app — out of scope) |

`vanta migrate --report` lists every item with its fidelity, so a migration is auditable and nothing disappears quietly.

## Per-tool notes

- **Homebrew.** CLI formulae that have a Vanta provider map to `[tools]`; **casks/GUI apps are out of scope** (Vanta manages developer tools, not desktop apps — [01. Vision](01-vision.md) non-goals) and are reported as dropped. System libraries installed via brew are also out of scope.
- **System packages (apt/dnf/pacman).** Out of scope — those manage the OS; Vanta does not import them.
- **Language library dependencies.** Out of scope by design: Vanta imports the *tools* (the right `node`/`pnpm`/`uv`), not your app's `package.json`/`requirements.txt`/`Cargo.toml` graph — those stay with their native managers.
- **mise tasks / env.** `[env]` maps directly; tasks map best-effort and are flagged `manual` where they rely on mise-specific features.
- **asdf plugins without a Vanta provider.** Reported as unmapped with a pointer to request or author a provider ([22. Provider SDK](22-provider-sdk.md)).

## Coexistence and uninstalling the old tool

- **Run side by side during transition.** Vanta and the old manager can coexist; PATH ordering decides who wins. `vanta doctor` checks that `~/.vanta/bin` is ahead of the old manager's shims/bin so Vanta's versions take effect, and warns if not.
- **Verify, then remove.** Once `vanta sync` reproduces the toolset and `vanta doctor` is green, remove the old manager's shell hook/shims and uninstall it. Vanta does not delete another tool's files automatically (it never touches what it did not create — a safety rule), but `vanta migrate --cleanup` prints the exact steps for the detected source.
- Removing foreign version files is optional; if kept, `vanta.toml` continues to take precedence.

## Failure modes

| Scenario | Behavior |
| --- | --- |
| Source tool not detected | `vanta migrate <name>` to specify; clear message listing supported sources |
| A version has no Vanta provider | reported as unmapped (`dropped`/`manual`); the rest still migrate |
| Foreign and `vanta.toml` disagree | `vanta.toml` wins; `doctor` warns about the stale foreign file |
| PATH still resolves the old manager | `doctor` flags ordering with the exact fix |
| A mapped version no longer exists upstream | `vanta sync` reports `VTA-RES-0001`; pick an available version |

## Cross-references

- [05. Configuration](05-configuration.md) — `vanta.toml`, foreign-file interop, and precedence.
- [11. Reproducibility](11-reproducibility.md) — why `vanta sync` (locking) is the step that adds reproducibility.
- [18. Developer Experience](18-developer-experience.md) — `vanta init` auto-detection and onboarding.
- [22. Provider SDK](22-provider-sdk.md) — authoring a provider for an unmapped tool.
- [04. CLI](04-cli.md) — `vanta migrate` flags and `vanta doctor`.
