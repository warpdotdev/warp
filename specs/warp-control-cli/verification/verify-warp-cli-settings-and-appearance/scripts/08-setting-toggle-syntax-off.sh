#!/usr/bin/env bash
cd /workspace/warp
printf "$ ./target/debug/warpctrl --output-format ndjson setting toggle terminal.input.syntax_highlighting\n"
./target/debug/warpctrl --output-format ndjson setting toggle terminal.input.syntax_highlighting 2>&1 | tee /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/08-setting-toggle-syntax-off.txt
code=${PIPESTATUS[0]}
printf "\n[exit code: $code]\n"
printf "%s\n" "$code" > /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/08-setting-toggle-syntax-off.exit
touch /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/08-setting-toggle-syntax-off.done
sleep 600
