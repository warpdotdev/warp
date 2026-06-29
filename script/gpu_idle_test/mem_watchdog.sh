#!/usr/bin/env bash
# Optional OOM guard during release builds on memory-constrained machines.
# Kills cargo/rustc if MemAvailable drops below THRESHOLD_KB (default 2.5GB).
THRESHOLD_KB="${1:-2500000}"
LOG=/tmp/mem_watchdog.log
echo "watchdog started $(date) threshold=${THRESHOLD_KB}kB" > "$LOG"
while true; do
  avail=$(awk '/MemAvailable/{print $2}' /proc/meminfo)
  if [ "${avail:-0}" -lt "$THRESHOLD_KB" ]; then
    echo "$(date) MemAvailable=${avail}kB < ${THRESHOLD_KB}kB -> KILLING BUILD" >> "$LOG"
    pkill -f 'cargo build --release' 2>/dev/null
    pkill -x rustc 2>/dev/null
    sleep 5
  fi
  sleep 2
done
