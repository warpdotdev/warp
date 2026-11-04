# Per-Directory Command History — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/5336
## Summary
Add an opt-in mode that scopes the up/down arrow command history to the current working directory, mirroring the zsh `per-directory-history` plugin the issue references. When enabled, pressing Up in the terminal input recalls only commands previously run from the directory the session is currently in, and a keyboard toggle (default `Ctrl+G`) switches back to full global history at any time. AI prompt history is unaffected. The behavior change is implemented once in the shared up-arrow combiner so the GUI inline history menu, the GUI history search, and the headless TUI history menu all behave identically.
## Problem
Users who work across multiple project directories (the issue cites git workflows inside per-project directories) get little value from Up-arrow history: the list mixes commands from unrelated directories, so the useful entries for the current project drown in noise. The zsh community solves this with `per-directory-history` (https://github.com/jimhester/per-directory-history), which scopes history navigation to the current directory and reserves `^G` for switching to global history. Related, overlapping requests: #2193, #2582, and #4987. Warp already records the working directory of every command run inside Warp (`HistoryEntry.pwd`), so the data needed for scoping exists; it is simply not used to filter up-arrow navigation.
## Goals
- Opt-in per-directory command history for up/down arrow navigation (default off).
- Global history remains reachable instantly via a toggle (default `Ctrl+G`), matching the reference plugin's muscle memory.
- One shared implementation in the up-arrow combiner so GUI and TUI surfaces stay in lockstep.
- No behavior change when the setting is off, and no changes to history storage or persistence.
## Non-goals
- Filtering the Ctrl+R Command Search panel (listed as a follow-up; it has its own filter-chip UX).
- Fuzzy or canonicalized directory matching — v1 compares the recorded `pwd` strings exactly.
- A standalone directory-history box (that is #2582, a different feature).
- Changing ordering, de-duplication, ignored-suggestions, or agent-command-history rules.
- Changing how history is read from the shell history file or persisted to sqlite.
## Behavior
1. With `directory_scoped_history` off (the default), up/down arrow history behaves exactly as it does today on every surface: the GUI inline history menu, the GUI history search, and the TUI history menu.
2. A new setting `directory_scoped_history` (default off) is available in Settings and in the settings file under `terminal.input.directory_scoped_history`. Changing it takes effect for the next history navigation, with no restart or session reload.
3. With the setting on and a current directory known for the active session, the command portion of up-arrow history contains only commands whose recorded `pwd` equals the session's current working directory.
4. With the setting on, commands recorded without a directory (imported from the shell history file and never executed in Warp) never appear in the scoped list — they cannot be attributed to any directory.
5. AI prompt history is never directory-filtered; prompts have no directory metadata. With the setting on, prompts continue to appear alongside scoped commands following the existing rules (Agent Mode enabled, prompt inclusion settings, ignored suggestions).
6. All existing result processing applies unchanged to the scoped list: ordering (items from other sessions precede current-session items, oldest-first within each group), de-duplication keeping the newest occurrence, exclusion of ignored suggestions, exclusion of agent-executed commands when that setting is off, and prefix filtering of the open menu while typing.
7. With the setting on but no current directory known yet (for example, a freshly bootstrapped session before the first prompt), directory scoping is skipped for that navigation and the full history is shown. Scoping degrades to a no-op rather than to an empty list.
8. When the scoped list is empty (no commands recorded for this directory), the history menu opens with the existing "No history" empty state, and a query that matches nothing shows the existing "No matching history" state. Down past the newest item closes the menu as it does today.
9. A toggle action with default binding `Ctrl+G` (rebindable via the Keybindings settings) flips directory scoping on/off while the input is focused. The next history navigation reflects the new scope immediately. The toggle updates the same persisted setting, so it survives restart and is reflected in Settings.
10. Scoping never leaks across hosts: history results are already scoped to the session's shell host, and the directory comparison uses that session's current working directory (for remote/SSH sessions, the remote path string of that session).
11. Accepting an item from the scoped list behaves exactly as today: commands fill the input and execute, prompts fill and submit, and workflow-linked metadata is preserved.
12. While a process owns the input (alt-screen applications), Up continues to be forwarded to the process and the history menu does not open, regardless of the setting.
## Open questions
- Should the default be off (proposed here) or on? zsh users coming from the plugin expect opt-in; making it on-by-default changes muscle memory for all existing users.
- Is `Ctrl+G` acceptable as the default toggle binding? It mirrors the reference plugin, but conflicts with the emacs-style "cancel" habit some keybinding presets encourage. The binding is user-rebindable either way.
