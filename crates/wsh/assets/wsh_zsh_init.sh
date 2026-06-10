#!/usr/bin/env zsh
# wsh zsh integration — emits OSC 133 semantic prompt markers.
# Sourced automatically by wsh; do not source manually.

# Source the user's real .zshrc if it exists.
if [[ -n "$WSH_REAL_ZDOTDIR" ]]; then
    if [[ -f "$WSH_REAL_ZDOTDIR/.zshrc" ]]; then
        source "$WSH_REAL_ZDOTDIR/.zshrc"
    fi
elif [[ -f "$HOME/.zshrc" ]]; then
    source "$HOME/.zshrc"
fi

# --- OSC 133 hooks ---

__wsh_precmd() {
    local exit_code=$?
    # D = command finished (with exit code from the previous command).
    printf '\e]133;D;%d\a' "$exit_code"
    # A = prompt start.
    printf '\e]133;A\a'
}

__wsh_preexec() {
    # C = command execution start.
    printf '\e]133;C\a'
}

__wsh_install_prompt_end_marker() {
    # Append B marker (prompt end) to PROMPT if not already present.
    if [[ "$PROMPT" != *$'\e]133;B\a'* ]]; then
        PROMPT="${PROMPT}%{$(printf '\e]133;B\a')%}"
    fi
}

precmd_functions+=(__wsh_precmd __wsh_install_prompt_end_marker)
preexec_functions+=(__wsh_preexec)
