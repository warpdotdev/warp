# Support Vim fold keybindings in the code editor — Tech Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/9748
Product spec: `specs/GH9748/product.md`

## Problem
The code editor has a Vim command parser and a hidden-line rendering path, but those systems are not connected for Vim folds. `crates/vim` does not currently parse `z` fold commands, `CodeEditorView` has no Vim event handlers for fold operations, and `CodeEditorModel` has no view-local manual fold state. The implementation needs to add fold commands without treating folds as text edits and without breaking existing hidden-line behavior used by diff navigation and code review.

## Relevant code
- `crates/vim/src/vim.rs:241` — `PendingAction` enumerates multi-key Vim command prefixes, currently without a `z` command family.
- `crates/vim/src/vim.rs:521` — `VimEventType` enumerates emitted editor operations, currently without fold events.
- `crates/vim/src/vim.rs (844-1043)` — normal-mode command parsing; unsupported characters clear pending state and return no event, so `z` is currently ignored as a Vim command prefix.
- `crates/vim/src/vim.rs (1127-1326)` — operator operand parsing for motions and counts; `zf{motion}` should reuse this shape rather than adding a separate motion parser.
- `crates/vim/src/vim.rs (1753-2006)` — `VimSubscriber` dispatches `VimEventType` into the `VimHandler` trait; fold events need to be dispatched here.
- `app/src/code/editor/view.rs (216-414)` — `CodeEditorView::new` creates the editor model, creates `VimModel`, subscribes to Vim events, and initializes Vim mode when settings enable it.
- `app/src/code/editor/view.rs (1808-2161)` — `CodeEditorView::render` builds `RichTextElement` from the model's `RenderState` and passes Vim state into rendering.
- `app/src/code/editor/view/actions.rs (491-689)` — `CodeEditorViewAction` includes `HiddenSectionExpansion` and Vim input actions but no explicit manual-fold actions.
- `app/src/code/editor/view/actions.rs (746-1102)` — typed action handling routes `VimUserTyped` into `vim_user_insert`, and hidden-section gutter clicks into `expand_hidden_section`.
- `app/src/code/editor/view/vim_handler.rs:1` — `CodeEditorView` implements `VimHandler`; this is the right place to translate new Vim fold events into code-editor model operations.
- `app/src/code/editor/model.rs (299-394)` — `CodeEditorModel` owns `hidden_lines: ModelHandle<HiddenLinesModel>`, `render_state`, selections, diff state, and lazy layout state.
- `app/src/code/editor/model.rs (544-579)` — `set_hidden_lines`, `hidden_ranges`, and `set_visible_line_range` expose the current hidden-line path to the view.
- `app/src/code/editor/model.rs (1292-1348)` — `calculate_hidden_lines` derives hidden ranges from active diff context and currently replaces the hidden-line model wholesale.
- `crates/editor/src/content/hidden_lines_model.rs (1-285)` — anchored hidden-line ranges already track buffer edits and expose hidden-range queries used by selection/rendering.
- `app/src/code/editor/element.rs (646-1024)` — `EditorWrapper` detects hidden-section blocks and renders gutter expansion controls.
- `app/src/code/editor/line.rs:1` — `EditorLineLocation::Collapsed` represents hidden sections in the gutter/event path.
- `app/src/editor/view/model/display_map/fold_map.rs:16` — an older display-map `FoldMap` exists in a different editor stack; it is useful prior art but is not wired into `CodeEditorView`'s `RenderState`/`HiddenLinesModel` path.

## Current state
`CodeEditorView` receives Vim keystrokes as `CodeEditorViewAction::VimUserTyped`, sends characters to `VimModel`, and handles emitted events through the `VimHandler` implementation in `view/vim_handler.rs`. The Vim parser already supports multi-key pending actions for operators (`d`, `c`, `y`), `g`, find-char commands, bracket jumps, registers, visual operators, and motion counts.

The code editor already has one hidden-line abstraction: `CodeEditorModel` owns a `HiddenLinesModel`, passes it into `RenderState`, and renders hidden sections through `EditorWrapper`. Today that path is used for hiding lines outside active diffs. It is not a general manual-fold model, and `calculate_hidden_lines` can replace all hidden ranges when diff state changes.

Because hidden lines already integrate with layout, gutter rendering, and selection invalidation, the implementation should build manual folds on top of the hidden-line path. The key design requirement is to keep diff-owned hidden ranges and user-owned manual folds as separate sources, then materialize their union into `HiddenLinesModel`.

## Proposed changes

