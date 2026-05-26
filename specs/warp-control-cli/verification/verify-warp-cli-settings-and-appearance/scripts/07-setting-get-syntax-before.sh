#!/usr/bin/env bash
cd /workspace/warp
printf "$ ./target/debug/warpctrl --output-format ndjson setting get terminal.input.syntax_highlighting\n"
./target/debug/warpctrl --output-format ndjson setting get terminal.input.syntax_highlighting 2>&1 | tee /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/07-setting-get-syntax-before.txt
code=${PIPESTATUS[0]}
printf "\n[exit code: $code]\n"
printf "%s\n" "$code" > /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/07-setting-get-syntax-before.exit
touch /workspace/warp/specs/warp-control-cli/verification/verify-warp-cli-settings-and-appearance/outputs/07-setting-get-syntax-before.done
sleep 600
