# Policy and arbitration

The implementation is in `crates/zcore-engine/src/policy.rs` and `arbitration.rs`, and the
tests sit next to it.

## Rules (evaluated in this order; all outcomes are recorded)

| Rule | Tier | Denies when |
|---|---|---|
| `builtin.request_valid` | integrity | The request is malformed |
| `builtin.authorized` | integrity | The principal has no grant for the operation kind. Also when the origin does not match the principal: only the Intelligence Engine may recommend, it may *only* recommend, only Core uses the `core` origin, and only users may confirm. |
| `builtin.capability` | safety | The resource's capability is not `read_write` here |
| `builtin.context_fresh` | safety | A raising action arrives while the device context is older than `max_context_age_ms` |
| `builtin.thermal` | safety | A raising action arrives at thermal status SEVERE or hotter, or the status is unknown or stale while `safety.require_thermal_status` is set (the default) |
| `builtin.battery_critical` | safety | A raising action arrives at battery ≤ `battery_critical_percent` while not charging |
| `builtin.battery_saver_while_charging` | safety | The battery saver is changed while charging (Android ignores it) |
| `builtin.recommendation` | adaptive | Confidence is below `recommendation_min_confidence` |
| configured `[[rules]]` | thermal_power | Their `when` matches. The effect is `deny` or `require_confirmation`. |

"Raising" means the operation increases performance demand compared with the current value:
- a profile with a higher intensity;
- enabling FAS;
- disabling battery saver.

Lowering actions stay possible when hot, stale or low on battery.

The verdict is computed as follows:
- any `deny` → **deny**;
- otherwise an unconfirmed `require_confirmation` → **needs confirmation** (resubmit with
  `confirmed: true`, as a user principal);
- otherwise → **allow**.

## Grants

| Principal | Grants |
|---|---|
| `root` | `root_grants` (default: every operation) |
| `app` | its `grants` |
| `intelligence_engine` | its `grants` (only through `recommend`) |
| `core` | everything (rollback, expiry) |
| unknown uid | nothing, and is refused before any method runs |

## Arbitration (leases)

There is one lease per resource. Tiers, highest first:
safety > integrity > user > thermal_power > performance > adaptive.

- **Same owner:** the lease is renewed.
- **Higher tier:** it preempts the current lease.
- **Equal or lower tier:** the request is rejected with the holder named. There is never a
  silent last-writer-wins.
- **Baseline:** the value before the first lease is kept across stacked leases, and restored
  when the last temporary lease expires.
- **External change:** the lease is dropped and nothing is re-forced.

## Example: confirm raising actions on battery

```toml
[[rules]]
id = "confirm-raise-below-30"
effect = "require_confirmation"
when = { direction = "raise", battery_below = 30, charging = false }
explanation = "raising performance on battery below 30%"
```
