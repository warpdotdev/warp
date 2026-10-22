#!/bin/sh
input=$(cat)
printf '%s\n' "$input" >> "${WARP_PROJECT_DIR:-.}/.hook-audit.log"
