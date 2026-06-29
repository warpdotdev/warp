#!/usr/bin/env bash
#
# Profile ACTIVE-path GPU/CPU for a Warp binary in one of three regimes, to
# attribute where the cost goes (see METHODOLOGY.md):
#
#   type   - continuous prompt typing + clear (no command run). Render-bound;
#            the regime the partial-repaint damage path targets.
#   scroll - PageUp/PageDown over pre-built scrollback (no new output).
#            Render-bound (every visible row changes).
#   flood  - one self-sustaining `seq` flood. PTY-parse-bound (CPU), not render.
#
# Samples the whole-GPU i915 PMU (gpu_pmu_sampler.py) and warp's per-thread CPU
# (cpu_thread_profile.py) over the same window. Use release builds only (debug
# is CPU-starved and misreports GPU).
#
# Usage: profile_active.sh <warp-binary> [type|scroll|flood]
#   env: PROFILE_S (default 10), SCROLLBACK (default 50000)
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
BIN="${1:?usage: profile_active.sh <warp-binary> [type|scroll|flood]}"
MODE="${2:-type}"
PROFILE_S="${PROFILE_S:-10}"
SCROLLBACK="${SCROLLBACK:-50000}"
BIN_ABS="$(realpath "$BIN")"
export YDOTOOL_SOCKET="${YDOTOOL_SOCKET:-$XDG_RUNTIME_DIR/.ydotool_socket}"
ESC=1; ENTER=28; LCTRL=29; U=22; PAGEUP=104; PAGEDOWN=109
tap() { ydotool key "$1:1" "$1:0"; }

# Kill the launched warp by its absolute path anchored to the start of the
# command line: matches the process (argv0 == BIN_ABS) and its self-reexec
# children, but never this script or shell (they start with `bash`).
kill_warp() { pkill -f "^${BIN_ABS}" >/dev/null 2>&1; true; }

pgrep -x ydotoold >/dev/null || { echo "ERROR: ydotoold not running"; exit 1; }
ydotool key 0:0 >/dev/null 2>&1 || { echo "ERROR: ydotool cannot reach ydotoold"; exit 1; }
[ -x "$BIN_ABS" ] || { echo "ERROR: not executable: $BIN_ABS"; exit 1; }

kill_warp; sleep 1.5
echo "launching $(basename "$BIN_ABS") [mode=$MODE]..."
WARP_ENABLE_WAYLAND=1 setsid "$BIN_ABS" >/tmp/warp-oss.active.log 2>&1 &
sleep 3
tap $ESC; sleep 0.3

WORKLOAD_PID=""
case "$MODE" in
  flood)
    # One injection, then self-sustaining (hands-free).
    ydotool type --key-delay 2 "seq 1 30000000"; tap $ENTER; sleep 2.5
    ;;
  scroll)
    echo "building ${SCROLLBACK} lines of scrollback..."
    ydotool type --key-delay 2 "seq 1 ${SCROLLBACK}"; tap $ENTER; sleep 3
    END=$(( $(date +%s) + PROFILE_S + 4 ))
    ( while [ "$(date +%s)" -lt "$END" ]; do
        for _ in $(seq 1 10); do tap $PAGEUP; done
        for _ in $(seq 1 10); do tap $PAGEDOWN; done
      done ) & WORKLOAD_PID=$!
    sleep 1.5
    ;;
  type)
    ydotool type --key-delay 2 "seq 1 5000"; tap $ENTER; sleep 2
    END=$(( $(date +%s) + PROFILE_S + 4 ))
    ( line="the quick brown fox jumps over the lazy dog 0123456789 "
      while [ "$(date +%s)" -lt "$END" ]; do
        for _ in 1 2 3; do ydotool type --key-delay 2 "$line"; done
        ydotool key ${LCTRL}:1 ${U}:1 ${U}:0 ${LCTRL}:0   # clear, no command run
      done ) & WORKLOAD_PID=$!
    sleep 1.5
    ;;
  *) echo "ERROR: unknown mode '$MODE' (type|scroll|flood)"; kill_warp; exit 2;;
esac

# Focus-miss / no-load gate: a real active workload drives the GPU up.
"$HERE/gpu_pmu_sampler.py" 1.5 100 /tmp/gpu_precheck.csv >/dev/null 2>&1
PRE=$(awk -F, 'NR>1{s+=$2;n++} END{if(n)printf "%.1f",s/n; else printf 0}' /tmp/gpu_precheck.csv)
echo "  pre-check render avg = ${PRE}%"
if awk "BEGIN{exit !($PRE < 5)}"; then
  echo "ABORT: workload did not register (focus miss?). GPU still idle."
  [ -n "$WORKLOAD_PID" ] && kill "$WORKLOAD_PID" 2>/dev/null
  kill_warp; exit 3
fi

echo "profiling ${PROFILE_S}s (i915 PMU @100ms + warp per-thread CPU)..."
"$HERE/gpu_pmu_sampler.py" "$PROFILE_S" 100 /tmp/gpu_active.csv >/tmp/gpu_active.summary 2>&1 &
GP=$!
"$HERE/cpu_thread_profile.py" "$PROFILE_S"
wait "$GP"
[ -n "$WORKLOAD_PID" ] && kill "$WORKLOAD_PID" 2>/dev/null

echo
echo "=== GPU (i915 PMU, whole system incl. compositor) ==="
awk -F, 'NR>1{s+=$2;n++;if($2>p)p=$2} END{if(n)printf "  render rcs0: avg=%.1f%% peak=%.1f%%\n",s/n,p}' /tmp/gpu_active.csv
kill_warp
