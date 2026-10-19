# Implementation status (0.1.0, 2026-10-19)

## Implemented and tested (host, fixtures and fakes)

- Domain types and IPC protocol v1 (`zcore-domain`): 9 tests.
- Policy engine, registry, arbitration, audit and config (`zcore-engine` unit tests): 14 tests.
- Pipeline and fault injection (`zcore-engine/tests/pipeline.rs`): 16 tests covering:
  - permission denial, missing capability, stale or hot context;
  - adapter error, timeout, unverifiable write, wrong effect with rollback, failed rollback;
  - unreadable resource;
  - conflicting requests, recommendation tiering, expiry restore, external modification,
    shutdown, release ownership.
- Platform (`zcore-platform`): 14 tests.
  - Discovery from getprop, root-manager markers, SELinux and Power HAL.
  - Telemetry parsing with explicit unavailability.
  - Module inventory.
  - FAS adapter against a socket fake.
  - Battery saver adapter.
  - AFID lookup with path and identity checks.
  - Command allowlist.
- Daemon (`zcored`): 4 tests, including an end-to-end run over the real Unix socket. It
  covers:
  - health and context;
  - FAS profile applied and verified;
  - a temporary battery saver lease restored on expiry by reconciliation;
  - duplicate-id, unknown-method and wrong-version rejection;
  - module report and the single-instance lock;
  - persisted leases and audit;
  - refusal to start with an invalid config.
- Packaging: Android arm64 cross-build (NDK r27d), module scripts (shellcheck clean), CI
  workflow (actionlint clean).

## Implemented, not physically validated

These have never run on a phone:
- `zcored` itself;
- the FAS adapter against a real `zfasd` (it needs the FAS status fields from FAS commit
  `9d43894`);
- `cmd power set-mode` and `settings` on real ROMs, including whether uid 0 passes the
  POWER_SAVER check;
- SELinux behaviour under each root manager;
- idle CPU and memory use.

## Partially implemented

- **Learning.** Only `verified_on_device` marking. There are no effectiveness statistics.
- **Module contract.** Manifests are parsed and checked for compatibility. Modules cannot own
  resources or call Core through anything other than root; there is no signature verification.
- **App access.** The authorization model for app uids is complete. The transport is not
  (ADR-0005).

## Not implemented (blocked or planned)

- **ThermalTools, Graphics, ART, License Server, SDK and Intelligence Engine adapters.**
  Their repositories are empty, except ART, which has a read-only inspector on a topic branch
  but no Core adapter. The contracts are in `MODULES.md`.
- **Entitlement verification.** No licence server exists, and no feature is gated.
- **Power HAL calls.** Deliberately not done: system_server owns the HAL.
- **Kernel or sysfs writes.** Deliberately none in Core; they belong to the owning component
  (FAS).

## Next milestone

1. Merge the FAS status change. Deploy FAS and Core on 2–3 devices from different vendors.
   Run `discover`, a dry run, and apply/verify for each operation. Record SELinux denials.
2. Measure idle CPU, RSS and wakeups (`dumpsys batterystats`, `/proc/<pid>/status`) over 1 h.
3. Choose and validate the app transport (ADR-0005), then build the SDK client.
4. Specify the License Server and its entitlement format before gating any feature.
5. Add the first ThermalTools contract test once that repository has code.