### 1. Add fold events to the Vim parser
Extend `crates/vim/src/vim.rs` with a fold-specific command model:

- Add a public `VimFoldCommand` enum with variants for:
  - `OpenCurrent`
  - `CloseCurrent`
  - `ToggleCurrent`
  - `DeleteCurrent`
  - `OpenAll`
  - `CloseAll`
  - `Create { operand: VimOperand }`
  - `VisualCreate`
- Add `VimEventType::Fold(VimFoldCommand)`.
- Add `PendingAction::Z` or `PendingAction::Fold { pending_operand: Option<PendingOperand> }`.
- In normal mode, make `z` enter the fold pending action instead of clearing state.
- In the pending `z` state:
  - `o` emits `Fold(OpenCurrent)`.
  - `c` emits `Fold(CloseCurrent)`.
  - `a` emits `Fold(ToggleCurrent)`.
  - `d` emits `Fold(DeleteCurrent)`.
  - `R` emits `Fold(OpenAll)`.
  - `M` emits `Fold(CloseAll)`.
  - `f` starts a fold-create operand, reusing the same motion/text-object/count parsing shape as existing operators.
- In visual mode, support `zf` by using the visual pending-action path and emitting `Fold(VisualCreate)`.
- Unsupported `z` suffixes should clear pending state and emit no event, matching current behavior for unsupported Vim commands.

Counts should be treated pragmatically:

- Counts inside a `zf` motion, such as `zf2j`, should work because `Create { operand }` reuses existing operand parsing.
- Count-prefixed non-create commands (`3zo`, `2zd`) can be parsed but may initially behave the same as the uncounted command. Full count semantics are not required by the product spec.

Add parser unit tests near existing Vim FSA tests for each emitted event, invalid suffixes, normal-mode `zf{motion}`, motion counts, and visual-mode `zf`.

### 2. Extend `VimHandler` and dispatch fold events
Update `VimSubscriber` in `crates/vim/src/vim.rs (1753-2006)` to dispatch `VimEventType::Fold(command)` to a new trait method:

- `fn fold(&mut self, count: u32, command: &VimFoldCommand, ctx: &mut ViewContext<Self>);`

Implement this method in `app/src/code/editor/view/vim_handler.rs`.

For fold creation with a normal-mode operand, reuse the existing selection-construction logic from `operation` rather than duplicating every motion rule. The current `operation` implementation already converts `VimOperand::Motion`, `VimOperand::Line`, and `VimOperand::TextObject` into selections before applying an operator. Factor that selection-building closure into a helper on `CodeEditorModel` or a private helper in `vim_handler.rs`, then use it for both text operators and `zf`.

The handler flow should be:

1. For `Create { operand }`, build the selection that the operand covers without mutating text or registers.
2. Convert the resulting selection to a normalized full-line range.
3. Clear temporary selections as needed.
4. Call `CodeEditorModel::create_manual_fold(line_range, ctx)`.
5. Ensure Vim mode ends in normal mode.

For `VisualCreate`, derive the full-line range from the current visual selection by using the same visual selection helpers used by `visual_operator`, then create a fold and clear visual selection state.

For current/all commands, delegate directly to model methods:

- `open_current_manual_fold`
- `close_current_manual_fold`
- `toggle_current_manual_fold`
- `delete_current_manual_fold`
- `open_all_manual_folds`
- `close_all_manual_folds`

### 3. Add view-local manual fold state to `CodeEditorModel`
Add a manual fold data structure owned by `CodeEditorModel`, not by the shared buffer:

- `manual_folds: Vec<ManualFold>`
- `ManualFold` should store stable start/end anchors, a closed/open state, and enough cached line-range metadata to identify the fold under the cursor or under a collapsed hidden-section marker.
- A fold's full range is the user-selected line range. Its hidden range is the body after the first visible line.
- Folds spanning fewer than two lines should not be stored.

The model should expose methods for the Vim handler:

- `create_manual_fold(line_range, ctx)` creates anchors, closes the fold, recomputes hidden lines, and moves the cursor to the first visible line if necessary.
- `close_current_manual_fold(ctx)` finds the innermost open fold containing the cursor line or represented by the collapsed section at the cursor and closes it.
- `open_current_manual_fold(ctx)` finds the innermost closed fold at the cursor/collapsed marker and opens it.
- `toggle_current_manual_fold(ctx)` toggles the same target selection.
- `delete_current_manual_fold(ctx)` removes the target fold definition.
- `close_all_manual_folds(ctx)` sets all manual folds to closed.
- `open_all_manual_folds(ctx)` sets all manual folds to open.

