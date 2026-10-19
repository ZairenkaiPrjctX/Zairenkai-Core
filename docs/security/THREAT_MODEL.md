# Threat model

## Assets

- **Device stability and safety:** thermal protection, battery, user settings.
- **Integrity of Core's decisions:** policy, leases, configuration.
- **Audit trail:** it must not be lost silently, and must not contain secrets.
- **Privacy:** no personal data or app contents are collected.

## Trust boundaries

| Boundary | Trusted side | Untrusted input | Controls (code) |
|---|---|---|---|
| Local socket `/data/adb/zairenkai-core/zcored.sock` | `zcored` (root) | Any local process | Directory 0700 and socket 0600. Identity comes from `SO_PEERCRED` and is never claimed in the message (`server.rs`). Unknown uids are denied (`engine.rs`). Requests are capped at 16 KiB, read within 5 s, rate-limited per uid, at most 32 connections, through a bounded queue (`protocol.rs`, `ratelimit.rs`). |
| Request content | Policy engine | JSON from clients | Closed method enum with `deny_unknown_fields`, typed operations (no free-form command), id and reason validation, duplicate-id rejection (`protocol.rs`, `request.rs`, `engine.rs::duplicate`). |
| Intelligence Engine | Core policy | Recommendations, which may be model output | Only `recommend` is accepted, at the lowest arbitration tier, with a confidence gate and a forced expiry. The engine cannot confirm or submit as a user, and model text is never executed (`policy.rs`, `core.rs`). |
| Configuration file | `zcored` | A root-owned file that may be corrupt or hostile | Strict schema with no unknown keys. It can grant operations but cannot weaken the compiled safety rules. System uids are rejected in `principals`. Path overrides must be absolute without `..`. An invalid file at start means refusal to start; on reload the old config is kept (`config.rs`). |
| Device knowledge (AFID) | Policy | Catalog JSON | Read as data only. Only verified records of schema 1.0 are used, and record identity must match its path. Size cap, no symlinks. A record never grants anything (`afid.rs`). |
| Installed modules | Core | `/data/adb/modules/*` | Only `module.prop` and the optional manifest (64 KiB cap, regular files) are read. Nothing is executed or loaded. A manifest declares; it never grants (`modules.rs`, `module.rs`). |
| Adapters and Android tools | Core | `dumpsys` / `settings` / `cmd` output and the FAS socket | Executable allowlist (`getprop`, `dumpsys`, `cmd`, `settings`, `service`). Arguments limited to `[A-Za-z0-9._-/:=@]`. Cleared environment. Timeouts and output caps. Defensive parsing. Results verified by read-back (`exec.rs`, `core.rs`). |
| Persisted state | `zcored` | `leases.json`, which may be corrupt | A corrupt file is moved aside and reported. Stored leases are only reconciled, never re-executed (`storage.rs`, `core.rs::load_leases`). |

## Threats and mitigations

| Threat | Mitigation | Residual risk |
|---|---|---|
| Unprivileged app drives privileged changes | Socket unreachable (0700/0600 under `/data/adb`); uid-based principals; default grants for root only | If a root process is compromised, it can use the socket. That is out of scope: root is already total. |
| Command injection through parameters | No shell; allowlist and argument validation; typed values only (`0`/`1`, enum profile names) | None known |
| Recommendation abuse (a prompt-injected AI) | Typed operation, confidence gate, lowest tier, forced expiry, no confirmation rights | A compromised engine with a grant can still request granted operations within policy. Mitigation: grants are opt-in per operation. |
| Overriding thermal protection | Builtin rule: no raising action at SEVERE or hotter, or with unknown or stale thermal status. Core writes no thermal or kernel node. | It depends on Android's thermal status being accurate. |
| Fighting other owners (OEM, user, Android) | External changes drop the lease; nothing is re-forced | None known |
| Resource exhaustion and DoS | Size, time, rate and connection caps; bounded queue answered `busy`; bounded audit buffer that counts drops | A flood from root can still fill the log rotation (1 MiB × 2). |
| Replay of a captured request | Duplicate ids rejected per principal (last 256). Peer credentials are re-checked on every connection. | Ids older than the window are accepted again. Acceptable: the transport is local, and replaying requires the same uid. |
| Stall or crash with temporary overrides active | Engine watchdog aborts; the supervisor restarts; startup reconciliation restores expired baselines | Overrides stay in place until the restart (up to 90 s + backoff) |
| Secret leakage | No secrets are handled in this release. Audit holds principals, operations, outcomes and reasons (≤ 200 chars, no control characters). Logs go to stderr → `zcored.log` (rotated). | A `reason` text could carry user-entered data. Clients should not put personal data there. |

## Unresolved

- **App-uid access.** On Android, app processes cannot reach `/data/adb`. A future
  Zairenkai-App needs a validated transport: an abstract socket with an SELinux policy, or
  Binder. The authorization model for apps is already implemented and tested.
- **License Server.** There is no entitlement verification yet, because the server does not
  exist. No feature is licence-gated, so nothing fails open or closed today. See
  `../MODULES.md` for the required design.
- **Module signature verification** is not implemented. Modules are never executed or loaded,
  so this only matters once modules can own resources.
- **On-device testing.** Behaviour on real devices, and SELinux denials for `zcored` under each
  root manager, have not been tested.
