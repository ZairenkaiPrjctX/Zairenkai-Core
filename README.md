# Zairenkai-Core

The local control plane of the Zairenkai ecosystem. Core observes the device, discovers what
can genuinely be controlled, evaluates typed requests against deterministic policy, and
arbitrates ownership of resources. It changes things only through adapters, verifies each
change by reading it back, and audits every decision.

```
observe → validate → evaluate → arbitrate → plan → execute → verify → commit/rollback → audit
                       ↑ deny by default                    ↑ read-back
reconcile: leases changed by someone else are dropped (never re-forced); expired ones restore baselines
```

> **Status 0.1.0:** implemented and tested on the host with fixtures and fakes (57 tests),
> cross-built for arm64. **Not yet run on a physical device.** See `docs/STATUS.md`.

## What Core controls today

| Resource | Owner adapter | How |
|---|---|---|
| `fas.profile`, `fas.enabled` | `fas` | the Zairenkai-FAS control socket (`profile`/`enable`/`disable`), verified by `status` |
| `android.low_power_mode` | `android_power` | `cmd power set-mode 0\|1`, verified by `settings get global low_power` |

Core never writes `/sys`, `/proc` or system properties, never runs a shell, and has no
free-form command operation.

## Quick start

```sh
cargo test --workspace
ANDROID_NDK_HOME=/path/to/android-ndk-r27d scripts/build-android.sh   # dist/zairenkai-core-v*.zip

# on a rooted device, after installing the module:
su -c /data/adb/modules/zairenkai_core/bin/zcored discover          # read-only report
su -c /data/adb/modules/zairenkai_core/bin/zcored ctl health
su -c '/data/adb/modules/zairenkai_core/bin/zcored ctl submit "{\"operation\":{\"op\":\"fas_set_profile\",\"profile\":\"performance\"},\"dry_run\":true}"'
```

## Documentation

- Architecture: [current state](docs/architecture/CURRENT_STATE.md), [target architecture](docs/architecture/TARGET_ARCHITECTURE.md), [reference audit](docs/architecture/REFERENCE_AUDIT.md), [ADRs](docs/adr/)
- [IPC protocol](docs/api/PROTOCOL.md), [policy and arbitration](docs/POLICY.md), [ecosystem integration and module contract](docs/MODULES.md)
- [Threat model](docs/security/THREAT_MODEL.md), [operations and diagnostics](docs/operations/OPERATIONS.md)
- [Implementation status and limitations](docs/STATUS.md), [third-party components](docs/THIRD_PARTY.md)

## Licence

Not chosen yet (see `docs/THIRD_PARTY.md`).
