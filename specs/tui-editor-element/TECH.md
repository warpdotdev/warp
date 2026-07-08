# TECH: Core TUI editor element (`TuiEditorElement`)

Extracts a reusable char-cell editor element — the TUI analogue of the GUI's `RichTextElement` — and migrates the TUI prompt input onto it. Code references are repo-relative `path:line` on this branch.

## Motivation

The TUI needs to render editor content in more than one surface. Today the prompt input assembles its own rows (wrapping, cursor math, selection spans, mouse hit-testing) inside `TuiInputElement`; the upcoming TUI inline diff viewer for agent file edits (`specs/CODE-1800`, built on top of this branch) needs the same machinery plus structural overlays — line-number gutter, interleaved removed-line "ghost" rows, hidden-range elision. Without an extraction, each surface re-implements row assembly and interaction, and the input already duplicates wrap math the editor crate owns (`char_cell_cursor_pos` re-implemented `char_cell_offset_to_softwrap_point`). This branch creates the shared core; the diff viewer is a follow-up consumer and is referenced here only as motivation.

## Context (existing architecture)

The shared editor runs headlessly in the TUI: `CodeEditorModel::new_tui` (`app/src/code/editor/model.rs:365`) builds the same model constellation as the GUI (Buffer, SelectionModel, DiffModel, HiddenLinesModel, RenderState) with `LayoutMode::CharCell` (monospace wrap math, no font engine). `CharCellState` (`crates/editor/src/render/model/mod.rs`) holds the char-cell layout state — terminal width and wrap tables (`line_starts`/`char_widths`), rebuilt synchronously per edit — and `RenderState`'s softwrap queries (`offset_to_softwrap_point`/`softwrap_point_to_offset`) already branch on char-cell mode.

The GUI layering this design mirrors:
- **Model** decides *what*: `CodeEditorModel` owns `DiffModel`/`HiddenLinesModel`; `refresh_diff_state` turns hunks into removed-line `TemporaryBlock`s and decorations.
- **RenderState** stores render *mechanisms*, diff-agnostic: temporary-block storage, decorations, the `hidden_lines` handle; it answers geometry queries.
- **Core element** renders any conforming model: `RichTextElement` (`crates/editor/src/render/element/mod.rs:91`). The input's view renders it directly; the GUI diff view wraps it in `EditorWrapper` (`app/src/code/editor/element.rs:381`), which adds gutter and chrome. Read-only is achieved by not wiring editing bindings — there is no read-only mode in the core.

## Architecture

### Why structural overlays are core-element mechanisms

Gutter, ghost rows, and hidden ranges all change the row lattice itself — which terminal rows exist and where (wrap width, row count, scroll math). In char-cell rendering the row lattice *is* the layout, and the row list only materializes at element layout time (TUI elements build in `render()` but learn their width in `layout()`). The design rule: **anything that changes which rows exist or where they sit is core-element mechanism, configured by the consumer; anything about what rows mean and how they're styled is consumer policy.** This matches the GUI: ghost and hidden-section blocks are materialized *inside* `RichTextElement`'s block tree by core layout, and `EditorWrapper` builds its gutter by iterating the core's laid-out blocks (`gutter_elements()`, `app/src/code/editor/element.rs:591`) — the GUI wrapper annotates rows the core created; it never creates rows.

