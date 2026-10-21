#!/bin/sh
# sessionStart hook for the sample project: sets one variable that every later hook in the same
# conversation receives in its environment.

cat > /dev/null
printf '{"env":{"SAMPLE_HOOK_SESSION":"started"}}\n'
