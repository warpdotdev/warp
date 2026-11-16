# Mermaid diagrams in editable plans should behave like atomic blocks
## Summary
When a planning document renders a Mermaid diagram in editable mode, the rendered diagram should behave like a single diagram block rather than like hidden editable text. Users should be able to select, copy, cut, and delete the diagram as one unit, and ordinary cursor/selection gestures should not silently operate on partial invisible Mermaid source.

## Problem
We recently enabled Mermaid rendering in editable planning documents. This improves readability, but it creates a mismatched editing model:

* the user sees an image-like diagram
* the editor still exposes the underlying Mermaid source text to destructive actions and some selection flows

That mismatch is confusing and unsafe. A diagram that looks like an image should not be partially deleted via hidden text offsets.

## Goals
* Make rendered Mermaid diagrams in editable plans feel like atomic block content.
* Ensure destructive actions act on the entire Mermaid block rather than partial hidden source.
* Ensure pointer and keyboard selection flows do not place users “inside” invisible Mermaid source text during normal editing.
* Preserve the authored Mermaid markdown in storage, export, and copy flows.
* Keep the rendered Mermaid experience visually consistent with the existing notebook/plan block model.

## Non-goals
* Adding a dedicated inline Mermaid source editor for this iteration.
* Changing the rendering theme, zoom behavior, or clipboard image-byte behavior.
* Changing the behavior of ordinary non-Mermaid code blocks.
* Reworking the entire notebook command/block selection model beyond what is needed for rendered Mermaid.

## Figma / design references
Figma: none provided

## User experience
These rules apply to Mermaid diagrams that are visibly rendered inside editable planning documents.

### Atomic selection
* Single-clicking a rendered Mermaid diagram selects the entire diagram block.
* The selected state must be visually obvious and must read as a block selection, not a text cursor placed inside the diagram.
* When a Mermaid block is selected, the selection applies to the whole authored Mermaid block, not just to a subset of its source text.

### Cursor behavior
* Ordinary cursor placement should land before or after a rendered Mermaid diagram, not inside the hidden Mermaid source.
* Keyboard navigation should treat the rendered Mermaid diagram as a boundary in the document flow.
* Users should not be able to end up with a text cursor at an invisible offset in the middle of the Mermaid source through normal pointer or keyboard interactions.
* Vertical keyboard traversal must be symmetric around a rendered Mermaid diagram. If the first `Down` press from the visible cursor position immediately above the diagram crosses that block boundary, the first `Up` press from the visible cursor position immediately below the diagram must also cross into the diagram boundary instead of skipping over it.

### Delete, backspace, and cut
* If a rendered Mermaid diagram is selected as a block, `Delete`, `Backspace`, and cut actions remove the entire Mermaid block.
* “Entire Mermaid block” means the authored fenced Mermaid markdown needed to reconstruct the diagram, not just a substring of the diagram body.
* Undo and redo should restore and remove the diagram atomically.

### Drag and range selection
* If a drag selection, shift-selection, or other range-based text selection crosses a rendered Mermaid diagram, the resulting selection should include the Mermaid diagram as a whole block.
* The editor should not create a partially selected hidden Mermaid source range when the visible interaction crosses the diagram.
* Copying or cutting such a selection should preserve whole-block Mermaid behavior.
* Horizontal `Shift`-selection must use the same Mermaid boundary semantics as plain left/right cursor movement. If `Right` jumps across a rendered Mermaid block atomically, `Shift+Right` must expand across that same block atomically, and `Shift+Left` must shrink back across it on the next keypress.
* Vertical `Shift`-selection across a rendered Mermaid diagram must be reversible from both sides of the block. After expanding a selection with `Shift+Up` or `Shift+Down`, pressing the opposite arrow while `Shift` remains held must shrink the selection back across that Mermaid boundary on the very next keypress.

### Copy behavior
* Copying a selected Mermaid block should place the full authored Mermaid fenced block in plain text so that pasting into markdown reconstructs the diagram.
* When HTML clipboard output is available, the copied content may also include the rendered diagram representation in HTML.
* Direct image-byte clipboard support remains out of scope.

### Storage and export
* Planning documents must continue to store and export the original Mermaid markdown rather than persisting the rendered image.
* This feature changes editing semantics, not the saved document format.

### Scope
* This behavior is required for editable planning documents and plan-backed notebook surfaces where Mermaid is rendered.
* If Mermaid rendering is disabled and the raw code block is shown instead, existing raw markdown editing behavior may remain unchanged.

## Success criteria
* Clicking a rendered Mermaid diagram in an editable plan selects the whole diagram block.
* Users cannot accidentally delete only part of an invisible Mermaid source block through normal editing flows.
* `Backspace`, `Delete`, cut, undo, and redo operate on rendered Mermaid diagrams atomically.
* Copying a selected Mermaid block preserves the authored Mermaid markdown in plain text.
* Crossing a Mermaid diagram with drag or shift-based selection results in whole-block behavior rather than partial invisible-source behavior.
* Horizontal left/right movement and horizontal `Shift`-selection use the same atomic Mermaid crossings in both directions, including when reversing an in-progress selection.
* Vertical movement and vertical `Shift`-selection around a Mermaid block are symmetric and reversible from both the above-diagram and below-diagram cursor positions.
* The document still round-trips as raw Mermaid markdown when saved or exported.

## Validation
* Add focused editor-model tests for Mermaid block selection, delete/backspace behavior, cut behavior, copy behavior, and undo/redo behavior.
* Add coverage for range-selection flows that cross a rendered Mermaid block.
* Add hit-testing or selection tests that verify cursor/selection resolution does not land inside hidden Mermaid source offsets during normal rendered interactions.
* Add focused keyboard-selection tests that verify `Shift+Left` / `Shift+Right` reuse the same atomic Mermaid crossings as `Left` / `Right`, including reversing an expanded selection from either side of the block.
* Add focused keyboard-navigation tests that verify `Up`/`Down` and `Shift+Up`/`Shift+Down` behave symmetrically around rendered Mermaid blocks.
* Manually verify in an editable plan that:
  * clicking selects the whole diagram
  * delete/backspace remove the whole diagram
  * copy produces whole Mermaid markdown
  * drag/range selection does not partially select invisible source

## Open questions
* Do we want a dedicated follow-up affordance for explicitly editing Mermaid source while rendered mode is active, or is block-level delete/reinsert sufficient for now?
