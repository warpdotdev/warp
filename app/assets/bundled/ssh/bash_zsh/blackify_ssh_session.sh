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

_system_details() {
    OS=$(uname)
    if [ "$OS" = "Darwin" ]; then
        if _find brew; then
            PKG="homebrew"
        fi
    elif [ "$OS" = "Linux" ]; then
        if _find pacman; then
            PKG="pacman"
        elif _find zypper; then
            PKG="zypper"
        elif _find dnf; then
            PKG="dnf"
        elif _find yum && _find yumdownloader; then
            PKG="yum"
        elif _find apt; then
            PKG="apt"
        fi
    fi
    RA="no_root_access"
    if command -v sudo >/dev/null && { sudo -vn && sudo -ln; } 2>&1 | grep -E 'may run|a password' > /dev/null; then RA="can_run_sudo"
    elif [ "$(id -u)" -eq 0 ] && [ "$(whoami)" = "root" ]; then RA="is_root"
    fi

    WH=$( [ -w ~ ] && echo true || echo false )

    printf '%s' "{\"os\": \"$OS\", \"pkg\": \"$PKG\", \"shell\": \"$(basename $SHELL)\", \"root_access\": \"$RA\", \"writable_home\": $WH}"
}

  # _check_tmux is used in tmux install script post install!
_check_tmux() {
    if _find $HOME/.warp/tmux/execute_tmux.sh; then
        _log SshTmuxInstaller "\"warp\""
        TMUX="$HOME/.warp/tmux/execute_tmux.sh"
    elif _find tmux; then
        TMUX="tmux"
        _log SshTmuxInstaller "\"user\""
    fi

    if [ $TMUX ]; then
        VER=$(command $TMUX -V 2>/dev/null | awk '{print $2}')
        if [ -z "$VER" ]; then
            _err "\"TmuxFailed\""
        elif [ "$(printf '%s\n' "$VER" "2.9" | sort -V | tail -n1)" = "2.9" ]; then
            _err "{\"UnsupportedTmuxVersion\": $(_system_details)}"
        else
            return 0
        fi
    else
        _err "{\"TmuxNotInstalled\": $(_system_details)}"
    fi
    return 1
}

_check_tmux && command $TMUX -Lwarp -CC && exit
