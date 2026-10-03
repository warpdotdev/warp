# Delete selected terminal blocks — Tech spec

Product spec: `specs/GH50/product.md`
GitHub issue: [https://github.com/warpdotdev/warp/issues/50](https://github.com/warpdotdev/warp/issues/50)

HEAD researched: `[0b737e22a](https://github.com/warpdotdev/warp/blob/0b737e22a2c75cfef4aa76ac2179112a691bc3b7)`

## Context

See `product.md` for user-visible behavior.

Today a pane can wipe its whole block list (`terminal:clear_blocks` → `TerminalAction::ClearBuffer`) but cannot remove an arbitrary subset. `BlockList` already has a private `remove_block_at_index` that refuses to drop the active block. Persistence only deletes **all** blocks for a pane (`ModelEvent::DeleteBlocks(pane_id)`), which is what pane close uses.

`⌘+K` / `Ctrl+Shift+K` **Clear Blocks** is a `CustomAction` with a macOS **Blocks** menu item. Menu key equivalents are safe for that chord. Unmodified Backspace is not — it is a fixed editor binding used for typing, so it must not become a menu equivalent.

The default delete-blocks chord is `cmdorctrl-backspace`. That overlaps the editor's **Delete all left** (`cmd-backspace` on Mac) and **Delete word left** (`ctrl-backspace` on Windows/Linux). Those are editable editor bindings, so when the input is focused they match before `TerminalView` unless they are suppressed.

Relevant code at HEAD:

- `app/src/terminal/view/init.rs` [(472-480) @ 0b737e22a](https://github.com/warpdotdev/warp/blob/0b737e22a2c75cfef4aa76ac2179112a691bc3b7/app/src/terminal/view/init.rs#L472-L480) — `terminal:clear_blocks` editable binding + `CustomAction::ClearBlocks`
- `app/src/app_menus.rs` [(549-551) @ 0b737e22a](https://github.com/warpdotdev/warp/blob/0b737e22a2c75cfef4aa76ac2179112a691bc3b7/app/src/app_menus.rs#L549-L551) — Blocks app menu
- `app/src/editor/view/mod.rs` [(760-820) @ 0b737e22a](https://github.com/warpdotdev/warp/blob/0b737e22a2c75cfef4aa76ac2179112a691bc3b7/app/src/editor/view/mod.rs#L760-L820) — `editor:delete_word_left` / `editor_view:delete_all_left`
- `app/src/terminal/view.rs` [(16531-16547) @ 0b737e22a](https://github.com/warpdotdev/warp/blob/0b737e22a2c75cfef4aa76ac2179112a691bc3b7/app/src/terminal/view.rs#L16531-L16547) — Clear Blocks context-menu item
- `app/src/terminal/model/blocks.rs` [(1425) @ 0b737e22a](https://github.com/warpdotdev/warp/blob/0b737e22a2c75cfef4aa76ac2179112a691bc3b7/app/src/terminal/model/blocks.rs#L1425) — `remove_block_at_index`
- `app/src/persistence/mod.rs` [(242) @ 0b737e22a](https://github.com/warpdotdev/warp/blob/0b737e22a2c75cfef4aa76ac2179112a691bc3b7/app/src/persistence/mod.rs#L242) / `sqlite.rs` [(585) @ 0b737e22a](https://github.com/warpdotdev/warp/blob/0b737e22a2c75cfef4aa76ac2179112a691bc3b7/app/src/persistence/sqlite.rs#L585) / `block_list.rs` [(255) @ 0b737e22a](https://github.com/warpdotdev/warp/blob/0b737e22a2c75cfef4aa76ac2179112a691bc3b7/app/src/persistence/block_list.rs#L255) — pane-wide `DeleteBlocks`
- `app/src/resource_center/utils.rs` [(8-25) @ 0b737e22a](https://github.com/warpdotdev/warp/blob/0b737e22a2c75cfef4aa76ac2179112a691bc3b7/app/src/resource_center/utils.rs#L8-L25) — Blocks keybinding catalog (must stay aligned with [docs.warp.dev keyboard shortcuts](https://docs.warp.dev/getting-started/keyboard-shortcuts))



## Proposed changes



### Binding as `CustomAction` (like Clear Blocks)

Register `terminal:delete_blocks` as an `EditableBinding` with `CustomAction::DeleteBlocks`. Default keystroke is `cmdorctrl-backspace` via `custom_tag_to_keystroke` (same pattern as `CustomAction::ClearBlocks` → `cmdorctrl-shift-k`). Predicate: `Terminal & !IMEOpen & TerminalView_HasDeletableBlockSelection`.

`TerminalView_HasDeletableBlockSelection` is set when the selection contains at least one non-active block. That keeps the macOS menu item **disabled** (so the OS does not steal `⌘+Backspace`) unless delete can actually succeed.

Add **Delete Selected Blocks** to the macOS **Blocks** menu next to **Clear Blocks**.

Unmodified Backspace stays a fixed editor binding. Do not attach it to this action.

### Yield the overlapping editor chord while delete can succeed

When a deletable selection exists **and** `terminal:delete_blocks` is currently bound to `cmdorctrl-backspace`, the input editor's keymap context sets `TERMINAL_BACKSPACE_DELETES_BLOCKS`.

Attach that flag only to the editor action that actually uses that chord on the current platform:

- macOS: `editor_view:delete_all_left` (`cmd-backspace`)
- Windows/Linux: `editor:delete_word_left` (`ctrl-backspace`)

Do not attach it to the other platform's keystroke for the same action (e.g. Linux `ctrl-y` for delete-all-left must keep working).

Long-running-command bindings that also use `cmd-backspace` / `ctrl-backspace` add `!id!("TerminalView_HasDeletableBlockSelection")` so a block selection wins.

### Model + view

`BlockList::remove_blocks_at_indices` skips the active index, removes high-to-low, and returns the removed `BlockId`s. The view remaps bookmark / hover / mouse-down / filter-editor indices by arithmetic, clears selection, re-runs or clears find, and sends `ModelEvent::DeleteBlockIds`.

### Persistence

New `ModelEvent::DeleteBlockIds(Vec<BlockId>)` → `delete_blocks_by_ids`. Distinct from pane-wide `DeleteBlocks(pane_id)`.

## Testing and validation


| Product invariant                                                               | Verification                                                                                                                                                                                                      |
| ------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1. `⌘/Ctrl+Backspace` deletes non-active selection; plain Backspace still edits | `cmd_or_ctrl_backspace_deletes_selected_blocks_when_editor_is_focused`; `backspace_edits_input_when_blocks_are_selected`; `test_delete_selected_blocks`                                                           |
| 2. No selection / only-active → editor chord; active never deleted              | `cmd_or_ctrl_backspace_edits_input_when_no_blocks_are_selected`; `cmd_or_ctrl_backspace_edits_input_when_only_active_block_is_selected`; `test_delete_blocks_skips_active_block`; `test_remove_blocks_at_indices` |
| 3. Context menu                                                                 | `test_context_menu_includes_delete_when_completed_block_is_selected`; `test_context_menu_omits_delete_when_only_active_block_is_selected`                                                                         |
| 4–5. Editable Blocks shortcut + Blocks menu                                     | Binding registration + Resource Center catalog + `CustomAction::DeleteBlocks` in `make_new_blocks_menu`; manual: Settings remap, confirm editor chord returns                                                     |
| 6. Order, bookmarks, selection                                                  | `test_delete_selected_blocks`                                                                                                                                                                                     |
| 7. Session restore                                                              | Manual: delete, restore pane, blocks stay gone                                                                                                                                                                    |
| 8. Find                                                                         | Covered in `delete_blocks` (re-run / clear matches); manual with find bar open                                                                                                                                    |
| 9. IME                                                                          | Same predicate as other terminal bindings; manual if an IME is available                                                                                                                                          |
| 10. Clear Blocks unchanged                                                      | Existing clear-buffer tests                                                                                                                                                                                       |


Manual: select completed blocks, `⌘/Ctrl+Backspace`; unmodified Backspace still edits; overflow/right-click and Blocks menu **Delete Selected Blocks**; rebound shortcut; restore session.

## Parallelization

Not useful — keymap, view, model, and persistence are coupled and the implementation already lives in one diff.

## Risks

- Rebinding `terminal:delete_blocks` to another editor-owned key does not get a yield flag. Only the default `cmdorctrl-backspace` chord is special-cased.
- Persistence send failure is logged; the in-memory list is already updated.



## Follow-ups

- Update public docs in [warpdotdev/docs](https://github.com/warpdotdev/docs): Blocks table on [keyboard shortcuts](https://docs.warp.dev/getting-started/keyboard-shortcuts) (`⌘+Backspace` / `Ctrl+Backspace` · Delete Selected Blocks · `terminal:delete_blocks`) and a short section on [block actions](https://docs.warp.dev/terminal/blocks/block-actions). Those pages are not in this repo; `BLOCKS_KEYBINDINGS` already matches the in-app catalog.
- History purge.
