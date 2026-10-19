# Architecture: Zairenkai-Core

> Owned by this repository. Document only what inspected code supports, citing file paths.

The full description is in `docs/architecture/TARGET_ARCHITECTURE.md`. This page is a summary.

| Path | Responsibility |
|---|---|
| `crates/zcore-domain/src/` | Typed contracts: `observation`, `device`, `capability`, `resource`, `request`, `decision`, `module`, `audit`, `protocol` |
| `crates/zcore-engine/src/policy.rs` | Deny-by-default typed policy |
| `crates/zcore-engine/src/arbitration.rs` | Leases, tiers, baselines |
| `crates/zcore-engine/src/core.rs` | The pipeline and reconciliation |
| `crates/zcore-engine/src/adapter.rs` | Adapter contract, fake adapter with fault injection |
| `crates/zcore-platform/src/` | Discovery, telemetry, module inventory, FAS / battery saver adapters, AFID lookup, allowlisted runner |
| `crates/zcored/src/` | Engine thread, tokio IPC server, lifecycle, watchdog, storage, CLI |

## Entry points

- `zcored run|discover|ctl|check-config|default-config|bench|version`
- `module/service.sh` (supervisor), `module/uninstall.sh`, `module/action.sh`

## Trust boundaries

- A root-only Unix socket. Principals come from SO_PEERCRED.
- Config can only restrict policy.
- Database records and module manifests are data.
- Commands are allowlisted, and there is no shell.
- Details: `docs/security/THREAT_MODEL.md`.

## Generated artifacts

`dist/` from `scripts/build-android.sh`. `module.prop` is generated from `module/module.prop.in`.
