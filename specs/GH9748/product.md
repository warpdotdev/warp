# Support Vim fold keybindings in the code editor — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/9748
Figma: none provided

## Summary
Warp's code editor Vim mode should support the standard manual fold commands `zf`, `zo`, `zc`, `za`, `zd`, `zR`, and `zM`. Users who already rely on Vim folds should be able to create, collapse, expand, toggle, delete, open-all, and close-all folds from the keyboard without leaving the editor or changing editing modes.

This spec covers view-local manual folds in `CodeEditorView` surfaces that already support Vim mode. It does not require language-aware fold discovery, persistence across sessions, or a new fold-column UI.

## Problem
Warp's code editor supports many Vim motions and operators, but fold commands are currently missing. A user editing or reviewing a file with Vim keybindings enabled cannot hide a function, class, long block, or selected region with `zf`, cannot reopen or close that region with `zo`/`zc`/`za`, and cannot use `zR`/`zM` to quickly restore or collapse the buffer view.

This breaks a common Vim workflow for managing code visibility. Users must scroll through long sections or rely on mouse-only hidden-section controls that do not match their established editing muscle memory.

## Goals
- Support the fold key sequences requested in the issue: `zo`, `zc`, `za`, `zf`, `zd`, `zR`, and `zM`.
- Make folds available only when the code editor is accepting Vim-mode input.
- Let users create manual folds from normal-mode motions and from visual selections.
- Keep folds as editor view state: folding changes visibility only and never edits file contents.
- Make cursor movement, selection, search, editing, and diff-hidden sections behave safely around collapsed lines.
- Reuse existing code-editor hidden-section affordances where possible instead of requiring new visual design.
- Provide automated coverage for Vim parsing, code-editor fold state, and representative end-to-end editor interactions.

## Non-goals
- Language-aware automatic folding by syntax node, indentation, imports, comments, or LSP folding ranges.
- Persisting folds to disk, cloud sync, notebooks, workspaces, or restored app sessions.
- Vim fold methods beyond manual folds, such as `foldmethod=indent`, `foldmethod=syntax`, fold levels, `zm`/`zr`, or Ex commands like `:fold` and `:set foldmethod`.
- Full Vim count parity for every fold command. Counts on motions used by `zf` should work through the existing motion parser, but count-prefixed variants like `3zo` are not required for the first implementation.
- A new fold column, minimap, settings UI, or keybinding-settings entries for the literal Vim command sequences.
- Changing terminal Vim mode or shell vi-mode behavior. This feature is scoped to Warp's code editor surfaces.

## User experience

### Availability
1. Fold commands are recognized only when all of the following are true:
   - The focused surface is a `CodeEditorView`.
   - The `VimCodeEditor` capability is enabled for that editor.
   - App editor settings have Vim mode enabled.
   - The editor is in a state where it already accepts Vim command input.
2. In insert mode, typing `z`, `o`, `c`, `a`, `f`, `d`, `R`, or `M` continues to insert text normally.
3. In non-Vim mode, the same keys continue to behave as ordinary text input.
4. Surfaces that render code through `CodeEditorView` but intentionally do not accept Vim input do not gain fold shortcuts until they opt into the same Vim input path.

### Creating folds
1. `zf{motion}` in normal mode creates a manual fold over the full line range covered by `{motion}`.
   - Existing Vim motion counts apply to the motion itself. For example, `zf2j` folds from the current line through the line reached by `2j`.
   - Motions that resolve to a character range are expanded to whole lines before the fold is created.
   - Motions that resolve to fewer than two lines are no-ops because there is nothing useful to collapse.
2. `zf` in visual mode creates a manual fold over the full line range covered by the current visual selection.
   - Characterwise and linewise visual selections both fold whole lines.
   - After the fold is created, the editor exits visual mode and returns to normal mode.
3. Creating a fold closes it immediately.
   - The first line of the folded range remains visible as the fold anchor.
   - The body lines after the first line are hidden behind Warp's existing hidden-section treatment.
   - The cursor moves to the first visible line of the folded range if it would otherwise land inside hidden text.
4. Creating a fold never changes the buffer text, register contents, clipboard contents, undo stack, or diagnostics.
5. Creating an overlapping fold is allowed. The resulting visible state is the union of all closed fold bodies, and later open/delete commands operate on the innermost applicable manual fold.

### Opening, closing, toggling, and deleting the current fold
1. `zc` closes the nearest open manual fold containing the cursor or represented by the collapsed section under the cursor.
   - If there is no applicable open fold, `zc` is a no-op.
   - Closing a fold keeps its first line visible and hides the fold body.
