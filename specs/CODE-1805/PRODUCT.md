# TUI Shell Command Execution (`!` shell mode)

Linear: [CODE-1805](https://linear.app/warpdotdev/issue/CODE-1805/shell-command-execution)

## Summary

The TUI input currently only sends prompts to the agent; there is no way to run a shell command. Typing `!` at the start of the input enters a shell mode — visually mirroring the GUI's `!` shell mode — where submitting executes the text as a shell command in the session's PTY and renders the result as a terminal block in the transcript.

## Figma

Figma: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=323-23270&m=dev (shows the hint-line placement below the input, where the `← for conversations` hint appears; the shell-mode callout occupies that spot).

## Non-goals

- Command history recall (`↑`/`↓`) in shell mode.
- A keybinding toggle for shell mode (the GUI's `⌘I` equivalent). `!`, backspace, and esc are the only mode controls.
- Natural-language autodetection between shell and agent input (the GUI's unlocked/autodetect modes).
- Cloud handoff (`&`) prefix in the TUI.
- Placeholder/ghost text in shell mode.

## Behavior

### Entering shell mode

1. Typing `!` when the cursor is at the very start of the input enters shell mode. This works whether the input is empty or already contains text (existing text becomes the pending shell command). This matches the GUI's trigger.
2. The `!` is never inserted into the text buffer. It renders as a UI affordance: a blue `!` followed by a space at the start of the input area. It cannot be selected, highlighted over, or have the cursor placed on or before it. Select-all selects only the command text.
3. Only a literally typed `!` triggers shell mode. Pasting text that begins with `!` inserts it as ordinary text and does not change modes. (Matches the GUI, which only activates on typed input.)
4. Typing `!` anywhere other than the start of the input inserts a literal `!` character.

### While in shell mode

5. The input box border turns blue — the same theme token the GUI uses for shell mode (`ansi_fg_blue`), replacing the default border color.
6. The `!` affordance renders in the same blue.
7. A callout renders in the hint line below the input box (the spot the `← for conversations` hint occupies in the Figma design): `shell mode · esc to exit`, styled in the same blue. When not in shell mode, that hint line reverts to whatever it would otherwise show (currently nothing).
8. All normal text editing (cursor movement, selection, kill/yank, undo, multiline via Shift+Enter, wrapping, scrolling) works on the command text exactly as in agent mode. The editable text area is inset by the affordance width so wrapped rows align under the first character of the command.
9. Shell mode is a property of the input only; it does not affect a running agent, the transcript, or conversation selection.

### Exiting shell mode

10. Backspace with the cursor at the very start of the text exits shell mode (deleting the `!` affordance). Any typed command text is preserved and becomes a normal agent prompt.
11. Esc exits shell mode the same way: the affordance is removed and any typed text is preserved as agent-prompt text.
12. Exiting shell mode restores the default border color and hint line.

### Submitting

13. Enter with non-empty command text executes the text as a shell command in the session's PTY, exactly as if the user had typed it in a normal terminal: it starts a new command block, which renders as a terminal block in the transcript (same rendering as agent-executed commands), streams output live, and records completion state.
14. The command is a plain user command: it is not sent to the agent, and it is not attached to any conversation as AI context. (Matches the GUI, where `!` commands execute with user provenance.)
15. After a successful submission the input clears and exits shell mode, returning to agent input. Running a second command requires typing `!` again.
16. Enter with an empty command (just the affordance) is a no-op: nothing is submitted and the input stays in shell mode.
17. Whitespace-only command text is treated as empty (no-op).

### Interaction with a running agent

18. Shell mode can be entered at any time, including while an agent conversation is streaming or executing tools.
19. Submitting a shell command while a conversation is in progress cancels that conversation (same as the GUI: the conversation ends as cancelled; it is not paused or queued). The command then executes normally.
20. If the agent currently holds the PTY with an active long-running command, submission is blocked: the shell command does not execute, the input retains its text, and a transient message appears in the hint line (e.g. `cannot run — command already running`). The running agent command is unaffected.
21. Transient hint-line messages display for ~3 seconds, then the hint line reverts to its persistent content (the shell-mode callout while in shell mode). This transient-message treatment is a repeatable pattern: future features will surface short-lived notices in the same spot.

### Invariants

22. Agent-requested command execution (tool calls bridged to the PTY) is unchanged by this feature.
23. Agent prompts submitted while not in shell mode behave exactly as today.
24. All colors come from the active theme; no hard-coded colors.