The GUI wrapper can annotate *after* inner layout only because GUI layout is retained (the persistent `SumTree<BlockItem>` outlives any element). TUI elements are transient — the row list is born and dies inside a single `layout()` call — so row knowledge must be co-located with row construction, inside the core element. Pixel space also lets the GUI paint annotations in a separate x-band; in char-cell, annotating a row occupies cells within it, shrinking the wrap width, which must be known before the first row is produced. Alternatives (consumers splicing rows into the element's output, or composing `[gutter][row]` pairs from an exported row projection) were rejected: both relocate the row lattice plus everything that spans rows (cursor, selection drags, hit-testing, scroll windowing) into every consumer — the arrangement this extraction removes.

### One row-structure implementation, on `CharCellState`

Painting (what's on row N) and interaction geometry (what a click on row N means) must agree exactly; implementing them separately would duplicate the row-structure algorithm and any drift shifts the whole row mapping. So the row structure is computed in exactly one place — `CharCellState`, which already owns every input (wrap tables, width, ghost blocks) and already answers char-cell geometry. The GUI analogue is exact: in pixels mode the display structure lives in `RenderState`'s retained block tree, which both painting and geometry read; char-cell mode computes the equivalent on demand (affordable: O(chars), no font shaping). Following the file's existing idiom (free functions over wrap-table slices with thin borrowing delegates), there is no named "map" type.

Two coordinate spaces stay explicit:
- **Buffer visual-row space**: soft-wrapped buffer rows only (existing softwrap functions). Used by cursor navigation; unaffected by overlays.
- **Display-row space**: what is painted — ghost and gap rows interleaved, hidden rows removed. Answered by the `CharCellState` methods below. With no overlays the two spaces are identical.

## Changes

### `crates/editor` — char-cell display-row surface

`crates/editor/src/render/model/char_cell_display.rs` (new) plus additions to `mod.rs`:

- `CharCellTemporaryBlock` + storage on `CharCellState` (`temporary_blocks()`, replace-all semantics): ghost lines flattened from the GUI's `TemporaryBlock` (fills → `ColorU`). The `LayoutAction::LayoutTemporaryBlock` char-cell arm stores them instead of no-op'ing (previously a designed-in `TODO(TUI-diff)` extension point). Fixing this arm also fixed a latent counter leak: its early `return` skipped the outstanding-layouts bookkeeping, hanging `layout_complete()` after any char-cell block push.
- `DisplayRow` / `DisplayRowKind` (public): one entry per terminal row — `Buffer { line_index }` | `Ghost { ghost_index }` | `Gap { line_range }`, plus a 0-based `char_range` (into buffer text or ghost content) and `is_continuation`. Style- and text-free: consumers supply strings and colors.
- `CharCellState::display_rows(hidden)` — buffer lines wrapped, ghosts interleaved before their `insert_before` line (same width, wide-char aware, trailing-newline stripped; at/past EOF appended), hidden lines elided into single interior gap rows (edge runs emit nothing; a ghost inside a hidden run splits the gap).
- `CharCellState::offset_to_display_point(char_idx, hidden)` / `display_point_to_offset(row, col, hidden)` — cursor placement and mouse hit-testing over those same rows; non-buffer rows resolve to the nearest buffer offset; deferred-wrap phantom row mirrors `char_cell_line_gap_position`.
- `CharCellState::visual_row_char_range(char_idx)` — buffer visual-row space: the soft-wrapped row containing an offset (backs the input's kill-to-visual-line ranges).
- `RenderState::hidden_line_ranges(app)` — `HiddenLinesModel` offset ranges projected to 0-based line ranges via `line_starts`. Wired by a new `hidden_lines` parameter on `RenderState::new_tui` (the GUI's `RenderState::new` already takes one), passed from `CodeEditorModel::new_tui`.

Hidden line ranges are a *parameter* to every display-row method rather than internal state, so painting and geometry provably see the same overlays and consumers can append structural extras (e.g. eliding a trailing empty line).

### `crates/warp_tui` — the core element

`editor_element.rs` (new): `TuiEditorElement`, snapshot-based (built fresh per render), configured by builder methods:

```rust
TuiEditorElement::new(editor, app)        // snapshots text, cursor, selection, hidden ranges
    .editable()                           // cursor + printable-char insertion; omitted = read-only
    .with_scroll(offset, max_visible_rows)   // viewport windowing; omitted = full height
    .with_gutter(GutterConfig { .. })     // line numbers; blanks on continuation/ghost/gap rows
    .with_styles(TuiEditorStyles { .. })  // text/ghost/gap styles + per-line overrides — all policy
    .hide_trailing_empty_line()           // appends the final empty line to the hidden set
    .on_action(handler)                   // TuiEditorAction → consumer's typed action; omitted = inert
    .finish()
```

The element *paints and interacts*; it does not compute row structure: it calls `display_rows` at layout time (pushing `width − gutter columns` into `set_terminal_width` first, so softwrap math agrees), slices its text snapshot by each row's `char_range`, applies styles and gutter cells, renders gaps as `… {N} lines`, windows by scroll, and keeps the phantom-row / empty-row-`" "` invariants. Mouse events hit-test via `display_point_to_offset` at event time (fresh from the render state, since the presenter caches elements) and map to a `TuiEditorAction` enum dispatched through `.on_action`; no handler ⇒ no event handling. Keybindings, kill/yank, submit, focus remain consumer policy.

### Input migration (`crates/warp_tui/src/input/view.rs`)

`TuiInputView` renders the core element verbatim (`.editable().with_scroll(...).with_drag_in_progress(...).on_action(map to TuiInputAction)`). Deleted: `TuiInputElement`, `char_cell_cursor_pos` (duplicate wrap math), the selection-span loop, `offset_at` internals, and the kill-helper row segmentation (kill ranges now via `visual_row_char_range`). Kept: `tui:input:*` keybindings, `KillBuffer`, scroll policy, submit/clear, focus, shell-mode composition. Net **−501 lines** in the input view; input behavior unchanged.

## Testing and validation

- `char_cell_display_tests.rs`: row structure (wrapping incl. wide chars, ghost interleaving/wrapping/trailing-newline stripping, interior-vs-edge gap elision, ghost-inside-hidden-run splitting) and geometry (offset ↔ display point round-trips with overlays, hidden-offset → gap-row resolution, nearest-offset semantics, deferred-wrap phantom row, `visual_row_char_range`).
- `mod_tests.rs`: `TemporaryBlock → CharCellTemporaryBlock` flattening and replace-all storage semantics.
- `editor_element_tests.rs`: painted rows, gutter numbering/blank rules, trailing-empty-line elision, scroll windowing, empty-buffer row invariant.
- Input parity: `input/view_tests.rs` behavioral assertions unchanged (harness updated to the element's types) — empty input occupies one row, wide-char cursor columns, mouse cell → offset mapping, wheel scrolling, kill/yank.
- Suites: `cargo nextest run -p warp_editor -p warp_tui`; `./script/format` + presubmit clippy.

## Risks

- Input regression: covered by the unchanged behavioral test suite; stakes low while the TUI is unreleased.
- `crates/editor` changes are shared with the GUI: additive and char-cell-gated; existing softwrap functions untouched. The `LayoutTemporaryBlock` restructure preserves the GUI arms verbatim (lazy and direct layout paths).
- The overlay mechanisms (ghosts, hidden ranges, gutter) have no consumer on this branch — they are exercised by unit tests here and consumed by the TUI inline diff viewer stacked on top.
