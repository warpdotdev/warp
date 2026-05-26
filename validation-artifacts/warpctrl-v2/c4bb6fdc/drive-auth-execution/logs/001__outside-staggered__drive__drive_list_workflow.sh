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
printf '$ %s\n' '$WARPCTRL --output-format json drive list --type workflow'
 bash -lc '$WARPCTRL --output-format json drive list --type workflow' > >(tee "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/001__outside-staggered__drive__drive_list_workflow.stdout.log") 2> >(tee "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/001__outside-staggered__drive__drive_list_workflow.stderr.log" >&2)
code=$?
printf '\nexit_code=%s\n' "$code"
printf '%s\n' "$code" > "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/001__outside-staggered__drive__drive_list_workflow.exit_code"
touch "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/001__outside-staggered__drive__drive_list_workflow.done"
sleep 8
