# 25. Error & Exit-code Catalog

> The stable, searchable error taxonomy: the `VTA-<AREA>-<NNNN>` scheme, per-area code tables covering every failure the other documents reference, the error-message style guide, the stable exit codes, the `--json` error shape, and the mapping from `vanta doctor` checks to codes. Codes are generated from a single registry so behavior, docs, and tests cannot drift.

**Contents**

- [The scheme](#the-scheme)
- [Message style](#message-style)
- [Code tables](#code-tables)
- [Exit codes](#exit-codes)
- [The JSON error shape](#the-json-error-shape)
- [Doctor checks to codes](#doctor-checks-to-codes)
- [Cross-references](#cross-references)

---

## The scheme

Every user-facing error carries a stable code of the form **`VTA-<AREA>-<NNNN>`**. The area names the subsystem; the number is stable forever once assigned (a retired code is never reused). Codes are defined in one registry in `vanta-diag`, from which the catalog docs, the `--json` schema, and the test that asserts "every code is exercised" are generated — so a code cannot exist without a test and a doc entry ([ADR-0019](24-architecture-decision-records.md)).

| Area | Subsystem | Owning crate |
| --- | --- | --- |
| `CFG` | config / manifest | `vanta-config` |
| `RES` | resolution / versioning | `vanta-resolve` |
| `REG` | registry | `vanta-registry` |
| `PROV` | provider / sandbox | `vanta-provider` |
| `NET` | network / download | `vanta-net` |
| `VRF` | verification / security | `vanta-security` |
| `STORE` | store / state / IO | `vanta-store`, `vanta-state` |
| `INST` | install engine | `vanta-install` |
| `ENV` | environment / activation | `vanta-env` |
| `LOCK` | lockfile | `vanta-lock` |
| `SYS` | platform | `vanta-platform` |
| `INT` | internal (a bug) | any |

## Message style

Every error renders four things: the **code**, the **problem**, the **cause/context**, and a **suggested fix** (plus a docs link). Example:

```
error[VTA-RES-0002]: conflicting tool dependencies
  `terraform 1.9.8` requires `go >=1.22`, but the project pins `go = "1.21"`
help: raise the pin (`vanta add go@1.22`) or pin an older terraform (`vanta add terraform@1.8`)
docs: https://vanta.dev/errors/VTA-RES-0002
```

Rules: name the offending entity and the exact values; never blame the user; always offer at least one concrete next step; keep the first line scannable; put detail on subsequent lines; redact secrets. `INT` errors additionally invite a bug report with a diagnostic bundle.

## Code tables

Representative codes per area (the registry holds the complete set; these are the ones other docs reference).

### CFG — config / manifest

| Code | Meaning | Typical cause | Fix |
| --- | --- | --- | --- |
| `VTA-CFG-0001` | manifest TOML parse error | syntax error in `vanta.toml` | fix the syntax at the shown span |
| `VTA-CFG-0002` | unknown key | a misspelled/unsupported key | remove it or consult [27](27-config-reference.md) |
| `VTA-CFG-0003` | wrong value type | e.g. integer where a version string is expected | quote/retype the value |
| `VTA-CFG-0007` | unknown tool value form | `node = 24` (bare integer) | `node = "24"` |
| `VTA-CFG-0010` | task dependency cycle | `[tasks]` `depends` loop | break the cycle |
| `VTA-CFG-0012` | duplicate workspace member path | two members claim a path | dedupe `members` |

### RES — resolution / versioning

| Code | Meaning | Cause | Fix |
| --- | --- | --- | --- |
| `VTA-RES-0001` | no version satisfies the constraint | constraint excludes all available | relax it / check `vanta info` |
| `VTA-RES-0002` | conflicting dependencies | incompatible declared deps | adjust pins per the message |
| `VTA-RES-0003` | unknown tool | not in any configured registry | check the name / add a registry |
| `VTA-RES-0004` | ambiguous request | spec matches multiple candidates | qualify it |
| `VTA-RES-0005` | no artifact for this platform | provider lacks this os/arch | use another version / platform |
| `VTA-RES-0006` | policy denied | org policy forbids tool/version/license | per the cited rule ([14](14-enterprise.md)) |

### REG / PROV — registry & provider

| Code | Meaning | Cause | Fix |
| --- | --- | --- | --- |
| `VTA-REG-0001` | registry unreachable | network/DNS/registry down | retry / mirror / `--offline` |
| `VTA-REG-0002` | registry metadata invalid | malformed/incompatible index | refresh; report if persistent |
| `VTA-REG-0003` | auth required/failed | missing/expired credential | `vanta registry login` |
| `VTA-PROV-0001` | provider sandbox violation | hook exceeded capability/fuel/epoch | the provider is faulty/hostile; report |
| `VTA-PROV-0002` | provider returned invalid data | malformed artifact descriptor | report the provider |
| `VTA-PROV-0003` | provider ABI unsupported | provider targets an ABI this binary lacks | upgrade Vanta / the provider |

### NET — network

| Code | Meaning | Cause | Fix |
| --- | --- | --- | --- |
| `VTA-NET-0001` | download failed | timeout/5xx/exhausted mirrors | retry; check connectivity/mirrors |
| `VTA-NET-0002` | offline and not cached | `--offline` with a cache miss | go online, mirror, or `vanta restore` a bundle |
| `VTA-NET-0003` | TLS error | cert/proxy problem | configure CA bundle/proxy |

### VRF — verification / security

| Code | Meaning | Cause | Fix |
| --- | --- | --- | --- |
| `VTA-VRF-0001` | checksum mismatch | corrupted/substituted artifact | blob quarantined; retry; report if persistent |
| `VTA-VRF-0002` | signature invalid/missing | unsigned or bad signature | do not override unless trusted; report |
| `VTA-VRF-0003` | untrusted registry/provider | first use of a third party | `vanta trust` after review |
| `VTA-VRF-0004` | provenance below policy | SLSA level too low | use a compliant artifact |
| `VTA-VRF-0005` | `--no-verify` forbidden by policy | org policy blocks it | remove the flag |

### STORE / INST — store, state, install

| Code | Meaning | Cause | Fix |
| --- | --- | --- | --- |
| `VTA-STORE-0001` | store entry corrupt | disk error/tampering | `vanta doctor --repair` re-fetches |
| `VTA-STORE-0002` | IO / disk-full / cross-filesystem | environment problem | free space; keep `$VANTA_HOME` on one FS |
| `VTA-STORE-0003` | lock wait timeout | another `vanta` holds a lock | wait / kill the named pid |
| `VTA-STORE-0004` | state DB newer than binary | DB written by a newer Vanta | upgrade Vanta |
| `VTA-INST-0001` | archive extraction failed | corrupt archive / path traversal | report; integrity check |
| `VTA-INST-0002` | source build failed | upstream build error | check logs; prefer a prebuilt version |

### ENV / LOCK / SYS / INT

| Code | Meaning | Cause | Fix |
| --- | --- | --- | --- |
| `VTA-ENV-0001` | shell hook not installed | activation not set up | add `eval "$(vanta activate <shell>)"` |
| `VTA-ENV-0002` | tool not found in environment | not declared/installed here | `vanta add` it / check scope |
| `VTA-ENV-0003` | untrusted env/tasks not applied | manifest not trusted | `vanta trust` |
| `VTA-LOCK-0001` | lock drift (manifest ≠ lock) | manifest changed without relock | `vanta lock` / drop `--frozen` |
| `VTA-LOCK-0002` | lock format too new | lock written by a newer Vanta | upgrade Vanta |
| `VTA-LOCK-0003` | no locked entry for platform | platform not in `targets` | `vanta target add` + `vanta lock` |
| `VTA-SYS-0001` | unsupported platform | no compatible artifact/runtime | check support matrix |
| `VTA-INT-0001` | internal error (bug) | invariant violated | report with the diagnostic bundle |

## Exit codes

Stable for scripting (canon §13); each maps to one or more error areas:

| Exit | Meaning | Areas |
| --- | --- | --- |
| `0` | success | — |
| `1` | generic failure | `INT`, uncategorized |
| `2` | usage / CLI error | argument parsing |
| `3` | config / manifest invalid | `CFG` |
| `4` | resolution failed | `RES` |
| `5` | network / offline | `NET`, `REG` (reachability) |
| `6` | verification / security failure | `VRF` |
| `7` | store / IO / install failure | `STORE`, `INST`, `LOCK` (IO) |
| `8` | not found | `RES-0003`, `ENV-0002` |
| `9` | trust required | `VRF-0003`, `ENV-0003` |

CI scripts can branch on these without parsing text (e.g. treat `5` as retryable, `6` as fatal).

## The JSON error shape

With `--json`, errors are emitted as a stable, versioned object (a public API — [29. Public APIs](29-public-apis.md)):

```json
{
  "schema_version": 1,
  "ok": false,
  "error": {
    "code": "VTA-VRF-0001",
    "area": "VRF",
    "exit": 6,
    "message": "checksum mismatch for node 24.6.0 (linux/x86_64/gnu)",
    "context": { "tool": "node", "version": "24.6.0", "platform": "linux/x86_64/gnu",
                 "expected": "sha256:5f2c…", "actual": "sha256:91ab…" },
    "help": "the download was quarantined; retry, or report if it persists",
    "docs": "https://vanta.dev/errors/VTA-VRF-0001"
  }
}
```

The `code`, `area`, `exit`, and `context` keys are stable; `message`/`help` are human text and may change. Editors and CI integrate against the structured fields.

## Doctor checks to codes

`vanta doctor` runs checks that each map to a code and a fix ([18. Developer Experience](18-developer-experience.md)):

| Check | On failure |
| --- | --- |
| shell hook installed (per shell) | `VTA-ENV-0001` + the exact line to add |
| `~/.vanta/bin` on PATH, ahead of other managers | warning + PATH-ordering guidance |
| shim integrity (dispatcher hardlinks present) | `VTA-ENV-*` + `vanta doctor --repair` |
| store integrity (re-hash entries) | `VTA-STORE-0001` + auto re-fetch |
| state DB health / schema version | `VTA-STORE-0004` + upgrade guidance |
| registry reachability | `VTA-REG-0001` (informational offline) |
| lock completeness (all targets present) | `VTA-LOCK-0003` + `vanta target add` |
| `$VANTA_HOME` single-filesystem | `VTA-STORE-0002` warning |

## Cross-references

- [04. CLI](04-cli.md) — how errors and exit codes surface in the CLI.
- [29. Public APIs](29-public-apis.md) — the `--json` error schema as a stable surface.
- [08. Installation](08-installation.md) & [15. Security](15-security.md) — the `VRF`/`INST`/`STORE` failures in context.
- [18. Developer Experience](18-developer-experience.md) — `vanta doctor` and the error experience.
- [23. Data & State Model](23-data-and-state-model.md) — `STORE`/state and locking failures.
- [24. ADRs](24-architecture-decision-records.md) — ADR-0019, the taxonomy decision.