2. `zo` opens the nearest closed manual fold at the cursor, on the fold's first visible line, or on its collapsed hidden-section marker.
   - If there is no applicable closed fold, `zo` is a no-op.
   - Opening a fold reveals the lines hidden by that fold, except for any nested folds that are still closed.
3. `za` toggles the nearest applicable manual fold.
   - If the current fold is open, it closes.
   - If the current fold is closed, it opens.
   - If there is no applicable fold, `za` is a no-op.
4. `zd` deletes the nearest applicable manual fold.
   - Deleting a fold removes its fold definition and reveals text hidden only by that fold.
   - Nested folds are preserved. If a nested fold remains closed, its body remains hidden after the outer fold is deleted.
   - `zd` does not delete file content and does not write to Vim registers.
5. These commands leave the editor in normal mode.

### Opening or closing all folds
1. `zM` closes all manual folds in the current code editor buffer.
2. `zR` opens all manual folds in the current code editor buffer.
3. `zR` does not delete fold definitions. A later `zM` can close them again.
4. `zR` and `zM` affect only manual Vim folds. They must not disable or permanently alter hidden line ranges owned by code-review diff navigation, active-diff context hiding, or other non-fold features.

### Cursoring, selection, editing, and search around folds
1. Normal cursor movement skips hidden fold bodies using the same hidden-line navigation behavior already used by code-editor hidden sections.
2. When a command closes a fold that contains the active cursor or selection, the editor clears invalid hidden selections and places the cursor on the fold's first visible line.
3. If the user edits visible text inside a fold range, the fold anchors track the edit as long as the fold still spans at least two lines.
4. If edits cause a fold to span fewer than two lines, Warp removes that fold definition and recomputes visibility.
5. Text search should still find matches in the full buffer. If moving to a search result would place the cursor in a closed fold body, Warp opens the containing fold before focusing the match or otherwise ensures the cursor lands on visible text.
6. Copy, cut, paste, undo, redo, commenting, and case-changing commands operate on visible selections exactly as they do today. Fold state changes themselves are view operations and are not text undo entries.

### Visual treatment
1. Collapsed fold bodies use Warp's existing hidden-section visual treatment in the code editor gutter/content area.
2. Mouse expansion controls that already work for hidden sections may open a collapsed fold body, but keyboard support via `zo`/`za` remains the primary requested interaction.
3. No Figma mock was provided. The initial implementation should avoid introducing new visual assets unless the existing hidden-section affordance is insufficient.
4. Fold state is view-local. Closing and reopening a file, reloading the app, or recreating the editor view may reset folds.

## Success criteria
1. In a code editor with Vim mode enabled, `zf}` over a multi-line block creates a collapsed manual fold while leaving the file contents unchanged.
2. In visual line mode, selecting several lines and typing `zf` collapses those selected lines into a fold and exits visual mode.
3. `zo`, `zc`, and `za` open, close, and toggle the fold at the cursor without affecting unrelated folds.
4. `zd` removes the current manual fold definition and reveals text hidden only by that fold.
5. `zM` closes every manual fold in the editor; `zR` reopens every manual fold while preserving definitions.
6. Fold commands are ignored or treated as ordinary input outside supported code-editor Vim command contexts.
7. Collapsing folds does not corrupt diff-hidden sections, code-review hidden context, comments, diagnostics, line numbers, or scroll state.
8. Cursor movement and search never leave the active cursor inside an invisible fold body.
9. Editing visible text inside or near a fold keeps fold ranges stable, and invalid folds are removed rather than causing rendering or selection errors.

## Validation
- Add unit tests in `crates/vim` for parsing `zo`, `zc`, `za`, `zf{motion}`, `zd`, `zR`, `zM`, unsupported `z` suffixes, and visual-mode `zf`.
- Add code-editor model tests for manual fold creation, open/close/toggle/delete, all-open/all-close, overlapping/nested folds, cursor relocation, and edit-driven anchor updates.
- Add `CodeEditorView` Vim interaction tests that simulate user input through `CodeEditorViewAction::VimUserTyped`.
- Add regression tests showing that insert mode and non-Vim mode still treat the same characters as normal text.
- Add tests or focused assertions confirming manual folds merge safely with existing hidden-line ranges used by diff navigation.
- Manually verify a representative Rust or TypeScript file on macOS with Vim mode enabled:
  - `zf}` creates and closes a fold.
  - Visual `Vjjzf` folds the selected lines.
  - `zo`/`zc`/`za` operate at the fold.
  - `zd` deletes the fold without deleting text.
  - `zM` and `zR` affect all manual folds.

## Open questions
- Should a future follow-up add language-aware default folds from syntax tree or LSP folding ranges? This is intentionally out of scope for the first manual-fold implementation.
- Should fold state eventually persist per file or per notebook cell? This spec treats fold state as view-local to keep the initial implementation small and predictable.
