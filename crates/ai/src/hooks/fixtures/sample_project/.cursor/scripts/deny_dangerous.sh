#!/bin/sh
# jq is not assumed, so the command is matched with grep against the JSON text of the "command"
# field.

input=$(cat)
project_dir=${WARP_PROJECT_DIR:-.}

# Keep the latest input, so a person (or a test) can see exactly what the hook received.
printf '%s\n' "$input" > "$project_dir/.hook-last-shell-input.json"

# Any JSON string character: anything but a quote or a backslash, or a backslash escape.
char='([^"\\]|\\.)'
in_command='"command":[[:space:]]*"'"$char"'*'
rm_rf='rm[[:space:]]+-(rf|fr)'
force_push='git[[:space:]]+push'"$char"'*--force'

reason=
if printf '%s' "$input" | grep -Eq "$in_command$rm_rf"; then
  reason="This project's hook does not allow rm -rf. Remove files one at a time, or ask the user to do it."
elif printf '%s' "$input" | grep -Eq "$in_command$force_push"; then
  reason="This project's hook does not allow force pushes. Push without --force, or ask the user to do it."
fi

if [ -n "$reason" ]; then
  printf '{"permission":"deny","user_message":"Blocked by the sample project hook.","agent_message":"%s"}\n' "$reason"
else
  printf '{"permission":"allow"}\n'
fi
