# 21. Threat Model

> The adversarial analysis behind Vanta's security design: scope and assumptions, the assets worth protecting, the trust boundaries, a STRIDE pass per boundary, concrete abuse cases with attack→impact→mitigation→residual-risk, a consolidated threat→control matrix tying each risk to a control in [15. Security](15-security.md), and the explicit non-mitigations.

**Contents**

- [Scope and assumptions](#scope-and-assumptions)
- [Assets](#assets)
- [Trust boundaries](#trust-boundaries)
- [STRIDE by boundary](#stride-by-boundary)
- [Abuse cases](#abuse-cases)
- [Threat to control matrix](#threat-to-control-matrix)
- [Out of scope / non-mitigations](#out-of-scope--non-mitigations)
- [Cross-references](#cross-references)

---

## Scope and assumptions

Vanta downloads and executes software from the internet on a developer's machine and in CI. The threat model covers the path from "a tool exists somewhere" to "its bytes run on PATH," plus the local state that governs it.

Assumptions:
- The **local operating system and the user account are trusted**; a fully compromised host is out of scope (see non-mitigations).
- The **network is untrusted** (mirrors, proxies, DNS may be hostile).
- **Registries and providers are semi-trusted**: the official registry's *signing keys* are trusted (pinned), but individual providers/publishers and any third-party registry are treated as potentially hostile.
- **Project repositories are untrusted** until a user trusts their env/task sections.
- The adversary may be a network attacker, a malicious provider/publisher, a malicious repository author, or a supply-chain attacker targeting an upstream artifact or Vanta itself.

## Assets

| Asset | Why it matters |
| --- | --- |
| The content-addressed **store** | the bytes that will execute; corruption/poisoning = code execution |
| The user's **PATH / shell** | what runs when they type `node`; hijack = code execution |
| **Registry signing keys / trust DB** | the root of metadata trust; compromise = mass forgery |
| **Credentials** (registry tokens) | access to private artifacts; theft = lateral movement |
| The **lockfile** | the reproducibility/integrity contract; tampering = silent substitution |
| **CI** | runs Vanta with tokens and ships artifacts; a high-value target |
| The **Vanta binary** itself | a compromised updater compromises everything |

## Trust boundaries

```
   ┌───────────────────────── trusted: local OS + user account ──────────────────────────┐
   │                                                                                       │
   │   project vanta.toml ──(TOFU trust)──► vanta process ──► content-addressed store ──► PATH
   │        ▲ untrusted repo                   │   │  ▲                                     │
   │                                           │   │  └── state.db / trust DB / keychain    │
   └───────────────────────────────────────── │ ──│ ────────────────────────────────────── ┘
                                               │   │
              ════════ network boundary (untrusted) ════════
                                               │   │
            registry metadata (signed roles) ──┘   └── artifacts (signed + hashed)
                 ▲                                          ▲
          official / private registry                mirrors · publishers · CDNs
                 ▲                                          ▲
            providers (declarative + WASM sandbox)    upstream build/source
```

Boundaries, from inside out: **project config** (TOFU), the **vanta process** (the enforcement point), the **store/state/keychain** (local trusted but integrity-checked), the **network** (untrusted transport), **registries/providers** (semi-trusted, signature-gated), and **artifact publishers/upstream** (semi-trusted, signature+provenance-gated).

## STRIDE by boundary

| Boundary | Spoofing | Tampering | Repudiation | Info disclosure | DoS | Elevation |
| --- | --- | --- | --- | --- | --- | --- |
| Network transport | fake mirror/registry (→ TLS + signed metadata) | bytes altered in flight (→ hash+sig) | — | sniffing (→ TLS) | slow/oversized responses (→ timeouts, size caps) | — |
| Registry metadata | forged index (→ TUF roles, pinned root) | rollback/freeze (→ snapshot+timestamp) | unsigned changes (→ signed roles) | — | huge index (→ bounded parse) | — |
| Provider logic (WASM) | impersonate a tool (→ signing/review) | return bad URL/hash (→ artifact re-verify vs lock) | — | exfiltrate via network (→ no ambient net) | infinite loop / memory (→ fuel/epoch/limits) | escape sandbox (→ capability deny-by-default) |
| Artifact / publisher | unsigned/forged artifact (→ sig+provenance) | corrupted bytes (→ checksum) | — | — | giant archive / zip-bomb (→ bounded extraction) | malicious build/postinstall (→ build/hook sandbox) |
| Project config | — | malicious `[env]`/`[tasks]` (→ TOFU trust) | — | leak env (→ trust gate) | — | PATH hijack / run code (→ trust gate, no ambient exec) |
| Local store/state | — | external tampering with a store entry (→ re-hash detect) | — | read tokens (→ keychain, redaction) | fill disk (→ quotas, GC, clear errors) | — |
| Vanta self-update | fake release (→ signed releases) | swapped binary (→ sig+provenance, atomic+rollback) | — | — | — | — |

## Abuse cases

Each as **attack → impact → mitigation → residual risk**.

- **Malicious provider.** A community provider returns a URL/hash pointing at malware. → It would install attacker code. → The provider runs **sandboxed** (no ambient authority), and the resulting artifact must still pass checksum + signature + provenance **against the signed metadata/lock**, not against what the provider claims; signing+review gates entry. → *Residual:* a provider that points at a *legitimately signed but malicious* upstream — mitigated by publisher verification, advisories/yanks, and review.

- **Compromised mirror.** An attacker controls a mirror and serves altered bytes. → Substituted tool. → Every artifact is hash-verified against the lock/signed metadata regardless of which mirror served it; a mismatch fails closed and falls through to another source. → *Residual:* availability only (a bad mirror can deny, not deceive).

- **Typosquatted tool.** A user runs `vanta add expres` (typo). → Installs an impostor. → Verified publishers + reserved namespaces + name-similarity warnings before install. → *Residual:* a determined user overriding the warning.

- **Lockfile tampering in a PR.** An attacker edits `vanta.lock` hashes/URLs in a pull request. → CI installs attacker artifacts. → Artifacts are re-verified against signatures (not just the lock's hash), so a changed hash pointing at unsigned/wrongly-signed bytes fails; `--frozen` + human review of lock diffs (canonical, minimal) surface the change. → *Residual:* an attacker who can also forge a valid signature — reduced to the metadata-key threat.

- **TOCTOU on store materialization.** Race between verify and publish to swap bytes. → Poisoned store entry. → Verification happens on the cached blob, materialization is temp-then-rename of the *verified* bytes, and entries are read-only and content-addressed (the path is the hash). → *Residual:* a local attacker with write access to `$VANTA_HOME` mid-operation — i.e. an already-compromised account (out of scope).

- **PATH hijack via a malicious repo.** A cloned repo's `vanta.toml` injects `[env] PATH` or a `[tasks]` command to run on entry. → Code execution on `cd`/run. → **Trust-on-first-use**: env injection and tasks are inert until `vanta trust`; tools resolve/install (independently verified) but nothing runs. → *Residual:* a user who trusts a malicious repo without reading it.

- **Sandbox escape (provider/build).** A WASM provider or a source build tries to read the filesystem or open a socket. → Exfiltration or tampering. → Capabilities are deny-by-default; no ambient fs/net/env; builds run network-off-after-fetch; fuel/epoch bound execution. → *Residual:* a Wasmtime/sandbox vulnerability — mitigated by keeping the runtime updated and the attack surface tiny (scoped HTTP GET + hashing only).

- **Stolen registry signing key.** An attacker obtains a signing key. → Mass metadata/artifact forgery. → TUF-style roles limit blast radius (a compromised targets key ≠ root); **threshold M-of-N root** keys kept offline; **key rotation** via the root role; snapshot/timestamp limit rollback. → *Residual:* simultaneous compromise of threshold root keys (very high bar).

- **Compromised Vanta release.** The updater is fed a malicious binary. → Total compromise. → Releases are cosign-signed with SLSA provenance, reproducible, and `self update` verifies signature+provenance before an atomic swap with rollback ([32. Release Engineering](32-release-engineering.md)). → *Residual:* compromise of Vanta's own release keys — mitigated identically to registry keys.

- **Credential theft.** Malware reads stored registry tokens. → Access to private artifacts. → Tokens live in the OS keychain (not plaintext config/lock), are scoped per host, and are redacted from logs. → *Residual:* malware running as the user (out of scope).

## Threat to control matrix

| Threat | Control(s) | Where |
| --- | --- | --- |
| Bytes altered in transit / bad mirror | TLS (rustls) + mandatory checksum + signature, fail-closed | [15](15-security.md), [08](08-installation.md) |
| Forged / rolled-back metadata | TUF-style signed roles, pinned root, snapshot+timestamp | [15](15-security.md), [26](26-registry-and-metadata-reference.md) |
| Malicious provider logic | WASM capability sandbox, fuel/epoch, determinism; signing/review | [22](22-provider-sdk.md), [15](15-security.md) |
| Arbitrary install-time code | no ambient scripts; sandboxed builds/hooks; policy off-switch | [08](08-installation.md), [15](15-security.md) |
| Malicious project config | trust-on-first-use for env/tasks | [05](05-configuration.md), [15](15-security.md) |
| Typosquatting | verified publishers, reserved namespaces, similarity warnings | [15](15-security.md) |
| Lockfile tampering | signature re-verification + canonical, reviewable lock + `--frozen` | [11](11-reproducibility.md), [15](15-security.md) |
| Store poisoning / TOCTOU | verify-before-publish, temp-then-rename, read-only content-addressed entries, re-hash | [09](09-store.md), [08](08-installation.md) |
| Key compromise | threshold offline root, role separation, rotation | [15](15-security.md), [32](32-release-engineering.md) |
| Malicious self-update | signed + provenance + atomic swap + rollback | [12](12-updates.md), [32](32-release-engineering.md) |
| Credential theft | OS keychain, scoped tokens, log redaction | [23](23-data-and-state-model.md) |
| Resource-exhaustion DoS | timeouts, size caps, bounded extraction, fuel/epoch | [08](08-installation.md), [22](22-provider-sdk.md) |

## Out of scope / non-mitigations

Stated honestly so operators know the boundary:

- **A fully compromised local OS or user account.** If malware already runs as the user, it can tamper with `$VANTA_HOME` or PATH directly; Vanta's integrity checks detect tampering after the fact but cannot prevent an attacker with equal privilege.
- **Malicious-but-validly-signed upstream software.** If a legitimate, verified publisher ships malware, Vanta will install exactly the signed bytes; the defense is publisher reputation, advisories/yanks, and review — not cryptography.
- **Side channels and hardware attacks.**
- **The correctness of the tools themselves.** Vanta guarantees *integrity and reproducibility* of delivery, not that `terraform` has no bugs.

## Cross-references

- [15. Security](15-security.md) — the controls every row above maps to.
- [22. Provider SDK](22-provider-sdk.md) — the capability sandbox for provider hooks.
- [08. Installation](08-installation.md) — the fail-closed verification gate and atomic publish.
- [11. Reproducibility](11-reproducibility.md) — lock integrity and `--frozen`.
- [26. Registry & Metadata Reference](26-registry-and-metadata-reference.md) — the signed-role metadata model.
- [32. Release Engineering](32-release-engineering.md) — Vanta's own supply-chain defenses and key management.
- [23. Data & State Model](23-data-and-state-model.md) — credential storage and state integrity.
