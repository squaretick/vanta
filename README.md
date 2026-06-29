# Vanta

[![crates.io](https://img.shields.io/crates/v/vanta.svg)](https://crates.io/crates/vanta)
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

```sh
# Shell installer (Linux / macOS) — downloads the prebuilt release binary
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/squaretick/vanta/main/scripts/install.sh | sh

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
echo 'eval "$(vanta activate zsh)"' >> ~/.zshrc    # or: bash, fish, pwsh
```

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

## Commands

| Command | Purpose |
| --- | --- |
| `vanta add <tool>[@ver] …` | Resolve and install tools into the current scope |
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
