function _is
    command -v $argv[1] >/dev/null 2>&1
end

function _log
    set _hook $argv[1]
    set _value $argv[2]
    set _m (printf "{\"hook\": \"%s\", \"value\": %s}" $_hook $_value | od -An -v -tx1 | tr -d " \n")
    printf '\033\120\044\144%s\234' $_m
end

function _err
    _log RemoteWarpificationIsUnavailable $argv[1]
end

function _system_details
    set -l OS (uname)
    set -l PK ""

    if test "$OS" = "Darwin"
        if _is brew
            set PK "homebrew"
        end
    else if test "$OS" = "Linux"
        if _is pacman
            set PK "pacman"
        else if _is zypper
            set PK "zypper"
        else if _is dnf
            set PK "dnf"
        else if _is yum; and _is yumdownloader
            set PK "yum"
        else if _is apt
            set PK "apt"
        end
    end

    set -l CHECK (begin; command -v sudo > /dev/null 2>&1 && sudo -vn && sudo -ln; end 2>&1)
    set -l RA "no_root_access"
    if string match -qr '.*(may run|a password).*' "$CHECK"
        set RA "can_run_sudo"
    else if test (id -u) -eq 0; and test (whoami) = "root"
        set RA "is_root"
    end

    set -l WH $( [ -w ~ ] && echo true || echo false )

    printf '%s' "{\"os\": \"$OS\", \"pkg\": \"$PK\", \"shell\": \"fish\", \"root_access\": \"$RA\", \"writable_home\": $WH}"
end

  # _check_tmux is used in the install script post install!
function _check_tmux
    set -g TMUX ""

    if _is tmux
        set TMUX "tmux"
        _log SshTmuxInstaller "\"user\""
    else if _is $HOME/.warp/tmux/execute_tmux.sh
        set TMUX "$HOME/.warp/tmux/execute_tmux.sh"
        _log SshTmuxInstaller "\"warp\""
    end

    if test -n "$TMUX"
        command $TMUX -V | awk '{print $2}' | read VER;
        if test -z "$VER"
            _err "\"TmuxFailed\""
        else if test (printf '%s\n' "$VER" "2.9" | sort -V | tail -n1) = "2.9"
            set -l DETAILS (_system_details)
            _err "{\"UnsupportedTmuxVersion\": $DETAILS}"
        else;
          return 0
        end
    else;
        set -l DETAILS (_system_details)
        _err "{\"TmuxNotInstalled\": $DETAILS}"
    end
    return 1
end

_check_tmux; and command $TMUX -Lwarp -CC; and exit
