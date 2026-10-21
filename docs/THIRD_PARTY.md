# Third-party components and provenance

## Project licence

**Apache License 2.0** (`Apache-2.0`), chosen by the maintainer on 2026-10-21 for this
repository and for Zairenkai-ThermalTools.
- `LICENSE` is the official text from https://www.apache.org/licenses/LICENSE-2.0.txt,
  byte-identical to Zairenkai-Database-Source's `LICENSE`.
- `NOTICE` names the copyright holder "ZairenkaiPrjctX contributors", the convention already
  used by Zairenkai-Database-Source. The year 2026 is the year of the first commit.
- Cargo metadata: `license = "Apache-2.0"` for every crate.
- Every `.rs` and shell file starts with `SPDX-License-Identifier: Apache-2.0` and the
  copyright line.
- The module zip built by `scripts/build-android.sh` contains `LICENSE`, `NOTICE` and this
  file.

Every commit is the maintainer's work. Agent-made commits carry the maintainer's
`Signed-off-by`, and no third-party code is vendored.

Open item for releases: the MIT-licensed crates are statically linked. Their licences ask that
the copyright notice travel with copies, so a release should add a generated licence bundle
(for example with `cargo about`). Not done yet.

## Rust dependencies (`Cargo.lock`, built with `--locked`)

| Crate | Version | Licence |
|---|---|---|
| tokio, tokio-macros | 1.53.3 / 2.7.2 | MIT |
| mio | 1.2.4 | MIT |
| socket2 | 0.6.5 | MIT OR Apache-2.0 |
| bytes | 1.12.1 | MIT |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT |
| signal-hook-registry | 1.4.8 | MIT OR Apache-2.0 |
| serde, serde_core, serde_derive, serde_json | 1.0.x | MIT OR Apache-2.0 |
| toml and its parser crates, winnow, indexmap, hashbrown, equivalent | 1.x | MIT OR Apache-2.0 (winnow: MIT) |
| libc | 0.2.191 | MIT OR Apache-2.0 |
| errno | 0.3.14 | MIT OR Apache-2.0 |
| itoa, zmij, memchr | – | MIT OR Apache-2.0 / MIT / Unlicense OR MIT |
| proc-macro2, quote, syn, unicode-ident | build time | MIT OR Apache-2.0 (unicode-ident also Unicode-3.0) |

Regenerate with `cargo metadata --format-version 1 --filter-platform aarch64-linux-android`.

## Code shared with Zairenkai-FAS

`crates/zcore-platform/src/{sysroot,fsutil,testutil}.rs` come from Zairenkai-FAS
(`crates/zfas-platform/src`), which has the same owner and no licence of its own yet. The
copies here are under Apache-2.0. Licensing FAS is a separate decision.

## References studied, not copied

See `architecture/REFERENCE_AUDIT.md`. No code was copied from KernelSU, KernelSU-Next, AOSP,
Cedar, OPA, the OpenTelemetry Collector, systemd or Linux.

`module/META-INF/com/google/android/update-binary` is an independently written stub. It calls
Magisk's documented installer interface (`util_functions.sh`, `install_module`).
