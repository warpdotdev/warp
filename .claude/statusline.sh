#!/bin/bash

# Read JSON input from stdin
input=$(cat)

# Extract model display name
MODEL=$(echo "$input" | jq -r '.model.display_name // "Claude"')

# Extract current working directory (just the folder name)
CURRENT_DIR=$(echo "$input" | jq -r '.workspace.current_dir // "~"')
DIR_NAME="${CURRENT_DIR##*/}"

# Get git branch if in a git repository
GIT_BRANCH=""
if git rev-parse --git-dir > /dev/null 2>&1; then
    BRANCH=$(git branch --show-current 2>/dev/null)
    if [ -n "$BRANCH" ]; then
        GIT_BRANCH=" | Branch: $BRANCH"
    fi
fi

# Output formatted status line
echo "[$MODEL] 📁 $DIR_NAME$GIT_BRANCH"
