INSTALL_TMUX='set -e

_on_error() {
    local _msg=$(printf "{\"hook\": \"TmuxInstallFailed\", \"value\": { \"line\": \"$1\", \"command\": \"$2\" } }" | command -p od -An -v -tx1 | command -p tr -d " \n")
    printf '\''\033\120\044\144%s\234'\'' "$_msg"
    rm -rf "$HOME/.warp/tmux"
}
trap "_on_error \"\${LINENO}\" \"\$BASH_COMMAND\"" ERR

sudo apt update -y
sudo apt install -y tmux'

bash <<< "$INSTALL_TMUX" && _check_tmux && command tmux -Lwarp -CC && exit
