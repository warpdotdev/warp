# Reliable Ctrl+Tab switching on rapid release

GitHub issue: [#11221](https://github.com/warpdotdev/warp/issues/11221)

## Summary

When Ctrl+Tab is configured to cycle through recently used sessions or tabs, releasing Control must commit the intended destination and dismiss the switcher regardless of how quickly the shortcut is pressed and released. The same key sequence must produce the same result whether the switcher is already visible or is still appearing.

## Problem

A rapid Ctrl+Tab press and Control release can leave the MRU switcher visible without activating its selected destination. The user must press and release Control again to complete an action that should already have finished.

## Goals and non-goals

- Make rapid and deliberate Ctrl+Tab interactions produce the same final selection and closed state.
- Preserve the existing MRU ordering, forward/reverse cycling, and setting choices.
- Do not add or redesign UI, change shortcut defaults, or redefine which sessions and tabs are eligible.

## Figma

Figma: none provided. This is a timing and interaction-state correction with no new visual surface.

## Behavior

1. These invariants apply when the Ctrl+Tab behavior setting is either **Cycle most recent session** or **Cycle most recent tab**. The **Activate previous/next tab** setting continues to switch directly without showing the MRU switcher.

2. Pressing Ctrl+Tab while Control remains held opens the existing MRU switcher and advances its selection using the current ordering rules. No rows, labels, ordering rules, or other visual treatment change as part of this work.

3. Pressing Ctrl+Shift+Tab while Control remains held opens the same switcher and advances in the existing reverse direction.

4. Additional Tab or Shift+Tab presses while Control remains held continue moving the selection. Releasing Control commits exactly the destination selected by the complete sequence of presses.

5. Releasing the Control modifier commits the selected session or tab exactly once and closes the switcher. This must hold whether the release occurs:
   - after the switcher is visible;
   - immediately after the first Ctrl+Tab action, before the switcher becomes visible; or
   - while the switcher is appearing.

6. A rapid Ctrl+Tab tap selects the same destination that a slower press-and-release would select from the same starting MRU state. The switcher does not need to become noticeably visible during a very rapid tap, but it must not remain visible after Control is released.

7. After a successful commit, the chosen tab or session is active, its normal input target is focused, and neither the Ctrl+Tab switcher nor the regular command palette remains open.

8. When both physical Control keys are held, releasing only one of them does not commit the destination. Switching remains in progress until both Control keys have been released.

9. Modifier changes unrelated to Control do not commit or dismiss the Ctrl+Tab switcher.

10. If the switcher was already canceled or closed before Control is released, that later release does not activate a destination or reopen the switcher. This includes dismissal with Escape, clicking away, or another UI transition that closes overlays.

11. If there is no eligible destination to activate, releasing Control closes the switcher and leaves the current tab and session unchanged.

12. Selecting a switcher row through an existing direct interaction commits it once and closes the switcher; a subsequent Control release does not perform a second navigation.

13. Slow Ctrl+Tab interaction remains unchanged: the visible switcher continues to accept the highlighted result on Control release, and repeated forward and reverse cycling continues to wrap as it does today.

14. The corrected behavior applies to both MRU modes in every Warp window. A rapid release in one window affects only that window's active Ctrl+Tab interaction.

15. The fix introduces no new loading, error, empty, or accessibility UI. Existing accessible labels and focus behavior remain available when the switcher is visible.
