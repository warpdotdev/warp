#!/usr/bin/env bash
export HOME="/tmp/warpctrl-validation-drive-auth-execution-corrected/home_enabled_corrected"
export XDG_CONFIG_HOME="/tmp/warpctrl-validation-drive-auth-execution-corrected/home_enabled_corrected/.config"
export XDG_DATA_HOME="/tmp/warpctrl-validation-drive-auth-execution-corrected/home_enabled_corrected/.local/share"
export XDG_STATE_HOME="/tmp/warpctrl-validation-drive-auth-execution-corrected/home_enabled_corrected/.local/state"
export XDG_CACHE_HOME="/tmp/warpctrl-validation-drive-auth-execution-corrected/home_enabled_corrected/.cache"
export XDG_RUNTIME_DIR="/tmp/warpctrl-validation-drive-auth-execution-corrected/runtime_enabled_corrected"
export WARP_LOCAL_CONTROL_DISCOVERY_DIR="/tmp/warpctrl-validation-drive-auth-execution-corrected/discovery_enabled_corrected"
export WARPCTRL="/workspace/warpctrl-validation/drive-auth-execution/target/debug/warpctrl"
cd "/workspace/warpctrl-validation/drive-auth-execution"
printf '$ %s\n' '$WARPCTRL --output-format json input run "printf warpctrl-validation"'
WARPCTRL_TERMINAL_PROOF_ID=fake-proof WARPCTRL_TERMINAL_SESSION_ID=fake-session WARPCTRL_TERMINAL_PROOF_SECRET=fake-secret bash -lc '$WARPCTRL --output-format json input run "printf warpctrl-validation"' > >(tee "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/022__outside-staggered__execution__input_run_fake_inside_enabled.stdout.log") 2> >(tee "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/022__outside-staggered__execution__input_run_fake_inside_enabled.stderr.log" >&2)
code=$?
printf '\nexit_code=%s\n' "$code"
printf '%s\n' "$code" > "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/022__outside-staggered__execution__input_run_fake_inside_enabled.exit_code"
touch "/workspace/warpctrl-validation/drive-auth-execution/validation-artifacts/warpctrl-v2/c4bb6fdc/drive-auth-execution/logs/022__outside-staggered__execution__input_run_fake_inside_enabled.done"
sleep 8
