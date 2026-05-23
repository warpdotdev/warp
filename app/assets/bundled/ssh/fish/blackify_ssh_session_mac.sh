function _is
    command -v $argv[1] >/dev/null 2>&1
end

function _l
    set _m (printf "{\"hook\": \"%s\", \"value\": %s}" $argv[1] $argv[2] | od -An -v -tx1 | tr -d " \n")
    printf '\033\120\044\144%s\234' $_m
end

function _e
    _l RemoteWarpificationIsUnavailable $argv[1]
end

function _sd
    set -l PK ""

    if _is brew
      set PK "homebrew"
    end

    printf '{"os": "Darwin", "pkg": "%s", "shell": "fish", "root_access": "no_root_access", "writable_home": %s}' "$PK" $( [ -w ~ ] && echo true || echo false )
end

  # _check_tmux is used in tmux install script post install!
function _check_tmux
    set -g TMUX "$HOME/.warp/tmux/execute_tmux.sh"
    if _is "$TMUX"
        _l SshTmuxInstaller "\"warp\""
    else if _is tmux
        set TMUX "tmux"
        _l SshTmuxInstaller "\"user\""
    end

    if test -n "$TMUX"
        $TMUX -V | awk '{print $2}' | read V;
        if test -z "$V"
            _e "\"TmuxFailed\""
        else if test (printf '%s\n' "$V" "2.9" | sort -V | tail -n1) = "2.9"
            set -l D (_sd)
            _e "{\"UnsupportedTmuxVersion\": $D}"
        else;
          return 0
        end
    else;
            set -l D (_sd)
        _e "{\"TmuxNotInstalled\": $D}"
    end
    return 1
end

_check_tmux; and $TMUX -Lwarp -CC; and exit
