#!/bin/sh
# afterShellExecution hook for the sample project: appends the hook's whole input to
# .hook-audit.log in the project, one JSON object per line. It prints nothing, so it never
# changes what the agent does.

input=$(cat)
printf '%s\n' "$input" >> "${WARP_PROJECT_DIR:-.}/.hook-audit.log"
