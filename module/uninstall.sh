#!/system/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 ZairenkaiPrjctX contributors
# Stop zcored (it ends temporary overrides and restores their baselines on SIGTERM), then
# remove its data.
for pid in $(pidof zcored); do kill -TERM "$pid" 2>/dev/null; done
i=0
while [ -n "$(pidof zcored)" ] && [ "$i" -lt 30 ]; do sleep 1; i=$((i + 1)); done
rm -rf /data/adb/zairenkai-core
