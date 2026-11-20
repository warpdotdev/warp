# Delete selected terminal blocks

GitHub issue: [https://github.com/warpdotdev/warp/issues/50](https://github.com/warpdotdev/warp/issues/50)
Figma: none provided

## Summary

Users can delete one or more selected terminal blocks from the current pane — via `⌘+Backspace` (macOS) / `Ctrl+Backspace` (Windows/Linux), or **Delete Selected Blocks** in the block context menu and the macOS **Blocks** menu — without clearing the rest of the session. The active (current input) block is never deleted. Deletion is persisted locally so restored sessions do not bring those blocks back.

This is a focused first cut of [#50](https://github.com/warpdotdev/warp/issues/50). Hide/collapse and purging commands from Ctrl+R history are out of scope.

## Problem

Warp groups every command and its output into a block that can be selected, copied, bookmarked, and shared — but not removed. Users who mistype a command, print a secret, or want to clean a demo session have only **Clear Blocks** (`⌘+K` / `Ctrl+Shift+K`), which wipes the entire pane. That is the wrong tool when they meant to delete the selection.

## Goals / Non-goals

**Goals:**

- Delete the selected completed blocks from the pane and from local session restore.
- Default shortcut is `⌘+Backspace` on macOS and `Ctrl+Backspace` on Windows/Linux, remappable as `terminal:delete_blocks`.
- Unmodified Backspace always edits the input, even when blocks are selected.
- With no deletable selection, `⌘+Backspace` / `Ctrl+Backspace` keep their existing editor meaning (delete all left on macOS, delete word left on Windows/Linux).
- **Clear Blocks** is unchanged.

**Non-goals:**

- Hide or collapse ([#23](https://github.com/warpdotdev/warp/issues/23)).
- Removing the command from Ctrl+R / shell history.
- Deleting the active/current input block.
- Deleting shared-block permalinks on the server.

## Behavior

1. With at least one **non-active** block selected, `⌘+Backspace` (macOS) or `Ctrl+Backspace` (Windows/Linux) deletes those selected blocks (skipping the active block if it is also selected). Unmodified Backspace still edits the input.

2. With no block selection, or with **only** the active block selected, `⌘+Backspace` / `Ctrl+Backspace` keep their existing editor behavior. The active block is never removed; selecting only it is a no-op for delete.

3. The block context menu (overflow, right-click, or keyboard) and the macOS **Blocks** menu show **Delete Selected Blocks**, with the current `terminal:delete_blocks` shortcut label, only when at least one non-active block is selected. Choosing it has the same effect as the keybinding.

4. `terminal:delete_blocks` is an editable Blocks shortcut. Default trigger is `cmdorctrl-backspace`. It appears in Settings → Keyboard shortcuts, the Resource Center Blocks section, and the Command Palette under the description **Delete Selected Blocks**.

5. If the user rebinds `terminal:delete_blocks` away from `⌘+Backspace` / `Ctrl+Backspace`, that chord returns to its editor meaning even when a deletable selection exists. The menu shortcut labels follow the current binding.

6. Remaining blocks keep their relative order. Bookmarks on surviving blocks stay attached after indices remap. Selection is cleared after a successful delete.

7. Deleted blocks disappear from the pane immediately and do not reappear on session restore.

8. Find-in-terminal matches for deleted content disappear. If the find bar is open, search re-runs against the remaining list.

9. IME composition (`IMEOpen`) blocks the keybinding, matching other terminal block bindings.

10. **Clear Blocks** / empty-buffer behavior is unchanged. Delete is selective; Clear still wipes the whole list.
