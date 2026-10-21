# CLAUDE.md: Zairenkai-Core

Instructions for Claude Code in `ZairenkaiPrjctX/Zairenkai-Core`. Keep this file under 200 lines;
put detail in `AGENT/`.

@AGENTS.md
@AGENT/PROJECT_CONTEXT.md

## Current state (read first)

This repository is **Zairenkai-Core**, a Rust workspace:
- `crates/zcore-domain`: contracts and protocol;
- `crates/zcore-engine`: decisions, no I/O;
- `crates/zcore-platform`: discovery and adapters;
- `crates/zcored`: the daemon.

It also contains a Magisk/KernelSU module (`module/`). Architecture is described in
`docs/architecture/TARGET_ARCHITECTURE.md`. Nothing has run on a physical device yet
(`docs/STATUS.md`).

Non-negotiables in this repo:
- No free-form command operation, no shell. New actions are new typed `Operation` variants
  with an owning adapter.
- Core writes no `/sys`, `/proc` or property nodes. Resources belong to their component (FAS
  owns cpufreq).
- Policy: deny by default. Configured rules may only restrict. Safety rules are compiled in.
- Every change is verified by read-back. A value changed externally is never re-forced.
- Recommendations (Intelligence Engine) never self-authorize and never outrank a user choice.
- Tests use `SysRoot` fixtures, fake runners and fake adapters; never the host's `/sys`, `/proc`
  or Android services.
- Do not copy code from the references in `docs/architecture/REFERENCE_AUDIT.md`.

## Commands

| Purpose | Command | Status |
|---|---|---|
| Build | `cargo build --workspace` | Verified. Evidence: 2026-10-19 |
| Test | `cargo test --workspace` | Verified. Evidence: 65 passed, 2026-10-21 |
| Lint / format | `cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check` | Verified. Evidence: clean, 2026-10-19 |
| MSRV | `cargo +1.85.0 test --workspace` | Verified. Evidence: passed, 2026-10-21 |
| Shell lint | `shellcheck -s sh module/*.sh module/META-INF/com/google/android/update-binary && shellcheck scripts/*.sh` | Verified. Evidence: clean (0.10.0), 2026-10-19 |
| Android module | `ANDROID_NDK_HOME=<ndk> scripts/build-android.sh` | Verified. Evidence: NDK r27d, 2026-10-19 |
| Benchmark | `cargo run --release -p zcored -- bench 20000` | Verified. Evidence: mean 2.1 µs/submit on host, 2026-10-19 |
| Validate agent foundation | `python3 <path-to-.github>/agent-foundation/scripts/validate_agent_setup.py .` | Configured |

Only list commands that exist in this repository. Never invent one.

## Working rules

- Inspect the relevant code before editing it. Preserve unrelated and uncommitted changes.
- Generated files are edited at their source, then regenerated. Managed foundation files
  (listed in `AGENT/.foundation-manifest.json`) are edited in `ZairenkaiPrjctX/.github`, not here.
- Destructive or outward-facing actions (deleting data, force-push, history rewrites,
  publishing releases, deploying, flashing devices) need explicit human approval each time.
- Do not claim a build, test, or signature succeeded without showing the output.

## Git

- Work on a topic branch; open **draft** pull requests; never merge or approve.
- Commits must be cryptographically signed. If signing fails, stop and report the error.
  Trailer format and verification: `AGENT/policies/git-and-commit-signing.md`.

## Where things are

- `AGENT/README.md`: index of the agent library (prompts, policies, playbooks, templates).
- `.claude/skills/`: run `/repo-audit`, `/implement-feature`, `/bug-fix`, `/test-and-verify`,
  `/security-review`, `/release-readiness`.
- `.claude/rules/`: path-scoped rules, loaded automatically.
