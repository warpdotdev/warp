#!/bin/sh
# Denies deploys and asks before publishing; allows everything else.
input=$(cat)
in_command='"command":[[:space:]]*"'
if printf '%s' "$input" | grep -Eq "${in_command}deploy"; then
  printf '{"permission":"deny","agent_message":"Reviewers do not deploy. Hand the change to the release agent."}\n'
elif printf '%s' "$input" | grep -Eq "${in_command}publish"; then
  printf '{"permission":"ask","agent_message":"Publishing needs a person to approve it."}\n'
else
  printf '{"permission":"allow"}\n'
fi
