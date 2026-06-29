# 01. Vision

> The product thesis behind Vanta: the mission, the fragmentation problem it removes, the ten design pillars held as constraints, who it is for, what it deliberately is not, an honest comparison with today's tools, and the five-year arc. Vanta is a brand-new, original tool — it learns from mise, asdf, Homebrew, Nix, uv, pnpm, winget, scoop, and the rest, and copies none of them.

**Contents**

- [Mission](#mission)
- [The problem: a fragmented toolbox](#the-problem-a-fragmented-toolbox)
- [Design philosophy](#design-philosophy)
- [Target users](#target-users)
- [Non-goals](#non-goals)
- [Comparison with existing tools](#comparison-with-existing-tools)
- [The synthesis Vanta is making](#the-synthesis-vanta-is-making)
- [Long-term roadmap (5+ years)](#long-term-roadmap-5-years)
- [Cross-references](#cross-references)

---

## Mission

**Vanta is the single command a developer uses to install, manage, update, and reproduce every developer tool — across Linux, macOS, and Windows — without ever caring where the tool comes from.**

The mental model Vanta sells is one sentence:

> *If I need any developer tool, I install it with Vanta.*

`node`, `python`, `rust`, `go`, `bun`, `deno`, `java`, `terraform`, `gh`, `docker`, `ripgrep` — runtimes, toolchains, and CLIs are all "tools," and all are installed the same way:

```sh
vanta add node@24
vanta add python@3.13
vanta add rust
vanta add terraform
vanta remove node
vanta update
vanta sync
vanta doctor
```

Success is measured concretely:

- A newcomer installs Vanta, runs `vanta add node@24`, and has the right `node` on `PATH` in seconds, having read nothing and chosen no "backend."
- A teammate clones a repository, runs `vanta sync`, and gets a **byte-for-byte identical** toolset — same versions, same artifacts, verified — on a different operating system.
- A developer never reaches for a second tool to install a runtime, a toolchain, or a CLI. There is no "use nvm for Node, pyenv for Python, brew for the CLI, and Nix when you need reproducibility."
- Every install is verified, atomic, and reversible: a bad update is one `vanta rollback` away, and nothing is ever left half-installed.

## The problem: a fragmented toolbox

Installing developer tools in 2026 means assembling a personal stack of single-purpose managers, each with its own command vocabulary, configuration file, security posture, update model, and reproducibility story:

| Category | Examples | What it solves | What it leaves broken |
| --- | --- | --- | --- |
| Per-language version managers | nvm, fnm, pyenv, rbenv, gvm, jenv | "I need Node 20 here, 24 there" | one per language; no cross-language story; no lockfile; shell-specific |
| Polyglot version managers | asdf, mise, pkgx | one tool for many runtimes | plugins are arbitrary shell (security, no Windows); weak/no reproducible lock |
| OS package managers | Homebrew, apt, dnf, pacman, winget, scoop, Chocolatey | install system + CLI software | OS-specific; mostly single-version; root or global prefix; not reproducible across machines/time |
| Language-native managers | npm, pnpm, pipx, cargo, uv | install that language's packages/CLIs | scoped to one ecosystem; can't manage the runtime *and* every other tool |
| Functional managers | Nix, Flox | true reproducibility, rollback | steep learning curve, a new language, large stores, no native Windows |

A typical developer therefore runs **three to six** of these simultaneously. The costs compound:

- **Cognitive load.** Every tool has different verbs (`brew install` vs `npm i -g` vs `asdf install` vs `nix profile install`), different config files (`.nvmrc`, `.tool-versions`, `Brewfile`, `flake.nix`), and different mental models.
- **No unified reproducibility.** `node_modules` is locked but your Node version is not; your Python is pinned by pyenv but the lock has no artifact hashes; Homebrew is rolling and unversioned. "Works on my machine" persists because no single layer pins *all* the tools, with hashes, across platforms.
- **Inconsistent security.** Some managers verify checksums; some run arbitrary install scripts as you; few verify signatures or provenance; plugin ecosystems frequently execute untrusted shell.
- **Platform fault lines.** The polyglot managers and the functional managers are effectively Unix-only; Windows users get a different toolchain entirely (winget/scoop/choco), so a cross-platform team cannot share an environment.

Vanta exists to collapse this matrix into one interface, one config format, one store, and one reproducibility contract — on every operating system — without giving up the safety and reproducibility that the heavyweight tools provide.

## Design philosophy

Ten pillars. Each is a **constraint the architecture is held to**, not a slogan. They are enumerated in the canon and realized across the rest of these documents.

1. **One command for everything.** `vanta add <tool>` installs a runtime, a toolchain, a CLI, or a service. The user learns one verb, not one tool per category. *Rationale:* the fragmentation above is the core pain; unification is the product.
2. **One consistent UX.** The same verbs, flags, output, and exit codes regardless of where a tool actually comes from. *Rationale:* "you should never care which package manager originally distributed it" is only true if the interface never leaks the source.
3. **Deterministic and reproducible.** A committed `vanta.lock` pins exact versions *and artifact hashes* for *all* target platforms; the same lock yields the same environment anywhere. *Rationale:* reproducibility is the property every incumbent half-implements; Vanta makes it the default. (See [11. Reproducibility](11-reproducibility.md).)
4. **Extremely fast.** Cold start < 5 ms, warm activation < 1 ms, parallel everything. *Rationale:* a tool on the `cd` hot path and in every shim invocation cannot be slow; performance is a feature, measured not asserted. (See [16. Performance](16-performance.md).)
5. **Secure by default.** Every artifact is checksum- and signature-verified; providers run sandboxed; you opt *out* of safety, never *in*. *Rationale:* supply-chain compromise and "curl | sh" are the real-world risks; the default must be safe. (See [15. Security](15-security.md).)
6. **Offline-friendly.** A content-addressed cache, mirror support, and portable air-gapped bundles mean Vanta works without the network once things are fetched. *Rationale:* CI, planes, and air-gapped enterprises are first-class, not afterthoughts. (See [13. Offline](13-offline.md).)
7. **Atomic operations.** Every mutation produces a new immutable generation; rollback is a pointer swap; nothing is ever half-applied. *Rationale:* a package manager that can leave you in a broken state is a liability. (See [12. Updates & Rollback](12-updates.md).)
8. **Cross-platform.** One identical model on Linux, macOS, and Windows. *Rationale:* mixed teams must share one environment; Windows is not a second-class port. (See [17. Cross-platform](17-cross-platform.md).)
9. **Human-readable configuration.** TOML manifests — not a bespoke DSL, not YAML sprawl, not annotation soup. *Rationale:* configuration is a primary surface; it should be obvious to read and diff. (See [05. Configuration](05-configuration.md), [ADR-0005](24-architecture-decision-records.md).)
10. **Zero unnecessary complexity.** The simplest workflow is the default; the design **removes user decisions rather than adding configuration.** *Rationale:* every knob is a chance to be wrong; defaults should be correct so most users never configure anything.

A single discipline ties them together: **the simplest correct path is the default path.** Advanced capability (workspaces, private registries, policy, source builds) is present but quiet — revealed progressively as a project grows, never taxing the developer who just wants `node@24`.

## Target users

Vanta is designed for three concentric audiences, in priority order. The discipline is to **serve the team and the enterprise without taxing the solo developer.**

| Audience | Who they are | What they need from Vanta | Primary docs |
| --- | --- | --- | --- |
| **Solo developer / indie team** | Works across a few languages; wants the right tools on `PATH` per project; has no platform team | `vanta add`, automatic per-directory version switching, `vanta x` for one-offs, zero config beyond `vanta.toml` | [18. DX](18-developer-experience.md), [10. Environments](10-environments.md) |
| **Team / platform engineering** | Standardizes tooling across many repos and a mixed OS fleet; cares about reproducibility and onboarding | a committed cross-platform lock, `vanta sync --frozen` in CI, private registries, policy | [11. Reproducibility](11-reproducibility.md), [14. Enterprise](14-enterprise.md) |
| **Enterprise** | Compliance, air-gapped networks, supply-chain assurance, support | signed metadata + provenance, SBOM export, air-gapped bundles, audit, policy enforcement | [14. Enterprise](14-enterprise.md), [15. Security](15-security.md), [13. Offline](13-offline.md) |

Secondary audiences explicitly designed for: **provider authors** (who need a safe, declarative way to teach Vanta about a new tool — [22. Provider SDK](22-provider-sdk.md)) and **tool integrators / editors** (who consume the stable `--json` surface and `vanta which` — [29. Public APIs](29-public-apis.md)).

## Non-goals

Stating what Vanta is *not* is as important as what it is. These are deliberate and durable.

- **Not a replacement for a language's own dependency manager.** Vanta installs *tools* (`node`, `uv`, `cargo` themselves), not your application's library dependencies. Your project still uses npm/pnpm/uv/cargo for its package graph; Vanta makes sure the *right npm/uv/cargo* is on `PATH`. (Vanta can install and run them; it does not resolve your `package.json`.)
- **Not a system package manager.** Vanta does not manage the kernel, system shared libraries, drivers, or OS services, and does not require root. It installs developer tools into a user-owned store. apt/dnf/pacman/winget remain the right tools for the operating system itself.
- **Not a build system.** Vanta has an optional, deliberately minimal task runner for "the dev commands of this project," but it is not Make/Bazel/CMake and will not grow into one.
- **Not a container runtime or orchestrator.** `vanta add docker` installs the Docker CLI; Vanta is not Docker, not a VM manager, and not Kubernetes.
- **Not a secrets manager.** Vanta stores its *own* registry credentials in the OS keychain, but it is not a vault for your application secrets.
- **Not a CI system.** Vanta runs *inside* CI (`vanta sync --frozen`); it does not schedule or execute pipelines.
- **Not a research playground for novel reproducibility theory.** Vanta deliberately chooses a pragmatic content-addressed store over a full functional build language. It targets *practical* reproducibility for tools, not a proof system for arbitrary builds (that is Nix's domain; see [33. Prior Art](33-prior-art.md)).

## Comparison with existing tools

An honest positioning across the dimensions that matter, against representative incumbents. Deep per-tool teardowns (philosophy, architecture, strengths, weaknesses, UX, performance, package model, versioning, dependency resolution, layout, updates, plugins, security, reproducibility, enterprise) are in [33. Prior Art](33-prior-art.md).

| Dimension | **Vanta** | mise | asdf | Homebrew | Nix / Flox | uv | winget / scoop |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Scope | all dev tools | runtimes + env + tasks | runtimes | system + CLIs | everything (functional) | Python only | Windows software |
| Cross-platform incl. native Windows | ✅ | ⚠️ (weak on Windows) | ❌ | ⚠️ (mac/Linux) | ❌ (WSL only) | ✅ | ❌ (Windows-only) |
| Multiple versions side-by-side | ✅ | ✅ | ✅ | ⚠️ awkward | ✅ | ✅ (Python) | scoop ✅ / winget ❌ |
| Reproducible lock with artifact hashes | ✅ cross-platform | ❌ | ❌ | ❌ | ✅ | ✅ (Python) | ❌ |
| Content-addressed store + dedup | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ (cache) | ❌ |
| Atomic generations + instant rollback | ✅ | ❌ | ❌ | ❌ | ✅ | ⚠️ | ❌ |
| Extension model | declarative + **WASM-sandboxed** | bash/lua plugins + backends | **bash** plugins | Ruby formulae | Nix language | n/a | JSON/YAML manifests |
| Secure by default (checksum + signature) | ✅ | ⚠️ partial | ⚠️ minimal | ⚠️ checksums | ✅ | ✅ | ⚠️ checksums |
| Ephemeral run (npx/pipx for anything) | ✅ `vanta x` | ⚠️ | ❌ | ❌ | ✅ `nix run` | ✅ `uvx` | ❌ |
| Single static binary, no runtime deps | ✅ | ✅ | ❌ (shell) | ❌ (Ruby) | ❌ | ✅ | ❌ |
| Learning curve | low | low | low | low | **high** | low | low |

The reading: each incumbent is excellent at part of the problem and structurally incapable of the rest. mise has the breadth and speed but inherits asdf's unsafe bash plugins and lacks a hashed cross-platform lock. Nix has the reproducibility and rollback but demands a new language and abandons Windows. uv has the perfect model — Rust speed, a universal lock, a hardlinked content cache, runtime *and* package management — but only for Python. winget/scoop are Windows-native but are installer-runners, not reproducible environment managers.

## The synthesis Vanta is making

Vanta takes the best idea from each and rejects each one's anti-pattern, unified in a single memory-safe binary:

- From **uv**: a *universal, hashed lockfile* and Rust-class speed — generalized from Python to every tool.
- From **pnpm** and **Nix**: a *content-addressed store with hardlinked views* for dedup and integrity — generalized to all tools, without Nix's language.
- From **Nix/dnf**: *atomic generations and instant rollback* — in user space, with no functional DSL and no SAT solver users must understand.
- From **mise/asdf/pkgx**: *polyglot breadth and per-directory auto-switching* — but with **declarative, WASM-sandboxed providers** instead of arbitrary shell, so it is safe and works on Windows.
- From **scoop**: *user-space, no-admin, multi-version* installation — generalized cross-platform with verification and locking.
- From **winget**: *an official, signed, reviewed catalog* — but delivering isolated, reproducible installs rather than running vendor installers.
- From **pipx/pkgx**: *ephemeral, install-free execution* (`vanta x`) — backed by the same verified store.
- From **Cargo**: *resolver-and-lockfile discipline* and a healthy security ecosystem (audit/deny/provenance) — applied to tools, not just crates.

The net is a tool that is **broader than mise, as reproducible as Nix but far simpler, as fast as uv, as safe as the strictest of them, and the only one that is genuinely cross-platform including native Windows.** This is the claim every subsequent document must make true.

## Long-term roadmap (5+ years)

A directional arc; dated phases are in [19. Milestones](19-milestones.md).

**Year 1 — "it just works" on one machine.** The store, resolver, registry, providers, the install engine, the manifest/lock, per-directory activation, and the core CLI. Target: a developer's favorite single-machine tool manager. Public 0.x after the end-to-end `vanta add` path, then a stable manifest/lock format.

**Year 2 — reproducibility and trust at team scale.** Cross-platform locking, `vanta sync`, generations/rollback/GC hardened; the full security model (signed metadata with TUF-style roles, provenance, the WASM provider sandbox); SBOM export. Target: the team's standard, committed to every repo. Tag 1.0 with a frozen format and provider ABI.

**Year 3 — the enterprise and the ecosystem.** Private registries, authentication/SSO, policy enforcement, air-gapped bundles and mirrors; a community provider registry with review and signing; migration importers mature. Target: credible regulated and air-gapped deployments.

**Year 4 — fleet and cloud (additive, never required).** An optional cloud control plane for sharing locked environments across a team and managing fleet policy and private registries — nodes stay fully self-sufficient offline. Target: large mixed-OS organizations.

**Year 5+ — the intelligent environment.** AI-assisted diagnostics (`vanta doctor` that explains *why* a tool fails and proposes the fix), reproducible source builds for tools that lack good prebuilts, and deep editor integration. Vanta becomes not just an installer but an environment co-pilot.

Two invariants hold throughout: **the one-command/one-config experience never regresses**, and **the open-source core stays genuinely capable** — enterprise features are additive, never a crippling of the OSS edition ([20. Future](20-future.md)).

## Cross-references

- [02. Architecture](02-architecture.md) — how this philosophy becomes a system (the store-centric model and lifecycle).
- [33. Prior Art](33-prior-art.md) — the deep, honest teardown of all seventeen incumbents behind the comparison table.
- [05. Configuration](05-configuration.md) — the TOML manifest that realizes "human-readable configuration."
- [11. Reproducibility](11-reproducibility.md) — the cross-platform lock behind the reproducibility pillar.
- [15. Security](15-security.md) — "secure by default" in detail.
- [19. Milestones](19-milestones.md) — the dated plan behind this roadmap.
- [20. Future](20-future.md) — the additive enterprise and ecosystem arc.
