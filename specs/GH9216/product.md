# PRODUCT.md — Configurable `clear` command behavior

**GitHub Issue:** [warpdotdev/warp#9216](https://github.com/warpdotdev/warp/issues/9216)

## Summary

Add a user setting that controls how Warp responds when the shell requests a clear-screen operation, including the common `clear` command. Warp keeps the current behavior by default, but users can opt into a history-deleting behavior that makes `clear` remove earlier session output from the current terminal view.

## Problem

Warp currently treats `clear` like a viewport clear: it inserts blank vertical space so the next prompt starts at the top of the view, while previous blocks remain scrollable above the clear point. Some users expect `clear` to behave like a session-history reset so they can scroll to the top of the latest command output without passing older blocks.

## Goals

1. Let users choose between preserving session history and deleting session history when `clear` runs.
2. Preserve Warp's current behavior as the default so existing users do not lose scrollback unexpectedly.
3. Make the behavior explicit in settings and persistent across sessions.
4. Ensure the history-deleting mode gives users a reliable "fresh top of session" after `clear`, especially before reading long output such as `git diff`.

## Non-goals

1. Changing the default behavior of `clear`.
2. Removing or changing the existing "Clear Buffer" shortcut (`Cmd+K` on macOS, `Ctrl+Shift+K` on Windows/Linux).
3. Changing alternate-screen applications such as `vim`, `less`, `top`, or full-screen TUIs.
4. Changing terminal-standard saved-scrollback clearing sequences that already request saved history deletion explicitly.
5. Adding per-shell, per-profile, per-tab, or per-command configuration.

## Figma

Figma: none provided. This is a settings and terminal-behavior change with no bespoke visual mock.

## Behavior

1. Warp exposes a setting named **Clear command behavior** in the Terminal settings area.

2. The setting has exactly two choices:
   - **Preserve session history** — default. `clear` visually clears the current viewport while keeping earlier blocks available above the clear point.
   - **Delete session history** — `clear` removes earlier session output from the current terminal view so old blocks are no longer reachable by scrolling.

3. Existing users and new users default to **Preserve session history** unless they explicitly choose another value.

4. The selected value persists across app restarts and applies to new and existing terminal panes after the setting changes.

5. **Preserve session history** mode matches the current user-visible behavior:
   - Running `clear` moves the next prompt or output to a fresh viewport.
   - Previous blocks remain in the session and can still be reached by scrolling above the clear gap.
   - Block selection, copying older blocks, and search over retained blocks continue to work as they do today.

6. **Delete session history** mode removes session history that appeared before the clear request:
   - After `clear` completes, scrolling to the top of the current terminal view lands at the current prompt or the first output produced after the clear.
   - Blocks that were above the clear point are no longer visible, selectable, searchable, copyable from the block list, or attachable as AI context in that terminal view.
   - The active prompt remains usable and focused according to the same rules as before the clear.

7. In **Delete session history** mode, a long-running command that emits a clear-screen request deletes blocks before that active command but does not discard output emitted after the clear request. Subsequent output remains in the active command's block.

8. In **Delete session history** mode, if the current clear request occurs before any user command has produced visible output, the operation is effectively a no-op except for preserving a clean prompt/input state.

9. The setting applies to shell-originated clear-screen requests in the primary block-list terminal surface, including:
   - The common `clear` command.
   - Shell line-editor clear-screen hooks that Warp receives as a clear request.
   - Primary-screen full-screen erase sequences that Warp treats as the same clear-visible-screen operation.

10. The setting does not change alternate-screen behavior. Clearing inside a full-screen terminal application only affects that application screen and does not delete Warp block history behind it.

11. The setting does not change the explicit "Clear Buffer" action. That action continues to delete visible session blocks regardless of the `clear` command setting.

12. The setting does not change explicit saved-scrollback deletion requests, such as terminal sequences whose semantic meaning is already "clear saved history." Those continue to clear saved history according to existing terminal semantics.

13. When **Delete session history** removes blocks, any UI state that points at removed blocks is cleared or reset:
   - Selected blocks and text selections in removed blocks disappear.
   - Find results in removed blocks disappear.
   - Bookmarks and block-level affordances for removed blocks disappear.
   - AI context references to removed blocks are removed from the current terminal view.

14. The setting is safe to change while terminal sessions are open. A change affects the next clear request received by each terminal; it does not retroactively re-apply to previous clear requests.

15. Shared-session viewers see the clear behavior reflected in the shared session stream. A viewer's local setting does not rewrite history that the sharer has already preserved or deleted.

16. Restored sessions follow the same rule as live sessions for future clear requests. Switching to **Delete session history** and then running `clear` removes restored visible history from the current terminal view just like live history.

17. The settings file representation is documented through Warp's normal settings schema so users who edit settings directly can discover the setting and valid values.

## Success Criteria

1. With the default setting, running `clear` behaves the same as it does before this change: old blocks remain scrollable above a clear gap.
2. With **Delete session history** enabled, running `clear` removes older blocks so scrolling to the top reaches the current prompt or post-clear output.
3. Toggling the setting updates behavior in existing terminal panes without restarting Warp.
4. The "Clear Buffer" shortcut remains unchanged.
5. Alternate-screen applications do not delete block-list history when they clear their own screen.
6. Removed blocks do not remain in block selection, find results, bookmarks, copy output, or AI context UI.
7. The setting is visible in the settings UI and available in the settings schema with clear labels.

## Validation

1. Manual verification on Linux:
   - Run several commands with visible output.
   - Run `clear` with the default setting and verify old output remains scrollable above the clear gap.
   - Switch to **Delete session history**, run several commands, then run `clear` and verify old output is not reachable by scrolling.
   - Run `git diff` after `clear` and verify scrolling to the top lands at the start of the latest diff.
2. Regression verification on macOS and Windows/Linux keybindings:
   - Use "Clear Buffer" and confirm it still deletes blocks independently of this setting.
3. Alternate-screen verification:
   - Open `less` or `vim`, trigger an in-app clear/redraw, exit, and confirm prior Warp blocks remain governed only by the block-list setting and were not deleted by the TUI.
4. Settings verification:
   - Change the setting from the UI, restart Warp, and confirm the selection persists.
   - Change the setting through the settings file and confirm invalid values fall back through the existing settings error/default behavior.

## Open Questions

None outstanding.
