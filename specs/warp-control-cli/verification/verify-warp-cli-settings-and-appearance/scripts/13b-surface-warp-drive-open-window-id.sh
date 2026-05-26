#!/usr/bin/env bash
cd /workspace/warp
printf "$ ./target/debug/warpctrl --output-format ndjson surface warp-drive open --window-id 0\n"
./target/debug/warpctrl --output-format ndjson surface warp-drive open --window-id 0 2>&1 | tee /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/13b-surface-warp-drive-open-window-id.txt
code=${PIPESTATUS[0]}
printf "\n[exit code: $code]\n"
printf "%s\n" "$code" > /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/13b-surface-warp-drive-open-window-id.exit
touch /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/13b-surface-warp-drive-open-window-id.done
sleep 600
