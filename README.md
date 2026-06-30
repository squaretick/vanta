<p align="center">
  <a href="https://crates.io/crates/vanta"><img src="assets/banner.svg" alt="Vanta — every developer tool, one command" width="820"></a>
</p>

# Vanta

[![crates.io](https://img.shields.io/crates/v/vanta.svg)](https://crates.io/crates/vanta)
[![release](https://img.shields.io/github/v/release/squaretick/vanta?filter=!vanta-*&sort=semver)](https://github.com/squaretick/vanta/releases/latest)
[![crates.io downloads](https://img.shields.io/crates/d/vanta.svg)](https://crates.io/crates/vanta)
[![release downloads](https://img.shields.io/github/downloads/squaretick/vanta/total.svg)](https://github.com/squaretick/vanta/releases)
[![docs.rs](https://img.shields.io/docsrs/vanta-core)](https://docs.rs/vanta-core)
[![CI](https://github.com/squaretick/vanta/actions/workflows/ci.yml/badge.svg)](https://github.com/squaretick/vanta/actions/workflows/ci.yml)
[![MSRV](https://img.shields.io/badge/rustc-1.83%2B-orange.svg)](https://www.rust-lang.org)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](#license)

**Every developer tool, one command.** Vanta installs, manages, updates, and
reproduces developer tools — runtimes, toolchains, and CLIs — across Linux, macOS,
and Windows, behind one consistent interface. One static binary, one config file,
one lockfile. You never have to care which package manager a tool comes from.

```sh
vanta add node@24          # a runtime
vanta add rust             # a toolchain
vanta add terraform gh     # CLIs
vanta x ruff check         # run a tool once, without installing it
vanta sync                 # reproduce a project's tools exactly (after `git clone`)
vanta rollback             # undo the last change, instantly
```

Every artifact is checksum- and signature-verified, materialized into an immutable
content-addressed store, and pinned in a cross-platform lockfile — so a teammate on
a different OS gets a byte-for-byte identical toolset from the same `vanta.lock`.

Install two identical binaries: **`vanta`** and its short alias **`vt`**.

## Install

### Install (prebuilt, no compile)

The fastest path — downloads a verified prebuilt binary, no Rust toolchain
required. Works on Linux and macOS (x86_64 and arm64):

```sh
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/squaretick/vanta/main/scripts/install.sh | sh
```

The installer detects your OS/arch, downloads the matching release archive,
verifies its SHA256 checksum, and installs `vanta` and `vt` into `~/.local/bin`
(override with `INSTALL_DIR=…`). Pin a version with `VANTA_VERSION=v0.1.0`.

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/squaretick/vanta/main/scripts/install.ps1 | iex
```

Or download the `vanta-<version>-x86_64-pc-windows-msvc.zip` asset from the
[latest release](https://github.com/squaretick/vanta/releases/latest), unzip it,
and put `vanta.exe` / `vt.exe` on your `PATH`. (The shell installer above also
works from Git Bash / MSYS2.)

### Other channels

```sh
# Cargo (any platform with Rust)
cargo install vanta
cargo binstall vanta              # prebuilt binary, no compile

# Homebrew (macOS / Linux)
brew install squaretick/tap/vanta

# Debian / Ubuntu
curl -fsSLO https://github.com/squaretick/vanta/releases/latest/download/vanta_amd64.deb
sudo apt install ./vanta_amd64.deb

# Fedora / RHEL
sudo dnf install https://github.com/squaretick/vanta/releases/latest/download/vanta.x86_64.rpm

# Docker
docker run --rm ghcr.io/squaretick/vanta:latest --version
```

Or build from source (Rust 1.83+):

```sh
cargo build --release
```

Then enable automatic, per-directory version switching by adding the shell hook to
your shell's startup file:

```sh
# bash
echo 'eval "$(vanta activate bash)"' >> ~/.bashrc

# zsh
echo 'eval "$(vanta activate zsh)"' >> ~/.zshrc

# fish
echo 'vanta activate fish | source' >> ~/.config/fish/config.fish

# PowerShell (Windows / pwsh)
Add-Content $PROFILE 'Invoke-Expression (& vanta activate pwsh | Out-String)'
```

Restart your shell (or `source` the file) to pick up the hook.

## Quick start

```sh
# Add tools to the current project (writes vanta.toml + vanta.lock)
$ vanta add node@24 pnpm@9
installing node 24.6.0
  ✓ node 24.6.0 → blake3-aa3f…
installing pnpm 9.7.0
  ✓ pnpm 9.7.0 → blake3-7b21…

$ node --version
v24.6.0

# Reproduce a checked-out project on any OS
$ git clone git@github.com:acme/app.git && cd app
$ vanta sync

# Run a tool once without adding it
$ vanta x ripgrep@14 "TODO"

# Inspect, update, and roll back
$ vanta list
$ vanta outdated
$ vanta update node
$ vanta rollback
```

## What Vanta does

- **One command for everything** — runtimes (`node`, `python`, `go`), toolchains
  (`rust`, `java`), and CLIs (`terraform`, `gh`, `ripgrep`) all install the same way.
- **Reproducible by default** — a cross-platform `vanta.lock` pins exact versions
  *and* artifact hashes for every target OS; `vanta sync --frozen` reproduces them.
- **Content-addressed store** — immutable, deduplicated, integrity-checked entries
  under `~/.vanta/store`, with atomic *generations* and instant `vanta rollback`.
- **Secure by default** — every artifact is verified (SHA-256/BLAKE3 + Ed25519/
  minisign signatures, fail-closed); providers run sandboxed in WebAssembly.
- **Automatic version switching** — a fast shell hook (plus a universal shim
  fallback) puts the right versions on `PATH` as you move between directories.
- **Offline-friendly** — a content-addressed cache, mirrors, and portable
  `vanta bundle` / `vanta restore` archives for air-gapped environments.
- **Cross-platform** — one identical model on Linux, macOS, and Windows.

## Configuration

A project declares its tools in `vanta.toml`:

```toml
[tools]
node = "24"
python = "3.13"
terraform = "1.9"

[env]
NODE_ENV = "development"

[tasks]
dev  = "pnpm dev"
test = "pnpm test"
```

`vanta add` / `vanta sync` resolve these to exact, hashed artifacts in
`vanta.lock` (committed to version control). Configuration is documented in
[05. Configuration](docs/05-configuration.md) and
[27. Configuration Reference](docs/27-config-reference.md).

## Registry

Out of the box, `vanta` resolves tools against the **official, minisign-signed
registry** in [`registry/registry.toml`](registry/registry.toml). On every run the
CLI fetches the index and its detached signature, verifies the signature against a
**pinned root key compiled into the binary**, and only then trusts any entry — then
each artifact is gated by its published SHA-256. This is the trust anchor described
in [15. Security](docs/15-security.md) (no entry, checksum, or signing key from the
index is trusted until the index itself is root-verified).

The seed set ships real, current-stable releases across Linux (x86_64/aarch64) and
macOS (x86_64/aarch64):

| Tool | Versions |
| --- | --- |
| `node` | 22.11.0, 20.18.0 |
| `go` | 1.23.4, 1.22.10 |
| `python` (python-build-standalone) | 3.12.7, 3.11.10 |
| `ripgrep` | 14.1.1 |
| `fd` | 10.2.0 |
| `jq` | 1.7.1 |
| `uv` | 0.5.11 |

```sh
vanta add ripgrep@14.1.1     # resolve + verify + install from the official registry
vanta install jq@1.7.1       # `install` is an alias for `add`
```

**Override the registry source** with `$VANTA_REGISTRY`:

- an `https://` URL — must carry a `<url>.minisig` signed by a pinned root
  (add your own root to `~/.vanta/trust/roots.toml`), or
- a **local file path** — user-owned and trusted as-is (handy for development and
  air-gapped mirrors).

Maintainers regenerate and re-sign the registry with
`cargo xtask registry-gen`; see [registry/README.md](registry/README.md) for the
schema, the per-artifact-signature policy, and root-key handling.

## Commands

| Command | Purpose |
| --- | --- |
| `vanta add <tool>[@ver] …` (alias `install`) | Resolve and install tools into the current scope |
| `vanta remove <tool>` | Remove a tool |
| `vanta update [tool]` | Update within the manifest's version constraints |
| `vanta sync` | Reconcile to `vanta.toml` + `vanta.lock` (reproduce a project) |
| `vanta x <tool> [args]` | Run a tool ephemerally, without adding it |
| `vanta exec -- <cmd>` | Run a command with the project's tools on `PATH` |
| `vanta list` / `which` | Show active tools / a tool's resolved path |
| `vanta search` / `info` | Search the registry / show a tool's versions |
| `vanta outdated` | Show what could update |
| `vanta rollback` / `generations` | Revert to / list prior generations |
| `vanta gc` | Garbage-collect unreferenced store entries |
| `vanta bundle` / `restore` | Create / import an offline, air-gapped bundle |
| `vanta init` / `migrate` | Create a `vanta.toml` (incl. from asdf/nvm/pyenv/…) |
| `vanta doctor` | Diagnose installation, PATH, and store health |
| `vanta activate <shell>` | Print the shell hook for `eval` |

The full reference is [04. CLI & Command Design](docs/04-cli.md).

## How it works

Vanta is a short-lived CLI (no daemon). A `vanta.toml` declares tools; the resolver
turns each request into an exact, verified artifact for every target platform and
records it in `vanta.lock`. The install engine fetches (resumable, mirror-aware),
verifies (checksum + signature, fail-closed), extracts, and atomically publishes
each artifact into the content-addressed store. Every change produces a new
generation, making rollback a pointer swap. A shell hook and a shim dispatcher
expose the right versions on `PATH` per directory. See
[02. Architecture](docs/02-architecture.md).

## Workspace

Vanta is a Cargo workspace of focused crates (full catalog in
[03. Repository](docs/03-repository.md)):

| Crate | Responsibility |
| --- | --- |
| `vanta` / `vanta-shim` | The `vanta`/`vt` binary and the per-tool shim dispatcher |
| `vanta-core` | Shared vocabulary, traits, and the `VTA-*` error taxonomy |
| `vanta-config` / `vanta-lock` | `vanta.toml` / `vanta.lock` models and diagnostics |
| `vanta-resolve` / `vanta-registry` / `vanta-provider` | Resolution, the registry index, and providers (incl. the WASM sandbox) |
| `vanta-store` / `vanta-state` / `vanta-net` | Content-addressed store, redb state, and downloads |
| `vanta-install` / `vanta-env` | Install engine and environment composition/activation |
| `vanta-security` | Checksums, Ed25519/minisign signatures, and policy |
| `vanta-cli` / `vanta-diag` / `vanta-migrate` | Commands, diagnostics, and importers |

### Crate versions

| Crate | Version | Downloads | Docs |
| --- | --- | --- | --- |
| [`vanta`](https://crates.io/crates/vanta) | ![v](https://img.shields.io/crates/v/vanta.svg) | ![d](https://img.shields.io/crates/d/vanta.svg) | [![docs](https://img.shields.io/docsrs/vanta)](https://docs.rs/vanta) |
| [`vanta-cli`](https://crates.io/crates/vanta-cli) | ![v](https://img.shields.io/crates/v/vanta-cli.svg) | ![d](https://img.shields.io/crates/d/vanta-cli.svg) | [![docs](https://img.shields.io/docsrs/vanta-cli)](https://docs.rs/vanta-cli) |
| [`vanta-config`](https://crates.io/crates/vanta-config) | ![v](https://img.shields.io/crates/v/vanta-config.svg) | ![d](https://img.shields.io/crates/d/vanta-config.svg) | [![docs](https://img.shields.io/docsrs/vanta-config)](https://docs.rs/vanta-config) |
| [`vanta-core`](https://crates.io/crates/vanta-core) | ![v](https://img.shields.io/crates/v/vanta-core.svg) | ![d](https://img.shields.io/crates/d/vanta-core.svg) | [![docs](https://img.shields.io/docsrs/vanta-core)](https://docs.rs/vanta-core) |
| [`vanta-diag`](https://crates.io/crates/vanta-diag) | ![v](https://img.shields.io/crates/v/vanta-diag.svg) | ![d](https://img.shields.io/crates/d/vanta-diag.svg) | [![docs](https://img.shields.io/docsrs/vanta-diag)](https://docs.rs/vanta-diag) |
| [`vanta-env`](https://crates.io/crates/vanta-env) | ![v](https://img.shields.io/crates/v/vanta-env.svg) | ![d](https://img.shields.io/crates/d/vanta-env.svg) | [![docs](https://img.shields.io/docsrs/vanta-env)](https://docs.rs/vanta-env) |
| [`vanta-install`](https://crates.io/crates/vanta-install) | ![v](https://img.shields.io/crates/v/vanta-install.svg) | ![d](https://img.shields.io/crates/d/vanta-install.svg) | [![docs](https://img.shields.io/docsrs/vanta-install)](https://docs.rs/vanta-install) |
| [`vanta-lock`](https://crates.io/crates/vanta-lock) | ![v](https://img.shields.io/crates/v/vanta-lock.svg) | ![d](https://img.shields.io/crates/d/vanta-lock.svg) | [![docs](https://img.shields.io/docsrs/vanta-lock)](https://docs.rs/vanta-lock) |
| [`vanta-migrate`](https://crates.io/crates/vanta-migrate) | ![v](https://img.shields.io/crates/v/vanta-migrate.svg) | ![d](https://img.shields.io/crates/d/vanta-migrate.svg) | [![docs](https://img.shields.io/docsrs/vanta-migrate)](https://docs.rs/vanta-migrate) |
| [`vanta-net`](https://crates.io/crates/vanta-net) | ![v](https://img.shields.io/crates/v/vanta-net.svg) | ![d](https://img.shields.io/crates/d/vanta-net.svg) | [![docs](https://img.shields.io/docsrs/vanta-net)](https://docs.rs/vanta-net) |
| [`vanta-platform`](https://crates.io/crates/vanta-platform) | ![v](https://img.shields.io/crates/v/vanta-platform.svg) | ![d](https://img.shields.io/crates/d/vanta-platform.svg) | [![docs](https://img.shields.io/docsrs/vanta-platform)](https://docs.rs/vanta-platform) |
| [`vanta-provider`](https://crates.io/crates/vanta-provider) | ![v](https://img.shields.io/crates/v/vanta-provider.svg) | ![d](https://img.shields.io/crates/d/vanta-provider.svg) | [![docs](https://img.shields.io/docsrs/vanta-provider)](https://docs.rs/vanta-provider) |
| [`vanta-registry`](https://crates.io/crates/vanta-registry) | ![v](https://img.shields.io/crates/v/vanta-registry.svg) | ![d](https://img.shields.io/crates/d/vanta-registry.svg) | [![docs](https://img.shields.io/docsrs/vanta-registry)](https://docs.rs/vanta-registry) |
| [`vanta-resolve`](https://crates.io/crates/vanta-resolve) | ![v](https://img.shields.io/crates/v/vanta-resolve.svg) | ![d](https://img.shields.io/crates/d/vanta-resolve.svg) | [![docs](https://img.shields.io/docsrs/vanta-resolve)](https://docs.rs/vanta-resolve) |
| [`vanta-sdk`](https://crates.io/crates/vanta-sdk) | ![v](https://img.shields.io/crates/v/vanta-sdk.svg) | ![d](https://img.shields.io/crates/d/vanta-sdk.svg) | [![docs](https://img.shields.io/docsrs/vanta-sdk)](https://docs.rs/vanta-sdk) |
| [`vanta-security`](https://crates.io/crates/vanta-security) | ![v](https://img.shields.io/crates/v/vanta-security.svg) | ![d](https://img.shields.io/crates/d/vanta-security.svg) | [![docs](https://img.shields.io/docsrs/vanta-security)](https://docs.rs/vanta-security) |
| [`vanta-shim`](https://crates.io/crates/vanta-shim) | ![v](https://img.shields.io/crates/v/vanta-shim.svg) | ![d](https://img.shields.io/crates/d/vanta-shim.svg) | [![docs](https://img.shields.io/docsrs/vanta-shim)](https://docs.rs/vanta-shim) |
| [`vanta-state`](https://crates.io/crates/vanta-state) | ![v](https://img.shields.io/crates/v/vanta-state.svg) | ![d](https://img.shields.io/crates/d/vanta-state.svg) | [![docs](https://img.shields.io/docsrs/vanta-state)](https://docs.rs/vanta-state) |
| [`vanta-store`](https://crates.io/crates/vanta-store) | ![v](https://img.shields.io/crates/v/vanta-store.svg) | ![d](https://img.shields.io/crates/d/vanta-store.svg) | [![docs](https://img.shields.io/docsrs/vanta-store)](https://docs.rs/vanta-store) |
| [`vanta-ui`](https://crates.io/crates/vanta-ui) | ![v](https://img.shields.io/crates/v/vanta-ui.svg) | ![d](https://img.shields.io/crates/d/vanta-ui.svg) | [![docs](https://img.shields.io/docsrs/vanta-ui)](https://docs.rs/vanta-ui) |

## Building and testing

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Releases — cross-platform binaries, `.deb`/`.rpm` packages, the container image,
and crates.io publishing — are produced for every tagged release; see
[`RELEASING.md`](RELEASING.md) and [32. Release Engineering](docs/32-release-engineering.md).

## Documentation

The complete design and reference lives in [`docs/`](docs/README.md):

- [01. Vision](docs/01-vision.md) — what Vanta is and why.
- [02. Architecture](docs/02-architecture.md) — the system from the inside out.
- [11. Reproducibility](docs/11-reproducibility.md) — the cross-platform lockfile.
- [15. Security](docs/15-security.md) — verification, signatures, and the sandbox.
- [33. Prior Art](docs/33-prior-art.md) — how Vanta compares to seventeen other tools.

## License

Apache-2.0. A separately-licensed enterprise edition adds fleet-scale operational
features without crippling the open-source core — see
[14. Enterprise](docs/14-enterprise.md) and [20. Future](docs/20-future.md).
