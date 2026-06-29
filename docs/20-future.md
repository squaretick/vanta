# 20. Future

> The additive arc beyond 1.0. Everything here builds on the open-source core without weakening it: the signature innovations maturing, a community provider ecosystem, team/cloud environment sharing, an optional fleet control plane, reproducible source builds, deep editor integration, and AI-assisted diagnostics. Each item is explicitly a *future* direction, not a launch promise, and is structured so commercial success requires the open core to thrive.

**Contents**

- [Guiding constraint](#guiding-constraint)
- [The innovations, matured](#the-innovations-matured)
- [A community provider ecosystem](#a-community-provider-ecosystem)
- [Team and cloud environment sharing](#team-and-cloud-environment-sharing)
- [An optional fleet control plane](#an-optional-fleet-control-plane)
- [Reproducible source builds](#reproducible-source-builds)
- [Deep editor integration](#deep-editor-integration)
- [AI-assisted diagnostics](#ai-assisted-diagnostics)
- [The open-core invariant](#the-open-core-invariant)
- [Cross-references](#cross-references)

---

## Guiding constraint

Two invariants hold across everything below ([01. Vision](01-vision.md), [14. Enterprise](14-enterprise.md)):

1. **The one-command/one-config experience never regresses.** No future feature is allowed to make the simple path harder.
2. **A single node is always complete.** Every cloud/fleet feature is *additive convenience*; losing the network or the service degrades convenience, never core function. Nothing moves into the install path.

## The innovations, matured

The signature innovations (canon §16) have post-1.0 depth to mine:

- **Sub-file deduplication** in the store (content-defined chunking) so patch releases of large tools share unchanged blocks, not just whole entries ([09. Store](09-store.md)).
- **Delta artifacts** — fetch only the diff between a held version and the target, cutting bandwidth for frequent updates.
- **Richer cross-platform targets** (BSDs, more libc/arch combinations) added to the lock as demand appears ([17. Cross-platform](17-cross-platform.md)).
- **Partial/streamed activation** — materialize a tool's most-used binaries first so `vanta x` of a large tool starts running sooner.

## A community provider ecosystem

The provider model ([07. Providers](07-providers.md), [22. Provider SDK](22-provider-sdk.md)) is built for an ecosystem:

- A **community registry** where anyone can publish a declarative (or WASM-hooked) provider, with **mandatory signing and review** before entry into the official, trusted index — the safe analog of asdf's plugin sprawl, without arbitrary shell.
- A **provider marketplace/discovery** surface (`vanta search`, a web catalog) with verified-publisher badges, download stats, and advisories.
- **Provider testing-as-a-service** — the provider test harness run in CI on submission, so a provider that resolves wrong or escapes its sandbox never ships.
- **Federation** — organizations run private registries that overlay or mirror the community one, with the same trust model ([14. Enterprise](14-enterprise.md)).

## Team and cloud environment sharing

The reproducibility model already lets a team share an environment via a committed lock. A future, optional service makes sharing *outside* a single repo effortless:

- **Named, shareable environments** — publish a locked toolset (`vanta env push @acme/backend`) and adopt it elsewhere (`vanta env use @acme/backend`), cross-platform and verified. This is the goal Flox's FloxHub serves, but cross-platform and not built on Nix ([33. Prior Art](33-prior-art.md)).
- **Org defaults** distributed centrally (registries, mirrors, policy, baseline tools) so a new machine is configured by one command.
- The service stores **locks and metadata, never the only copy of artifacts** — content lives in the content-addressed store/mirrors, so the service is convenience, not a dependency.

## An optional fleet control plane

For large, mixed-OS organizations, a hosted/self-hosted control plane (additive — [14. Enterprise](14-enterprise.md)):

- **Fleet inventory** from the existing `--json`/SBOM/audit outputs (no special agent required).
- **Central policy and private-registry management** with signed distribution to clients.
- **Rollout orchestration** — stage a tool/version bump across a fleet with the same atomic-generation/rollback guarantees each node already has.
- Crucially, it is **never in the request path**: a node with no control-plane connectivity installs, switches, and reproduces exactly as before.

## Reproducible source builds

Today Vanta is prebuilt-binary-first and treats source builds as a sandboxed exception ([08. Installation](08-installation.md), [15. Security](15-security.md)). A future direction is a **reproducible build layer** for tools lacking good prebuilts: hermetic, hash-pinned, sandboxed builds whose outputs are verified to be deterministic, expanding coverage without sacrificing the reproducibility guarantee — borrowing Nix's rigor for the long tail while keeping the common path on verified prebuilts.

## Deep editor integration

Because tools are real binaries on `PATH` with a stable `--json` surface ([29. Public APIs](29-public-apis.md)), first-party editor extensions can:

- Surface the active per-directory toolset and switch/upgrade from the editor.
- Auto-configure interpreter/SDK paths via `vanta which`.
- Show lock drift, outdated tools, and policy violations inline.
- Offer "trust this project's environment" prompts in-editor.

## AI-assisted diagnostics

`vanta doctor` is a natural home for assistance ([18. Developer Experience](18-developer-experience.md)):

- **Explainers** — "why is `node` resolving to 20 here?" traces the precedence chain and the manifest that won.
- **Failure diagnosis** — map a tool's runtime failure (missing glibc, wrong arch, broken PATH) to a specific cause and a one-command fix.
- **Upgrade advice** — surface advisories/yanks affecting locked versions and propose safe bumps.

These operate on Vanta's own structured data; they augment the deterministic engine, never replace its guarantees.

## The open-core invariant

Vanta is Apache-2.0 open core ([ADR-0016](24-architecture-decision-records.md)). The boundary is published and stable: **private registries, auth, policy, audit, SBOM, air-gapped bundles, and reproducibility are all in the open core** — enterprise needs are met without a paywall, deliberately unlike the freemium gating of some incumbents ([33. Prior Art](33-prior-art.md)). A separately-licensed enterprise edition/cloud adds *operational convenience at fleet scale* (the control plane, managed sharing, support, attestations), never a crippling of the OSS edition. The commercial layer succeeds only if the open core is widely adopted — "no rug-pulls" is a design constraint, not a marketing line.

## Cross-references

- [01. Vision](01-vision.md) — the five-year roadmap this elaborates.
- [07. Providers](07-providers.md) & [22. Provider SDK](22-provider-sdk.md) — the ecosystem this grows.
- [14. Enterprise](14-enterprise.md) — the fleet/policy features the cloud plane extends.
- [11. Reproducibility](11-reproducibility.md) — the lock that underpins environment sharing.
- [09. Store](09-store.md) — sub-file dedup and delta artifacts.
- [15. Security](15-security.md) — the trust model the community ecosystem relies on.
