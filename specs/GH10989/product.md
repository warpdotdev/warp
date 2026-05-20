# Product spec: Expand selected blocks keybindings

## Summary

Warp should let users expand the current block selection with the configured “Expand selected blocks above” and “Expand selected blocks below” shortcuts. The default `Shift-Up` and `Shift-Down` bindings must work after selecting a block with keyboard navigation, matching the behavior users had in previous releases.

## Problem

On macOS stable `v0.2026.05.06.15.42.stable_04`, a user can create multiple command blocks and select one block with `Cmd-Up`, but pressing `Shift-Up` to include the neighboring block does nothing. The adjacent block-selection shortcuts still work, so the failure appears limited to the expand-selection actions rather than block selection overall.

## Goals

1. Restore the default block-selection expansion shortcuts for terminal block lists.
2. Preserve text-selection behavior for input editors and active terminal text selections.
3. Keep the Settings > Keyboard Shortcuts representation of the expand actions accurate and rebindable.
4. Prevent future regressions with focused-input and Agent View states.

## Non-goals

1. Do not redesign block selection, block navigation, or the Blocks menu.
2. Do not change the default shortcut values unless investigation proves the existing defaults cannot be made reliable.
3. Do not alter how terminal text selection expands with `Shift-Up`/`Shift-Down` when text is actively selected.
4. Do not change shell, PTY, or alt-screen arrow-key behavior.

## Figma

Figma: none provided. This is a keyboard interaction regression with no new visual design expected.

## Behavior

1. When a terminal block list contains at least two selectable output blocks and the user selects one block, pressing the configured “Expand selected blocks above” shortcut extends the current block selection to include the nearest selectable block visually above the current selection.

2. When a terminal block list contains at least two selectable output blocks and the user selects one block, pressing the configured “Expand selected blocks below” shortcut extends the current block selection to include the nearest selectable block visually below the current selection.

3. The default shortcuts remain:
   - `Shift-Up` for “Expand selected blocks above”.
   - `Shift-Down` for “Expand selected blocks below”.

4. Expansion is anchored at the original pivot block. If the user selects block B, expands above to include block A, then expands below, the selection contracts or extends relative to the same pivot according to the existing range-selection behavior rather than creating a disjoint selection.

5. Repeated expansion continues one neighboring selectable block at a time until the selection reaches the top or bottom selectable block. Pressing the shortcut again at the boundary leaves the selection unchanged and does not clear the selection, move focus unexpectedly, or send arrow input to the shell.

6. “Above” and “below” follow the user’s current block-list layout:
   - In the default input-at-bottom and waterfall layouts, “above” moves toward less recent blocks and “below” moves toward more recent blocks.
   - In input-at-top layout, “above” and “below” map to the visually correct neighboring blocks even though recency order is inverted.

7. Hidden, debug, or otherwise non-selectable blocks are skipped. Expansion must only include blocks that other block-selection actions can select.

8. The shortcuts work immediately after selecting a block via `Cmd-Up`/`Cmd-Down` on macOS or the equivalent configured “Select previous block” / “Select next block” actions on other platforms, even if the input editor had focus before the block-selection action.

9. The shortcuts work after selecting a block with the mouse, after selecting all blocks, and after toggling block selections, as long as there is a valid most-recent selection range to expand from.

10. If no block is currently selected, invoking either expand shortcut should match existing block-selection semantics rather than no-oping silently: it may select the most recent block or otherwise enter a single-block selection state consistent with neighboring block navigation.

11. If terminal text is actively selected inside a block or alt screen, `Shift-Up` and `Shift-Down` continue to expand the text selection rather than block selection.

12. If a text editor surface is intentionally focused and no block selection is active, `Shift-Up` and `Shift-Down` continue to select text in that editor.

13. If a block selection is active while the input editor still has focus, pressing the expand shortcuts expands the block selection rather than selecting text in the empty or previously focused input editor.

14. The shortcuts do not type escape sequences or visible characters into the shell, input editor, notebook editor, code editor, or agent prompt when the user intended block expansion.

15. The selected-block visual state updates immediately after expansion or contraction, including selected block outlines and any block-action affordances that depend on selection cardinality.

16. Find-within-block stays in sync. If the find bar is open with find-within-block enabled, expanding or contracting the selected block range updates the set of searched blocks immediately.

17. Accessibility announcements for expand actions report the updated selected-block count and the active selected block content.

18. Custom keybindings remain supported. If the user rebinds either expand action in Settings > Keyboard Shortcuts, the custom binding follows the same behavior and context rules as the defaults.

19. Other Blocks shortcuts, including select previous/next block, select all blocks, scroll to top/bottom of selected blocks, copy block, copy command, and copy output, continue to work with their existing bindings and scopes.

20. Agent View and AI input states do not suppress block expansion. Selecting blocks for agent context and expanding the selection are both valid keyboard workflows.

## Success criteria

1. The issue reproduction works on macOS: run multiple commands, press `Cmd-Up`, press `Shift-Up`, and observe a two-block selection.
2. `Shift-Down` works symmetrically after expanding up and after selecting an earlier block.
3. The behavior passes in default input-at-bottom, waterfall, and input-at-top layouts.
4. Text selection in the terminal, input editor, notebook editor, and code editor remains unchanged when no block selection should own the shortcut.
5. Existing block-selection tests continue to pass, and at least one regression test covers the focused-input/active-block-selection case.

## Validation

1. Add or update an automated integration test that reproduces the reported sequence: create several blocks, leave/focus the input as a normal user would, select a block via keyboard, press `Shift-Up`, and assert the selected range contains two blocks.
2. Add coverage for `Shift-Down` and input-at-top layout if those are not already covered by existing tests.
3. Manually verify on macOS with the default keybindings and with a custom binding assigned to each expand action.
4. Manually verify that editor text selection still owns `Shift-Up`/`Shift-Down` when no block selection is active.

## Open questions

1. Should expand actions be available from the Blocks application menu, or only from Settings > Keyboard Shortcuts as they are today?
2. If no block is selected, should an expand action select the most recent block or remain disabled? The current implementation tends toward selecting the most recent block through existing navigation helpers; implementation should either preserve that or explicitly document a different choice.
