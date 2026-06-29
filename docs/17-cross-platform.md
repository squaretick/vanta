# 17. Cross-platform

> One identical model on Linux, macOS, and Windows is a pillar (canon §2), and native Windows support is a core differentiator versus the Unix-only incumbents. This document specifies the platform-abstraction layer, the canonical platform identifiers, the per-OS link and layout strategy, and the Windows, macOS, and Linux specifics that the rest of the design depends on. Owned by `vanta-platform`.

**Contents**

- [The abstraction layer](#the-abstraction-layer)
- [Platform identifiers](#platform-identifiers)
- [Links and layout per OS](#links-and-layout-per-os)
- [Windows specifics](#windows-specifics)
- [macOS specifics](#macos-specifics)
- [Linux specifics](#linux-specifics)
- [Reproducibility across platforms](#reproducibility-across-platforms)
- [Testing](#testing)
- [Cross-references](#cross-references)

---

## The abstraction layer

`vanta-platform` is a leaf crate that all higher layers use for everything OS-specific, so the rest of the codebase is platform-agnostic. It abstracts:

| Concern | What it provides |
| --- | --- |
| Platform detection | the current os/arch/libc triple (incl. glibc-vs-musl and Rosetta detection) |
| Paths | `$VANTA_HOME` resolution, path normalization, long-path handling, case-fold rules |
| Links | `LinkStrategy` implementations (reflink/hardlink/symlink/copy) with a fast `probe` |
| Executables | exe extension/bit handling, launcher-shim creation, PATH mutation |
| Shells | per-shell hook generation and detection (bash/zsh/fish/pwsh/nu/elvish/cmd) |
| Secrets | OS keychain bindings ([23. Data & State Model](23-data-and-state-model.md)) |
| Process | `exec`/spawn, signal handling, quarantine-attribute handling |

Higher crates depend only on these traits, so adding or fixing a platform is localized to `vanta-platform` and its tests.

## Platform identifiers

Vanta uses a canonical `os/arch[/libc]` token for every platform — in the lock's `targets`, in provider artifact maps, and in store/cache keys ([26. Registry Reference](26-registry-and-metadata-reference.md)):

| Token | OS | Arch | libc / note |
| --- | --- | --- | --- |
| `linux/x86_64/gnu` | Linux | x86-64 | glibc |
| `linux/x86_64/musl` | Linux | x86-64 | musl (static; Alpine, distro-agnostic) |
| `linux/aarch64/gnu` | Linux | ARM64 | glibc |
| `linux/aarch64/musl` | Linux | ARM64 | musl |
| `macos/aarch64` | macOS | Apple Silicon | — |
| `macos/x86_64` | macOS | Intel | — |
| `windows/x86_64` | Windows | x86-64 | MSVC ABI |
| `windows/aarch64` | Windows | ARM64 | MSVC ABI |

- **Detection.** Vanta detects the running platform precisely, including **glibc vs musl** (probing the loader / `ldd`-equivalent, not just "Linux") so it selects a compatible artifact, and **Rosetta** on macOS (an x86_64 process on Apple Silicon) so it can prefer a native arm64 artifact.
- **Selection.** A provider declares artifacts per token; resolution picks the best match for the current/target token, with documented fallbacks (e.g. a `macos/x86_64` artifact run under Rosetta if no `macos/aarch64` exists; a glibc artifact only when glibc is present).
- **Cross-platform locking** records an entry per token in `targets`, which is what makes one lock serve a mixed-OS team ([11. Reproducibility](11-reproducibility.md)).

## Links and layout per OS

The store and environment views use the cheapest link mechanism the filesystem/OS supports, probed at runtime ([09. Store](09-store.md#link-strategies)):

| OS | Preferred order | Notes |
| --- | --- | --- |
| Linux | reflink (Btrfs/XFS) → hardlink → symlink → copy | symlinks always available |
| macOS | reflink (APFS, always CoW) → hardlink → symlink → copy | APFS clonefile is near-free |
| Windows | reflink (ReFS, if present) → hardlink → copy; **launcher `.exe` instead of symlink** | unprivileged symlinks not guaranteed |

The logical layout (`store/`, `envs/`, `cache/`, …) is identical everywhere; only the link primitive and the executable representation differ.

## Windows specifics

Windows is a first-class target, not a port. The design avoids every assumption that breaks there:

- **No reliance on symlink privilege.** Creating symlinks on Windows historically required admin or Developer Mode. Vanta therefore uses **hardlinks or copies** for store/env composition and exposes the shim dispatcher as a real **launcher `.exe`** per tool name (not a symlink). Developer Mode is used opportunistically (for reflink/symlink) but never required.
- **Shims, not a hook, for `cmd.exe`.** `cmd.exe` has no prompt-hook mechanism, so it relies on the shim dispatcher; **PowerShell** gets the full activation hook ([10. Environments](10-environments.md#shell-integration)).
- **PATH management.** Vanta adds `~/.vanta/bin` (i.e. `%LOCALAPPDATA%\Vanta\bin`) to the **user** PATH via the registry (no admin), and the PowerShell hook adjusts the session PATH for per-directory switching.
- **Long paths.** The content-addressed store can nest; Vanta uses the `\\?\` extended-length path prefix and enables long-path support so deep store trees never hit `MAX_PATH`.
- **Case-insensitive filesystem.** Hashing canonicalizes case so a tree hashes identically on NTFS (case-insensitive) and ext4 (case-sensitive) ([09. Store](09-store.md#hashing-and-canonicalization)).
- **Executable handling.** `.exe`/`.cmd`/`.bat`/`.ps1` are recognized; provider `bin` entries name the Windows form (`node.exe`, `npx.cmd`).
- **Code-signing & SmartScreen.** The Vanta binary is Authenticode-signed so Windows Defender SmartScreen does not flag it ([32. Release Engineering](32-release-engineering.md)); installed tool artifacts are still verified by Vanta's own signature/checksum checks.

The result: scoop-style user-space, no-admin, multi-version installation — but cross-platform, verified, and reproducible, which scoop is not ([33. Prior Art](33-prior-art.md)).

## macOS specifics

- **Gatekeeper / quarantine.** Files downloaded from the network may carry the `com.apple.quarantine` extended attribute, which makes macOS block execution. Vanta removes the quarantine attribute on tool binaries **only after** its own signature/checksum verification passes — so the security check is Vanta's verification, not Gatekeeper's, and a tool never runs unverified.
- **Codesigning & notarization.** The Vanta binary itself is codesigned and notarized so it runs cleanly on a fresh Mac ([32. Release Engineering](32-release-engineering.md)).
- **Architecture.** Both `macos/aarch64` and `macos/x86_64` are supported; Vanta detects Apple Silicon vs Intel and Rosetta, preferring native arm64 artifacts and falling back to x86_64-under-Rosetta only when necessary. Vanta may ship as a universal binary.
- **APFS reflink.** APFS supports `clonefile`, so environment composition is near-free via reflink.
- **Keychain.** Credentials use the macOS Keychain ([23. Data & State Model](23-data-and-state-model.md)).

## Linux specifics

- **glibc vs musl.** The biggest portability hazard on Linux is the C library. Vanta detects which is present and selects a `…/gnu` or `…/musl` artifact accordingly; musl/static artifacts make Vanta itself and many tools distro-agnostic (Alpine, minimal containers).
- **No root, distro-agnostic.** Everything installs under `$VANTA_HOME` in user space; Vanta does not touch system package managers or require root, so it works identically on Debian, Fedora, Arch, Alpine, and inside containers.
- **XDG.** When XDG variables are set, Vanta honors them for cache/state placement, but defaults to `~/.vanta` for one predictable cross-OS location.
- **Older-glibc fallback.** Where a tool's prebuilt requires a newer glibc than present, resolution prefers a musl/static artifact or reports a clear `VTA-SYS-*` rather than installing something that won't run.
- **Containers/CI.** A static Vanta binary plus a warm/cached `~/.vanta/store` makes container and CI usage fast and reproducible ([18. Developer Experience](18-developer-experience.md)).

## Reproducibility across platforms

The cross-platform lock ([11. Reproducibility](11-reproducibility.md)) records an artifact per platform token, so a single committed `vanta.lock` reproduces the right, verified environment on every OS a team uses. Canonical, case- and mode-normalized hashing ([09. Store](09-store.md#hashing-and-canonicalization)) ensures the store key for a given artifact is the same identity regardless of the filesystem it lands on, which is what makes "one lock, three operating systems" a real guarantee rather than a hope.

## Testing

- The CI matrix runs unit/integration/e2e on Linux (x86_64, aarch64; gnu + musl), macOS (aarch64, x86_64), and Windows (x86_64), plus a Windows-ARM and an Alpine/musl job ([03. Repository](03-repository.md), [28. Testing](28-testing.md)).
- Platform-specific behavior (link strategy probing, quarantine removal, long-path handling, PATH mutation, per-shell hooks) has targeted tests on the relevant OS.
- Reproducibility tests assert byte-identical store keys for the same artifact across runners/OSes.

## Cross-references

- [09. Store](09-store.md) — link strategies, canonical hashing, and the single-filesystem requirement.
- [10. Environments](10-environments.md) — per-shell hooks and the Windows launcher-shim model.
- [11. Reproducibility](11-reproducibility.md) — per-platform lock entries and cross-OS reproduction.
- [26. Registry & Metadata Reference](26-registry-and-metadata-reference.md) — platform tokens in artifact descriptors.
- [32. Release Engineering](32-release-engineering.md) — codesigning/notarization/Authenticode for the Vanta binary.
- [23. Data & State Model](23-data-and-state-model.md) — per-OS keychain bindings.
