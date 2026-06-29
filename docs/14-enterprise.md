# 14. Enterprise

> What an organization needs to adopt Vanta at scale: private registries, authentication and SSO, policy and governance, team configuration distribution, audit and compliance (SBOM, provenance, license reporting), fleet management, and the open-core/edition boundary. Enterprise capability is **additive** — a single Vanta node is always fully functional on its own, and nothing here is required for the solo developer.

**Contents**

- [Private registries](#private-registries)
- [Authentication](#authentication)
- [Policy and governance](#policy-and-governance)
- [Team configuration distribution](#team-configuration-distribution)
- [Audit and compliance](#audit-and-compliance)
- [Fleet management](#fleet-management)
- [Support and editions](#support-and-editions)
- [Failure modes and trade-offs](#failure-modes-and-trade-offs)
- [Cross-references](#cross-references)

---

## Private registries

An organization runs its own registry to publish internal tools, vet public ones, and operate disconnected. A private registry is the same signed, content-addressed index as the official one ([07. Providers](07-providers.md), [26. Registry Reference](26-registry-and-metadata-reference.md)); it is added with a name, URL, and priority and **overlays** the official registry rather than replacing it:

```toml
# ~/.vanta/config.toml (or an org-distributed base config)
[registries.acme]
url = "https://vanta.acme.internal"
priority = 10          # lower number = checked first; overlays official (priority 100)
auth = "oidc"

[registries.official]
url = "https://registry.vanta.dev"
priority = 100
```

- **Overlay precedence.** A tool present in `acme` is served from `acme`; otherwise resolution falls through to the official registry. This lets an org publish `acme/deploy-cli`, pin vetted versions of public tools, and still reach the public catalog for the long tail — or, by removing the official entry, run a fully closed catalog.
- **Internalizing public tools.** `vanta registry mirror` copies vetted public tools (with their signed metadata and artifacts) into the private registry for review and air-gapped use ([13. Offline](13-offline.md)).
- **Publishing internal tools.** An internal tool is published by adding a signed provider + artifacts to the private registry (same format as community providers; [22. Provider SDK](22-provider-sdk.md)).
- **Snapshot pinning.** An org can pin a `registry_revision` so resolution across the whole company is reproducible and reviewed, not subject to upstream drift ([11. Reproducibility](11-reproducibility.md)).

## Authentication

Registries and mirrors can require credentials. Vanta supports the mechanisms enterprises actually use, and **never stores secrets in plaintext config**:

| Method | Use | Storage |
| --- | --- | --- |
| Bearer token | CI, service accounts | `VANTA_REGISTRY_TOKEN_<NAME>` env / OS keychain |
| OIDC / SSO | interactive developer login | short-lived token via device/auth-code flow; cached in keychain |
| `.netrc` | existing corporate tooling | read-only, honored per host |
| OS keychain | default at-rest store | macOS Keychain, Windows Credential Manager, Linux Secret Service (0600 file fallback) |

- `vanta registry login acme` runs the configured flow (e.g. OIDC device login) and stores the resulting token in the OS keychain ([23. Data & State Model](23-data-and-state-model.md#secret-handling)).
- Per-registry credentials are scoped; a token for `acme` is never sent to another host.
- CI uses a token in an environment variable; no interactive step.

## Policy and governance

An organization enforces what may be installed via an **org policy** distributed to clients (a signed `policy.toml` referenced by config or served by the registry). Policy is evaluated at **resolution and install time**, so a violation fails before anything materializes.

```toml
# org policy (signed, distributed)
[policy]
require_signature = true            # refuse unsigned artifacts
forbid_no_verify = true             # `--no-verify` is disallowed
allow_source_builds = false         # prebuilt only (no sandboxed source builds)
min_slsa_level = 2                  # provenance floor

[policy.tools]
allow = ["node", "python", "go", "rust", "terraform", "acme/*"]   # allow-list
deny  = ["leftpad-cli"]

[policy.versions]
node = ">=20"                       # org floor
terraform = ">=1.5, <2.0"

[policy.licenses]
allow = ["Apache-2.0", "MIT", "BSD-3-Clause", "MPL-2.0"]
deny  = ["GPL-3.0-only"]
```

- **Enforcement points.** Resolution rejects a tool/version/license outside policy (`VTA-RES-*` policy-denied); install refuses unsigned/under-provenance artifacts (`VTA-VRF-*`); `--no-verify` is blocked when `forbid_no_verify` is set.
- **Distribution.** Policy ships in the org base config or is fetched from the registry; it is signed so a developer cannot quietly weaken it. Clients verify the policy signature against a pinned org key.
- **Scope.** Policy can be global (all projects) or per-registry; teams can layer stricter local policy but never loosen org policy.
- This is governance without a daemon: enforcement is in every `vanta` invocation, driven by signed declarative policy.

## Team configuration distribution

The reproducibility model *is* the distribution model:

- A repository commits `vanta.toml` + `vanta.lock`; every member runs `vanta sync` and gets the identical, verified toolset on their OS ([11. Reproducibility](11-reproducibility.md)).
- An **org base config** (a `config.toml` fragment distributed via dotfiles, MDM, or `vanta config import <url>`) sets the registries, mirrors, policy reference, and global tools every machine should have.
- CI uses `vanta sync --frozen`, guaranteeing the committed lock is used unchanged.
- Onboarding collapses to: install Vanta (one line), apply the org base config, `git clone`, `vanta sync`.

## Audit and compliance

- **Audit logging.** Installs, updates, removals, trust decisions, and policy denials are written to a structured, append-only audit log (`~/.vanta/audit.log`, JSON lines; optionally forwarded to a SIEM via a log shipper). Each record carries the actor, command, resolved versions, store keys, signatures verified, and the generation produced.
- **SBOM export.** `vanta sbom [--format cyclonedx|spdx]` emits a software bill of materials for a project or environment — every tool, exact version, artifact hash, source, and license — suitable for compliance pipelines.
- **Provenance.** SLSA provenance is verified at install (to the configured level) and recorded; `vanta provenance <tool>` shows the verified chain ([15. Security](15-security.md)).
- **License reporting.** `vanta licenses` lists the license of every installed tool and flags policy violations.
- **Reproducible evidence.** Because the lock pins hashes, an audit can prove that what shipped is exactly what was reviewed.

## Fleet management

- **Self-sufficient nodes.** Every Vanta installation works fully offline with its store and lock; there is no required control plane ([ADR-0020-equivalent in the no-daemon decision](24-architecture-decision-records.md)).
- **Optional cloud control plane (future, additive).** A hosted/self-hosted service for sharing locked environments across a team, distributing policy and private-registry config, and viewing fleet inventory — described in [20. Future](20-future.md). It is never in the install path; losing it degrades convenience, not function.
- **Inventory.** `vanta list --json` and the SBOM/audit outputs feed existing asset-management systems without a Vanta-specific agent.

## Support and editions

Vanta follows an **open-core** model ([ADR-0016](24-architecture-decision-records.md), [01. Vision](01-vision.md)):

- The **open-source core** (Apache-2.0) is genuinely complete: private registries, auth, policy, audit, SBOM, air-gapped bundles, and reproducibility are all in the core — the enterprise needs are met without a paywall, deliberately unlike the freemium gating of some incumbents.
- A separately-licensed **enterprise edition / cloud service** adds *operational convenience at fleet scale* (the cloud control plane, managed private registries, support SLAs, compliance attestations), never a crippling of the OSS edition. Commercial success requires the open core to thrive ("no rug-pulls").

## Failure modes and trade-offs

| Scenario | Behavior |
| --- | --- |
| Tool denied by org policy | resolution fails with the policy rule cited; nothing installs |
| Unsigned artifact under `require_signature` | install fails `VTA-VRF-*`; no override unless policy allows |
| Private registry unreachable | falls through to lower-priority registries/mirrors, or fails cleanly offline |
| Credential expired | re-login prompt (interactive) or clear CI error; no silent unauthenticated fetch |
| Policy signature invalid | client refuses the policy and reports tampering rather than running unprotected |

- **Trade-off: policy adds friction by design.** A locked-down org may refuse a tool a developer wants; the message names the rule and the owner, turning a silent failure into a governance conversation.
- **Trade-off: overlay precedence must be understood.** `vanta which`/`vanta info` always show which registry served a tool, so resolution is never opaque.

## Cross-references

- [07. Providers](07-providers.md) — the registry/provider model private registries reuse.
- [15. Security](15-security.md) — signatures, provenance, and the verification policy enforces.
- [13. Offline](13-offline.md) — internalization, mirrors, and air-gapped bundles.
- [11. Reproducibility](11-reproducibility.md) — committed locks and registry-revision pinning for org-wide reproducibility.
- [23. Data & State Model](23-data-and-state-model.md) — credential storage in the OS keychain and the audit log.
- [20. Future](20-future.md) — the optional cloud control plane and fleet features.
- [26. Registry & Metadata Reference](26-registry-and-metadata-reference.md) — signed-metadata roles for private registries.
