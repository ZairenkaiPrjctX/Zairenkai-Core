# Operations: build, test, install, diagnose, recover

## Build and test

| Purpose | Command |
|---|---|
| Tests (unit, fault injection, end-to-end socket) | `cargo test --workspace` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check` |
| MSRV | `cargo +1.85.0 test --workspace` |
| Shell | `shellcheck -s sh module/*.sh module/META-INF/com/google/android/update-binary && shellcheck scripts/*.sh` |
| Android module | `ANDROID_NDK_HOME=<ndk r27d> scripts/build-android.sh` → `dist/zairenkai-core-v<ver>.zip` |
| Hot-path cost (host) | `cargo run --release -p zcored -- bench 20000` |

No test touches the host's `/sys`, `/proc` or Android services. Platform code runs against
`SysRoot` fixtures and fake runners. `zfasd` is simulated by a socket fake.

## Install

1. Flash the zip in Magisk, KernelSU or APatch. Requirements: arm64, API 29 or newer.
2. Reboot. `service.sh` waits for boot completion, then supervises `zcored run`:
   - backoff from 5 s up to 300 s;
   - after 5 starts in 10 minutes it gives up and writes `/data/adb/zairenkai-core/failed`.
3. Start with the read-only report: `su -c /data/adb/modules/zairenkai_core/bin/zcored discover`.

## Files (`/data/adb/zairenkai-core`, mode 0700)

| File | Content |
|---|---|
| `config.toml` | Optional; defaults apply. `zcored check-config` validates it, and `zcored default-config` prints the defaults. |
| `zcored.sock` | Control socket (0600) |
| `leases.json` | Active leases (atomic). A corrupt file is moved to `leases.json.corrupt`. |
| `audit.jsonl` | Audit events, rotated at 1 MiB to `audit.jsonl.1` |
| `zcored.log` | stderr, rotated at 512 KiB by `service.sh` |
| `zcored.lock` | Single-instance lock |
| `failed` | Present when the supervisor gave up |

## Optional: ThermalTools diagnostics

Core reports `thermaltools.diagnostics` and `thermaltools.sensors` when a `zthermal` binary
is configured:

```toml
[paths]
thermaltools_binary = "/data/adb/zairenkai/bin/zthermal"
```

The file must be named `zthermal`, executable, owned by root, and neither it nor any
directory above it may be group- or world-writable (`/data/local/tmp` does not qualify).
Core runs only `zthermal snapshot` (read-only, redacted), with a 5 s timeout and a 1 MiB
output cap. Without the setting, or when a check fails, ThermalTools is reported as not
installed and the reason is in `zcored ctl modules`.

Cost: the snapshot runs once per discovery pass (`telemetry.discovery_ms`, default 60 s,
minimum 500 ms) on the engine thread. On a Linux host it took a median of 2.1–2.3 ms (max
5.4 ms) for 11–24 KB of output (30 runs each, fixtures and live, 2026-10-21). Device timing
is unmeasured. A hung binary can delay IPC replies by at most the 5 s timeout, well within the
90 s engine watchdog. There is no caching: at this cost it is not needed.

## Diagnostics

```sh
zcored ctl health          # ready | degraded + reasons
zcored ctl capabilities    # what is readable/writable here, with evidence
zcored ctl context         # telemetry with source/age for every value; AFID knowledge
zcored ctl modules         # installed Zairenkai modules + components not yet available
zcored ctl leases          # who owns which resource, until when, and the baseline
zcored ctl policy          # policy version, grants, configured rules
zcored ctl audit '{"limit":100}'
zcored ctl submit '{"operation":{"op":"fas_set_profile","profile":"performance"},"dry_run":true}'
```

A dry run shows exactly which rules would allow or deny, and what would be done.

## Errors and recovery

| Situation | Behaviour |
|---|---|
| Invalid config at start | Refuses to start (exit 2). Fix the file; the supervisor retries with backoff. |
| Invalid config on `SIGHUP` | Keeps the previous config and audits `config_rejected` |
| Second instance | Exits 3; the supervisor stops trying |
| Engine stalls for more than 90 s | `abort`; the supervisor restarts it; startup reconciliation restores expired temporary overrides |
| Adapter error or timeout | The result is `failed` or `timed_out`; a wrong effect is rolled back if possible |
| Value changed by someone else | The lease is dropped and `externally_modified` audited; Core does not re-force it |
| `zfasd` not running | `fas.*` becomes `temporarily_unavailable`, and FAS requests are rejected (capability rule) |
| `SIGTERM` / uninstall | Temporary overrides end and their baselines are restored; state is persisted |

## Performance (measured)

`zcored bench 20000` on the CI-like host, 2026-10-19:
- hardware: Intel Xeon @ 2.80 GHz, 4 vCPU;
- result per full `submit` (policy, arbitration, plan, in-memory execute, read-back, lease,
  audit): **mean 2.1 µs, p50 1.6 µs, p99 3.5 µs**.

This excludes adapter I/O (`dumpsys`, socket round trips), which dominates on a device. Idle
CPU, memory and wakeups on a device have **not** been measured yet.

Design bounds:
- one refresh every 5 s (about 3 short commands and 1 socket call);
- discovery every 60 s;
- no busy loops.
