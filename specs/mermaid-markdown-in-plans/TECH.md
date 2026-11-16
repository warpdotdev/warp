# Problem
Editable planning documents now render Mermaid diagrams, but the notebook editor still fundamentally stores those diagrams as fenced code blocks. The render layer shows an image-like block while parts of the editor interaction model still operate on underlying text offsets. We need to make rendered Mermaid diagrams behave atomically in editable plans without changing the persisted markdown format.

## Relevant code
* `app/src/notebooks/editor/model.rs (1102-1242)` — block selection currently goes through `select_command_at`, `select_command_up`, `select_command_down`, and `single_selected_command_range`.
* `app/src/notebooks/editor/model.rs (1513-1549)` — `backspace` already deletes an entire selected block when `single_selected_command_range` resolves to one selected child model.
* `app/src/notebooks/editor/model.rs (1626-1756)` — `ChildModels` creates `NotebookCommand` child models for every code block in the outline, including Mermaid code blocks.
* `app/src/notebooks/editor/notebook_command.rs (132-246, 688-717)` — Mermaid code blocks already map to `CodeBlockType::Mermaid`, participate in block selection, and are always selectable through the child-model abstraction.
* `editor/src/content/edit.rs (709-724, 1031-1074)` — editable Mermaid rendering converts Mermaid code blocks into `BlockItem::MermaidDiagram` while preserving the original buffer content and content length.
* `editor/src/render/model/location.rs (140-191)` — `BlockItem::MermaidDiagram` hit-tests as a block, including for forced-selection drag paths, with the editor/view layer mapping that block hit to the appropriate selection boundary when needed.
* `editor/src/render/element/mod.rs (574-649)` — normal mouse down uses default hit testing, while drag hit testing forces text selection.
* `app/src/notebooks/editor/view.rs (1675-1681, 2855-2860, 3313-3315)` — block clicks dispatch `SelectBlock`, which currently routes through command-oriented selection behavior.
* `app/src/notebooks/editor/model.rs (1267-1357)` — Mermaid copy behavior currently reuses `command_clipboard_content`, which reads inner Mermaid source from the selected code block model.
* `app/src/notebooks/editor/model_tests.rs (1960-2235)` — existing tests already cover atomic code-block deletion and Mermaid rendering gate behavior.

## Current state
* Mermaid rendering in editable plans is implemented as a render/layout concern, not as a first-class editing primitive.
* The buffer still contains a normal fenced Mermaid code block even when the user sees a rendered diagram.
* `ChildModels` already creates a `NotebookCommand` model for Mermaid blocks, so Mermaid is not missing from the block-selection system.
* A plain click on a rendered Mermaid diagram can already resolve to `Location::Block` and dispatch `SelectBlock`.
* Once a Mermaid block is selected through the child-model path, existing block-selection deletion behavior is already close to what we want.
* The remaining mismatch is that some interaction paths still resolve Mermaid in text terms:
  * drag and some range-selection flows use forced text selection
  * text-range operations can still target underlying hidden Mermaid offsets
  * clipboard behavior for selected Mermaid blocks currently returns only the inner Mermaid body rather than the full fenced block
  * horizontal `Shift`-selection still relies on range expansion semantics, so reversing `Shift+Left` / `Shift+Right` after crossing Mermaid does not collapse with the same atomic boundary behavior as plain cursor movement
  * vertical line navigation is asymmetric at Mermaid boundaries because the buffer exposes a distinct cursor position immediately after the fenced block, and the current normalization does not treat that position as the reversible counterpart to the block start
* The result is a mixed model where Mermaid sometimes behaves like a block and sometimes like hidden text.

## Proposed changes
### 1. Treat rendered Mermaid as an atomic block in editable mode
Keep the persisted representation as fenced markdown, but define a stronger editing invariant:

* when Mermaid is rendered in editable mode, normal user interactions should resolve to Mermaid block boundaries rather than interior Mermaid text offsets

This should be scoped to rendered Mermaid only. Raw code-block mode should keep normal text semantics.

### 2. Reuse the existing child-model selection system
Do not introduce a completely separate Mermaid child model in the first iteration. `NotebookCommand` is already created for Mermaid code blocks and already participates in selection, copy-button rendering, and block-range resolution.

Instead:
* treat Mermaid as a specialized use of the existing code-block child model
* broaden the “command selection” mental model in implementation to mean “selectable block selection” where needed
* optionally rename comments/helpers in touched code if that improves clarity, but avoid a large refactor

