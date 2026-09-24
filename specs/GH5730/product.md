# Support Elvish shell — Product Spec

GitHub issue: https://github.com/warpdotdev/warp/issues/5730
Figma: none provided

## Summary

Warp recognizes [Elvish](https://elv.sh) as a supported local shell alongside zsh, bash, fish and PowerShell. Users who pick Elvish as their startup shell (or whose login shell is Elvish) get Warp blocks, the Warp input editor, prompt handling and history, instead of the current behavior where an Elvish login shell is rejected and Warp silently falls back to another supported shell (`ShellStarterSource::Fallback` in `app/src/terminal/local_tty/terminal_manager.rs`).

## Goals

- Elvish can be selected as a startup shell and as a new-session shell, and is auto-detected when it is the login shell.
- A local Elvish session bootstraps into full block mode: each command produces its own block with correct exit status, duration and working directory.
- The user's `rc.elv` (functions, variables, aliases defined as functions, prompt) is loaded and usable from the Warp input editor.
- The user's Elvish prompt is honored when "Honor PS1" is on, and hidden in favor of the Warp prompt when it is off.

## Non-goals

- Warpifying Elvish over SSH or in subshells (`elvish` typed into a bash session, docker/podman exec). Follow-up; the local path must land first.
- Native Elvish tab completions (`edit:completion:arg-completer`) surfaced in Warp's completion menu. Warp's own command signatures still work; native-completion passthrough is a follow-up.
- Elvish on Windows. Elvish runs there, but Warp's Windows shell path (ConPTY, MSYS2) is out of scope.
- Importing Elvish's history database (`~/.local/state/elvish/db.bolt`) into Warp's history search. Elvish's history is a bbolt database, not a text file; Warp's own per-session history still works.
- Syntax highlighting of Elvish-specific syntax in the input editor.

## Behavior

1. When `elvish` is found on `PATH`, it appears in Settings → Features → Session → "Startup shell for new sessions" and in the new-tab shell menu, labeled **Elvish**.
2. When the user's login shell (`$SHELL` / directory services) is an `elvish` binary, "Default" resolves to Elvish.
3. Opening a new tab with Elvish shows the normal bootstrap (no raw script echo) and lands in the Warp input editor within the same time budget as fish.
4. `rc.elv` is sourced exactly once per session. Functions, variables and `use`d modules defined there are callable from commands typed in Warp.
5. Each submitted command creates one block. The block records the command text, the exit status (`0` on success, the external command's exit code on `external-cmd-error`, `1` for any other Elvish exception), and the working directory after the command.
6. A command that throws an Elvish exception shows Elvish's exception trace in the block output and marks the block as failed.
7. With "Honor PS1" on, the block header shows the user's `edit:prompt` / `edit:rprompt` output. With it off, Warp's prompt is shown and the Elvish prompt is suppressed. Toggling the setting takes effect on the next prompt without restarting the session.
8. `clear` in an Elvish session clears the block list, same as in other shells.
9. Command-line environment variable prefixes added through Warp (e.g. "Run with env vars") are rendered in Elvish syntax (`env FOO=bar cmd` or `tmp E:FOO = bar; cmd`), not POSIX `FOO=bar cmd`.
10. AI-generated and workflow commands that Warp chains with `&&` / `;` use Elvish-compatible separators (`and`/`or` are not valid; commands are separated with `;` and failure short-circuits via exceptions, which is Elvish's default).
11. Context chips that run a shell command (git branch, cwd, node version, etc.) work in Elvish sessions.
12. If Elvish is older than the minimum supported version (0.17: `edit:after-command` landed in 0.16 and the XDG `~/.config/elvish/rc.elv` path in 0.17), Warp does not attempt to bootstrap it and falls back to another supported shell, the same way an unsupported login shell is handled today, and logs the `UnsupportedShell` telemetry event with the version.
13. Closing the tab or typing `exit` ends the session normally with no hung process.
14. Existing zsh, bash, fish and PowerShell behavior is unchanged.

## Open questions

- Should Elvish-over-SSH warpification be tracked as a separate issue once this lands?
