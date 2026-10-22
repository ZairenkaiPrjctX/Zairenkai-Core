# Project context: Zairenkai-Core

> Owned by this repository. Update it whenever verified facts change. Label every statement
> **Verified** (with `Evidence:`), **Unverified**, or **Unknown / Not Configured**.

## Identity

| Field | Value | Status |
|---|---|---|
| Repository | `ZairenkaiPrjctX/Zairenkai-Core` | Verified. Evidence: GitHub API |
| GitHub description | Core Zairenkai | Verified. Evidence: GitHub API |
| Visibility | public | Verified. Evidence: `list_repos`, 2026-10-22 (was private until made public; earlier entries said private) |
| Default branch | `dev` | Verified. Evidence: `git branch -a`, 2026-10-19 |
| Purpose | Local control plane: capability discovery, policy decisions, arbitration, verified execution through adapters, audit | Verified. Evidence: `crates/`, `docs/architecture/TARGET_ARCHITECTURE.md` |
| Licence | Apache-2.0 (`LICENSE`, `NOTICE`, Cargo `license`, SPDX headers) | Verified. Evidence: maintainer decision 2026-10-21; `cargo metadata` reports Apache-2.0 for all crates |

## Contents

| Area | Location | Status |
|---|---|---|
| Rust workspace (MSRV 1.85) | `Cargo.toml`, `Cargo.lock` | Verified. Evidence: `cargo +1.85.0 test --workspace` passed 2026-10-19 |
| Domain types and protocol v1 | `crates/zcore-domain` | Verified. Evidence: 9 tests |
| Decision engine (no I/O) | `crates/zcore-engine` | Verified. Evidence: 14 unit + 16 pipeline/fault-injection tests |
| Discovery, telemetry, adapters (FAS, battery saver), AFID lookup, module inventory | `crates/zcore-platform` | Verified on fixtures. Evidence: 14 tests. **Unverified** on a device. |
| Daemon and CLI | `crates/zcored` | Verified on the host. Evidence: 4 tests including an end-to-end socket test. **Unverified** on a device. |
| Android component | `scripts/build-android.sh`, `.cargo/config.toml` | Emits `dist/zairenkai-core/` (`bin/zcored`, licences, `component.json`); the root module is assembled in Zairenkai-Modules (maintainer decision 2026-10-22). The 16 KB LOAD-segment check is unchanged. **Unverified** with the NDK since this change |
| CI | `.github/workflows/ci.yml` | Verified on GitHub. Evidence: Actions run 39528674638 (workflow_dispatch, 3562c7c), all 4 jobs green, 2026-10-21 |
| Docs | `README.md`, `docs/` | Verified. Evidence: files present |

## Key facts

- Core writes no kernel nodes and runs no shell. Its only actions are the FAS socket commands,
  `cmd power set-mode` and `ztune apply` (Zairenkai-Tuning writes the scheduler/governor node,
  not Core).
  Evidence: `crates/zcore-platform/src/{fas,power,exec}.rs`.
- The FAS adapter needs the `enabled` and `default_profile` status fields, added on the FAS topic
  branch (commit `9d43894`); that change is not merged yet. **Unverified** against a deployed
  zfasd.
- The socket is root-only. App access needs a validated transport (ADR-0005).

## Authoritative sources

| Topic | Source |
|---|---|
| Repository facts | this file |
| Architecture | `AGENT/ARCHITECTURE.md` → `docs/architecture/` |
| Protocol | `docs/api/PROTOCOL.md`, `crates/zcore-domain/src/protocol.rs` |
| Policy | `docs/POLICY.md` |
| Security | `docs/security/THREAT_MODEL.md` |
| Status and limitations | `docs/STATUS.md` |
| Organization standards | `ZairenkaiPrjctX/.github` → `agent-foundation/standards/` |

## Related repositories

- `Zairenkai-FAS`: integrated through its control socket. Verified. Evidence: `crates/zcore-platform/src/fas.rs`, against a fake.
- `Zairenkai-Database-Source`: device record lookup, read-only. Verified. Evidence: `crates/zcore-platform/src/afid.rs`, against a fixture.
- `Zairenkai-ART`: Android Runtime. A read-only inspector (`zart`) exists on its topic branch
  `ccr-63f8aa87-1b0paj`, not merged; Core has no ART adapter. Verified. Evidence: ART
  `crates/zart`, 2026-10-19.
- `Zairenkai-Tuning` (GPL-3.0): `tuning` adapter runs `ztune capabilities|telemetry|apply` and
  owns `tuning.io_scheduler`, `tuning.gpu_governor` (read-only unless ztune reports an
  exact-device write authorization); separate process, no shared code.
  Verified. Evidence: `crates/zcore-platform/src/tuning.rs` tests against captured documents,
  pipeline test `tuning_operations_go_through_policy_and_read_back`, 2026-10-22. **Unverified**
  on a device.
- `Zairenkai-ThermalTools`: read-only diagnostics through `zthermal snapshot` (schema 1),
  from a root-owned binary configured in `paths.thermaltools_binary`.
  Verified. Evidence: `crates/zcore-platform/src/thermaltools.rs` tests (fakes, a generated zthermal 0.2.0 snapshot) and one host run of the real binary, 2026-10-21.
  **Unverified** on a device.
- `Zairenkai-Modules` packages zcored, zfasd and ztune into the single `zairenkai` module, and
  `Zairenkai-App` reaches Core through `zcored ctl` over root. Verified. Evidence: Modules
  `components.lock.json`, App `integration-core/.../core/Core.kt`, 2026-10-22.
- SDK, License-Server, Intelligence-Engine and Graphics-Platform contained only agent foundation
  files on 2026-10-22. Evidence: `git ls-tree origin/dev`. Their contracts are in
  `docs/MODULES.md`.

## Open questions

- Should releases ship a generated third-party licence bundle (`docs/THIRD_PARTY.md`)?
- Which transport should serve app clients: an abstract socket with SELinux policy, or Binder?
- The License Server's entitlement format and offline policy.
- Which devices to use for the first on-device validation.