### 3. Normalize hit-testing and selection resolution for Mermaid
The main technical gap is the path that still forces text selection during drag/range interactions.

We should add Mermaid-aware normalization so that rendered Mermaid never exposes interior hidden-source offsets through normal editing gestures.

Reasonable implementation options:
* update `editor/src/render/model/location.rs` so `BlockItem::MermaidDiagram` continues to return a block hit even when forced-selection drag paths are active
* add selection normalization in the core editor selection model so keyboard navigation and selection extension clamp Mermaid-intersecting selections to block boundaries before they become ordinary text edits

Preferred direction:
* keep block hit-testing behavior in the render model authoritative
* keep Mermaid selection clamping in the core `editor` crate, not notebook-specific model code
* map Mermaid drag/range interactions to start/end boundaries or whole-block selection before they become ordinary text edits

This keeps the behavior closest to what the render tree is already signaling: Mermaid is a block, not a paragraph of visible text.

For horizontal character navigation and horizontal `Shift`-selection, the moving selection head should reuse the same Mermaid boundary normalization as plain cursor movement instead of going through range-union expansion:
* `Right` from immediately before a rendered Mermaid block and `Shift+Right` from the same position should both land on the same “after Mermaid” boundary
* `Left` from immediately after the block and `Shift+Left` from that position should both land on the same “before Mermaid” boundary
* when a selection has already expanded across Mermaid, pressing the opposite horizontal arrow with `Shift` held should move the head back across the block boundary on the next keypress rather than re-expanding the same range

That keeps horizontal traversal and horizontal selection reversal aligned with one another and avoids maintaining separate Mermaid rules for movement vs. extension.

For vertical line navigation, we should explicitly normalize the two visible cursor anchors around a rendered Mermaid block:
* `block_start` is the visible cursor position immediately above the rendered Mermaid block
* `block_end + 1` is the first visible cursor position immediately below the rendered Mermaid block in the markdown buffer

Those anchors should behave as a reversible pair for line-based navigation:
* `Down` / `Shift+Down` from `block_start` lands at `block_end + 1`
* `Up` / `Shift+Up` from `block_end + 1` lands at `block_start`

That keeps plain vertical traversal symmetric and makes reversing a shift-selection collapse back across the Mermaid boundary on the very next keypress.

### 4. Normalize destructive operations around Mermaid block selection
Ensure `backspace`, `delete`, and cut treat Mermaid atomically whenever the active selection resolves to a Mermaid block.

Concretely:
* clicking a Mermaid block should reliably produce block selection
* drag/shift/range flows that cross Mermaid should normalize into whole-block selection or whole-block-inclusive ranges
* destructive actions should not operate on a partial Mermaid source slice, including backspace from the first cursor position immediately after the rendered diagram and Delete from immediately before it
* horizontal character extension should not get stuck in a fully-expanded Mermaid range when the user reverses `Shift+Left` / `Shift+Right`
* line-based navigation should not skip Mermaid when entering from below or get stuck in a non-reversible boundary state after expanding a vertical selection across the block

The existing `single_selected_command_range` path is a good base for single-block behavior. The new work is mostly ensuring Mermaid reliably enters that path instead of falling back to raw text selection.

### 5. Change Mermaid block clipboard semantics
Update Mermaid block copy behavior so the plain-text clipboard payload is the full authored fenced Mermaid block, not only the inner diagram body.

That means:
* plain text should reconstruct the original Mermaid markdown block
* HTML may continue to include the rendered Mermaid representation

Implementation-wise, this likely means replacing the current inner-source-only logic in `command_clipboard_content` / `mermaid_block_source` with a Mermaid-specific markdown reconstruction helper based on:
* block type
* inner source text
* the existing markdown language representation helpers

### 6. Keep storage/export unchanged
Do not change how Mermaid is persisted in the notebook buffer, the markdown export path, or document storage. This feature is strictly about editable rendered behavior and selection semantics.

### 7. Gate rollout behind a dedicated feature flag
Add a dedicated editable-Mermaid feature flag so the existing `MarkdownMermaid` flag continues to mean “Mermaid rendering exists” while this feature specifically gates the new editable atomic behavior.

Concretely:
* add an `EditableMarkdownMermaid` runtime flag and matching app feature
* default it on for dogfood builds
* require the new flag for editable-mode Mermaid rendering, Mermaid boundary hit-testing, whole-block destructive edits, and the fenced-markdown clipboard behavior introduced by this feature
* leave existing non-editable Mermaid rendering under `MarkdownMermaid`

