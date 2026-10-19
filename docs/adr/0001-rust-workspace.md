# ADR-0001: Rust workspace with four crates

**Status:** accepted (2026-10-19)

## Context

The repository was empty, so there was no existing language or interface to preserve. The
only Zairenkai component with running code is Zairenkai-FAS: Rust, MSRV 1.85, cross-built to
arm64 Android with NDK r27d and packaged as a Magisk/KernelSU module. Core runs as a root
daemon on the same devices.

## Decision

Use a Rust workspace with the same toolchain, MSRV and packaging as FAS. It has four crates,
each with a reason to exist:
- `zcore-domain`: contracts shared with clients. No I/O.
- `zcore-engine`: deterministic decisions. No I/O, so all of it can be tested on a host.
- `zcore-platform`: everything device-specific, behind `SysRoot` and the `Runner` trait.
- `zcored`: process lifecycle and IPC.

A finer split (separate policy, planner and arbitration crates) was rejected. Those parts
share one data model and change together.

## Consequences

- One build and CI pipeline across Core and FAS.
- `sysroot.rs`, `fsutil.rs` and `testutil.rs` are duplicated from FAS (same owner) and marked
  as such. A shared crate is worth it once a third component needs them.
