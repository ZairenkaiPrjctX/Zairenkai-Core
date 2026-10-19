# ADR-0005: The socket is root-only in 1.x; app access needs a validated transport

**Status:** accepted (2026-10-19)

## Context

The socket lives under `/data/adb` (0700). On Android, app processes cannot traverse that
directory, and SELinux restricts `connectto` from app domains. Making the socket world-writable
would not make it reachable, and would weaken the boundary.

## Decision

- 1.x serves root clients: root shells, the root manager's module UI through `ksu.exec`, and
  other root daemons such as service scripts.
- The authorization model for app uids already exists and is tested: `principals` maps uids to
  `app` or `intelligence_engine` with explicit grants.
- A transport for apps is future work. It needs a threat review and on-device SELinux
  validation. Candidates: an abstract socket with a policy, or a Binder service.

## Consequences

- The SDK contract (`../api/PROTOCOL.md`) is transport-independent JSON, so it carries over.
