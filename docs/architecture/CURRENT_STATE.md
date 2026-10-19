# Current state audit (2026-10-19)

## Before this work

`ZairenkaiPrjctX/Zairenkai-Core` held only the agent foundation:
- files: `AGENTS.md`, `CLAUDE.md`, `AGENT/`, `.claude/`, `.gitignore`;
- branches: `dev` (default) and `ccr-63f8aa87-1b0paj`, both at `5ebcedf`;
- no source code, build system, CI, public interface or compatibility obligation.

Evidence: `git ls-files` and `git log --all`.

## The ecosystem Core coordinates (inspected 2026-10-19)

| Repository | Contents on `dev` | Relevance to Core |
|---|---|---|
| Zairenkai-FAS | Rust workspace: `zfasd` daemon, Magisk/KernelSU module, root-only control socket (`status`/`enable`/`disable`/`profile`), `state.json`. Merged in PR #1. | Integrated: Core's `fas` adapter speaks its socket protocol. |
| Zairenkai-Database-Source | AFID (Python): firmware metadata catalog. JSON records in `database/devices/<brand>/<codename>/device.json` (schema `device.schema.json`, `schema_version = "1.0"`). | Integrated read-only: device knowledge lookup, treated as data. The committed catalog has no verified devices yet. |
| Zairenkai-ThermalTools, -SDK, -License-Server, -Intelligence-Engine, -Graphics-Platform, -ART, -Modules, -App | Agent foundation only (`git ls-tree origin/dev`) | No code to integrate. Core defines the contracts (protocol, module manifest, recommendation path) and reports these components as `not_installed`. |

## Gaps found and changes made outside this repository

The FAS status snapshot had no master switch, and only reported the per-app effective profile.
Without those, Core could not verify `enable`, `disable` or `profile` by reading the value back.

A small additive FAS change adds `enabled` and `default_profile` to the snapshot. It is
committed on the FAS topic branch (commit `9d43894`). Until it is merged and deployed, Core
reports FAS control as `read_only`.

## Constraints that shaped the design

- Android with root (Magisk / KernelSU / APatch), arm64, API 29 and newer: the same target as FAS.
  The NDK r27d cross-build pipeline is already proven there.
- No physical device in this environment. Everything device-facing is validated against
  fixtures and fakes only (see `../STATUS.md`).
- `/data/adb` is reachable by root only. App processes cannot reach a socket there, which
  shapes the IPC trust model (ADR-0005).
