#!/usr/bin/env bash
# Build zcored for Android arm64 and package the Magisk/KernelSU module zip.
#
# Requires: Rust with the aarch64-linux-android target, and ANDROID_NDK_HOME pointing at an
# Android NDK (CI uses r27d, checksum-verified). Output: dist/zairenkai-core-v<version>.zip
set -euo pipefail

cd "$(dirname "$0")/.."
: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to an Android NDK directory}"
API=29
LINKER="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android${API}-clang"
[ -x "$LINKER" ] || { echo "linker not found: $LINKER" >&2; exit 1; }

export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$LINKER"
cargo build --locked --release --target aarch64-linux-android -p zcored

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
IFS=. read -r MAJ MIN PAT <<<"$VERSION"
VERSION_CODE=$(( MAJ * 10000 + MIN * 100 + PAT ))

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
cp -r module/. "$STAGE/"
rm -f "$STAGE/module.prop.in"
sed -e "s/@VERSION@/$VERSION/" -e "s/@VERSION_CODE@/$VERSION_CODE/" module/module.prop.in >"$STAGE/module.prop"
mkdir -p "$STAGE/bin"
cp target/aarch64-linux-android/release/zcored "$STAGE/bin/zcored"
cp LICENSE "$STAGE/LICENSE" 2>/dev/null || true
cp docs/THIRD_PARTY.md "$STAGE/THIRD_PARTY.md"
chmod 0755 "$STAGE"/*.sh "$STAGE/bin/zcored" "$STAGE/META-INF/com/google/android/update-binary"

mkdir -p dist
OUT="dist/zairenkai-core-v${VERSION}.zip"
rm -f "$OUT"
# Fixed timestamps and order for a reproducible archive.
find "$STAGE" -exec touch -h -d '2000-01-01T00:00:00Z' {} +
(cd "$STAGE" && find . -type f | LC_ALL=C sort | sed 's|^\./||' | zip -X -q -@ "$OLDPWD/$OUT")
sha256sum "$OUT" | tee "$OUT.sha256"
