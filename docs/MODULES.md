# Ecosystem integration and module contract

## Status per component (2026-10-19)

| Component | Integration in Core 0.1 | Status |
|---|---|---|
| Zairenkai-FAS | `fas` adapter over the `zfasd` socket. It owns `fas.profile` and `fas.enabled` and reads its state into telemetry. | **Implemented**. Tested against a socket fake; not run on a device. Requires the zfasd status fields from FAS commit `9d43894`; older builds are reported `read_only`. |
| Android battery saver | `android_power` adapter: `cmd power set-mode`, read back with `settings get global low_power` | **Implemented**; not validated on a device |
| Android Power HAL | Presence detection only (`service check`) | Detected; Core does not call it (see `architecture/REFERENCE_AUDIT.md`) |
| Zairenkai-Database-Source (AFID) | Read-only device lookup from a catalog copy at `paths.afid_catalog` (default `/data/adb/zairenkai-core/afid`) | **Implemented** (data only, never grants anything); no verified devices are in the catalog yet |
| Zairenkai-Intelligence-Engine | Protocol method `recommend` and the principal role `intelligence_engine` | Contract **implemented**; no engine exists yet |
| Zairenkai-SDK / Zairenkai-App | The protocol (`api/PROTOCOL.md`) is the contract | Contract defined; the SDK repository is empty; app transport per ADR-0005 |
| Zairenkai-ThermalTools | none | **Not available**: repository empty. Reported `not_installed`. |
| Zairenkai-License-Server | none | **Not available**: repository empty. No feature is licence-gated. |
| Zairenkai-Graphics-Platform | none | **Not available**: repository empty |
| Zairenkai-ART | none yet. Candidate input: `zart inspect` / `zart dexopt-status` JSON (`schema: 1`) | **No adapter.** Scope is Android Runtime. A read-only inspector exists on the ART topic branch `ccr-63f8aa87-1b0paj` (not merged; never run on a device). |
| Zairenkai-Modules | Module manifest contract and inventory | Contract **implemented**; no modules exist |

## Module contract (`zairenkai-module.toml`)

```toml
id = "zairenkai_thermaltools"          # [a-z][a-z0-9_]{0,63}, must equal module.prop id
version = "0.1.0"                      # semver
core_api = ["1.0.0", "1.99.99"]        # inclusive range of Core API versions
required_capabilities = ["telemetry.thermal_status"]
requested_permissions = ["set_low_power_mode"]   # declarative only
owns = []                              # resources; only built-in adapters own resources in 1.x
```

Rules (`crates/zcore-domain/src/module.rs`, `crates/zcore-platform/src/modules.rs`):
- **Read, never run.** Core reads `module.prop` and this manifest (regular files, at most
  64 KiB). It never executes, sources or loads anything from a module.
- **Declarations grant nothing.** Permissions come only from `principals` in Core's config.
- **Strict validation.** Unknown keys, invalid ids or versions, and an empty `core_api` range
  make the module `degraded` (incompatible).
- **Flags respected.** `disable` or `remove` flags make the module `disabled`.

## Contracts for components that do not exist yet

**Intelligence Engine.** It connects as its configured uid and calls `recommend` with a typed
operation and a confidence value.
- Core treats the result as an untrusted recommendation.
- It is denied below `safety.recommendation_min_confidence`.
- It always becomes a temporary lease, at most `safety.recommendation_max_ms`, at the lowest
  tier, so it can never override a user choice.
- The engine cannot confirm, and cannot submit as a user.
- Execution feedback is available through `audit`.

**License Server.** A design is required before any feature is gated:
- signed entitlements (Ed25519) with issue and expiry times, verified locally against a
  pinned key;
- a cached copy with a bounded offline grace period;
- minimal device identifiers;
- no instructions from the server.

A licence can only *remove* a feature's availability. It never grants operating-system
permissions or bypasses policy.

**ThermalTools.**
- Parse and validate vendor thermal configurations, and back up originals before any
  permitted change.
- Never write trip points, thermal zone `mode` or `emul_temp`, and never disable throttling.
- Thermal protection stays authoritative. Core will add a `thermal.*` adapter only when a
  tested component exists.

**Graphics and ART.** These are capability reporting first. Optimizations go through typed
operations and adapters, like everything else. ART work (dexopt, profile compilation) must use
`cmd package` and must never run on Core's control loop.
