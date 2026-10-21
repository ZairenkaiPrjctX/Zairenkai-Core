#!/system/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 ZairenkaiPrjctX contributors
# Manager "Action" button: show Core health.
MODDIR=${0%/*}
"$MODDIR/bin/zcored" ctl --dir /data/adb/zairenkai-core health 2>&1 | head -c 4000
echo
