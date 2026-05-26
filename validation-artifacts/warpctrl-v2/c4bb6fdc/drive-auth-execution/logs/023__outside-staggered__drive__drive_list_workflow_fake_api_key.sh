#!/usr/bin/env bash
export HOME="/tmp/warpctrl-validation-drive-auth-execution-api-key/home"
export XDG_CONFIG_HOME="/tmp/warpctrl-validation-drive-auth-execution-api-key/home/.config"
export XDG_DATA_HOME="/tmp/warpctrl-validation-drive-auth-execution-api-key/home/.local/share"
export XDG_STATE_HOME="/tmp/warpctrl-validation-drive-auth-execution-api-key/home/.local/state"
export XDG_CACHE_HOME="/tmp/warpctrl-validation-drive-auth-execution-api-key/home/.cache"
export XDG_RUNTIME_DIR="/tmp/warpctrl-validation-drive-auth-execution-api-key/runtime"
export WARP_LOCAL_CONTROL_DISCOVERY_DIR="/tmp/warpctrl-validation-drive-auth-execution-api-key/discovery"
export WARPCTRL="/workspace/warpctrl-validation/drive-auth-execution/target/debug/warpctrl"
cd "/workspace/warpctrl-validation/drive-auth-execution"
printf '$ WARPCTRL_API_KEY=<redacted-nonsecret-placeholder> %s\n' '$WARPCTRL --output-format json drive list --type workflow'
WARPCTRL_API_KEY=placeholder-not-a-secret bash -lc '$WARPCTRL --output-format json drive list --type workflow' > >(tee "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/023__outside-staggered__drive__drive_list_workflow_fake_api_key.stdout.log") 2> >(tee "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/023__outside-staggered__drive__drive_list_workflow_fake_api_key.stderr.log" >&2)
code=$?
printf '\nexit_code=%s\n' "$code"
printf '%s\n' "$code" > "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/023__outside-staggered__drive__drive_list_workflow_fake_api_key.exit_code"
touch "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/023__outside-staggered__drive__drive_list_workflow_fake_api_key.done"
sleep 8
