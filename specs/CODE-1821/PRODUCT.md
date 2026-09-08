# PRODUCT: Warp TUI Conversation Management (CODE-1821)

Linear: [CODE-1821 — Conversation management](https://linear.app/warpdotdev/issue/CODE-1821/conversation-management)

## Summary

The Warp TUI provides a searchable inline `/conversations` menu for switching its single conversation surface to another user-owned Oz conversation. The menu combines local and cloud history, and selection safely loads and validates the requested conversation before replacing the current transcript.

## Figma

Figma: none provided. The existing TUI inline-menu styling is the visual reference.

## Goals

- Let users discover and continue their local and cloud Oz conversations without leaving the TUI.
- Preserve the TUI's one-session, one-visible-conversation model.
- Match the GUI inline conversation menu's core aggregation, deduplication, recency, and search semantics where they apply to the TUI.
- Keep failed, cancelled, or blocked switches from damaging the current conversation.

## Non-goals

- Displaying non-Oz or third-party CLI-agent conversations.
- Attaching to or following a cloud conversation that is still running.
- Supporting multiple simultaneous TUI conversation surfaces, panes, tabs, or windows.
- Adding conversation deletion, renaming, status display, timestamps, management actions, or a current-directory tab.
- Preserving per-conversation viewport position when switching away and back.

## Behavior

### Opening and browsing

1. The TUI supports the `/conversations` slash command. Selecting or submitting the command clears the slash-command text and opens the inline conversation menu.

2. The menu uses the existing TUI inline-menu visual treatment and keyboard behavior. It renders above the input, uses the active theme, supports up/down navigation, accepts the selected row with Enter, and closes with Escape.

3. The menu may be opened and browsed while a terminal command or the current conversation is in progress. Opening the menu never cancels work and never changes the selected conversation.

4. Until the initial local-and-cloud conversation aggregation finishes, the menu shows its loading state instead of a partial result set.

5. If cloud conversation metadata cannot be loaded, the loading state ends, locally available conversations remain searchable, and the transient message area shows:
   `Could not load cloud conversations. Showing local conversations only.`

6. Opening or reopening the menu after a cloud metadata failure continues to show local conversations and does not start a separate cloud refresh.

### Conversation universe

7. The menu contains user-owned Oz conversations that can be restored from at least one of:
   - Conversation data already loaded in the current process.
   - Locally persisted conversation data.
   - Cloud conversation metadata with a server conversation token.

8. Local, cloud, ambient-task-backed, live-in-memory, cleared-from-the-current-surface, and persisted records referring to the same underlying conversation appear as one entry.

9. The currently selected conversation is omitted.

10. The following entries are omitted:
    - Empty conversations that are not currently selected.
    - Child-agent conversations.
    - Internal, passive-only, shared-session-viewer, and transcript-viewer conversations.
    - Non-Oz conversations.
    - Entries without a local conversation identity or server conversation token that the TUI can restore.

11. A target conversation that is queued, pending, claimed, blocked, or actively running is unavailable and omitted. It becomes eligible when its status changes to a safely restorable terminal state.

12. Each row displays only the conversation title. The TUI does not show status, timestamps, open-pane indicators, working directory, harness, artifacts, or other management metadata.

### Ordering, search, and live updates

13. With an empty query, the menu shows at most the 50 most recently updated eligible conversations.

14. Typing while the conversation menu is open updates a menu-only fuzzy title query. The query is not submitted as a prompt.

15. Search uses the same case-insensitive fuzzy title matching and score threshold as the GUI inline conversation menu. Search returns at most 500 results.

16. Clearing the query restores the 50-entry recent-conversation view.

17. When no eligible conversations or search matches exist, the menu shows an explicit empty state rather than an empty bordered panel.

18. While the menu is open, local-history and cloud/RTC updates refresh the current query.

19. Refreshes preserve selection by the entry's stable identity. If the selected entry disappears, the nearest remaining selectable row becomes selected.

20. A title or status update may update, add, or remove an entry. Rows reorder only when the conversation's last-updated value changes or when the search score changes.

### Selecting a conversation

21. Pressing Enter revalidates the current TUI state and the selected target. Validation at menu-open time is not sufficient.

22. If a foreground terminal command is running, the switch is rejected, the menu remains open, and the transient message area shows:
    `Cannot switch conversations while a command is in progress.`

23. If the current conversation is in progress or blocked, the switch is rejected, the menu remains open, and the transient message area shows:
    `Cannot switch conversations while the current conversation is in progress.`

24. If another conversation restoration is already loading, the switch is rejected and the transient message area shows:
    `Another conversation is already loading.`

25. Rejected selection never cancels the command or conversation, clears the transcript, changes selection, or starts a load.

26. An accepted row closes the menu, clears its search query, and enters the same conversation-restoration loading state used by startup `--resume`.

### Loading and replacement

27. List-originated loading keeps the current transcript visible but disables prompt and command submission until loading succeeds, fails, or is cancelled.

28. A local restoration target loads from memory or local persistence and works offline.

29. A server-token restoration target uses the existing local-first, server-fallback behavior.

30. Local and server restoration produce the same visible transcript and continuation behavior.

31. Loading and validation complete before any part of the current transcript, selection, action state, or command history is removed.

32. A successful load atomically replaces the sole TUI conversation surface:
    - The old conversation's visible transcript and conversation-derived command blocks are removed.
    - The selected conversation's visible prompts, agent responses, supported reasoning, tool calls and recorded results, and conversation-derived command blocks are restored in their original order.
    - No restored command or tool action is executed.
    - The restored conversation becomes the target of the next prompt.
    - The viewport moves to the newest content and input regains focus.

33. Switching away does not delete the old conversation. Once eligible, it appears in the menu and can be selected again.

34. Previously loaded and newly loaded targets use the same restoration behavior as the GUI.

35. Only one list-originated restoration may run at a time. Late completion from a cancelled or superseded request must not replace the visible conversation.

36. If loading or validation fails, the current transcript and selection remain unchanged, submission becomes available again, and the transient message area shows a concise error.

37. A non-Oz result is treated as unsupported and does not replace the current conversation.

### Cancellation

38. While a list-originated restoration is loading, Escape or Ctrl-C cancels the request and returns to the previous conversation without showing an error.

39. While startup `--resume` restoration is loading, Escape or Ctrl-C cancels the request and continues into the provisional new TUI session as though `--resume` had not been supplied.

40. The startup loading screen shows a visible hint:
    `Esc or Ctrl-C to cancel and start a new session`

41. Cancelling startup restoration does not print or retain a resume hint for the requested conversation.

42. After either cancellation path, a late loader result is ignored and cannot replace the active conversation.
