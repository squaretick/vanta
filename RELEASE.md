# Releasing Vanta

How a version of Vanta gets cut, built, and published — and how to recover when
a release ends up with no downloadable assets.

Shipped binaries: **`vanta`**, **`vt`** (short alias), and **`vanta-shim`**
(per-tool shim helper), bundled into one archive per target.

## The pipeline

Four workflows in `.github/workflows/` cooperate:

| Workflow | Trigger | Job |
|---|---|---|
| `release-plz.yml` (`release-pr`) | push to `main` | Opens/updates a "release" PR that bumps versions + changelog. |
| `release-plz.yml` (`release`) | push to `main` (after release PR merges) | Publishes crates to crates.io, **pushes the `vX.Y.Z` tag**, creates the GitHub Release. |
| `release-binaries.yml` | `push` of a `v*` tag | Cross-compiles `vanta`+`vt`+`vanta-shim` for every target, uploads `.tar.gz`/`.zip` + `.sha256` to the Release. |
| `release.yml` | `push` of a `v*.*.*` tag | Builds `.deb`/`.rpm` and the multi-arch GHCR container image. |

Normal flow: merge the release PR → `release` job tags → the tag push fires
`release-binaries.yml` and `release.yml` → assets attach to the Release →
`scripts/install.sh` can download them.

### Targets built

Keep this in sync with the target map in `scripts/install.sh`.

| Target | Runner | Archive |
|---|---|---|
| `x86_64-unknown-linux-gnu` | ubuntu-latest | tar.gz |
| `aarch64-unknown-linux-gnu` | ubuntu-24.04-arm | tar.gz |
| `x86_64-unknown-linux-musl` | ubuntu-latest | tar.gz |
| `x86_64-apple-darwin` | macos-13 | tar.gz |
| `aarch64-apple-darwin` | macos-14 | tar.gz |
| `x86_64-pc-windows-msvc` | windows-latest | zip |

Each archive ships with a `<archive>.sha256`. `install.sh` fails closed if the
checksum file is missing or mismatched.

## The one required secret: `RELEASE_PLZ_TOKEN`

**Why it exists:** a tag or release created with the default `GITHUB_TOKEN` does
**not** trigger other workflows (GitHub's guard against recursive Actions runs).
If the `release` job tags with `GITHUB_TOKEN`, the tag push never fires
`release-binaries.yml` / `release.yml`, and the Release is published with **no
binaries** — `install.sh` then 404s on the missing archive. This is exactly what
left `v0.0.2` and `v0.0.3` empty.

So the `release` job uses a **fine-grained Personal Access Token** stored as the
repo secret `RELEASE_PLZ_TOKEN`. There is **no `GITHUB_TOKEN` fallback** on that
job on purpose: a missing PAT fails the job loudly instead of silently shipping
an empty release.

### Creating the token

1. GitHub → Settings → Developer settings → **Fine-grained tokens** → Generate.
2. **Resource owner:** `squaretick` (the org, not your personal account).
3. **Repository access:** `vanta` (and `pulsate` if reusing one token).
4. **Repository permissions:** Contents = Read and write, Pull requests = Read
   and write. (Metadata auto-selected.)
5. Generate, copy the `github_pat_...` value once.
6. Store it:
   ```
   gh secret set RELEASE_PLZ_TOKEN -R squaretick/vanta --body "github_pat_xxx"
   ```
   Verify: `gh secret list -R squaretick/vanta` shows `RELEASE_PLZ_TOKEN`.

Org-owned resources may require an org owner to approve fine-grained token
access (org Settings → Third-party Access).

## Cutting a normal release

1. Land your changes on `main`. `release-plz` opens a release PR.
2. Review + merge the release PR.
3. The `release` job publishes crates, pushes the tag, creates the Release.
4. The tag push fires `release-binaries.yml` + `release.yml`; assets attach.
5. Verify:
   ```
   gh release view vX.Y.Z -R squaretick/vanta --json assets -q '.assets[].name'
   ```
   Expect, per target, `vanta-vX.Y.Z-<target>.<ext>` + its `.sha256`, plus
   `.deb`/`.rpm`. The GHCR image lands at `ghcr.io/squaretick/vanta:X.Y.Z`.

## Backfilling a release that has no assets

Happens when a tag was pushed by `GITHUB_TOKEN` (PAT missing/expired) so the
tag-driven workflows never ran — the cause of empty `v0.0.2`/`v0.0.3`. Both
`release-binaries.yml` and `release.yml` support **manual dispatch** with a
`tag` input: run them from `main` and pass the already-published tag. They check
out that tag, build, and upload to its Release.

```
# Ensure RELEASE_PLZ_TOKEN is set first (see above), then:
gh workflow run release-binaries.yml -R squaretick/vanta -f tag=v0.0.3
gh workflow run release.yml          -R squaretick/vanta -f tag=v0.0.3
```

Watch them:
```
gh run list -R squaretick/vanta --workflow release-binaries.yml
```

Then re-verify assets with the `gh release view` command above.

Alternative backfill: delete and re-push the tag (with the PAT now set) to
re-fire everything from scratch.

## Verifying the installer end to end

```
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/squaretick/vanta/main/scripts/install.sh | sh
```

Useful `install.sh` overrides:

- `VANTA_VERSION` — pin a tag (e.g. `v0.0.3`); default = latest release.
- `VANTA_TARGET` — force a target triple (e.g. `x86_64-unknown-linux-musl`).
- `INSTALL_DIR` — install location; default `$XDG_BIN_HOME` or `~/.local/bin`.

Fallbacks when a prebuilt binary isn't available: `cargo install vanta`, or the
container image `ghcr.io/squaretick/vanta`.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `install error: no release asset for <target>` (404) | Release has no binaries. | Backfill (above). Confirm `RELEASE_PLZ_TOKEN` is set so future tags self-build. |
| `checksum file missing` | Archive uploaded, `.sha256` didn't. | Re-run `release-binaries.yml` for the tag. |
| `release` job fails immediately | `RELEASE_PLZ_TOKEN` unset/expired (no fallback, by design). | Set/rotate the PAT, re-run. |
| New tag published but no binaries | PAT expired at tag time. | Rotate PAT, then backfill the tag. |
| Missing target (e.g. `aarch64-pc-windows`) | Not in the build matrix. | Add to `release-binaries.yml` matrix **and** `install.sh` target map together. |
