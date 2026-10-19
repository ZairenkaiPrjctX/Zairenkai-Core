#!/system/bin/sh
# Manager "Action" button: show Core health.
MODDIR=${0%/*}
"$MODDIR/bin/zcored" ctl --dir /data/adb/zairenkai-core health 2>&1 | head -c 4000
echo
