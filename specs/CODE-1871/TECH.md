# Up-arrow prompt history in the TUI — implementation plan (CODE-1871)
See [`PRODUCT.md`](./PRODUCT.md) for user-facing behavior. This change depends on the TUI prompt-persistence prerequisite described in [`../CODE-1871-prompt-persistence/TECH.md`](../CODE-1871-prompt-persistence/TECH.md); persistence itself is outside this branch. Code references are pinned to `7ffccbb8223e3e66ef5b850a85a546fd3c3ab7e7`.
## Context
The TUI already routes slash commands, conversations, model selection, skills, and MCP through a common inline-menu abstraction:
- [`crates/warp_tui/src/inline_menu.rs` (134-337) @ 7ffccbb](https://github.com/warpdotdev/warp/blob/7ffccbb8223e3e66ef5b850a85a546fd3c3ab7e7/crates/warp_tui/src/inline_menu.rs#L134-L337) provides `TuiInlineMenuListState`, render snapshots, and the type-erased `TuiInlineMenuHandle` interface.
- [`crates/warp_tui/src/conversation_menu.rs` @ 7ffccbb](https://github.com/warpdotdev/warp/blob/7ffccbb8223e3e66ef5b850a85a546fd3c3ab7e7/crates/warp_tui/src/conversation_menu.rs) is the closest menu-model pattern: it subscribes to editor changes, refreshes rows while open, and coordinates exclusivity through `TuiInputSuggestionsModeModel`.
- [`crates/warp_tui/src/input/view.rs` (862-873) @ 7ffccbb](https://github.com/warpdotdev/warp/blob/7ffccbb8223e3e66ef5b850a85a546fd3c3ab7e7/crates/warp_tui/src/input/view.rs#L862-L873) currently sends Up to cursor movement when no menu is open. On the first visual row that is a no-op.

The GUI behavior to match is split between the editor trigger and shared history data:
- [`app/src/terminal/input.rs` (9097-9142) @ 7ffccbb](https://github.com/warpdotdev/warp/blob/7ffccbb8223e3e66ef5b850a85a546fd3c3ab7e7/app/src/terminal/input.rs#L9097-L9142) opens inline history when Up is pressed with a single caret on the first visual row.
- [`app/src/terminal/history/up_arrow.rs` (73-129) @ 7ffccbb](https://github.com/warpdotdev/warp/blob/7ffccbb8223e3e66ef5b850a85a546fd3c3ab7e7/app/src/terminal/history/up_arrow.rs#L73-L129) reads `BlocklistAIHistoryModel::all_ai_queries`, groups current- and different-session prompts, and de-duplicates them while keeping the newest occurrence.
- [`app/src/terminal/input/inline_history/data_source.rs` (87-152) @ 7ffccbb](https://github.com/warpdotdev/warp/blob/7ffccbb8223e3e66ef5b850a85a546fd3c3ab7e7/app/src/terminal/input/inline_history/data_source.rs#L87-L152) applies prefix filtering to the trimmed input query.
## Proposed changes
### Shared prompt-history query
Add `prompt_history_for_terminal_view` beside the existing up-arrow history logic in `app/src/terminal/history/up_arrow.rs`, use it from the GUI's mixed history getter, and expose it through `tui_export`.

The getter reads `BlocklistAIHistoryModel::all_ai_queries(Some(terminal_surface_id))`, removes ignored and blank prompts, and applies the existing `sort_and_dedupe_suggestions` ordering. It returns owned `AIQueryHistory` rows so both frontends use the same prompt-history retrieval path; each menu applies its own query filter.

Add a `test-util` constructor for `BlocklistAIHistoryModel` that seeds persisted query rows for cross-crate tests.
### Prompt-history menu model
Add `TuiPromptHistoryMenuModel` in `crates/warp_tui/src/prompt_history_menu.rs`, backed by `TuiInlineMenuListState`.

The open state owns:
- the rows, selection, and scroll position;
- the original input buffer restored on dismissal;
- the typed query, held separately from preview text so arrow navigation does not change filtering.

The model subscribes to editor content and `BlocklistAIHistoryModel` changes while open. On open, the default newest selection is immediately previewed, matching the GUI; subsequent selection previews also replace the editor buffer and reset its undo stack. Escape, Down past the newest row, and Down from an empty result set close the menu and restore the original buffer. Enter returns the selected prompt for immediate submission. Empty history and filtered-to-empty results produce explicit status rows.
### Input and session integration
Add `PromptHistory` to `TuiInputSuggestionsMode` and implement `TuiInlineMenuHandle` for the prompt-history model. Extend the type-erased inline-menu interface with direct opening so `TuiInputView` can open any registered menu by mode without retaining concrete model handles.

In `TuiInputView`, intercept Up only when:
- no inline menu is already handling the action;
- the input is not in shell mode;
- there is one caret with no selection on the first visual row.

Determine the visual row through the char-cell display lattice so soft-wrapped input behaves like the GUI. Lower-row Up remains cursor movement.

In `TuiTerminalSessionView`, construct and register the menu with the existing inline menus. Handle `AcceptedPromptHistory` by filling the selected text and passing it to the existing submission path.
## Testing and validation
- `app/src/terminal/history/up_arrow_tests.rs`: ordering, newest-occurrence de-duplication, ignored prompts, and blank-prompt exclusion.
- `crates/warp_tui/src/prompt_history_menu_tests.rs`: population, default selection and initial preview, restoration, acceptance, Down dismissal from empty results, selection reconciliation, and render-to-lines output.
- `crates/warp_tui/src/input/view_tests.rs`: first-visual-row opening and initial preview, lower-row cursor movement, shell-mode suppression, stable filtering during previews, undo isolation, Escape restoration, and Enter acceptance.
- Run the full `warp_tui` test suite and `./script/format`.
- Run `./script/run-tui` and verify opening, filtering, preview, acceptance, and dismissal in a real terminal.
## Risks and mitigations
- **Query and preview share one editor buffer.** Keep the typed query in menu state and ignore editor events matching the model's own preview write.
- **Soft wrapping can diverge from logical lines.** Use the editor's display lattice rather than logical row numbers and cover both first- and lower-visual-row behavior.
- **Shared history behavior can drift.** Keep ordering, de-duplication, and ignored-prompt filtering in the app-side getter with focused tests; cover each frontend's prefix filtering independently.
## Parallelization
Parallel implementation is not useful. The shared query helper, menu model, input routing, and session acceptance form one tightly coupled vertical slice with overlapping types and tests.
