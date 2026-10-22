#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 ZairenkaiPrjctX contributors
# Capture the zairenkai.tuning documents the tuning adapter tests use, from the real ztune
# binary running on Zairenkai-Tuning's synthetic fixture tree. Nothing is hand-written.
#
# Usage: scripts/capture-tuning-contract.sh <path to a Zairenkai-Tuning checkout>
# The checkout is only built and run (a separate GPL-3.0 program); no file is copied from it.
set -euo pipefail

TUNING=${1:?usage: $0 <Zairenkai-Tuning checkout>}
OUT="$(cd "$(dirname "$0")/.." && pwd)/crates/zcore-platform/tests/fixtures/tuning"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

commit=$(git -C "$TUNING" rev-parse HEAD)
(cd "$TUNING" && cargo build --locked --quiet -p ztune)
cp -r "$TUNING/crates/ztune/tests/fixtures/synthetic-kona" "$WORK/root"
ztune() { "$TUNING/target/debug/ztune" --sysroot "$WORK/root" --state-dir "$WORK/state" "$@"; }

mkdir -p "$OUT"
ztune capabilities >"$OUT/capabilities.json"
ztune telemetry --interval-ms 200 >"$OUT/telemetry.json"
ztune apply gpu.governor powersave >"$OUT/apply-gpu-governor.json"
printf 'Captured from ZairenkaiPrjctX/Zairenkai-Tuning %s on its synthetic fixture tree\nby scripts/capture-tuning-contract.sh. Do not edit by hand.\n' "$commit" >"$OUT/SOURCE"
echo "captured into $OUT from $commit"
