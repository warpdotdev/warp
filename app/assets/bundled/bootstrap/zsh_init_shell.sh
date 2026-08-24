# Each line starts with a leading space to leave these commands out of history.
#
# command -p resolves the given command with the system default PATH, ensuring the shell
# can find them even if the user has a clobbered PATH value.
#
# Put the tty into raw mode (disabling canonical line buffering and local echo) before we
# paste the multi-KB bootstrap script into the PTY, mirroring our approach in bash (see
# bash_init_shell.sh). Without this, the tty's default canonical/echo settings remain in
# effect for the entire paste; if the user's .zshrc reconfigures ZLE widgets (e.g. a vi-mode
# cursor-shape snippet calling 'bindkey -v', or a framework like prezto), any bootstrap bytes
# that haven't been consumed yet can be echoed back or picked up as typeahead once the line
# editor is re-enabled, leaking Warp's own bootstrap script onto the screen. We capture the
# user's actual pre-bootstrap tty settings here (rather than assuming a generic profile), so
# that zsh.sh can restore those exact settings right before we eval the bootstrap logic,
# instead of clobbering any of the user's own stty customizations (e.g. -ixon, a custom erase
# key, etc.) with a one-size-fits-all reset.
#
# Note: on GNU/Linux, the 'raw' alias for stty does not itself imply '-echo'
# (unlike BSD/macOS), so we disable echo explicitly to get the same hermetic
# behavior on every platform.
 WARP_ORIGINAL_STTY_STATE=$(command -p stty -g)
 command -p stty raw -echo
 unsetopt ZLE
 WARP_SESSION_ID=@@WARP_SESSION_ID@@
 _hostname=$(command -pv hostname >/dev/null 2>&1 && command -p hostname 2>/dev/null || command -p uname -n)
 _user=$(command -pv whoami >/dev/null 2>&1 && command -p whoami 2>/dev/null || echo $USER)
 _msg=$(printf "{\"hook\": \"InitShell\", \"value\": {\"session_id\": $WARP_SESSION_ID, \"shell\": \"zsh\", \"user\": \"%s\", \"hostname\": \"%s\"}}" "$_user" "$_hostname" | command -p od -An -v -tx1 | command -p tr -d " \n")
 WARP_USING_WINDOWS_CON_PTY=@@USING_CON_PTY_BOOLEAN@@
 # We send the InitShell hook via OSCs when on Windows and via DCSs otherwise.
 if [ "$WARP_USING_WINDOWS_CON_PTY" = true ]; then printf '\e]9278;d;%s\x07' "$_msg"; else printf '\x1b\x50\x24\x64%s\x1b\x5c' "$_msg"; fi
 unset _hostname _user _msg
