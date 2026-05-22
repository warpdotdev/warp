# This file is executed via `nu --execute` before Warp sources the full Nushell
# bootstrap. Keep each statement semicolon-friendly because Rust compacts this file.
$env.WARP_SESSION_ID = ((random int 1000000000..9999999999) | into string)
let _hostname = (try { ^hostname | str trim } catch { "" })
let _user = (try { ^whoami | str trim } catch { ($env.USER? | default "") })
$env.WARP_USING_WINDOWS_CON_PTY = "@@USING_CON_PTY_BOOLEAN@@"
let _msg = ({ hook: "InitShell", value: { session_id: ($env.WARP_SESSION_ID | into int), shell: "nu", user: $_user, hostname: $_hostname } } | to json -r | ^od -An -v -tx1 | ^tr -d ' \n')
if $env.WARP_USING_WINDOWS_CON_PTY == "true" { print -n $"(ansi escape)]9278;d;($_msg)(char bel)" } else { print -n $"(ansi escape)P$d($_msg)(ansi escape)\\" }
