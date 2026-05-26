#!/usr/bin/env bash
cd /workspace/warp
printf "$ ./target/debug/warpctrl --output-format ndjson surface vertical-tabs toggle --window-id 0\n"
./target/debug/warpctrl --output-format ndjson surface vertical-tabs toggle --window-id 0 2>&1 | tee /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/20b-surface-vertical-tabs-toggle-window-id.txt
code=${PIPESTATUS[0]}
printf "\n[exit code: $code]\n"
printf "%s\n" "$code" > /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/20b-surface-vertical-tabs-toggle-window-id.exit
touch /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/20b-surface-vertical-tabs-toggle-window-id.done
sleep 600
