# 22. Provider SDK & ABI

> The reference for provider authors: the declarative provider manifest format, a complete worked example, the WebAssembly hook interface (the WIT world Vanta exposes), the capability sandbox, the guest SDK (`vanta-sdk`), signing/publishing, ABI versioning, and provider testing. A provider teaches Vanta how to discover, fetch, verify, lay out, and expose one tool — safely, declaratively, and cross-platform. The exhaustive field tables are mirrored in [26. Registry & Metadata Reference](26-registry-and-metadata-reference.md).

**Contents**

- [Provider model](#provider-model)
- [The declarative manifest](#the-declarative-manifest)
- [A complete example](#a-complete-example)
- [When TOML is not enough: WASM hooks](#when-toml-is-not-enough-wasm-hooks)
- [The WIT world](#the-wit-world)
- [The capability sandbox](#the-capability-sandbox)
- [The guest SDK](#the-guest-sdk)
- [Signing, publishing, and ABI versioning](#signing-publishing-and-abi-versioning)
- [Testing providers](#testing-providers)
- [Trade-offs](#trade-offs)
- [Cross-references](#cross-references)

---

## Provider model

A **provider** defines, for one tool or a family of tools, the answers to four questions ([07. Providers](07-providers.md)):

1. **What versions exist?** (list-versions)
2. **For a given version and platform, what is the artifact?** (resolve → URL(s), checksum, signature, layout)
3. **How is it verified?** (which checksum/signature scheme)
4. **How is it laid out and exposed?** (strip components, subdir, bin entries, env, declared deps)

The design principle ([ADR-0006](24-architecture-decision-records.md)): **declarative-first.** The overwhelming majority of tools (GitHub-release binaries, versioned URL patterns, language-registry CLIs) are fully describable in TOML with **no code**. Only the rare tool whose version discovery or artifact mapping needs real logic ships a **sandboxed WASM hook**. This is the deliberate opposite of asdf's "every plugin is arbitrary bash" ([33. Prior Art](33-prior-art.md)).

## The declarative manifest

A provider manifest is TOML. Fields (exhaustive table in [26. Registry Reference](26-registry-and-metadata-reference.md)):

| Field | Purpose |
| --- | --- |
| `id` | provider identifier, e.g. `official/node` |
| `tool` / `tools` | the tool name(s) this provider serves |
| `provider_version` | the provider's own version (independent of the tool) |
| `version_source` | where versions come from: `github-releases`, `url-list`, `language-registry`, or `wasm` |
| `version_comparator` | `semver` (default), `calver`, or `wasm` (custom ordering) |
| `channels` | named channels (`lts`, `stable`, `nightly`) and how they map to versions |
| `[artifacts.<token>]` | per-platform artifact descriptor (one per platform token from [17. Cross-platform](17-cross-platform.md)) |
| `url` | URL template with `{version}`, `{os}`, `{arch}`, `{ext}` placeholders |
| `archive` | `tar.gz` / `tar.xz` / `zip` / `tar.zst` / `raw` |
| `checksum` | how to obtain the checksum: an inline value, a `sha256-url` template, or `wasm` |
| `signature` | signature scheme + key reference (`minisign`, `cosign`, or `none`) |
| `layout` | `strip` components, `subdir`, file-mode fixups |
| `bin` | the executables to expose (relative paths; Windows forms allowed) |
| `env` | env vars the tool needs when active |
| `deps` | other tools this one requires (resolved as a DAG) |

`version_source` and `version_comparator` and `checksum` each accept `wasm` to delegate just that step to a hook, so a provider can be *mostly* declarative with a single small hook for the one hard part.

## A complete example

A provider for a typical GitHub-release CLI (`ripgrep`) — entirely declarative, ~25 lines, covering all platforms:

```toml
id = "official/ripgrep"
tool = "ripgrep"
provider_version = "1"
version_source = "github-releases"
repo = "BurntSushi/ripgrep"
version_comparator = "semver"

[artifacts."linux/x86_64/gnu"]
url = "https://github.com/BurntSushi/ripgrep/releases/download/{version}/ripgrep-{version}-x86_64-unknown-linux-musl.tar.gz"
archive = "tar.gz"
checksum = { sha256-url = "{url}.sha256" }
layout = { strip = 1 }
bin = ["rg"]

[artifacts."macos/aarch64"]
url = "https://github.com/BurntSushi/ripgrep/releases/download/{version}/ripgrep-{version}-aarch64-apple-darwin.tar.gz"
archive = "tar.gz"
checksum = { sha256-url = "{url}.sha256" }
layout = { strip = 1 }
bin = ["rg"]

[artifacts."windows/x86_64"]
url = "https://github.com/BurntSushi/ripgrep/releases/download/{version}/ripgrep-{version}-x86_64-pc-windows-msvc.zip"
archive = "zip"
checksum = { sha256-url = "{url}.sha256" }
layout = { strip = 1 }
bin = ["rg.exe"]
```

Versions come from the GitHub releases API (handled by the `github-releases` source, with caching and auth via `vanta-net`/`vanta-registry`); the checksum is fetched from the published `.sha256` and verified before materialization. A user simply runs `vanta add ripgrep`; the provider is invisible.

## When TOML is not enough: WASM hooks

Some tools need logic the manifest cannot express — e.g. version lists behind a non-standard API, artifact URLs derived by a rule, or a checksum embedded in an HTML page. For these, a provider sets the relevant field to `wasm` and ships a **WebAssembly component** implementing the corresponding hook. The hook runs under Wasmtime with the capability sandbox below.

Hooks a provider may implement (any subset; the rest stay declarative):

| Hook | Signature (conceptual) | Purpose |
| --- | --- | --- |
| `list-versions` | `() -> list<string>` | return the available version strings |
| `compare-versions` | `(a, b) -> ordering` | custom ordering when not SemVer/CalVer |
| `resolve` | `(version, platform) -> artifact` | produce the artifact descriptor (url/checksum/layout/bin) |
| `checksum` | `(version, platform) -> digest` | obtain a checksum when not a simple URL |

A hook returns **data** (versions, an artifact descriptor); it never fetches the artifact, materializes files, or runs the tool. The host performs the actual download/verify/materialize, so the hook's influence is limited to *describing* what to fetch — and that description is then independently checksum/signature-verified ([15. Security](15-security.md)).

## The WIT world

The host↔guest contract is defined in WIT (the WebAssembly Component Model interface language), versioned independently as the **provider ABI** ([03. Repository](03-repository.md#versioning-policy)):

```wit
package vanta:provider@1.0.0;

world provider {
  // ── host capabilities granted to the guest (the ONLY authority it has) ──
  import http-get: func(req: http-request) -> result<bytes, http-error>;
  import hash: func(data: bytes, algo: hash-algo) -> string;
  import log: func(level: log-level, msg: string);

  // ── hooks the guest may export ──
  export list-versions: func() -> result<list<string>, provider-error>;
  export compare-versions: func(a: string, b: string) -> ordering;
  export resolve: func(version: string, platform: string) -> result<artifact, provider-error>;
  export checksum: func(version: string, platform: string) -> result<digest, provider-error>;
}

record http-request { url: string, headers: list<tuple<string, string>> }
record artifact {
  url: string, mirrors: list<string>, archive: archive-kind,
  checksum: digest, signature: option<signature>,
  layout: layout, bin: list<string>, env: list<tuple<string,string>>, deps: list<dep>,
}
record digest { algo: hash-algo, value: string }
enum  hash-algo { sha256, blake3 }
enum  ordering  { less, equal, greater }
// ... layout, signature, dep, errors elided for brevity
```

The host imports are the entire attack surface the guest sees: a **scoped** `http-get`, `hash`, and `log`. There is no filesystem, no arbitrary sockets, no environment, no clock, no RNG, no process spawn.

## The capability sandbox

Wasmtime enforces isolation; the host policy enforces capability scope ([15. Security](15-security.md), [21. Threat Model](21-threat-model.md)):

- **`http-get` is scoped.** The provider manifest declares the host patterns it may reach (e.g. `api.github.com`, `github.com`); a request outside the allow-list is denied. Responses are size-capped and time-bounded.
- **No ambient authority.** No fs/env/sockets/spawn. The guest cannot read the user's files, exfiltrate secrets, or persist anything.
- **Bounded execution.** Wasmtime **fuel** caps instructions and **epoch interruption** caps wall time; linear memory is bounded. A runaway hook is terminated with `VTA-PROV-0001`.
- **Determinism.** Hooks must be deterministic (no clock/RNG); identical inputs yield identical outputs, which makes resolution cacheable and reproducible and removes environment-variance attacks.
- **Result still verified.** Whatever a hook returns, the artifact is checksum + signature + provenance verified against the lock/metadata before it can enter the store — a malicious hook can at worst cause a *verification failure*, never a silent bad install.

## The guest SDK

`vanta-sdk` is the guest-side crate that makes authoring a WASM provider straightforward: it provides the `wit-bindgen`-generated bindings, ergonomic wrappers (a typed `http_get`, helpers for common version-list parsing), and a test harness. A minimal hook:

```rust
// a provider that lists versions from a custom JSON API
vanta_sdk::provider! {
    fn list_versions() -> Result<Vec<String>, ProviderError> {
        let body = vanta_sdk::http_get("https://api.example.com/tool/releases")?;
        let releases: Vec<Release> = serde_json::from_slice(&body)?;
        Ok(releases.into_iter().map(|r| r.version).collect())
    }
    // resolve()/checksum() left declarative in the manifest
}
```

Authors compile to a `wasm32-wasip2` component and reference it from the manifest. Providers can be written in any language with Component Model support; Rust + `vanta-sdk` is the first-class path.

## Signing, publishing, and ABI versioning

- **Signing.** Every provider (manifest + any WASM) is signed; the registry records the signer. Unsigned providers are refused by default ([15. Security](15-security.md)).
- **Publishing.** A community provider is submitted to the community registry, **reviewed and tested** (the harness below) in CI, then signed into the official index. Private providers are published to a private registry ([14. Enterprise](14-enterprise.md)).
- **ABI versioning.** The WIT world is SemVer-versioned (`vanta:provider@MAJOR.MINOR.PATCH`). The host supports a documented range of ABI majors; a provider declares the ABI it targets, and providers keep working across host upgrades within a major. A breaking ABI change is a new major with a migration window ([03. Repository](03-repository.md#versioning-policy)).

## Testing providers

`vanta-sdk` + `vanta-test` give authors a harness:

- **Golden resolutions.** Assert that `resolve(version, platform)` yields the expected artifact descriptor for a fixture set of versions/platforms.
- **Sandbox conformance.** The harness runs the hook under the real capability sandbox so an author catches a forbidden capability (e.g. an un-allow-listed host) before submission.
- **Determinism check.** Run a hook twice; outputs must match.
- **Live smoke (optional).** Against the real upstream in a gated job, verify list/resolve still work as upstreams evolve.
- CI on the community registry runs these on every submission, so a provider that resolves wrong or tries to escape its sandbox never ships ([20. Future](20-future.md)).

## Trade-offs

- **Declarative coverage vs. expressiveness.** Most tools fit the manifest; the rare ones use a hook. The cost is a richer manifest schema, justified by eliminating arbitrary code for the common case.
- **WASM overhead.** A hook adds Wasmtime startup + marshaling versus native code, but it runs only during resolution (cached), not on the hot path, and the safety is worth it.
- **vs. Nix derivations.** A Nix derivation is more powerful (it builds anything) but requires the Nix language and model; Vanta's provider is intentionally narrower (describe, don't build) so it stays simple, safe, and cross-platform ([33. Prior Art](33-prior-art.md)).

## Cross-references

- [07. Providers](07-providers.md) — the provider/registry/backend model this references.
- [26. Registry & Metadata Reference](26-registry-and-metadata-reference.md) — the exhaustive manifest/artifact field tables.
- [15. Security](15-security.md) — the sandbox and signature model.
- [21. Threat Model](21-threat-model.md) — provider-as-adversary analysis.
- [06. Resolution](06-resolution.md) — how `list-versions`/`resolve` feed `[2 Resolve]`.
- [03. Repository](03-repository.md) — `vanta-sdk`, `vanta-provider`, and ABI versioning.
