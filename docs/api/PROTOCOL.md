# Core IPC protocol, version 1

- **Transport:** Unix stream socket `/data/adb/zairenkai/core/zcored.sock` (mode 0600, root
  only; see ADR-0005).
- **Exchange:** one request line per connection, one response line back.
- **Canonical schema:** `crates/zcore-domain/src/protocol.rs`.

## Request

```json
{"v": 1, "id": "<correlation id>", "method": "<method>", "params": { ... }}
```

| Field | Rules |
|---|---|
| `v` | Must be `1`. Any other value is answered `unsupported_version`. |
| `id` | 1–64 characters from `[A-Za-z0-9_-]`. Echoed in the response and recorded in the audit. Reusing an id for `submit`, `recommend` or `release` is rejected; Core remembers the last 256 per principal. |
| `method` / `params` | A closed set (below). Unknown methods, unknown fields or extra top-level keys are answered `bad_request`. |
| size | At most 16 KiB, delivered within 5 s |

The caller's identity is the socket peer uid (`SO_PEERCRED`), mapped by configuration:
- uid 0 → `root`;
- a configured uid → `app` or `intelligence_engine`;
- any other uid → refused with `permission_denied`.

## Methods

| Method | Params | Result |
|---|---|---|
| `health` | – | `state` (`ready` or `degraded`), `reasons`, `version`, `core_api`, `protocol`, `policy_version`, `uptime_ms`, `context_age_ms`, `leases`, `audit_dropped` |
| `capabilities` | – | `CapabilityDescriptor[]`: `id`, `status`, `owner`, `evidence`, `verified_on_device`, `checked_at_ms` |
| `context` | – | `{context: DeviceContext, knowledge: Lookup}`. Every field is an `Observation` (see below). |
| `modules` | – | `ModuleDescriptor[]`: installed Zairenkai modules, plus first-party components reported as `not_installed` |
| `tuning` | – | The Zairenkai-Tuning telemetry document (`zairenkai.tuning` schema 1, `kind: "telemetry"`), as `ztune telemetry --interval-ms 200` printed it after Core checked its schema, version, kind and component. Fields: Zairenkai-Tuning `docs/CONTRACT.md`. `not_found` when `ztune` is missing or fails |
| `leases` | – | `ResourceLease[]` |
| `policy` | – | `version`, `safety`, `root_grants`, `principals`, `rules` |
| `audit` | `{limit: 1..=500}` (default 50) | `AuditEvent[]`, oldest first |
| `submit` | `{operation, duration_ms?, dry_run?, confirmed?, reason?}` | `ActionResult` |
| `recommend` | `{operation, confidence, duration_ms?, reason?}` (Intelligence Engine only) | `ActionResult` |
| `release` | `{lease_id}` | `ActionResult`. The baseline is restored only if the resource still holds Core's value. |

An `Observation` has these fields:
- `value.status`: one of `value`, `unavailable`, `unsupported`, `permission_denied`, `unknown`;
- `provenance`;
- `source`;
- `observed_at_ms`;
- `ttl_ms`.

## Operations

```json
{"op": "fas_set_profile", "profile": "power_saver|balanced|performance|fast_response|adaptive"}
{"op": "fas_set_enabled", "enabled": true}
{"op": "set_low_power_mode", "enabled": true}
{"op": "tuning_set_io_scheduler", "scheduler": "mq-deadline|kyber|bfq|none|cfq|deadline|noop"}
{"op": "tuning_set_gpu_governor", "governor": "msm-adreno-tz|simple_ondemand|performance|powersave"}
```

The tuning operations are carried out by Zairenkai-Tuning's `ztune` (a separate GPL-3.0
program in the module); the device must also advertise the value, which the capability
evidence lists. `tuning_set_gpu_governor` to a higher-demand governor is a raising action
(thermal, battery and staleness rules apply); `performance` is risk `high`. The I/O scheduler
is neutral.

There is no generic or free-form operation.

## ActionResult

| Field | Content |
|---|---|
| `request_id` | The correlation id |
| `outcome` | See the table below |
| `decision` | Verdict, tier, every rule with its explanation, policy version |
| `plan` | Preconditions, current, expected, rollback, risk, timeout, `no_op` |
| `verification` | Expected, observed, matched, detail |
| `lease_id` | The lease that now holds the resource, if any |
| `message` | Human-readable summary |

| `outcome` | Meaning |
|---|---|
| `applied` | Executed and verified by read-back |
| `already_applied` | The resource already held the value; ownership was recorded |
| `dry_run` | Evaluated and planned only |
| `rejected` | Policy, arbitration or a precondition refused it; nothing was executed |
| `unsupported` | The capability does not exist on this device |
| `timed_out` | The adapter did not answer, and the value is not confirmed |
| `unverified` | Executed, but the read-back did not confirm it, and the baseline could not be restored (or there was none) |
| `rolled_back` | Executed with a wrong effect; the previous value was restored |
| `failed` | The adapter reported an error |

## Errors

```json
{"v": 1, "id": "...", "ok": false, "error": {"code": "...", "message": "..."}}
```

`code` is one of `bad_request`, `unsupported_version`, `too_large`, `rate_limited`, `busy`,
`permission_denied`, `not_found`, `timeout`, `shutting_down`, `internal`.

- After `timeout`, the operation may still complete. Check `audit`.
- A policy denial is **not** an error. The method returns `outcome: "rejected"` with the
  reasons.

## Compatibility

- Adding a method, an optional parameter or a result field does not change the version.
  Clients must ignore unknown result fields.
- Removing or renaming anything requires `v: 2`. Core will then answer both versions for at
  least one minor release.

## Example

```sh
zcored ctl submit '{"operation":{"op":"fas_set_profile","profile":"performance"},"duration_ms":600000,"reason":"benchmark run"}'
```
