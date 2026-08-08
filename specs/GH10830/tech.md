# Keep Completion Arrow Navigation Preview-Only — Tech Spec

Product spec: `specs/GH10830/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/10830

## Context

Issue #10830 reports that Up/Down navigation through completion suggestions writes
the highlighted candidate into the terminal input before the user accepts it. The
report covers macOS and Windows. The reporter could not attach a new GitHub upload,
but [linked the Warp team's existing Slack reproduction
video](https://github.com/warpdotdev/warp/issues/10830#issuecomment-4453250132):
https://warpcommunity.slack.com/files/U0B2N6YL813/F0B3HMGPR7C/cleanshot_2026-05-13_at_11___.20.23.mp4

The code references below are pinned to
[`71fafb46cf716f94fdbbd0930476b138ca828fee`](https://github.com/warpdotdev/warp/tree/71fafb46cf716f94fdbbd0930476b138ca828fee),
the repository revision used to write this spec.

The bug comes from one shared selection event serving two different intents:

- [`InputSuggestions::select`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/input_suggestions.rs#L551-L605)
  changes the highlighted index and emits `Event::Select`. Both
  [`select_prev` and `select_next`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/input_suggestions.rs#L505-L533)
  use that path.
- Terminal
  [`editor_up`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/terminal/input.rs#L9146-L9294)
  and
  [`editor_down`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/terminal/input.rs#L9543-L9668)
  call those same selection methods while a completion menu is visible.
- The terminal's
  [`InputSuggestionsEvent::Select` handler](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/terminal/input.rs#L8672-L8730)
  treats any completion selection as a classic-completion cycle and calls
  `select_and_replace`, so an arrow-key highlight becomes a real editor edit.
- Tab and Shift-Tab also call `select_next`/`select_prev` from
  [`input_tab`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/terminal/input.rs#L13008-L13120)
  and
  [`input_shift_tab`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/terminal/input.rs#L12817-L12860).
  Simply deleting all completion-side effects from `Event::Select` would
  therefore fix arrows but regress classic Tab/Shift-Tab cycling.

The completion mode already separates highlighting from confirmation at the event
level:

- [`Event::Select`, `Event::ConfirmSuggestion`, and
  `Event::ConfirmAndExecuteSuggestion`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/input_suggestions.rs#L219-L235)
  are distinct.
- [`InputSuggestions::confirm`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/input_suggestions.rs#L607-L639)
  emits confirmation without relying on selection to edit the buffer.
- A mouse click deliberately
  [selects and then confirms](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/input_suggestions.rs#L925-L932).
  The
  [`SelectAndConfirm` action handler](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/input_suggestions.rs#L1058-L1063)
  makes that sequence explicit.

Classic completions are material to shipping builds: both
[`classic_completions` and `force_classic_completions` are default app
features](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/Cargo.toml#L512-L609),
and
[`is_classic_completions_enabled`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/terminal/input.rs#L12169-L12174)
returns true for either the enabled user setting or the forced feature.

## Proposed changes

### 1. Make completion `Select` preview-only

In `Input::handle_suggestions_event`, keep the `HistoryUp` branch unchanged but
remove the editor replacement from the `CompletionSuggestions` branch of
`InputSuggestionsEvent::Select`.

`InputSuggestions::select` remains responsible for:

- updating `selected_index`;
- scrolling the highlighted row into view;
- emitting the existing accessibility content; and
- notifying the view.

For completion suggestions, the terminal consumes that event as a menu-state update
only. This gives Up/Down the behavior in product invariants 1–5, 8, and 14 without
changing the shared history behavior in invariant 13.

Do not change `InputSuggestionsEvent::Select` globally or remove its item payload:
history selection still uses the selected item to populate the editor, and other
consumers rely on the event.

### 2. Apply classic candidates only from explicit Tab/Shift-Tab paths

Factor the current classic-completion `select_and_replace` block out of
`handle_suggestions_event` into a small `Input` helper. The helper:

1. Reads the `replacement_start` from the active
   `InputSuggestionsMode::CompletionSuggestions`.
2. Reads the newly selected item after the menu has moved its highlight.
3. Replaces `replacement_start..cursor_end_offset` with that item's text.
4. Uses the existing
   `PlainTextEditorViewAction::CycleCompletionSuggestion` action so current undo,
   filtering, and result-set-lifetime behavior is preserved.
5. Does nothing unless classic completions are enabled and a completion item is
   selected.

Call this helper immediately after `select_next` in `input_tab` and immediately
after `select_prev` in the completion branch of `input_shift_tab`. Up/Down continue
to call only the menu selection methods and never call the helper.

This separates intent at the terminal call sites without widening the generic
`InputSuggestions` API:

- Up/Down: move highlight, preview only.
- Tab/Shift-Tab: move highlight, then explicitly apply the classic-cycle candidate.
- Enter: use the existing `ConfirmSuggestion` path.
- Mouse: use the existing `SelectAndConfirm` path.

Regular completions skip the classic helper and retain their current Tab/Shift-Tab
selection behavior. History and workflow enum menus do not enter the helper.

### 3. Preserve confirmation, dismissal, and filtering behavior

Do not change
[`confirm_suggestion_internal`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/terminal/input.rs#L8846-L8945)
or
[`insert_completion_result_into_editor`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/terminal/input.rs#L12722-L12761).
Enter and mouse confirmation must continue to use the established acceptance action,
spacing rules, and menu-close behavior.

Do not add completion-specific restoration to
`close_input_suggestions_and_restore_buffer`. Once arrow selection stops editing the
buffer, Escape naturally preserves the user's text and editor state. Existing
history restoration remains unchanged.

Keep
[`update_tab_completion_menu`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/terminal/input.rs#L11802-L11899)
unchanged unless tests expose a narrowly related adjustment. Its distinction between
system-applied classic cycles and user edits remains necessary for explicit
Tab/Shift-Tab cycling and for stale-result dismissal.

## Testing and validation

### Unit coverage

Add focused cases beside
[`app/src/terminal/input_tests.rs`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/terminal/input_tests.rs#L3344-L3595)
using the existing terminal fixture and suggestion builders:

- In classic mode, Up/Down and wraparound change the selected item while preserving
  the buffer, cursor/selection snapshot, and undo state (product 1–5).
- Escape after arrow-only navigation preserves the original editor state; typing
  and backspace start from that state (product 8–9).
- Refreshed results remain preview-only, including both keybinding and
  completions-as-you-type triggers (product 10–12).
- Tab/Shift-Tab still advance then apply, while Enter and mouse click confirm the
  highlighted item (product 6–7).
- Existing history selection and accessibility behavior remain green (product
  13–14).

Retain the regular-completion assertion around
[`test_tab_completion`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/terminal/input_tests.rs#L3391-L3455)
and the classic-cycle tests
[`test_classic_tab_completions_close_after_user_backspace`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/terminal/input_tests.rs#L6915-L6975)
and
[`test_classic_tab_completions_keep_menu_open_while_cycling`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/terminal/input_tests.rs#L6978-L7025).

### GUI integration coverage

Add `test_completion_arrow_navigation_is_preview_only` beside
[`test_completions_as_you_type`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/crates/integration/src/test.rs#L4784-L4942).
Use its deterministic aliases to open a multi-result menu, send real `down`/`up`
keystrokes, assert `get_selected_item_text()` changes without changing the input,
verify Escape preservation, then reopen and confirm with Enter. Keep the existing
Tab advance-and-apply assertion green.

Register the test in `crates/integration/src/bin/integration.rs` and
`crates/integration/tests/integration/ui_tests.rs`. The GUI runner covers end-to-end
action dispatch on its supported platform; `warp` unit tests cover the shared logic
in the Windows CI matrix.

### Commands

Run:

- `./script/format`
- `cargo nextest run -p warp -E 'test(/terminal::input_tests/)'`
- `cargo nextest run -p integration -E 'test(test_completion_arrow_navigation_is_preview_only)'`
- The three Clippy commands in `./script/presubmit`:
  `cargo clippy --workspace --exclude warp_completer --all-targets --tests -- -D warnings`,
  `cargo clippy -p warp --all-targets --tests -- -D warnings`, and
  `cargo clippy -p warp_completer --all-targets --tests -- -D warnings`
- `./script/presubmit` as the final required gate

### Manual macOS evidence

1. Build and launch Warp from the implementation branch.
2. Record the active commit, environment, and a stable prefix yielding at least
   three results; capture the full input and cursor before navigation.
3. Press Down and Up, including a wrap boundary, and show the menu
   highlight moving while the input and cursor remain unchanged.
4. Press Escape and show that the exact original input remains.
5. Reopen the same menu, navigate with arrows, then press Tab/Shift-Tab and Enter
   in separate passes to show that explicit completion still works.
6. Repeat once with completions-as-you-type enabled and once with a manually opened
   menu.
7. Attach the narrated recording to the implementation PR. Reference
   the reporter's existing Slack video as prior reproduction evidence, not as a
   substitute for the implementation-branch recording.

Figma is not part of validation because none was provided and this change introduces
no visual design.

## Risks

- Tab regression: relocate the existing `CycleCompletionSuggestion` replacement to
  Tab/Shift-Tab-only call sites and retain classic-cycle tests.
- History regression: change only the completion `Select` arm and retain history
  tests.
- Mouse divergence: keep `SelectAndConfirm` ordering and test click confirmation;
  confirmation already performs the replacement.
- Stale results: retain
  [`handle_completion_suggestions_results`](https://github.com/warpdotdev/warp/blob/71fafb46cf716f94fdbbd0930476b138ca828fee/app/src/terminal/input.rs#L12550-L12557)
  snapshot rejection and read the active mode/current item only when Tab is handled.

## Parallelization

The implementation is intentionally small and tightly coupled across one selection
event handler, the Tab/Shift-Tab call sites, and their shared tests. Parallel code
changes would create overlap in `app/src/terminal/input.rs`; implement the behavior
and focused tests sequentially, then run independent automated and manual validation.
