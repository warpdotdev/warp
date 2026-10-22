#!/bin/sh
# jq is not assumed, so the prompt is cut out of the JSON text with sed. Its length counts the
# prompt as it appears in the JSON input, escapes included.

input=$(cat)
prompt=$(printf '%s' "$input" | sed -nE 's/.*"prompt":[[:space:]]*"(([^"\\]|\\.)*)".*/\1/p')
length=$(printf '%s' "$prompt" | wc -c | tr -d ' ')
printf 'prompt_length=%s session=%s\n' "$length" "${SAMPLE_HOOK_SESSION:-unset}" \
  >> "${WARP_PROJECT_DIR:-.}/.hook-prompts.log"
printf '{"continue":true}\n'
