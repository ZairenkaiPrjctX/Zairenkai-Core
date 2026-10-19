# ADR-0003: One engine thread for all device I/O; tokio only for IPC

**Status:** accepted (2026-10-19)

## Decision

- **Engine thread.** `zcored` runs `Core` and every adapter on a single dedicated thread
  (`engine.rs`). Requests arrive through a bounded `sync_channel(ipc.queue)`; when it is full,
  the client is answered `busy`.
- **IPC.** A tokio `current_thread` runtime handles the socket, timeouts and signals. Each
  reply comes back through a `oneshot`, with `ipc.request_timeout_ms`.

## Why

- Operations on a resource must never interleave. Serializing them avoids locks in the
  pipeline and makes the audit order the real order.
- Adapters do blocking I/O (`dumpsys`, the FAS socket) with their own timeouts. Running that
  I/O on the async runtime would stall IPC.
- The engine thread also runs the periodic refresh, discovery and reconcile, so no extra timer
  threads are needed.

## Consequences

- One slow adapter delays other requests. This is bounded by adapter timeouts, the request
  timeout, and the 90 s watchdog that aborts for a supervised restart.
- tokio 1.53.3 (MIT, MSRV 1.85) is the only new dependency beyond serde/toml/libc.
