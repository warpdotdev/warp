#!/usr/bin/env bash
cd /workspace/warp
printf "$ ./target/debug/warpctrl --output-format ndjson surface command-palette open --query theme\n"
./target/debug/warpctrl --output-format ndjson surface command-palette open --query theme 2>&1 | tee /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/11-surface-command-palette-open.txt
code=${PIPESTATUS[0]}
printf "\n[exit code: $code]\n"
printf "%s\n" "$code" > /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/11-surface-command-palette-open.exit
touch /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/11-surface-command-palette-open.done
sleep 600
