#!/usr/bin/env bash
#
# Autonomous idle-vs-active GPU verification for Warp on Linux (Intel i915).
#
# Samples GPU utilization while Warp is focused-and-idle (cursor blinking), then
# while actively typing/scrolling. Uses ydotool for keyboard input; does not
# move the mouse (Super / absolute mousemove can open GNOME overview).
#
# Usage: verify_idle_vs_active.sh [BIN] [LABEL] [SCROLLBACK seq_n]
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
BIN="${1:-$REPO_ROOT/target/release/warp-oss}"
LABEL="${2:-dev}"
SCROLLBACK="${3:-4000}"
TEST_BIN_DIR="${TEST_BIN_DIR:-$REPO_ROOT/target/gpu_idle_test/bin}"

INTERVAL_MS="${INTERVAL_MS:-500}"
IDLE_S="${IDLE_S:-25}"
ACTIVE_S="${ACTIVE_S:-12}"
SAMPLE_ACTIVE_S=$(( ACTIVE_S + 2 ))
export YDOTOOL_SOCKET="${YDOTOOL_SOCKET:-$XDG_RUNTIME_DIR/.ydotool_socket}"

IDLE_CSV="/tmp/gpu_idle_${LABEL}.csv"
ACTIVE_CSV="/tmp/gpu_active_${LABEL}.csv"

ESC=1; ENTER=28; LCTRL=29; U=22; PAGEUP=104; PAGEDOWN=109

key() { ydotool key "$@"; }
tap() { ydotool key "$1:1" "$1:0"; }

kill_warp() {
  pkill -x warp-oss >/dev/null 2>&1
  pkill -x warp-oss-base >/dev/null 2>&1
  pkill -f "${TEST_BIN_DIR}/warp-oss-rel-" >/dev/null 2>&1
  true
}

[ -x "$BIN" ] || { echo "ERROR: binary not executable: $BIN"; exit 1; }

if ! pgrep -x ydotoold >/dev/null; then
  ydotoold --socket-path "$YDOTOOL_SOCKET" --socket-own "$(id -u):$(id -g)" >/tmp/ydotoold.log 2>&1 &
  sleep 1.5
fi
ydotool key 0:0 >/dev/null 2>&1 || { echo "ERROR: ydotool cannot reach ydotoold"; exit 1; }

kill_warp
sleep 1.5
if [ "${RESET_SESSION:-0}" = "1" ]; then
  rm -f ~/.local/state/warp-oss/warp.sqlite ~/.local/state/warp-oss/warp.sqlite-wal ~/.local/state/warp-oss/warp.sqlite-shm 2>/dev/null
  echo "(reset dev warp-oss session -> empty terminal on launch)"
fi

echo "launching $BIN (WARP_ENABLE_WAYLAND=1)..."
WARP_ENABLE_WAYLAND=1 setsid "$BIN" >/tmp/warp-oss.verify.log 2>&1 &
sleep 3
tap $ESC; sleep 0.3

echo "IDLE sampling ${IDLE_S}s @ ${INTERVAL_MS}ms (no input)..."
"$HERE/gpu_pmu_sampler.py" "$IDLE_S" "$INTERVAL_MS" "$IDLE_CSV" >/dev/null 2>&1

if [ "$SCROLLBACK" -gt 0 ]; then
  ydotool type --key-delay 3 "seq 1 ${SCROLLBACK}"
  tap $ENTER
  sleep 1.0
fi

echo "ACTIVE sampling ${SAMPLE_ACTIVE_S}s @ ${INTERVAL_MS}ms (fast typing + flood + scroll)..."
"$HERE/gpu_pmu_sampler.py" "$SAMPLE_ACTIVE_S" "$INTERVAL_MS" "$ACTIVE_CSV" >/dev/null 2>&1 &
SP=$!

line="the quick brown fox jumps over the lazy dog 0123456789 "
WL_END=$(( $(date +%s) + ACTIVE_S ))
while [ "$(date +%s)" -lt "$WL_END" ]; do
  for _ in 1 2 3 4 5 6; do ydotool type --key-delay 3 "$line"; done
  key ${LCTRL}:1 ${U}:1 ${U}:0 ${LCTRL}:0
  ydotool type --key-delay 3 "seq 1 4000"
  tap $ENTER
  sleep 0.4
  for _ in $(seq 1 12); do tap $PAGEUP; done
  for _ in $(seq 1 12); do tap $PAGEDOWN; done
done
wait "$SP"

stat() {
  awk -F, -v metric="$2" -v which="$3" '
    NR>1{
      v = (metric=="render") ? $2 : $2;
      if (metric=="btopmax") { v=$2; if($3>v)v=$3; if($4>v)v=$4; if($5>v)v=$5; }
      s+=v; n++; if(v>p)p=v;
    }
    END{ if(n){ printf "%.1f", (which=="avg")? s/n : p } else printf "0.0" }' "$1"
}
IR_AVG=$(stat "$IDLE_CSV" render avg);   IR_PK=$(stat "$IDLE_CSV" render peak)
IM_AVG=$(stat "$IDLE_CSV" btopmax avg);  IM_PK=$(stat "$IDLE_CSV" btopmax peak)
AR_AVG=$(stat "$ACTIVE_CSV" render avg); AR_PK=$(stat "$ACTIVE_CSV" render peak)
AM_AVG=$(stat "$ACTIVE_CSV" btopmax avg);AM_PK=$(stat "$ACTIVE_CSV" btopmax peak)

echo "==================================================================="
echo "  RESULT ($LABEL, scrollback=$SCROLLBACK, interval=${INTERVAL_MS}ms)"
echo "  --- btop-equivalent (max engine = btop gpu-totals) ---"
echo "  IDLE    avg=${IM_AVG}%  peak=${IM_PK}%"
echo "  ACTIVE  avg=${AM_AVG}%  peak=${AM_PK}%"
echo "  --- render engine (rcs0) only ---"
echo "  IDLE    avg=${IR_AVG}%  peak=${IR_PK}%"
echo "  ACTIVE  avg=${AR_AVG}%  peak=${AR_PK}%"
echo "==================================================================="

kill_warp
