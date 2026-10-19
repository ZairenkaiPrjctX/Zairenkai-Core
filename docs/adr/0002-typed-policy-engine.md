# ADR-0002: A small typed policy engine instead of Cedar or OPA

**Status:** accepted (2026-10-19)

## Context

Core must authorize privileged operations deterministically, explain each decision, test
policies without a device, and fail closed. See `../architecture/REFERENCE_AUDIT.md`.

## Options

1. **Cedar 4.14.**
   - Pros: good semantics (deny by default, forbid overrides permit), schema validation.
   - Cons:
     - An erroring policy is *skipped*, so a failing forbid can turn into an allow.
     - MSRV 1.89, above our 1.85.
     - About 65 runtime crates, including assembly via `psm`.
     - Its entities are string-typed, while Core's operations are a closed enum.
2. **OPA.** It needs a Go runtime (a second process or cgo), a second toolchain and an IPC
   hop. Too heavy for an always-on daemon on a phone.
3. **Typed Rust engine.** Write the rules as total functions over typed input, keep Cedar's
   decision semantics, and record OPA-style decision logs.

## Decision

Option 3 (`zcore-engine::policy`):
- Compiled `builtin.*` rules cover integrity (validation, authorization) and safety
  (capability, freshness, thermal, battery, charging, recommendations).
- Configured rules can only add a *deny* or *require confirmation*. They cannot allow, and
  they cannot use the `builtin.` prefix.
- Any Deny wins. Otherwise an unconfirmed NeedsConfirmation wins. Otherwise Allow.
- Each decision records every rule outcome and a policy version fingerprint.
- A condition on unknown telemetry *matches*, so restrictions apply when in doubt.

## Consequences

- No evaluation errors exist, so there is nothing to skip.
- Adding an operation means adding an enum variant. The compiler then forces
  `kind`/`resource`/`target`/`direction`/`risk` to be defined for it.
- Policy expressiveness is limited to the `When` conditions. That is acceptable for 1.x;
  revisit Cedar if policies must be authored outside the codebase.
