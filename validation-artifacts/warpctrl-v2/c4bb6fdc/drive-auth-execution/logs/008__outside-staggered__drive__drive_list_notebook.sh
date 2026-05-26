#!/usr/bin/env bash
export HOME="/tmp/warpctrl-validation-drive-auth-execution/home_enabled"
export XDG_CONFIG_HOME="/tmp/warpctrl-validation-drive-auth-execution/home_enabled/.config"
export XDG_DATA_HOME="/tmp/warpctrl-validation-drive-auth-execution/home_enabled/.local/share"
export XDG_STATE_HOME="/tmp/warpctrl-validation-drive-auth-execution/home_enabled/.local/state"
export XDG_CACHE_HOME="/tmp/warpctrl-validation-drive-auth-execution/home_enabled/.cache"
export XDG_RUNTIME_DIR="/tmp/warpctrl-validation-drive-auth-execution/runtime_enabled"
export WARP_LOCAL_CONTROL_DISCOVERY_DIR="/tmp/warpctrl-validation-drive-auth-execution/discovery_enabled"
export WARPCTRL="/workspace/warpctrl-validation/drive-auth-execution/target/debug/warpctrl"
cd "/workspace/warpctrl-validation/drive-auth-execution"
printf '$ %s\n' '$WARPCTRL --output-format json drive list --type notebook'
 bash -lc '$WARPCTRL --output-format json drive list --type notebook' > >(tee "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/008__outside-staggered__drive__drive_list_notebook.stdout.log") 2> >(tee "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/008__outside-staggered__drive__drive_list_notebook.stderr.log" >&2)
code=$?
printf '\nexit_code=%s\n' "$code"
printf '%s\n' "$code" > "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/008__outside-staggered__drive__drive_list_notebook.exit_code"
touch "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/008__outside-staggered__drive__drive_list_notebook.done"
sleep 8
