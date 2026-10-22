#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 ZairenkaiPrjctX contributors
# Build zcored for Android arm64 as a component of the unified zairenkai module.
#
# Requires: Rust with the aarch64-linux-android target, and ANDROID_NDK_HOME pointing at an
# Android NDK (CI uses r27d, checksum-verified). Output: dist/zairenkai-core/, the files
# Zairenkai-Modules assembles into the module (bin/zcored, licences, component.json). The
# module scripts and zip belong to Zairenkai-Modules.
set -euo pipefail

cd "$(dirname "$0")/.."
: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to an Android NDK directory}"
API=29
LINKER="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android${API}-clang"
[ -x "$LINKER" ] || { echo "linker not found: $LINKER" >&2; exit 1; }

export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$LINKER"
cargo build --locked --release --target aarch64-linux-android -p zcored
# 16 KB page-size devices need 16 KB-aligned LOAD segments (.cargo/config.toml sets the flag).
for align in $("$(dirname "$LINKER")/llvm-readelf" -lW target/aarch64-linux-android/release/zcored | awk '$1 == "LOAD" { print $NF }'); do
  [ $((align)) -ge 16384 ] || { echo "zcored: LOAD segment aligned to $align, need 16 KB" >&2; exit 1; }
done

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
COMMIT=$(git rev-parse HEAD)
OUT=dist/zairenkai-core
rm -rf "$OUT"
mkdir -p "$OUT/bin"
cp target/aarch64-linux-android/release/zcored "$OUT/bin/zcored"
chmod 0755 "$OUT/bin/zcored"
cp LICENSE NOTICE "$OUT/"
cp docs/THIRD_PARTY.md "$OUT/THIRD_PARTY.md"

# component.json: what Zairenkai-Modules checks before packaging (format 1).
files=""
for f in bin/zcored LICENSE NOTICE THIRD_PARTY.md; do
  files="$files${files:+,}
    \"$f\": \"$(sha256sum "$OUT/$f" | cut -d' ' -f1)\""
done
cat >"$OUT/component.json" <<JSON
{
  "format": 1,
  "component": "zairenkai-core",
  "repository": "ZairenkaiPrjctX/Zairenkai-Core",
  "version": "$VERSION",
  "commit": "$COMMIT",
  "target": "aarch64-linux-android",
  "min_api": $API,
  "files": {$files
  }
}
JSON
cat "$OUT/component.json"