## End-to-end flow
1. A plan loads Mermaid markdown into the notebook editor buffer as a fenced Mermaid code block.
2. With Mermaid rendering enabled in editable mode, layout converts that block into `BlockItem::MermaidDiagram`.
3. `ChildModels` still creates a `NotebookCommand` child model for the underlying Mermaid code block.
4. Pointer or keyboard interactions that target the rendered Mermaid block resolve to the block model or to Mermaid-safe boundaries rather than hidden interior offsets.
5. If the Mermaid block becomes selected, copy/cut/delete/backspace operate on the entire authored Mermaid block.
6. Undo/redo restore the same block atomically.
7. Saving/export still emits raw Mermaid markdown.

## Risks and mitigations
### Risk: Mermaid behavior diverges from ordinary code blocks in confusing ways
Mitigation:
* scope the atomic behavior only to rendered Mermaid blocks
* keep raw markdown mode unchanged
* document the distinction clearly in tests and code comments

### Risk: selection normalization introduces offset bugs
Mitigation:
* add focused tests for cursor placement, drag selection, shift-selection, copy, cut, delete, and undo/redo
* prefer normalizing at one editor-core boundary layer rather than sprinkling Mermaid special cases across many edit actions
* keep Mermaid range discovery anchored to buffer-tracked child models rather than caching absolute render offsets on `BlockItem::MermaidDiagram`

### Risk: current command-oriented naming obscures the implementation
Mitigation:
* if touched code becomes hard to reason about, rename local helpers/comments toward “block selection” in the edited areas
* avoid broad renaming outside the affected path in this iteration

### Risk: copy semantics change for Mermaid but not for other code blocks
Mitigation:
* keep the Mermaid copy-path explicit and documented as a rendered-block exception
* leave ordinary code-block copy semantics unchanged

## Testing and validation
* Add notebook editor model tests for:
  * selecting a rendered Mermaid block
  * backspace/delete removing the whole Mermaid block
  * cut removing the whole Mermaid block
  * copy producing full fenced Mermaid markdown in plain text
  * undo/redo restoring Mermaid atomically
* Add tests for range-based interactions:
  * drag selection across Mermaid
  * shift-selection across Mermaid
  * cursor movement landing before/after Mermaid rather than inside hidden source
  * `Shift+Right` / `Shift+Left` across Mermaid are atomic in both directions
  * reversing a horizontal shift-selection from either side of Mermaid collapses the selection on the next keypress
  * `Up` from the cursor immediately below Mermaid lands on the block boundary instead of skipping past it
  * `Shift+Up` from below followed by `Shift+Down` returns to the original cursor position immediately below the block
* Add integration coverage for the editable-plan surface so we verify the full rendered interaction stack, not just the editor model:
  * open a plan document with Mermaid rendering enabled in editable mode
  * verify click selection chooses the whole Mermaid block
  * verify backspace from the first cursor position immediately after the diagram and Delete from immediately before it both remove the entire block on the first keypress
  * verify drag / shift-based selection across the diagram copies or cuts whole-block Mermaid markdown
  * verify undo/redo restores the rendered diagram and that saved/exported markdown still contains the fenced Mermaid source
  * keep these tests focused on Mermaid behavior only, so failures point to the current branch changes instead of unrelated notebook features
* Add render/hit-test coverage where useful for Mermaid block location resolution.
* Manually verify behavior in an editable AI plan opened with Mermaid rendering enabled.

## Follow-ups
* If profiling ever shows Mermaid range normalization is a measurable hot path, prefer a notebook-model cache of Mermaid block handles or start/end anchors over storing absolute ranges on render blocks. The right shape would be a Mermaid-only index derived from `ChildModels`, with offsets still resolved at action time so edits above the block continue to retarget correctly.
* Do not cache absolute Mermaid `Range<CharOffset>` values on `BlockItem::MermaidDiagram`. Render blocks already carry `content_length`, but their absolute start offsets come from the render tree and can lag buffer edits until layout catches up. Caching raw ranges there would add invalidation work without removing the need for per-action adjacency/intersection normalization.
* Add an explicit “edit Mermaid source” affordance if we decide rendered Mermaid needs a source-edit mode rather than delete/reinsert behavior.
* Consider renaming the notebook “command selection” abstractions to generic block selection over time.
* Consider image-byte clipboard support or a dedicated “Copy image” affordance in a later iteration.
