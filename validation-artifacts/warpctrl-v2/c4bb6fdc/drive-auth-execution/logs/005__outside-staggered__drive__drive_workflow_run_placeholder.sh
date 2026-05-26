#!/usr/bin/env bash
export HOME="/tmp/warpctrl-validation-drive-auth-execution/home_disabled"
export XDG_CONFIG_HOME="/tmp/warpctrl-validation-drive-auth-execution/home_disabled/.config"
export XDG_DATA_HOME="/tmp/warpctrl-validation-drive-auth-execution/home_disabled/.local/share"
export XDG_STATE_HOME="/tmp/warpctrl-validation-drive-auth-execution/home_disabled/.local/state"
export XDG_CACHE_HOME="/tmp/warpctrl-validation-drive-auth-execution/home_disabled/.cache"
export XDG_RUNTIME_DIR="/tmp/warpctrl-validation-drive-auth-execution/runtime_disabled"
export WARP_LOCAL_CONTROL_DISCOVERY_DIR="/tmp/warpctrl-validation-drive-auth-execution/discovery_disabled"
export WARPCTRL="/workspace/warpctrl-validation/drive-auth-execution/target/debug/warpctrl"
cd "/workspace/warpctrl-validation/drive-auth-execution"
printf '$ %s\n' '$WARPCTRL --output-format json drive workflow run placeholder-workflow-id --arg validation=warpctrl'
 bash -lc '$WARPCTRL --output-format json drive workflow run placeholder-workflow-id --arg validation=warpctrl' > >(tee "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/005__outside-staggered__drive__drive_workflow_run_placeholder.stdout.log") 2> >(tee "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/005__outside-staggered__drive__drive_workflow_run_placeholder.stderr.log" >&2)
code=$?
printf '\nexit_code=%s\n' "$code"
printf '%s\n' "$code" > "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/005__outside-staggered__drive__drive_workflow_run_placeholder.exit_code"
touch "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/005__outside-staggered__drive__drive_workflow_run_placeholder.done"
sleep 8
