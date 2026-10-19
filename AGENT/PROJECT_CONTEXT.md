# Project context: Zairenkai-Core

> Owned by this repository. Update it whenever verified facts change. Label every statement
> **Verified** (with `Evidence:`), **Unverified**, or **Unknown / Not Configured**.

## Identity

| Field | Value | Status |
|---|---|---|
| Repository | `ZairenkaiPrjctX/Zairenkai-Core` | Verified. Evidence: GitHub API |
| GitHub description | Core Zairenkai | Verified. Evidence: GitHub API |
| Visibility | private | Verified. Evidence: GitHub API |
| Default branch | `dev` | Verified. Evidence: `git branch -a`, 2026-10-19 |
| Purpose | Local control plane: capability discovery, policy decisions, arbitration, verified execution through adapters, audit | Verified. Evidence: `crates/`, `docs/architecture/TARGET_ARCHITECTURE.md` |
| Licence | none chosen yet | Verified. Evidence: no `LICENSE` file |

## Contents

| Area | Location | Status |
|---|---|---|
| Rust workspace (MSRV 1.85) | `Cargo.toml`, `Cargo.lock` | Verified. Evidence: `cargo +1.85.0 test --workspace` passed 2026-10-19 |
| Domain types and protocol v1 | `crates/zcore-domain` | Verified. Evidence: 9 tests |
| Decision engine (no I/O) | `crates/zcore-engine` | Verified. Evidence: 14 unit + 16 pipeline/fault-injection tests |
| Discovery, telemetry, adapters (FAS, battery saver), AFID lookup, module inventory | `crates/zcore-platform` | Verified on fixtures. Evidence: 14 tests. **Unverified** on a device. |
| Daemon and CLI | `crates/zcored` | Verified on the host. Evidence: 4 tests including an end-to-end socket test. **Unverified** on a device. |
| Module packaging | `module/`, `scripts/build-android.sh`, `.cargo/config.toml` | Verified. Evidence: shellcheck clean; NDK r27d build with 16 KB-aligned LOAD segments (checked by the script), 2026-10-19 |
| CI | `.github/workflows/ci.yml` | Verified locally. Evidence: actionlint clean 2026-10-19. **Unverified** on GitHub. |
| Docs | `README.md`, `docs/` | Verified. Evidence: files present |

## Key facts

- Core writes no kernel nodes and runs no shell. Its only actions are the FAS socket commands
  and `cmd power set-mode`.
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
- ThermalTools, SDK, License-Server, Intelligence-Engine, Graphics-Platform, Modules and App
  contained only agent foundation files on 2026-10-19. Evidence: `git ls-tree origin/dev`.
  Their contracts are in `docs/MODULES.md`.

## Open questions

- Licence choice.
- Which transport should serve app clients: an abstract socket with SELinux policy, or Binder?
- The License Server's entitlement format and offline policy.
- Which devices to use for the first on-device validation.
