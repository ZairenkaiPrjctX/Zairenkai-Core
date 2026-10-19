# Third-party components and provenance

## Project licence

None has been chosen yet: there is no `LICENSE` file, so all rights are reserved. Choose one
before distributing.

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
(`crates/zfas-platform/src`), which has the same owner.

## References studied, not copied

See `architecture/REFERENCE_AUDIT.md`. No code was copied from KernelSU, KernelSU-Next, AOSP,
Cedar, OPA, the OpenTelemetry Collector, systemd or Linux.

`module/META-INF/com/google/android/update-binary` is an independently written stub. It calls
Magisk's documented installer interface (`util_functions.sh`, `install_module`).
