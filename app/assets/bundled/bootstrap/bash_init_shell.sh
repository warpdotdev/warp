# command -p resolves the given command with the system default PATH, ensuring the shell
# can find them even if the user has a clobbered PATH value.
command -p stty raw
HISTCONTROL=ignorespace
HISTIGNORE=" *"
WARP_SESSION_ID="$(command -p date +%s)$RANDOM"
_hostname=$(command -pv hostname >/dev/null 2>&1 && command -p hostname 2>/dev/null || command -p uname -n)
_user=$(command -pv whoami >/dev/null 2>&1 && command -p whoami 2>/dev/null || echo $USER)
if [[ "$OS" == Windows_NT ]]; then WARP_IN_MSYS2=true; else WARP_IN_MSYS2=false; fi
# If we're in MSYS2, we want to send the hook via key-value pairs.
if [ "$WARP_IN_MSYS2" = true ]; then _msg="\e]9278;k;A;InitShell\a\e]9278;k;B;session_id;$WARP_SESSION_ID\a\e]9278;k;B;shell;bash\a\e]9278;k;B;user;$_user\a\e]9278;k;B;hostname;$_hostname\a\e]9278;k;C\a"; else _msg=$(printf "{\"hook\": \"InitShell\", \"value\": {\"session_id\": $WARP_SESSION_ID, \"shell\": \"bash\", \"user\": \"%s\", \"hostname\": \"%s\"}}" "$_user" "$_hostname" | command -p od -An -v -tx1 | command -p tr -d " \n"); fi
WARP_USING_WINDOWS_CON_PTY=@@USING_CON_PTY_BOOLEAN@@
# We send the InitShell hook via OSCs when on Windows and via DCSs otherwise.
if [ "$WARP_USING_WINDOWS_CON_PTY" = true ]; then if [ "$WARP_IN_MSYS2" = true ]; then printf "$_msg"; else printf '\e]9278;d;%s\x07' "$_msg"; fi; else printf '\x1b\x50\x24\x64%s\x1b\x5c' "$_msg"; fi
unset _hostname _user _msg
