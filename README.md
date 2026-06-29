# Vanta

[![license](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](#license)
[![docs](https://img.shields.io/badge/docs-reference-brightgreen.svg)](docs/README.md)
[![status](https://img.shields.io/badge/status-0.x-blue.svg)](docs/19-milestones.md)

**Every developer tool, one command.** Vanta is a cross-platform package manager,
toolchain manager, runtime manager, and dev-environment manager in one static Rust
binary — the simplest, most reliable way to install, manage, update, and reproduce
developer tools across Linux, macOS, and Windows.

You should never have to care where a tool comes from or which package manager
distributes it. If you need a developer tool, you install it with Vanta:

```sh
vanta add node@24
vanta add python@3.13
vanta add rust
vanta add bun
vanta add go
vanta add java@21
vanta add terraform
vanta add gh
vanta remove node
vanta update
vanta sync          # reproduce exactly from vanta.lock (run this after `git clone`)
vanta doctor
vanta list
```

> **Status:** 0.x. The core is implemented as a Rust workspace under
> [`crates/`](crates) with the full design reference under [`docs/`](docs/README.md).
> The roadmap is in [19. Milestones](docs/19-milestones.md).

## Why Vanta

Installing dev tools today means stitching together a per-language version manager,
a polyglot manager, an OS package manager, a language-native manager, and a
reproducibility tool — each with its own commands, config, security posture, and
platform reach. Vanta collapses all of that into one interface, one TOML config, one
content-addressed store, and one cross-platform lockfile.

It takes the best idea from each incumbent and rejects each one's anti-pattern (full
analysis in [33. Prior Art](docs/33-prior-art.md)):

- **uv's** universal, hashed lockfile and Rust speed — generalized to *every* tool.
- **pnpm's / Nix's** content-addressed store with hardlinked views — for dedup and integrity.
- **Nix's** atomic generations and instant rollback — in user space, with **no Nix language**.
- **mise/asdf's** polyglot breadth and per-directory auto-switching — but with
  **declarative, WASM-sandboxed providers** instead of arbitrary shell, and native Windows.
- **scoop's** user-space, no-admin, multi-version model — cross-platform, verified, and locked.
- **pipx/pkgx's** ephemeral run (`vanta x`) — backed by the same verified store.

The result: broader than a version manager, as reproducible as Nix but far simpler,
as fast as uv, secure by default, and the only one that is genuinely cross-platform
including native Windows.

## Design pillars

One command for everything · one consistent UX · deterministic & reproducible ·
extremely fast · secure by default · offline-friendly · atomic operations ·
cross-platform · human-readable config · zero unnecessary complexity.

The simplest workflow is always the default; the design removes user decisions
rather than adding configuration.

## How it works (in one paragraph)

A project's `vanta.toml` declares the tools it needs. Vanta resolves each request to
an exact, verified artifact for every target platform and records it in `vanta.lock`.
Artifacts are materialized into an immutable, content-addressed store
(`~/.vanta/store/blake3-…`), deduplicated and integrity-checked. Each change produces
a new atomic *generation*, so rollback is instant. A shell hook (with a universal
shim fallback) puts the right versions on `PATH` automatically as you move between
directories — sub-millisecond when warm. Every artifact is checksum- and
signature-verified by default, and providers run sandboxed in WebAssembly. See
[02. Architecture](docs/02-architecture.md).

## Documentation

The full design — 34 documents, implementation-ready — lives in
[`docs/`](docs/README.md). Start with:

- [01. Vision](docs/01-vision.md) — what Vanta is and why.
- [02. Architecture](docs/02-architecture.md) — the system from the inside out.
- [33. Prior Art](docs/33-prior-art.md) — the deep comparison with 17 existing tools.
- [19. Milestones](docs/19-milestones.md) — the phased build plan.

## License

Apache-2.0 (open core). A future, separately-licensed enterprise edition adds
fleet-scale operational features without ever crippling the open-source core —
see [14. Enterprise](docs/14-enterprise.md) and [20. Future](docs/20-future.md).
