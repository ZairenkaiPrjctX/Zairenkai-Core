# Target architecture

Core is a **local control plane**. It observes the device and evaluates typed requests
against deterministic policy. It arbitrates ownership of resources, and executes only through
adapters that each own a resource. Every change is verified by reading the value back, and is
audited.

Core is not a root manager, a shell runner or a sysfs writer. It never writes kernel nodes
itself: frequency control belongs to Zairenkai-FAS, and battery saver belongs to Android's
PowerManagerService.

## Layers and where they live

| Layer (prompt §5) | Implementation | Notes |
|---|---|---|
| 1 Core domain | `crates/zcore-domain` | Typed, validated, explicit units. Unknown, unavailable and unsupported states are first-class (`Availability`). One representation per concept, shared with clients. |
| 2 Discovery and capability registry | `zcore-platform::discovery`, adapters' `discover()`, `zcore-engine::registry` | Each capability has a status (`read_write`, `read_only`, `permission_restricted`, `unsupported`, `temporarily_unavailable`, `unknown`), evidence, and `verified_on_device`. |
| 3 Telemetry and context | `zcore-platform::telemetry`, `zcored::engine::refresh_context` | Every value is an `Observation` with source, time and TTL. Refresh is bounded (`telemetry.refresh_ms`); there are no duplicate collectors. |
| 4 Policy and decision | `zcore-engine::policy` | Deny by default. Compiled safety rules, plus configured restrict-only rules. Every evaluated rule is in the decision. |
| 5 Planner and arbitration | `zcore-engine::core` (plan), `zcore-engine::arbitration` (leases) | One lease per resource, tiered priority, baselines kept for rollback. |
| 6 Execution adapters | `zcore-engine::adapter` (contract, fake), `zcore-platform::{fas,power}` | Typed values only; the allowlisted runner has no shell. |
| 7 Runtime and lifecycle | `crates/zcored` | Engine thread, tokio IPC, watchdog, single-instance lock, persisted leases, rotating audit log. The `service.sh` of the zairenkai module (Zairenkai-Modules) adds backoff and a start limit. |

## Decision lifecycle (prompt §7), as implemented in `Core::submit`

1. **Observe:** the engine thread keeps a fresh `DeviceContext` (every `refresh_ms`).
2. **Validate:** protocol parsing (`protocol::parse_request`) and `ActionRequest::validate`.
3. **Interpret:** the owning adapter reads the resource's current value.
4. **Evaluate:** `PolicyEngine::evaluate` returns verdict, tier and every rule outcome.
5. **Arbitrate:** `LeaseTable::check` decides who owns the resource now.
6. **Plan:** `ActionPlan` with preconditions, expected value, read-back verification, timeout,
   rollback (the observed baseline) and risk class. If the current value cannot be read, the
   request is rejected, because the change could be neither verified nor rolled back.
7. **Authorize:** policy plus arbitration. Dry runs stop here and return the plan.
8. **Execute:** `Adapter::apply` (timeouts are enforced inside the adapter).
9. **Verify:** `Adapter::read`. The outcomes are `applied`, `already_applied`, `rejected`,
   `unsupported`, `timed_out`, `unverified`, `rolled_back` and `failed`.
10. **Commit or recover:** grant the lease, or restore the baseline when the effect was
    wrong.
11. **Audit:** `RequestEvaluated` and `ActionCompleted` events with the correlation id.
12. **Learn:** for now, verified capabilities are marked `verified_on_device`. Learning never
    changes permissions.

## Reconciliation (prompt §8)

On every refresh, Core reconciles each lease:
- If the resource no longer holds the leased value, someone else changed it, for example the
  user in the FAS WebUI or Android resetting battery saver at boot. The lease is dropped and an
  `externally_modified` event is audited. Core never re-forces the value.
- If a temporary lease has expired, Core restores its baseline, but only while the resource
  still holds Core's value.
- On shutdown, temporary leases end and restore their baselines. Indefinite user choices
  stay recorded.

Indefinite leases persist across restarts. On reload, their times are rebased onto the new
monotonic clock and then reconciled. A stored lease is never re-executed.

## Ecosystem contracts

See `../MODULES.md` for what exists per component. In summary:
- **FAS:** adapter over its socket, owning `fas.profile` and `fas.enabled`.
- **Database-Source:** read-only lookup, data only.
- **Intelligence Engine:** may only call `recommend`. A recommendation is arbitrated at the
  lowest tier, time-bounded, and gated on confidence.
- **SDK and App:** the protocol in `../api/PROTOCOL.md` is the contract.
- **License Server, ThermalTools, Graphics, ART, Modules:** no code exists yet. Their contracts
  are documented and they are reported as `not_installed`.
