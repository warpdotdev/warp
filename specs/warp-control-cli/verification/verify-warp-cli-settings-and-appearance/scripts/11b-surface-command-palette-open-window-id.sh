#!/usr/bin/env bash
cd /workspace/warp
printf "$ ./target/debug/warpctrl --output-format ndjson surface command-palette open --query theme --window-id 0\n"
./target/debug/warpctrl --output-format ndjson surface command-palette open --query theme --window-id 0 2>&1 | tee /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/11b-surface-command-palette-open-window-id.txt
code=${PIPESTATUS[0]}
printf "\n[exit code: $code]\n"
printf "%s\n" "$code" > /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/11b-surface-command-palette-open-window-id.exit
touch /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/11b-surface-command-palette-open-window-id.done
sleep 600
