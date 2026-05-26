# Each line starts with a leading space to leave these commands out of history.
#
# command -p resolves the given command with the system default PATH, ensuring the shell
# can find them even if the user has a clobbered PATH value.
 unsetopt ZLE
 WARP_SESSION_ID="$(command -p date +%s)$RANDOM"
 _hostname=$(command -pv hostname >/dev/null 2>&1 && command -p hostname 2>/dev/null || command -p uname -n)
 _user=$(command -pv whoami >/dev/null 2>&1 && command -p whoami 2>/dev/null || echo $USER)
 _msg=$(printf "{\"hook\": \"InitShell\", \"value\": {\"session_id\": $WARP_SESSION_ID, \"shell\": \"zsh\", \"user\": \"%s\", \"hostname\": \"%s\"}}" "$_user" "$_hostname" | command -p od -An -v -tx1 | command -p tr -d " \n")
 WARP_USING_WINDOWS_CON_PTY=@@USING_CON_PTY_BOOLEAN@@
 # We send the InitShell hook via OSCs when on Windows and via DCSs otherwise.
 if [ "$WARP_USING_WINDOWS_CON_PTY" = true ]; then printf '\e]9278;d;%s\x07' "$_msg"; else printf '\x1b\x50\x24\x64%s\x1b\x5c' "$_msg"; fi
 unset _hostname _user _msg
