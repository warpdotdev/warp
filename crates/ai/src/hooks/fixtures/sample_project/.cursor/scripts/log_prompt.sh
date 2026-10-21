#!/bin/sh
# beforeSubmitPrompt hook for the sample project: appends the prompt's length, and the variable
# the sessionStart hook set, to .hook-prompts.log in the project, then lets the prompt through.
# The length counts the prompt as it appears in the JSON input, escapes included.

input=$(cat)
prompt=$(printf '%s' "$input" | sed -nE 's/.*"prompt":"(([^"\\]|\\.)*)".*/\1/p')
length=$(printf '%s' "$prompt" | wc -c | tr -d ' ')
printf 'prompt_length=%s session=%s\n' "$length" "${SAMPLE_HOOK_SESSION:-unset}" \
  >> "${WARP_PROJECT_DIR:-.}/.hook-prompts.log"
printf '{"continue":true}\n'