Targeting rules:

- Prefer the innermost fold whose full line range contains the current cursor line.
- If the cursor is on a collapsed hidden-section marker, resolve that marker's line range back to the fold whose hidden body produced it.
- If multiple folds overlap, choose the smallest matching full range.
- If no fold matches, no-op.

Nested folds should remain valid. Opening an outer fold should reveal its body except for nested folds that are still closed; deleting an outer fold should not delete nested fold definitions.

### 4. Merge manual folds with existing diff-hidden lines
Do not let manual folds overwrite diff-owned hidden ranges. Replace the current "set hidden lines from one source" pattern with a small hidden-range recomputation layer inside `CodeEditorModel`:

- Keep `hide_lines_outside_of_active_diff: Option<usize>` as the diff-context source of truth.
- Add a helper that computes diff-hidden ranges from the active diff context without immediately applying them.
- Add a helper that computes closed manual fold hidden body ranges from `manual_folds`.
- Add `recompute_hidden_lines(ctx)` that unions both sources and calls the existing `set_hidden_lines` path exactly once.

Update these call sites to use the union helper:

- `calculate_hidden_lines` after diff updates.
- Manual fold open/close/toggle/delete/create methods.
- Content replacement or edit paths that can invalidate hidden ranges.

When buffer edits move anchors:

- Let anchors track range movement via `BufferSelectionModel` and `HiddenLinesModel` where possible.
- After each content change that affects fold anchors, drop folds whose resolved start/end no longer span at least two lines.
- Recompute hidden ranges and rebuild layout when the hidden range set changes.

This design keeps `zR`/`zM` scoped to manual folds. `zR` opens all manual folds but still materializes diff-hidden ranges if diff context hiding is active.

### 5. Cursor and selection safety
Collapsed fold bodies must not leave editable selections in hidden text. After any command that closes folds:

- If the active cursor or any selection head/tail resolves inside a newly hidden range, replace that selection with a single cursor at the fold's first visible line.
- Clear visual selection state after `zf` in visual mode.
- Request autoscroll to the visible cursor location.
- Notify the render state so current-line highlighting and hidden-section layout update.

Search integration should use the same safety rule. If the existing find flow moves the cursor to a result inside a closed manual fold, open the containing fold before setting the cursor, or set the cursor only after making the line visible. The relevant hook is `move_cursor_to_selected_match` in `app/src/code/editor/view.rs`.

### 6. UI and rendering
Reuse the existing hidden-section rendering path:

- Closed manual folds produce hidden line ranges consumed by `HiddenLinesModel`.
- `RenderState` emits hidden-section blocks as it does today.
- `EditorWrapper` renders `GutterElementType::HiddenSection` and dispatches `CodeEditorViewAction::HiddenSectionExpansion`.

Update `expand_hidden_section` so mouse expansion of a hidden section owned by a manual fold opens the corresponding fold state before recomputing hidden ranges. This keeps mouse and keyboard state consistent. Diff-owned hidden sections should continue to use the existing incremental `set_visible_line_range` behavior.

No new icon or Figma-driven treatment is required for the first implementation. If the current hidden-section UI cannot distinguish manual folds from diff-hidden context, that is acceptable for this issue as long as keyboard behavior is correct.

### 7. Keybinding settings impact
Do not register `zo`, `zc`, `za`, `zf`, `zd`, `zR`, or `zM` as editable keybindings in `app/src/code/editor/view/actions.rs`. They are Vim command sequences parsed by `VimModel`, not global keybindings or user-remappable single actions.

The only keybinding-settings surface that may need review is documentation/help text if Warp lists supported Vim commands. If such a list exists, update it in the same change; otherwise no settings UI change is required.

## End-to-end flow
1. User focuses a `CodeEditorView` with Vim mode enabled.
2. User types `zf}`.
3. `CodeEditorViewAction::VimUserTyped` routes `z`, `f`, and `}` into `VimModel`.
4. `VimFSA` enters the pending `z` state, recognizes `f`, parses `}` as a fold-create motion, and emits `VimEventType::Fold(VimFoldCommand::Create { operand })`.
5. `VimSubscriber` dispatches the fold event to `CodeEditorView::fold`.
6. The view/model derive a full-line fold range from the operand, create a closed `ManualFold`, and recompute hidden lines from manual-fold and diff-hidden sources.
7. `HiddenLinesModel` updates anchored hidden ranges, `RenderState` invalidates layout, and `EditorWrapper` renders the collapsed hidden-section affordance.
8. Later `zo`, `zc`, `za`, `zd`, `zM`, or `zR` emit fold events that mutate only `manual_folds` state and rematerialize hidden-line ranges.

