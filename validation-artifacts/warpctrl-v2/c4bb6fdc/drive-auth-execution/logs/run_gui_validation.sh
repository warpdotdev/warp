#!/usr/bin/env bash
REPO=/workspace/warpctrl-validation/drive-auth-execution
ART="$REPO/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution"
LOGS="$ART/logs"
SHOTS="$ART/screenshots"
WARP_APP="$REPO/target/debug/warp-oss"
WARPCTRL="$REPO/target/debug/warpctrl"
DISPLAY_NUM=:99
SCREEN=1400x900x24
BASE=/tmp/warpctrl-validation-drive-auth-execution
mkdir -p "$LOGS" "$SHOTS" "$BASE"
chmod 700 "$BASE" || true
cleanup_phase() {
  if [ -n "${APP_PID:-}" ]; then kill "$APP_PID" 2>/dev/null || true; wait "$APP_PID" 2>/dev/null || true; fi
  pkill -f "xterm.*warpctrl-validation" 2>/dev/null || true
}
cleanup_all() {
  cleanup_phase
  if [ -n "${OPENBOX_PID:-}" ]; then kill "$OPENBOX_PID" 2>/dev/null || true; wait "$OPENBOX_PID" 2>/dev/null || true; fi
  if [ -n "${XVFB_PID:-}" ]; then kill "$XVFB_PID" 2>/dev/null || true; wait "$XVFB_PID" 2>/dev/null || true; fi
}
trap cleanup_all EXIT
Xvfb "$DISPLAY_NUM" -screen 0 "$SCREEN" > "$LOGS/xvfb.log" 2>&1 &
XVFB_PID=$!
sleep 1
export DISPLAY="$DISPLAY_NUM"
openbox > "$LOGS/openbox.log" 2>&1 &
OPENBOX_PID=$!
sleep 2
write_prefs() {
  local home="$1"
  local enabled="$2"
  local prefs_dir="$home/.config/warp/Warp-Oss"
  mkdir -p "$prefs_dir"
  if [ "$enabled" = enabled ]; then
    cat > "$prefs_dir/user_preferences.json" <<'JSON'
{
  "prefs": {
    "LocalControlAllowOutsideWarp": "true",
    "LocalControlOutsideWarpMetadataReads": "true",
    "LocalControlOutsideWarpUnderlyingDataReads": "true",
    "LocalControlOutsideWarpAppStateMutations": "true",
    "LocalControlOutsideWarpMetadataConfigurationMutations": "true",
    "LocalControlOutsideWarpUnderlyingDataMutations": "true",
    "LocalControlInsideWarpAuthenticatedUserActions": "false"
  }
}
JSON
  fi
}
start_app() {
  local phase="$1"
  local enabled="$2"
  export HOME="$BASE/home_$phase"
  export XDG_CONFIG_HOME="$HOME/.config"
  export XDG_DATA_HOME="$HOME/.local/share"
  export XDG_STATE_HOME="$HOME/.local/state"
  export XDG_CACHE_HOME="$HOME/.cache"
  export XDG_RUNTIME_DIR="$BASE/runtime_$phase"
  export WARP_LOCAL_CONTROL_DISCOVERY_DIR="$BASE/discovery_$phase"
  mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR" "$WARP_LOCAL_CONTROL_DISCOVERY_DIR"
  chmod 700 "$XDG_RUNTIME_DIR" "$WARP_LOCAL_CONTROL_DISCOVERY_DIR" || true
  write_prefs "$HOME" "$enabled"
  env HOME="$HOME" XDG_CONFIG_HOME="$XDG_CONFIG_HOME" XDG_DATA_HOME="$XDG_DATA_HOME" XDG_STATE_HOME="$XDG_STATE_HOME" XDG_CACHE_HOME="$XDG_CACHE_HOME" XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" WARP_LOCAL_CONTROL_DISCOVERY_DIR="$WARP_LOCAL_CONTROL_DISCOVERY_DIR" DISPLAY="$DISPLAY" LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=gl "$WARP_APP" > "$LOGS/app_${phase}.stdout.log" 2> "$LOGS/app_${phase}.stderr.log" &
  APP_PID=$!
  local i=0
  while [ "$i" -lt 60 ]; do
    if ! kill -0 "$APP_PID" 2>/dev/null; then
      echo "app_exited_before_discovery phase=$phase" > "$LOGS/app_${phase}.status"
      return 1
    fi
    if ls "$WARP_LOCAL_CONTROL_DISCOVERY_DIR"/*.json >/dev/null 2>&1; then
      cp "$WARP_LOCAL_CONTROL_DISCOVERY_DIR"/*.json "$LOGS/discovery_${phase}.json" 2>/dev/null || true
      sleep 4
      xdotool search --name Warp windowmove %@ 520 70 windowsize %@ 840 760 2> "$LOGS/xdotool_warp_${phase}.log" || true
      return 0
    fi
    sleep 1
    i=$((i+1))
  done
  echo "discovery_timeout phase=$phase" > "$LOGS/app_${phase}.status"
  return 1
}
run_case() {
  local ordinal="$1"
  local phase="$2"
  local context="$3"
  local family="$4"
  local name="$5"
  local cmd="$6"
  local env_prefix="$7"
  local safe_name="${ordinal}__${context}__${family}__${name}"
  local script="$LOGS/${safe_name}.sh"
  local stdout="$LOGS/${safe_name}.stdout.log"
  local stderr="$LOGS/${safe_name}.stderr.log"
  local exitfile="$LOGS/${safe_name}.exit_code"
  local donefile="$LOGS/${safe_name}.done"
  cat > "$script" <<EOF
#!/usr/bin/env bash
export HOME="$HOME"
export XDG_CONFIG_HOME="$XDG_CONFIG_HOME"
export XDG_DATA_HOME="$XDG_DATA_HOME"
export XDG_STATE_HOME="$XDG_STATE_HOME"
export XDG_CACHE_HOME="$XDG_CACHE_HOME"
export XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR"
export WARP_LOCAL_CONTROL_DISCOVERY_DIR="$WARP_LOCAL_CONTROL_DISCOVERY_DIR"
export WARPCTRL="$WARPCTRL"
cd "$REPO"
printf '\$ %s\n' '$cmd'
$env_prefix bash -lc '$cmd' > >(tee "$stdout") 2> >(tee "$stderr" >&2)
code=\$?
printf '\nexit_code=%s\n' "\$code"
printf '%s\n' "\$code" > "$exitfile"
touch "$donefile"
sleep 8
EOF
  chmod +x "$script"
  rm -f "$donefile" "$exitfile"
  xterm -T "warpctrl-validation-${safe_name}" -geometry 94x20+20+20 -fa Monospace -fs 10 -e "$script" > "$LOGS/${safe_name}.xterm.log" 2>&1 &
  local xp=$!
  local i=0
  while [ "$i" -lt 45 ]; do
    [ -f "$donefile" ] && break
    sleep 1
    i=$((i+1))
  done
  sleep 1
  scrot "$SHOTS/${safe_name}__terminal_ui.png" || true
  wait "$xp" 2>/dev/null || true
}
# Phase 1: outside-Warp control disabled. Discovery record exists but has no endpoint authority.
if start_app disabled disabled; then
  run_case 001 disabled outside-staggered drive drive_list_workflow '$WARPCTRL --output-format json drive list --type workflow' ''
  run_case 002 disabled outside-staggered drive drive_list_notebook '$WARPCTRL --output-format json drive list --type notebook' ''
  run_case 003 disabled outside-staggered drive drive_list_env_var_collection '$WARPCTRL --output-format json drive list --type env-var-collection' ''
  run_case 004 disabled outside-staggered drive drive_inspect_placeholder '$WARPCTRL --output-format json drive inspect placeholder-drive-object-id' ''
  run_case 005 disabled outside-staggered drive drive_workflow_run_placeholder '$WARPCTRL --output-format json drive workflow run placeholder-workflow-id --arg validation=warpctrl' ''
  run_case 006 disabled outside-staggered execution input_run '$WARPCTRL --output-format json input run "printf warpctrl-validation"' ''
else
  scrot "$SHOTS/000__outside-staggered__blocked__app_launch_disabled__terminal_ui.png" || true
fi
cleanup_phase
sleep 2
# Phase 2: outside-Warp control and all outside permissions enabled. Authenticated-user actions should still reject external/API-key style invocation.
if start_app enabled enabled; then
  run_case 007 enabled outside-staggered drive drive_list_workflow '$WARPCTRL --output-format json drive list --type workflow' ''
  run_case 008 enabled outside-staggered drive drive_list_notebook '$WARPCTRL --output-format json drive list --type notebook' ''
  run_case 009 enabled outside-staggered drive drive_list_env_var_collection '$WARPCTRL --output-format json drive list --type env-var-collection' ''
  run_case 010 enabled outside-staggered drive drive_inspect_placeholder '$WARPCTRL --output-format json drive inspect placeholder-drive-object-id' ''
  run_case 011 enabled outside-staggered drive drive_workflow_run_placeholder '$WARPCTRL --output-format json drive workflow run placeholder-workflow-id --arg validation=warpctrl' ''
  run_case 012 enabled outside-staggered execution input_run '$WARPCTRL --output-format json input run "printf warpctrl-validation"' ''
  run_case 013 enabled outside-staggered drive drive_list_workflow_fake_inside '$WARPCTRL --output-format json drive list --type workflow' 'WARPCTRL_TERMINAL_PROOF_ID=fake-proof WARPCTRL_TERMINAL_SESSION_ID=fake-session WARPCTRL_TERMINAL_PROOF_SECRET=fake-secret'
  run_case 014 enabled outside-staggered execution input_run_fake_inside '$WARPCTRL --output-format json input run "printf warpctrl-validation"' 'WARPCTRL_TERMINAL_PROOF_ID=fake-proof WARPCTRL_TERMINAL_SESSION_ID=fake-session WARPCTRL_TERMINAL_PROOF_SECRET=fake-secret'
else
  scrot "$SHOTS/000__outside-staggered__blocked__app_launch_enabled__terminal_ui.png" || true
fi
