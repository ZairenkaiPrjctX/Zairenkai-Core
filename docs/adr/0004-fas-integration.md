# ADR-0004: Coordinate FAS through its control socket; never write its nodes

**Status:** accepted (2026-10-19)

## Decision

Core owns two resources:
- `fas.profile`, the FAS default profile;
- `fas.enabled`, the FAS master switch.

It changes them only through `zfasd`'s root socket (`profile`, `enable`, `disable`) and
verifies them through `status` (`enabled`, `default_profile`). FAS remains the sole owner of
the cpufreq floors. Core never touches cpufreq, governors or thermal nodes, which prevents two
frequency controllers competing.

## Consequences

- This needed a small additive FAS change (status fields), made on the FAS topic branch.
  Older `zfasd` builds are reported as `read_only` with that reason.
- Per-app FAS overrides remain FAS configuration. Core does not model them in 1.x.
