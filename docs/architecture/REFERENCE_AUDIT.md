# Upstream reference audit

Done on 2026-10-19 from shallow clones of the actual sources (sparse checkouts) and from
gitiles fetches for AOSP. No upstream build script or code was run, and **no upstream code was
copied into Zairenkai-Core**. Every adaptation below is an independent re-implementation of a
design lesson.

| Project | Revision | Licence | Obligations for Core |
|---|---|---|---|
| [KernelSU-Next](https://github.com/KernelSU-Next/KernelSU-Next) | `d65c9db` (dev) | GPL-3.0-or-later; `kernel/` GPL-2.0-only | None while no code is copied. Paths and file formats are used only as interoperability facts. |
| [KernelSU](https://github.com/tiann/KernelSU) | `b609757` (main) | Same as above; `js/package.json` says Apache-2.0 (no LICENSE file in that directory) | None (no code copied) |
| [AOSP power HAL](https://android.googlesource.com/platform/hardware/interfaces/+/refs/heads/main/power/aidl/default/), frameworks/base, frameworks/native | hardware/interfaces `1a56e38`, base `1cdfff5`, native `4f463a6` (public `main`, last updated 2025-03) | Apache-2.0 | None (no code copied) |
| [Cedar](https://github.com/cedar-policy/cedar) | `cf7e99c` (v4.14.0) | Apache-2.0 | Not a dependency |
| [Open Policy Agent](https://github.com/open-policy-agent/opa) | `7bcf861` | Apache-2.0 | Not a dependency |
| [OpenTelemetry Collector](https://github.com/open-telemetry/opentelemetry-collector) | `ebfdf90` | Apache-2.0 | Not a dependency |
| [Tokio](https://github.com/tokio-rs/tokio) | `0d322ce`; crate 1.53.3 used | MIT | Dependency: keep the MIT notice (`docs/THIRD_PARTY.md`) |
| [systemd](https://github.com/systemd/systemd) | `462e1cc` | LGPL-2.1-or-later (per-file SPDX) | None (behaviour re-implemented in shell) |
| [Linux PM docs](https://github.com/torvalds/linux/tree/master/Documentation/admin-guide/pm) | `c03c1a1` | GPL-2.0 | None (interface facts only) |

## KernelSU-Next and KernelSU

**Files read:**
- `uapi/supercall.h`, `uapi/app_profile.h`
- `kernel/supercall/{perm.c,dispatch.c}`, `kernel/policy/allowlist.c`
- `userspace/ksud/src/{defs,module,metamodule,init_event,ksucalls,feature,preinit,boot_patch}.rs`
- tiann `website/docs/guide/module.md`, `js/index.d.ts`

**Lessons adopted:**
- **A permission check on every command, deny-unknown.** `dispatch.c` attaches a
  `perm_check` to each ioctl. In Core, `protocol::Method` is a closed enum with
  `deny_unknown_fields`, unknown uids are refused before any method runs, and every
  mutation goes through the policy engine.
- **Interface version gating.** ksud refuses a mismatched kernel UAPI. Core's protocol
  requires `v: 1` and module manifests declare a `core_api` range; a mismatch is reported as
  incompatible, never loaded.
- **Capabilities report supported/value/forced (locked).** Core's `CapabilityStatus`
  separates read-write from read-only and restricted, and `verified_on_device` records
  whether a read-back ever succeeded.
- **Module lifecycle by flag files.** `disable`, `remove` and `update` in
  `/data/adb/modules/<id>/`. Core's inventory (`zcore-platform::modules`) reads `module.prop`
  and the `disable` and `remove` flags, and never executes module files.
- **Detection markers** (read-only): `/data/adb/ksud`, `/data/adb/ksu`, `/data/adb/ap`,
  `/data/adb/magisk`.
- **Boot-loop guard** (`preinit.rs`, `BOOTCOUNT_LIMIT = 3`). This is reflected in the
  `service.sh` start limit (5 starts in 10 minutes, then give up and leave a `failed` marker).
  Core applies nothing at boot.

**Not copied:**
- The private reboot-magic and ioctl interface.
- The GKI check in `boot_patch.rs`, which looks wrong: the patch level passes the version test.
- The permissive `module.prop` parser.
- Unvalidated module ids in `disable_module` and `set_template`; Core validates ids.
- Non-atomic config writes; Core uses temp file plus rename.
- su, mount and sepolicy machinery.
- The WebUI's unrestricted `ksu.exec` channel. Core exposes a typed socket instead.

## AOSP power

**Files read:**
- `power/aidl/default/*`
- frozen IPower API versions 1–5 and the current version
- `PowerManagerShellCommand.java`, `PowerManagerService.java`, `BatterySaverStateMachine.java`,
  `BatterySaverController.java`
- `cmds/service/service.cpp`

**Facts used:**
- `cmd power set-mode N` exists and calls `setPowerSaveModeEnabled(N == 1)`.
- It returns 0 even when nothing changed.
- It does nothing while the device is powered, or when `config_batterySaverSupported` is false.
- `BatterySaverStateMachine` writes `Settings.Global.LOW_POWER_MODE` (`"low_power"`), so
  `settings get global low_power` is the read-back.
- At boot completion the framework resets `low_power` to 0. Core's reconciliation treats that
  as an external change and does not fight it.
- `service check NAME` exits 0 whether or not the service is found, so Core parses the output.
- The default Power HAL is a stub: `isModeSupported` returns true for everything, so "supported"
  proves nothing.

**Adopted:**
- `LowPowerAdapter` writes through `cmd power` and verifies through the setting. The policy
  rule `builtin.battery_saver_while_charging` denies changes while charging.
- Power HAL presence is detected (`android.power_hal`, read-only) and never called. Calling
  IPower directly would compete with system_server, the HAL's owner.
- **Not verified:** whether uid 0 passes the `POWER_SAVER`/`DEVICE_POWER` permission check on
  every ROM. Read-back verification reports `unverified` or `rolled_back` if it does not.

## Cedar and OPA

**Cedar** (`cedar-policy-core/src/authorizer.rs`, `partial_response.rs`, `Cargo.toml`):
- Deny by default; forbid overrides permit.
- The only error mode is `Skip`: an erroring policy is ignored, so a failing *forbid* can turn
  a Deny into an Allow. That is wrong for safety rules, which must fail closed.
- MSRV 1.89, above this project's 1.85.
- About 65 runtime crates on Android, including `stacker`/`psm` assembly (the NDK build of it is
  not verified).

**OPA** (`v1/sdk/opa.go`, `v1/plugins/logs/plugin.go`):
- A Go runtime: a second toolchain and an IPC hop.
- The decision-log design (decision id, input snapshot, revision, determining rules, bounded
  buffer with a drop policy) is worth copying in spirit.

**Decision (ADR-0002):** a small typed Rust engine with Cedar's semantics.
- Deny by default.
- Any Deny wins, the equivalent of forbid overrides permit.
- Rules are total functions over typed input, so there is no evaluation error to skip.
- Decisions carry every rule outcome and a policy version, and are audited with a correlation
  id. The audit buffer is bounded and counts what it drops (`audit_dropped`), as OPA's
  decision logs do.

## OpenTelemetry Collector

**Files read:**
- `component/component.go`, `componentstatus/status.go`
- `service/internal/graph/graph.go`
- `internal/memorylimiter`
- `exporterhelper/internal/queue*`

**Adopted:**
- Start consumers before producers, and stop producers first. The engine is ready (audit sink
  and adapters) before the socket accepts. On stop the socket closes first, then the engine
  ends leases and flushes the audit log.
- Every queue is bounded and has an explicit overflow policy:
  - IPC to engine: `ipc.queue`, answered `busy`;
  - connections: 32, answered `busy`;
  - audit pending: 1000, oldest dropped and counted.
- Health is an aggregate of component statuses (`health.reasons`).

**Not copied:** the plugin and factory machinery, and GC forcing.

## Tokio

**Files read:** `sync/mpsc/bounded.rs`, `time/timeout.rs`, `task/blocking.rs`, `signal/unix.rs`,
tokio-util `cancellation_token.rs`.

**Adopted:**
- A `current_thread` runtime only for IPC, signals and timers.
- Device I/O runs on one dedicated engine thread, never on the async runtime.
- `try_send` into a bounded `std::sync::mpsc::sync_channel`; `oneshot` replies with a
  `timeout`.
- `spawn_blocking` only for the shutdown hand-off.

tokio 1.53.3 has MSRV 1.85, which matches this project.

## systemd

**Files read:** `src/core/service.c` (`service_shall_restart`, restart delay steps),
`src/core/unit.c` (start limit), `src/basic/ratelimit.c`.

**Adopted:**
- The `service.sh` supervisor: exponential backoff 5 s → 300 s; a start limit of 5 starts in
  10 minutes, after which it gives up and writes a `failed` marker; no restart after a clean
  stop (exit 0) or a duplicate instance (exit 3).
- `zcored` has an internal engine watchdog (90 s): on a stall it aborts, and the next start
  reconciles leases.

## Linux PM documentation

**Files read:** `cpufreq.rst`, `drivers/cpufreq/cpufreq.c` (`store_one`, `cpufreq_set_policy`),
`driver-api/thermal/sysfs-api.rst`, `drivers/thermal/thermal_sysfs.c`.

**Facts:**
- `scaling_min_freq` writes are QoS requests. The value read back may be rounded or clamped by
  a lower maximum.
- Thermal trip points are writable only when the driver opts in.
- `emul_temp` and `mode` can defeat thermal policy.

**Consequence for Core:**
- Core writes no kernel node at all.
- Frequency control stays in FAS, which already handles read-back clamping.
- Thermal nodes are inputs only (the thermal status comes from `dumpsys thermalservice`).
- ThermalTools contracts (`../MODULES.md`) forbid writing trip points, `mode` or `emul_temp`.
