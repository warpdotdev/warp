# Tech spec: Expand selected blocks keybindings

## Context

The product behavior is defined in `product.md`. The implementation should restore `Shift-Up` and `Shift-Down` block-selection expansion without regressing text selection in editors or terminal text selections.

Relevant current code:

- `app/src/terminal/view/init.rs:741` registers editable terminal bindings `terminal:expand_block_selection_above` and `terminal:expand_block_selection_below` with default `shift-up` and `shift-down`. Their predicates currently require `Terminal`, not `IMEOpen`, not `ActiveBlockTextSelection`, and not `AltScreen`.
- `app/src/terminal/view/init.rs:307` registers fixed `shift-up` / `shift-down` terminal bindings for expanding active block text selection when `ActiveBlockTextSelection` is set.
- `app/src/editor/view/mod.rs:316`, `app/src/code/editor/view/actions.rs:118`, and `app/src/notebooks/editor/view.rs:150` register fixed `shift-up` / `shift-down` bindings for text selection in editor surfaces.
- `app/src/terminal/view.rs:25324` dispatches `ExpandBlockSelectionAbove` and `ExpandBlockSelectionBelow`. It maps visual direction to `select_less_recent_block(..., is_shift_down = true)` or `select_more_recent_block(..., is_shift_down = true)` depending on input mode.
- `app/src/terminal/view.rs:18828` and `app/src/terminal/view.rs:18903` implement less-recent and more-recent block movement. With `is_shift_down`, both call `SelectedBlocks::range_select` and scroll the new tail into view.
- `app/src/terminal/model/terminal_model.rs:747` implements `SelectedBlocks::range_select`, preserving the latest selection range pivot and replacing the tail.
- `app/src/terminal/view.rs:26731` exposes `TerminalView_BlockSelectionCardinality` to the keymap context.
- `crates/integration/src/test.rs:3218` has `test_multi_block_selections`, which already asserts `cmdorctrl-up` followed by `shift-up` and `shift-down`, but the test is ignored in `crates/integration/tests/integration/ui_tests.rs:73` due Agent View UI changes.

The likely regression is not in `SelectedBlocks::range_select`; unit coverage for that model behavior exists in `app/src/terminal/model/terminal_model_tests.rs:454`. The more likely failure mode is keymap routing after keyboard block selection. `select_most_recent_blocks` avoids forcing terminal focus while Agent View is enabled, so the input editor can remain focused after `Cmd-Up`. In that state, the editor’s fixed `shift-up` / `shift-down` text-selection bindings can win over the terminal editable expand bindings even though a terminal block selection is active.

## Proposed changes

1. Reproduce and confirm keymap ownership.
   - Use the issue sequence with current `master`: execute several commands, ensure the input editor is focused, press `Cmd-Up`, then `Shift-Up`.
   - Inspect whether `TerminalAction::ExpandBlockSelectionAbove` is dispatched or whether an editor `SelectUp` action handles the keystroke.
   - Confirm the behavior with Agent View enabled, because the existing integration test ignore reason points to Agent View-related focus changes.

2. Prefer a keymap-context fix over broad focus changes.
   - The desired ownership rule is: when `TerminalView_BlockSelectionCardinality != None` and there is no active terminal text selection or alt-screen selection, block-selection expansion owns the configured expand binding.
   - Avoid simply focusing the terminal on every block selection if that would regress Agent View workflows that intentionally keep AI input focused while blocks are attached as context.
   - If the keymap can express precedence with the existing editable binding, tighten the terminal expand predicates to include an active block selection and ensure the terminal context is available when a descendant editor is focused.

3. If editable-binding precedence is insufficient, add explicit focused-editor guard logic.
   - Add a context key that editor surfaces can see when their containing terminal has an active block selection, or adjust existing terminal context propagation so the terminal expand binding can win while a descendant editor is focused.
   - The editor fixed `shift-up` / `shift-down` bindings should remain active when no block selection is active, preserving product invariants 11 and 12.
   - Keep `ActiveBlockTextSelection` and `ActiveAltScreenSelection` as higher-priority text-selection cases.

