_find() {
    command -v "$1" >/dev/null 2>&1
}

_log() {
    _msg=$(printf "{\"hook\": \"$1\", \"value\": $2}" | command -p od -An -v -tx1 | command -p tr -d " \n")
    printf '\033\120\044\144%s\234' "$_msg"
}

_err() {
    _log RemoteWarpificationIsUnavailable "$1"
}

_sd() {
    if _find brew; then
        PKG="homebrew"
    fi

    WH=$( [ -w ~ ] && echo true || echo false )

    printf '{"os": "Darwin", "pkg": "%s", "shell": "%s", "root_access": "no_root_access", "writable_home": %s}' "$PKG" "$(basename $SHELL)" $WH
}

  # _check_tmux is used in tmux install script post install!
_check_tmux() {
    TMUX="$HOME/.warp/tmux/execute_tmux.sh"
    if _find "$TMUX"; then
        _log SshTmuxInstaller "\"warp\""
    elif _find tmux; then
        TMUX="tmux"
        _log SshTmuxInstaller "\"user\""
    fi

    if [ $TMUX ]; then
        VER=$($TMUX -V 2>/dev/null | awk '{print $2}')
        if [ -z "$VER" ]; then
            _err "\"TmuxFailed\""
        elif [ "$(printf '%s\n' "$VER" "2.9" | sort -V | tail -n1)" = "2.9" ]; then
            _err "{\"UnsupportedTmuxVersion\": $(_sd)}"
        else
            return 0
        fi
    else
        _err "{\"TmuxNotInstalled\": $(_sd)}"
    fi
    return 1
}

_check_tmux && $TMUX -Lwarp -CC && exit
