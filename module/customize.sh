#!/system/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 ZairenkaiPrjctX contributors
# Install-time checks (sourced by the Magisk/KernelSU/APatch installer).
[ "$ARCH" = "arm64" ] || abort "! Zairenkai-Core supports arm64 only (found $ARCH)"
[ "$API" -ge 29 ] || abort "! Android 10 (API 29) or newer is required (found API $API)"
DATA=/data/adb/zairenkai-core
mkdir -p "$DATA" && chmod 0700 "$DATA"
[ -f "$DATA/config.toml" ] && ui_print "- Keeping existing $DATA/config.toml"
set_perm_recursive "$MODPATH" 0 0 0755 0644
set_perm "$MODPATH/bin/zcored" 0 0 0755
for f in service.sh uninstall.sh action.sh; do set_perm "$MODPATH/$f" 0 0 0755; done
ui_print "- Zairenkai-Core installed. Read-only report: $MODPATH/bin/zcored discover"
