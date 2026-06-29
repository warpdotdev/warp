#!/usr/bin/env bash
#
# Same-commit A/B: run idle/active verification on BASE (no fix) and CANDIDATE
# (with fix), then print a side-by-side comparison.
#
# Usage: compare_base_vs_candidate.sh [SCROLLBACK]
#   env: BASE=<control binary> CAND=<candidate binary>
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
TEST_BIN_DIR="${TEST_BIN_DIR:-$REPO_ROOT/target/gpu_idle_test/bin}"
BASE="${BASE:-$TEST_BIN_DIR/warp-oss-rel-base}"
CAND="${CAND:-$TEST_BIN_DIR/warp-oss-rel-candidate}"
SCROLLBACK="${1:-4000}"

[ -x "$BASE" ] || { echo "missing BASE binary: $BASE"; exit 1; }
[ -x "$CAND" ] || { echo "missing CANDIDATE binary: $CAND"; exit 1; }

echo "############ BASE / control ############"
"$HERE/verify_idle_vs_active.sh" "$BASE" base "$SCROLLBACK"
echo
echo "############ CANDIDATE ############"
"$HERE/verify_idle_vs_active.sh" "$CAND" cand "$SCROLLBACK"

stat() {
  awk -F, -v which="$2" '
    NR>1{ v=$2; if($3>v)v=$3; if($4>v)v=$4; if($5>v)v=$5; s+=v; n++; if(v>p)p=v }
    END{ if(n){ printf "%.1f", (which=="avg")? s/n : p } else printf "0.0" }' "$1"
}

IB_A=$(stat /tmp/gpu_idle_base.csv avg);   IB_P=$(stat /tmp/gpu_idle_base.csv peak)
IC_A=$(stat /tmp/gpu_idle_cand.csv avg);   IC_P=$(stat /tmp/gpu_idle_cand.csv peak)
AB_A=$(stat /tmp/gpu_active_base.csv avg); AB_P=$(stat /tmp/gpu_active_base.csv peak)
AC_A=$(stat /tmp/gpu_active_cand.csv avg); AC_P=$(stat /tmp/gpu_active_cand.csv peak)

echo
echo "================= BASE vs CANDIDATE (btop-equiv max-engine, scrollback=$SCROLLBACK) ================="
printf "  IDLE    base avg=%5s%% peak=%5s%%   ->   cand avg=%5s%% peak=%5s%%\n" "$IB_A" "$IB_P" "$IC_A" "$IC_P"
printf "  ACTIVE  base avg=%5s%% peak=%5s%%   ->   cand avg=%5s%% peak=%5s%%\n" "$AB_A" "$AB_P" "$AC_A" "$AC_P"
echo "==================================================================================================="

pkill -x warp-oss >/dev/null 2>&1
pkill -x warp-oss-base >/dev/null 2>&1
pkill -f "${TEST_BIN_DIR}/warp-oss-rel-" >/dev/null 2>&1
