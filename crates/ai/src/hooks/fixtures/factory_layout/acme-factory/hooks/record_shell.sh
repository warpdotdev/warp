#!/bin/sh
# Appends each shell command's input to the factory repository's log and lets the command run.
input=$(cat)
printf '%s\n' "$input" >> "${WARP_PROJECT_DIR:-.}/.factory-shell.log"
printf '{"permission":"allow"}\n'
