#!/usr/bin/env bash
REPO=/workspace/warpctrl-validation/drive-auth-execution
ART="$REPO/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution"
LOGS="$ART/logs"
SHOTS="$ART/screenshots"
WARP_APP="$REPO/target/debug/warp-oss"
WARPCTRL="$REPO/target/debug/warpctrl"
DISPLAY_NUM=:101
SCREEN=1400x900x24
BASE=/tmp/warpctrl-validation-drive-auth-execution-api-key
mkdir -p "$LOGS" "$SHOTS" "$BASE"
cleanup_all() {
  if [ -n "${APP_PID:-}" ]; then kill "$APP_PID" 2>/dev/null || true; wait "$APP_PID" 2>/dev/null || true; fi
  pkill -f "xterm.*warpctrl-validation-api-key" 2>/dev/null || true
  if [ -n "${OPENBOX_PID:-}" ]; then kill "$OPENBOX_PID" 2>/dev/null || true; wait "$OPENBOX_PID" 2>/dev/null || true; fi
  if [ -n "${XVFB_PID:-}" ]; then kill "$XVFB_PID" 2>/dev/null || true; wait "$XVFB_PID" 2>/dev/null || true; fi
}
trap cleanup_all EXIT
Xvfb "$DISPLAY_NUM" -screen 0 "$SCREEN" > "$LOGS/xvfb_api_key.log" 2>&1 & XVFB_PID=$!
sleep 1
export DISPLAY="$DISPLAY_NUM"
openbox > "$LOGS/openbox_api_key.log" 2>&1 & OPENBOX_PID=$!
sleep 2
export HOME="$BASE/home"
export XDG_CONFIG_HOME="$HOME/.config"
export XDG_DATA_HOME="$HOME/.local/share"
export XDG_STATE_HOME="$HOME/.local/state"
export XDG_CACHE_HOME="$HOME/.cache"
export XDG_RUNTIME_DIR="$BASE/runtime"
export WARP_LOCAL_CONTROL_DISCOVERY_DIR="$BASE/discovery"
mkdir -p "$HOME/.config/warp-oss" "$XDG_DATA_HOME" "$XDG_STATE_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR" "$WARP_LOCAL_CONTROL_DISCOVERY_DIR"
chmod 700 "$XDG_RUNTIME_DIR" "$WARP_LOCAL_CONTROL_DISCOVERY_DIR" || true
cat > "$HOME/.config/warp-oss/user_preferences.json" <<'JSON'
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
env HOME="$HOME" XDG_CONFIG_HOME="$XDG_CONFIG_HOME" XDG_DATA_HOME="$XDG_DATA_HOME" XDG_STATE_HOME="$XDG_STATE_HOME" XDG_CACHE_HOME="$XDG_CACHE_HOME" XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" WARP_LOCAL_CONTROL_DISCOVERY_DIR="$WARP_LOCAL_CONTROL_DISCOVERY_DIR" DISPLAY="$DISPLAY" LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=gl "$WARP_APP" > "$LOGS/app_api_key.stdout.log" 2> "$LOGS/app_api_key.stderr.log" & APP_PID=$!
for i in $(seq 1 60); do
  if ls "$WARP_LOCAL_CONTROL_DISCOVERY_DIR"/*.json >/dev/null 2>&1; then cp "$WARP_LOCAL_CONTROL_DISCOVERY_DIR"/*.json "$LOGS/discovery_api_key.json" 2>/dev/null || true; break; fi
  sleep 1
done
sleep 4
xdotool search --name Warp windowmove %@ 520 70 windowsize %@ 840 760 2> "$LOGS/xdotool_warp_api_key.log" || true
run_case() {
  local ordinal="$1" family="$2" name="$3" cmd="$4"
  local safe_name="${ordinal}__outside-staggered__${family}__${name}"
  local script="$LOGS/${safe_name}.sh" stdout="$LOGS/${safe_name}.stdout.log" stderr="$LOGS/${safe_name}.stderr.log" exitfile="$LOGS/${safe_name}.exit_code" donefile="$LOGS/${safe_name}.done"
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
printf '\$ WARPCTRL_API_KEY=<redacted-nonsecret-placeholder> %s\n' '$cmd'
WARPCTRL_API_KEY=placeholder-not-a-secret bash -lc '$cmd' > >(tee "$stdout") 2> >(tee "$stderr" >&2)
code=\$?
printf '\nexit_code=%s\n' "\$code"
printf '%s\n' "\$code" > "$exitfile"
touch "$donefile"
sleep 8
EOF
  chmod +x "$script"
  rm -f "$donefile" "$exitfile"
  xterm -T "warpctrl-validation-api-key-${safe_name}" -geometry 108x20+20+20 -fa Monospace -fs 10 -e "$script" > "$LOGS/${safe_name}.xterm.log" 2>&1 &
  xp=$!
  for i in $(seq 1 45); do [ -f "$donefile" ] && break; sleep 1; done
  sleep 1
  scrot "$SHOTS/${safe_name}__terminal_ui.png" || true
  wait "$xp" 2>/dev/null || true
}
run_case 023 drive drive_list_workflow_fake_api_key '$WARPCTRL --output-format json drive list --type workflow'
run_case 024 execution input_run_fake_api_key '$WARPCTRL --output-format json input run "printf warpctrl-validation"'
