#!/usr/bin/env bash
cd /workspace/warp
printf "$ ./target/debug/warpctrl --output-format ndjson surface warp-drive open\n"
./target/debug/warpctrl --output-format ndjson surface warp-drive open 2>&1 | tee /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/13-surface-warp-drive-open.txt
code=${PIPESTATUS[0]}
printf "\n[exit code: $code]\n"
printf "%s\n" "$code" > /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/13-surface-warp-drive-open.exit
touch /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/13-surface-warp-drive-open.done
sleep 600
