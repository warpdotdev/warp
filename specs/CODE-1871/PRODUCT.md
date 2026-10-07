# Up-arrow prompt history in the TUI (CODE-1871)
## Summary
Give the headless TUI (`crates/warp_tui`) the same up-arrow prompt-history recall the GUI has: pressing Up while the cursor is on the top line of the prompt input opens an inline menu of the user's previous agent prompts, searchable/filterable by what's already typed, with selection previewing into the input and dismissal restoring the buffer the user started with.
## Problem
In the TUI today, pressing Up runs the cursor-movement action `MoveUp`. When an inline menu (slash commands, `/conversations`, model, skills, MCP) is already open, Up cycles that menu's selection. When no menu is open, Up moves the cursor up one visual row, which is a no-op once the cursor is already on the first visual row of the input. There is no way to recall a previous prompt from the keyboard. The GUI, by contrast, opens an inline prompt-history menu when Up is pressed with the cursor on the first row. This spec closes that gap for the TUI's agent prompt input.
## Goals
- Keyboard recall of previous agent prompts in the TUI via Up arrow, reusing the existing TUI inline-menu surface.
- Behavioral parity with the GUI's prompt-history search/filter and buffer-restoration semantics.
- Reuse the shared prompt-history model and the GUI's ordering, de-duplication, and filtering semantics rather than introducing a second implementation.
## Non-goals
- Command (shell) history and conversation entries in the same menu. The GUI's inline history menu also surfaces shell commands and conversations behind tabs; this ticket scopes the TUI menu to agent **prompts** only. Tabs and command/conversation rows are out of scope.
- Changing GUI behavior.
- Writing or changing prompt-history persistence. Persistence of TUI-submitted prompts is provided by the prerequisite change in [`../CODE-1871-prompt-persistence/TECH.md`](../CODE-1871-prompt-persistence/TECH.md); this feature only reads available history.
## Behavior
### Opening
1. When the prompt input is focused, no inline menu is open, the cursor is a single caret (no active text selection) positioned on the **first visual row** of the input buffer, and the user presses Up (`↑` or `Ctrl+P`), the prompt-history inline menu opens. "First visual row" accounts for soft-wrapping: for a soft-wrapped single logical line, only the topmost wrapped row counts as the first row.
2. When the cursor is on any row other than the first visual row, Up moves the cursor up one visual row exactly as it does today, and the menu does not open.
3. When any inline menu is already open (slash commands, `/conversations`, model, skills, MCP, or this prompt-history menu), Up cycles that menu's selection and does not trigger the open logic in (1). Only one inline menu is open at a time; the prompt-history menu cannot open on top of another menu.
4. Whatever text is in the input when the menu opens becomes the menu's initial search query and is preserved as the "original buffer" for later restoration (see Dismissal). Opening with an empty input opens the menu with an empty query.
5. The menu opens anchored to the input, rendered in the same inline-menu region and visual style used by the existing TUI menus (bordered list above the input, capped at the shared maximum visible row count, with a header title identifying it as prompt history).
6. No feature flag: the prompt-history menu ships enabled by default. Once the conditions in (1) are met, Up opens the menu unconditionally.
### Contents, ordering, and search
7. The menu lists the user's previous agent prompts drawn from the same prompt-history data the GUI uses. Prompts from the current terminal session and prompts from other sessions are both included.
8. Prompts are de-duplicated by text: repeated identical prompts appear once, keeping the most recent occurrence.
9. Ordering matches the GUI's up-arrow ordering: prompts from other sessions appear before prompts from the current session, and within each group prompts are ordered oldest-first, so the most recently used current-session prompt sits at the bottom of the list, closest to the input.
10. On open, the selection defaults to the item nearest the input (the last/most-recent row) and that prompt is immediately previewed in the input, matching the GUI. Pressing Up moves the selection toward older prompts; pressing Down moves it toward newer prompts.
11. Typing in the input filters the list. Filtering is prefix-based and matches the GUI: an item is shown only when its prompt text starts with the trimmed query. Clearing the query shows the full (de-duplicated) list again.
12. Editing the query preserves a stable selection where possible: if the previously selected prompt still matches, it stays selected; otherwise selection falls back to the nearest valid row. If no prompts match the query, the menu shows an empty state and there is nothing to accept.
13. Whitespace-only and empty prompts are never shown.
### Selection preview
14. On open and as the selection changes (via Up/Down), the currently highlighted prompt is previewed in the input buffer, replacing the visible input text with the highlighted prompt so the user can see the full prompt they are about to accept. Previewing does not alter the menu's active search query — continuing to move the selection keeps filtering against the query the user actually typed, not the previewed text.
15. Preview writes do not push undo history the user could accidentally "undo" into a broken state; moving off a preview and dismissing restores cleanly (see Dismissal).
### Acceptance
16. Pressing Enter with a prompt highlighted accepts it: the input buffer is set to the selected prompt's text and the prompt is submitted immediately, matching the GUI's accept-a-prompt-from-history behavior. After acceptance the menu closes.
17. Accepting is only possible when a selectable prompt is highlighted. With an empty/filtered-to-nothing list, Enter does not accept a prompt; it behaves as a normal submit of whatever is in the input (which, with the menu open over an empty query, means submitting empty is a no-op as it is today).
### Dismissal and buffer restoration
18. Pressing Escape with the prompt-history menu open closes the menu and restores the input buffer to the exact text the user had before opening the menu (the "original buffer" from invariant 4), discarding any preview text. The cursor returns to a sensible position within the restored text (end of the restored buffer is acceptable).
19. Escape closes only the most-local surface: with the prompt-history menu open, one Escape closes the menu (and restores the buffer); it does not also exit shell mode or cancel a restore in the same keypress. This matches the existing TUI Escape priority order where inline menus are dismissed first.
20. Pressing Down past the last (newest) item closes the menu and restores the original buffer, matching the GUI (the newest item is at the bottom, and moving "down and out" returns to the live input). When the list is empty, pressing Down also closes the menu and restores the original buffer. Selecting a conversation/command is not applicable here since only prompts are listed.
21. Any event that would normally dismiss inline menus (e.g. accepting a different action, starting a conversation, losing input focus, a running process taking over the input) also closes the prompt-history menu and restores the original buffer rather than leaving preview text stranded in the input.
### Edge cases
22. Empty history: opening the menu when the user has no prior prompts shows an explicit empty state (e.g. "No prompt history") rather than an empty bordered box with no explanation. Escape closes it and restores the (empty or typed) buffer.
23. Opening with typed text that matches nothing shows the empty/"no matches" state; Escape restores the typed text exactly.
24. Multi-line input: if the input contains multiple lines and the cursor is on the first visual row, Up opens the menu (per invariant 1); accepting a prompt replaces the entire multi-line buffer with the selected prompt. Dismissing restores the full original multi-line buffer.
25. Shell mode (`!`): the prompt-history menu is an agent-prompt feature. When the input is in `!` shell mode, Up does not open the agent prompt-history menu. (Shell command history is out of scope for this ticket.)
26. While a process owns the input (alt-screen app or an inline long-running command under user control), Up is forwarded to the process and the menu does not open, consistent with how other TUI input handling is suppressed in that state.
27. History updates while the menu is open (e.g. a background conversation records a new prompt) refresh the list in place without losing the user's current selection where the selected prompt still exists.
28. The menu never opens on top of, or simultaneously with, another inline menu; opening it while another menu is visible is not possible because Up is consumed by the visible menu first (invariant 3).