4. Keep the action implementation unchanged unless reproduction proves otherwise.
   - `ExpandBlockSelectionAbove` / `Below` already route to the correct direction-aware helpers for input-at-bottom, waterfall, and input-at-top layouts.
   - `SelectedBlocks::range_select` already preserves pivot/tail semantics for expansion and contraction.
   - The implementation should avoid duplicating range-selection logic in keybinding handlers.

5. Ensure find-within-block and accessibility continue to update through existing selection mutation paths.
   - `change_block_selections` should remain the central path so `update_find_selection`, telemetry, selected-block visuals, and accessibility content continue to update consistently.
   - Do not mutate `selected_blocks` directly from any new keymap bridge.

6. Settings and custom bindings.
   - Keep the actions as editable bindings so users can rebind them in Settings > Keyboard Shortcuts.
   - If the fix requires adding a fixed binding for precedence, it should dispatch the same terminal actions and not create a second user-visible shortcut entry.
   - Verify custom bindings still route through `TerminalAction::ExpandBlockSelectionAbove/Below`.

## Testing and validation

1. Update `test_multi_block_selections` in `crates/integration/src/test.rs` so it covers the reported focus state:
   - Execute at least three commands.
   - Ensure the input editor is focused before keyboard block selection.
   - Press `cmdorctrl-up`.
   - Assert one block is selected and, on macOS, that this matches the Blocks “Select previous block” workflow.
   - Press `shift-up`.
   - Assert pivot is the original block and tail is the neighboring block above.
   - Press `shift-down`.
   - Assert the range contracts back to the original block.

2. Re-enable or split `test_multi_block_selections` if the broad Agent View ignore is masking this regression. If other Agent View UI changes still make the full test flaky, create a narrower non-ignored regression test for keyboard block expansion.

3. Add input mode coverage:
   - Existing unit or integration coverage should verify default input-at-bottom behavior.
   - Add input-at-top coverage for visual above/below mapping, either by parameterizing the integration test or adding a focused unit-level test around the dispatch helpers if integration cost is too high.
   - Waterfall can share default recency mapping unless the reproduction shows a distinct failure.

4. Add editor ownership checks:
   - With no active block selection and input editor focused, `shift-up` / `shift-down` should still select text in the editor.
   - With active block text selection, terminal `KeyboardSelectText(SelectionDirection::Up/Down)` should still handle `shift-up` / `shift-down`.
   - With alt screen active, expand block selection should not run.

5. Run targeted validation:
   - Relevant Rust unit tests around `SelectedBlocks`.
   - The targeted integration test for block selection.
   - Any keybinding or settings tests that exercise editable bindings, if present.

6. Manual validation on macOS:
   - Fresh default keybindings.
   - Custom binding assigned to each expand action.
   - Agent View enabled.
   - Find-within-block open while expanding selected blocks.

## Parallelization

Parallel child agents are not recommended for the implementation. The likely fix is tightly coupled across keymap precedence, focus context, and one integration test, so splitting code changes would increase merge and reasoning overhead. If an implementer wants help, a single sequential flow is preferable: one agent reproduces and patches keymap routing, then runs targeted validation.

## Risks and mitigations

1. Risk: stealing `Shift-Up` / `Shift-Down` from editor text selection.
   - Mitigation: require active block selection for block expansion to win over editor selection, and add explicit tests for editor behavior with no block selection.

2. Risk: breaking terminal text selection.
   - Mitigation: keep `ActiveBlockTextSelection` and `ActiveAltScreenSelection` excluded from block expansion and covered by tests.

3. Risk: regressing Agent View block attachment workflows by changing focus.
   - Mitigation: prefer keymap/context ownership over forcing terminal focus whenever blocks are selected.

4. Risk: fixing only default shortcuts while custom bindings still fail.
   - Mitigation: route all fixes through the existing editable `TerminalAction` bindings or dispatch the same action from any precedence shim.

## Follow-ups

1. Decide whether “Expand selected blocks above/below” should appear in the Blocks app menu alongside the other block actions.
2. Consider removing stale integration-test ignores tied to Agent View once the affected UI tests are updated for current behavior.
