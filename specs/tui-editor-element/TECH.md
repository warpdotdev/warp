# TECH: Core TUI editor element (`TuiEditorElement`)

Extracts a reusable char-cell editor element — the TUI analogue of the GUI's `RichTextElement` — and migrates the TUI prompt input onto it. Code references are repo-relative `path:line` on this branch.

## Motivation

The TUI needs to render editor content in more than one surface. Today the prompt input assembles its own rows (wrapping, cursor math, selection spans, mouse hit-testing) inside `TuiInputElement`; the upcoming TUI inline diff viewer for agent file edits (`specs/CODE-1800`, built on top of this branch) needs the same machinery plus structural overlays — line-number gutter, interleaved removed-line "ghost" rows, hidden-range elision. Without an extraction, each surface re-implements row assembly and interaction, and the input already duplicates wrap math the editor crate owns (`char_cell_cursor_pos` re-implemented `char_cell_offset_to_softwrap_point`). This branch creates the shared core; the diff viewer is a follow-up consumer and is referenced here only as motivation.

## Context (existing architecture)

The shared editor runs headlessly in the TUI: `CodeEditorModel::new_tui` (`app/src/code/editor/model.rs:365`) builds the same model constellation as the GUI (Buffer, SelectionModel, DiffModel, HiddenLinesModel, RenderState) with `LayoutMode::CharCell` (monospace wrap math, no font engine). `CharCellState` (`crates/editor/src/render/model/mod.rs`) holds the char-cell layout state — terminal width and wrap tables (`line_starts`/`char_widths`), rebuilt synchronously per edit — and `RenderState`'s softwrap queries (`offset_to_softwrap_point`/`softwrap_point_to_offset`) already branch on char-cell mode.

The GUI layering this design mirrors:
- **Model** decides *what*: `CodeEditorModel` owns `DiffModel`/`HiddenLinesModel`; `refresh_diff_state` turns hunks into removed-line `TemporaryBlock`s and decorations.
- **RenderState** stores render *mechanisms*, diff-agnostic: temporary-block storage and decorations; its `CharCellState` owns the TUI `hidden_lines` handle and answers char-cell geometry queries.
- **Core element** renders any conforming model: `RichTextElement` (`crates/editor/src/render/element/mod.rs:91`). The input's view renders it directly; the GUI diff view wraps it in `EditorWrapper` (`app/src/code/editor/element.rs:381`), which adds gutter and chrome. Read-only is achieved by not wiring editing bindings — there is no read-only mode in the core.

## Design overview

The stack, bottom-up — the design rule throughout: **anything that changes which rows exist or where they sit is a core mechanism, configured by the consumer; what rows mean and how they're styled is consumer policy.**

```
CodeEditorModel (app)               edits, selection, diff; visual-row kill APIs
 └─ RenderState (crates/editor)     LayoutMode::CharCell(CharCellState)
     └─ CharCellState               retained char-cell layout state: wrap tables,
          │                         ghost blocks, viewport scroll offset + policy
          └─ display_lattice(hidden)
               └─ DisplayLattice    per-scope row projection: rows() plus
                                    offset ↔ display-point queries
TuiEditorElement (crates/warp_tui)  paints rows, draws cursor/selection, hit-tests
 │                                  mouse; emits editor-generic TuiEditorActions
 └─ consumers (policy)              TuiInputView: editable, scroll-windowed,
                                    keybindings / kill buffer / submit / shell mode
                                    TUI diff view (stacked branch): read-only,
                                    gutter + diff styles
```

Who owns what:
- **Row structure**: computed in exactly one place, once per returned `DisplayLattice` — painting and hit-testing are projections of the same rows and cannot disagree.
- **Viewport scroll**: offset + clamp/follow-cursor policy on `CharCellState`; consumers supply the inputs (cursor, hidden ranges, viewport height).
- **Drag-selection state**: derived from `SelectionModel::has_pending_selection()`; never mirrored in views or elements.
- **Kill edits**: `CodeEditorModel` (buffer mutation is model semantics); the kill *buffer* stays in the input view (readline policy).
- **Keybindings, submit, shell mode, styles, gutter on/off**: consumer policy.

Two coordinate spaces stay explicit:
- **Buffer visual-row space**: soft-wrapped buffer rows only (existing softwrap functions). Used by cursor navigation and kill ranges; unaffected by overlays.
- **Display-row space**: what is painted — ghost and gap rows interleaved, hidden rows removed. Answered by the `DisplayLattice` queries. With no overlays the two spaces are identical.

