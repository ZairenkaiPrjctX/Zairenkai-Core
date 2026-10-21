#!/system/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 ZairenkaiPrjctX contributors
# Supervise zcored: exponential backoff, and a start limit (5 starts in 10 minutes) after
# which the supervisor gives up and leaves a marker instead of restart-looping.
MODDIR=${0%/*}
DATA=/data/adb/zairenkai-core
BIN="$MODDIR/bin/zcored"
LOG="$DATA/zcored.log"
mkdir -p "$DATA" && chmod 0700 "$DATA"
rm -f "$DATA/failed"
until [ "$(getprop sys.boot_completed)" = "1" ]; do sleep 5; done
sleep 5

starts=""
delay=5
while [ ! -f "$MODDIR/disable" ] && [ ! -f "$MODDIR/remove" ]; do
  if [ -f "$LOG" ] && [ "$(stat -c %s "$LOG" 2>/dev/null || echo 0)" -gt 524288 ]; then
    mv -f "$LOG" "$LOG.1"
  fi
  now=$(date +%s)
  recent=""
  count=0
  for t in $starts; do
    if [ $((now - t)) -lt 600 ]; then recent="$recent $t"; count=$((count + 1)); fi
  done
  if [ "$count" -ge 5 ]; then
    echo "service.sh: 5 starts within 10 minutes; giving up (see $LOG)" >>"$LOG"
    echo "start limit hit at $now" >"$DATA/failed"
    exit 1
  fi
  starts="$recent $now"
  "$BIN" run --dir "$DATA" >>"$LOG" 2>&1
  code=$?
  echo "service.sh: zcored exited with $code" >>"$LOG"
  [ "$code" -eq 0 ] && break
  [ "$code" -eq 3 ] && break   # another instance is running
  sleep "$delay"
  delay=$((delay * 2)); [ "$delay" -gt 300 ] && delay=300
done