## Testing and validation

### Vim parser tests
Add tests in the `crates/vim` test module for:

- `zo`, `zc`, `za`, `zd`, `zR`, and `zM` in normal mode.
- `zf}` and `zf2j` emitting `Create { operand }` with the expected motion/count.
- Visual-mode `zf` emitting `VisualCreate` and returning to normal mode after handler execution.
- Unsupported suffixes like `zx` emitting no event and clearing pending command state.
- Insert-mode `z` still emitting `InsertChar('z')`.

### Code editor model tests
Add focused tests around `CodeEditorModel` helpers for:

- Creating a closed fold over a multi-line range.
- Ignoring one-line and empty ranges.
- Opening, closing, toggling, and deleting the current fold.
- Nested and overlapping folds, including `zR` and `zM`.
- Unioning manual folds with active diff-hidden ranges.
- Cursor relocation when a fold closes over the active selection.
- Anchor updates after inserting or deleting lines before and inside a fold.
- Removing invalid folds after edits shrink them below two lines.

### Code editor Vim interaction tests
Extend `app/src/code/editor/view/vim_handler_tests.rs` using the existing `add_code_editor`, `vim_user_insert`, `set_cursor_position`, and `layout_editor_view` helpers:

- Create a fold with `zf}` in normal mode and assert the buffer text is unchanged while hidden ranges are present.
- Create a fold with visual `Vjjzf`, assert visual mode exits, and assert hidden lines are present.
- Toggle a fold with `za`, open it with `zo`, close it with `zc`, and delete it with `zd`.
- Create two folds, verify `zM` closes both and `zR` opens both.
- Verify insert mode text entry still inserts the literal characters.

### Manual validation
- Run the smallest relevant Rust test targets for `crates/vim` and `app/src/code/editor/view/vim_handler_tests.rs`.
- Run formatting and the repository's normal presubmit or a narrower code-editor test command if full presubmit is too expensive.
- Manually verify in a real code editor buffer that folds do not modify file contents, line numbers remain sensible, collapsed regions render with the existing hidden-section affordance, and diff-hidden context remains intact after `zR`/`zM`.

## Risks and mitigations

### Risk: manual folds conflict with diff-hidden sections
Both features need the same hidden-line model. If implementation calls `set_hidden_lines` independently for manual folds and diff context, the last caller wins and the other feature disappears.

Mitigation: store manual fold state and diff-hidden source state separately, then always materialize their union through a single recomputation helper.

### Risk: selections end up inside invisible text
The current hidden-line path can mark selections invalid when hidden ranges intersect selections. Vim folds should avoid leaving the editor in that invalid state after fold commands.

Mitigation: every close/create/all-close command checks active selections against the newly hidden range set and relocates invalid selections to visible fold anchor lines before notifying render state.

### Risk: duplicating motion semantics for `zf`
Reimplementing motion parsing or selection semantics for `zf` would likely drift from existing operator behavior.

Mitigation: parse `zf{motion}` as a fold create event carrying `VimOperand`, then factor existing operator selection construction so `d{motion}`, `c{motion}`, `y{motion}`, and `zf{motion}` resolve ranges consistently.

### Risk: old `FoldMap` creates architectural confusion
`app/src/editor/view/model/display_map/fold_map.rs` already has fold/unfold logic, but it belongs to a different editor display-map stack. Porting it directly risks bypassing `CodeEditorView`'s `RenderState`, hidden-line model, gutter controls, and diff-hidden handling.

Mitigation: use `FoldMap` only as prior art for concepts like anchored ranges and display remapping. Build the implementation on `CodeEditorModel` plus `HiddenLinesModel`, which is already wired into the code editor.

### Risk: incomplete Vim parity
Vim folding has many commands and options beyond the requested list.

Mitigation: keep the parser and model types extensible, but gate this issue to the requested commands and manual view-local folds. Document unsupported follow-ups rather than overfitting the first change.

## Follow-ups
- Add language-aware fold ranges from syntax tree or LSP folding providers.
- Persist fold state per file or notebook cell if users ask for it after manual folds ship.
- Add visible fold markers or a dedicated fold column if design wants a richer fold UI than the existing hidden-section affordance.
- Extend Vim support to additional fold commands such as `zr`, `zm`, `zO`, `zC`, `zD`, and Ex fold commands.