## Changes

### `crates/editor` — char-cell display-row surface

`crates/editor/src/render/model/char_cell_display.rs` (new) plus additions to `mod.rs`:

- `CharCellTemporaryBlock` + storage on `CharCellState` (replace-all semantics; `temporary_blocks()` returns a clone, never a `RefCell` guard, so callers can't hold a borrow across a layout push): ghost lines flattened from the GUI's `TemporaryBlock` (fills → `ColorU`). The `LayoutAction::LayoutTemporaryBlock` char-cell arm stores them instead of no-op'ing (previously a designed-in `TODO(TUI-diff)` extension point). Fixing this arm also fixed a latent counter leak: its early `return` skipped the outstanding-layouts bookkeeping, hanging `layout_complete()` after any char-cell block push.
- `DisplayRow` / `DisplayRowKind` / `DisplayPoint` (public): one row entry per terminal row — `Buffer { line_index }` | `Ghost { ghost_index }` | `Gap { line_range }`, plus a 0-based `Range<CharOffset>` (into buffer text or ghost content) and `is_continuation`. Style- and text-free: consumers supply strings and colors. `DisplayPoint { row: u32, col: u16 }` is the display-space analogue of `SoftWrapPoint` with `ColumnUnit::Chars` columns.
- `CharCellState::display_lattice(hidden)` — the single entry point: projects wrap tables + overlays once into a returned `DisplayLattice` that owns the immutable borrow guards for its inputs. `DisplayLattice::rows()` is the row list — buffer lines wrapped, ghosts interleaved before their `insert_before` line (same width, wide-char aware, trailing-newline stripped; at/past EOF appended), hidden lines elided into single interior gap rows (edge runs emit nothing; a ghost inside a hidden run splits the gap).
- `DisplayLattice::offset_to_display_point(char_offset)` / `display_point_to_offset(point)` — exact cursor-placement and mouse-hit-testing conversions over the lattice's own rows (no re-projection per query). Both return `Option`: offsets in hidden lines have no display point, while ghost, gap, and out-of-range rows have no buffer offset. The deferred-wrap phantom cursor mirrors `char_cell_line_gap_position` but skips interleaved ghost/gap rows: it lands on the next buffer row, or one past the entire display when none follows.
- `CharCellState::visual_row_char_range(char_offset)` — buffer visual-row space: the soft-wrapped `Range<CharOffset>` containing an offset (backs the model's kill-to-visual-row edits).
- `CharCellState` scroll state — `scroll_offset()` / `scroll_by(rows, viewport_rows, cursor_char_offset, hidden)` / `follow_cursor(cursor_char_offset, viewport_rows, hidden)`: the first visible display row of a scroll-windowed viewport plus its clamping and cursor-following policy, kept next to the display-row math it windows (the char-cell mirror of the GUI's `RenderState`-owned scroll). Both methods size the row total including the deferred-wrap phantom row the cursor can occupy.
- `SelectionModel::has_pending_selection()` (`crates/editor/src/selection.rs`) — whether a drag selection is in progress (begun by `begin_selection` on mouse down, cleared by `end_selection` on mouse up). Consumers derive drag gating from this instead of mirroring an `is_selecting` flag.
- `CharCellState::hidden_line_ranges(app)` — its attached `HiddenLinesModel` offset ranges projected to 0-based line ranges via `line_starts`. The method is only reachable after `RenderState::char_cell()` succeeds. The handle is supplied by `RenderState::new_tui`; pixel mode retains its separate optional handle for the font-layout pipeline.

Hidden line ranges are a *parameter* to `display_lattice` rather than internal state, so consumers can append structural extras (e.g. eliding a trailing empty line) to the model-derived set. Because the returned lattice binds the hidden and ghost sets at construction, painting and geometry queries against it cannot diverge.

### `app/src/code/editor/model.rs` — model-side kill edits

- `kill_to_char_cell_visual_row_end(ctx)` / `kill_to_char_cell_visual_row_start(ctx)` — char-cell-gated deletion from the primary cursor to its soft-wrapped visual-row boundary (via `visual_row_char_range`) as a user edit, returning the deleted text for the caller's kill buffer; `None` outside char-cell mode or when already at the boundary. Buffer mutation is model semantics, so the raw `BufferEditAction::Delete` plumbing lives behind these, not in views.

Scroll deliberately gets no model wrappers: it is read-side glue, so consumers gather the inputs (primary cursor, model-derived hidden ranges) through the public accessors and drive `CharCellState::follow_cursor` / `scroll_by` directly, keeping the shared model's TUI surface minimal.

### `crates/warp_tui` — the core element

`editor_element.rs` (new): `TuiEditorElement`, snapshot-based (built fresh per render), configured by builder methods:

```rust
TuiEditorElement::new(editor, app)        // snapshots text, cursor, selection, hidden ranges
    .editable()                           // cursor + printable-char insertion; omitted = read-only
    .with_viewport_rows(max_visible_rows) // window rows at the model-side scroll offset; omitted = full height
    .with_line_number_gutter()            // line numbers; blanks on continuation/ghost/gap rows
    .with_styles(TuiEditorStyles { .. })  // text/ghost/gap styles + per-line overrides — all policy
    .hide_trailing_empty_line()           // folds the final empty line into the hidden set
    .on_action(handler)                   // TuiEditorAction → consumer's typed action; omitted = inert
    .finish()
```

The element carries no scroll or drag state of its own: the first visible row is read from the char-cell render state (`CharCellState::scroll_offset`, at layout time and fresh at event time), and drag/up mouse handling is gated on the selection model's pending-selection state read fresh at event time — both model-owned, so a cached element can never disagree with the model about them.

The element *paints and interacts*; it does not compute row structure: at layout time it constructs one `DisplayLattice` (pushing `width − gutter columns` into `set_terminal_width` first, so softwrap math agrees) and builds rows, cursor position, and selection spans against that single projection — slicing its text snapshot by each row's `char_range`, applying styles and gutter cells, rendering gaps as `… {N} lines`, windowing by scroll, and keeping the phantom-row / empty-row-`" "` invariants. The structural extras (`hide_trailing_empty_line`) are folded into the hidden set via one helper (`effective_hidden_ranges`) used by both layout and events. Mouse events construct their own lattice at event time, re-deriving text and hidden ranges from the model (the presenter caches elements across frames, so construction-time snapshots may be stale) and map to a `TuiEditorAction` enum dispatched through `.on_action`; no handler ⇒ no event handling. Keybindings, kill/yank, submit, focus remain consumer policy.

### Input migration (`crates/warp_tui/src/input/view.rs`)

`TuiInputView` renders the core element verbatim (`.editable().with_viewport_rows(...).on_action(map to TuiInputAction)`). Deleted: `TuiInputElement`, `char_cell_cursor_pos` (duplicate wrap math), the selection-span loop, `offset_at` internals, the kill-helper row segmentation and buffer-edit plumbing (now `CodeEditorModel::kill_to_char_cell_visual_row_end/_start`), the view-held `scroll_offset` and scroll policy (now `CharCellState` scroll; two thin view helpers gather the cursor and hidden ranges and call `follow_cursor` / `scroll_by` — `handle_action` follows the cursor after every action except wheel scroll), and the mirrored `is_selecting` flag (derived from the selection model's pending selection; `update_pending_selection`/`end_selection` are already no-ops without one, so the view dispatches unconditionally). Kept — the input policy layer: `tui:input:*` keybindings, `KillBuffer`, `max_visible_rows` (viewport policy), submit/clear, focus, shell-mode composition. Scroll geometry is computed in display-row space inside `CharCellState` — the same space the element windows by — so viewport math and rendering agree even once overlays apply to an editable surface. Net **−600 lines** in the input view; input behavior unchanged.

## Design rationale

### Why structural overlays are core-element mechanisms

Gutter, ghost rows, and hidden ranges all change the row lattice itself — which terminal rows exist and where (wrap width, row count, scroll math). In char-cell rendering the row lattice *is* the layout, and the row list only materializes at element layout time (TUI elements build in `render()` but learn their width in `layout()`). This matches the GUI: ghost and hidden-section blocks are materialized *inside* `RichTextElement`'s block tree by core layout, and `EditorWrapper` builds its gutter by iterating the core's laid-out blocks (`gutter_elements()`, `app/src/code/editor/element.rs:591`) — the GUI wrapper annotates rows the core created; it never creates rows.

The GUI wrapper can annotate *after* inner layout only because GUI layout is retained (the persistent `SumTree<BlockItem>` outlives any element). TUI elements are transient — the row list is born and dies inside a single `layout()` call — so row knowledge must be co-located with row construction, inside the core element. Pixel space also lets the GUI paint annotations in a separate x-band; in char-cell, annotating a row occupies cells within it, shrinking the wrap width, which must be known before the first row is produced. Alternatives (consumers splicing rows into the element's output, or composing `[gutter][row]` pairs from an exported row projection) were rejected: both relocate the row lattice plus everything that spans rows (cursor, selection drags, hit-testing, scroll windowing) into every consumer — the arrangement this extraction removes.

### Why `CharCellState` owns the row structure and cross-frame state

Painting (what's on row N) and interaction geometry (what a click on row N means) must agree exactly; implementing them separately would duplicate the row-structure algorithm and any drift shifts the whole row mapping. So the row structure is computed in exactly one place — `CharCellState`, which already owns every input (wrap tables, width, ghost blocks) and already answers char-cell geometry. The GUI analogue is exact: in pixels mode the display structure lives in `RenderState`'s retained block tree, which both painting and geometry read; char-cell mode computes the equivalent on demand (affordable: O(chars), no font shaping). The projection is reified as a short-lived query object, `DisplayLattice`, returned from `CharCellState` while owning immutable borrow guards for its inputs: rows are projected once and every point query is answered against those same rows.

`CharCellState` itself is *retained* model state — the char-cell arm of `RenderState`'s `LayoutMode` — so it outlives the transient per-frame elements. That retention is what makes it the home for cross-frame session state (the scroll offset, the ghost blocks) alongside the wrap tables, and its fields are interior-mutable (`Cell`/`RefCell`) because its writers — the element pushing the wrap width during layout, scroll updates during event dispatch — hold only a shared `&AppContext`.

### Recompute, don't cache

The lattice is rebuilt on every `display_lattice` call rather than stored on `CharCellState`:
- Its inputs are not all state. The hidden set is a per-call parameter (consumers pass different sets — the diff body folds in `hide_trailing_empty_line`), and `terminal_width` is pushed during each element layout and varies with gutter columns — so a cached projection has no well-defined key.
- Every input mutates through `&self` (edits, ghost refreshes, async hidden-range recomputes, width pushes); invalidation tracking across them would reintroduce exactly the staleness class the per-scope projection eliminates. The GUI's retained block tree *is* the cached variant, and pays for it with the async layout pipeline (`LayoutAction`s, pending edits, `LayoutCache`) that char-cell mode exists to skip.
- Event-time hit-tests *want* a fresh projection: the presenter caches elements across frames, so a mouse event can arrive after the model changed.
- The cost is small — O(chars) with no shaping over prompt-sized or context-elided buffers, roughly two projections per keystroke. `display_lattice` is the single construction point, so memoization (keyed on text version × width × ghost generation × hidden set) can be added behind it later without touching consumers.

## Testing and validation

- `char_cell_display_tests.rs`: row structure (wrapping incl. wide chars, ghost interleaving/wrapping/trailing-newline stripping, interior-vs-edge gap elision, ghost-inside-hidden-run splitting) and geometry (offset ↔ display point round-trips with overlays, `None` for hidden offsets and synthetic rows, deferred-wrap phantom row incl. ghost-row skipping, `visual_row_char_range`).
- `mod_tests.rs`: `TemporaryBlock → CharCellTemporaryBlock` flattening and replace-all storage semantics; `CharCellState` scroll (scroll-by clamping, minimal-move cursor following, stale-offset clamping after content shrinks).
- `editor_element_tests.rs`: painted rows, gutter numbering/blank rules, trailing-empty-line elision, scroll windowing, empty-buffer row invariant.
- Input parity: `input/view_tests.rs` behavioral assertions unchanged (harness updated to the element's types) — empty input occupies one row, wide-char cursor columns, mouse cell → offset mapping, wheel scrolling, kill/yank.
- Suites: `cargo nextest run -p warp_editor -p warp_tui`; `./script/format` + presubmit clippy.

## Risks

- Input regression: covered by the unchanged behavioral test suite; stakes low while the TUI is unreleased.
- `crates/editor` changes are shared with the GUI: additive and char-cell-gated; existing softwrap functions untouched. The `LayoutTemporaryBlock` restructure preserves the GUI arms verbatim (lazy and direct layout paths).
- The overlay mechanisms (ghosts, hidden ranges, gutter) have no consumer on this branch — they are exercised by unit tests here and consumed by the TUI inline diff viewer stacked on top.
