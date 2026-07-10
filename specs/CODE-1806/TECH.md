# TECH: Text selection in the TUI transcript view

Linear: CODE-1806. Product behavior: [PRODUCT.md](./PRODUCT.md).

## Context

The headless TUI runs with terminal mouse capture, so host-terminal selection is unavailable. The transcript is rendered by `TuiViewportedList<TuiBlockListViewportSource>` over absolute content rows (`crates/warpui_core/src/elements/tui/viewported_list.rs`, `crates/warp_tui/src/tui_block_list_viewport_source.rs`). Selection therefore lives in the TUI element tree and uses the viewport's existing content coordinate system.

Selection is visual/grid-based rather than model-offset-based:

- points are absolute `(row, column)` cells;
- highlighting is reverse video over rendered cells;
- copy extracts symbols from rendered row buffers;
- horizontal resize clears selection because wrapping changes cell coordinates.

## Architecture

### Thin generic wrapper

`TuiSelectable<Child>` in `crates/warpui_core/src/elements/tui/selectable.rs` owns no selection geometry or content model. It:

- gives normal child interactions first refusal on mouse-down;
- captures drag/up while the child reports an active selection gesture;
- delegates selection events through `TuiSelectableElement`;
- dispatches selection-start and copy callbacks;
- asks the child to paint selection after normal rendering.

`TuiSelectionEventResult` communicates `Unhandled`, `Started`, `Changed`, or `Completed(Option<String>)`. The wrapper translates results into notifications and callbacks.

`TuiSelectable<Child>` implements `TuiScrollableElement` when its child does, so normal `TuiScrollable` wheel handling remains unchanged.

### Viewport-owned selection

`TuiViewportedList` optionally accepts `TuiSelectionConfig` and implements `TuiSelectableElement` (`crates/warpui_core/src/elements/tui/viewported_list.rs`).

Because the viewport owns resolved layout and scrolling, it performs all selection-specific work:

- screen-to-content hit-testing using its resolved clamped window;
- character, semantic-word, and line unit resolution;
- edge-drag autoscroll through its canonical row scrolling;
- visible-cell snapshots and glyph validation;
- reverse-video highlight painting;
- off-screen selected-row extraction;
- ordered row-resize rebasing after layout.

Mouse-wheel and page scrolling preserve absolute selection anchors. Selected rows stop highlighting while off-screen and highlight again when they return.

### Selection state

`TuiSelectionHandle` persists across per-frame element reconstruction (`crates/warpui_core/src/elements/tui/selectable/state.rs`). It stores:

- anchor and focus spans;
- character/word/line mode;
- active-gesture state;
- render width;
- selected-cell glyph snapshots.

The handle clears on width change, removed selected rows, or visible selected glyph mutation. It rebases points and glyph snapshots through ordered row-resize batches.

Word selection uses a resolver supplied by `TuiSelectionConfig`. The transcript resolver applies `SemanticSelection::smart_search` and configured boundary characters (`crates/warp_tui/src/transcript_word_selection.rs`), keeping `warpui_core` independent of Warp-specific semantic-selection settings.

### Viewport content contract

`TuiViewportedElement` directly provides two optional selection hooks (`crates/warpui_core/src/elements/tui/viewported_list.rs`):

- `selection_content(window, width, app)` returns arbitrary rows without mutating layout state;
- `take_selection_row_resizes()` drains original row ranges and new heights produced by the latest layout.

Defaults disable off-screen extraction and return no resizes. No extension trait or parallel viewport model exists.

`TuiBlockListViewportSource` implements both hooks:

- normal `visible_items` drains dirty rich content, measures heights, updates the canonical block list, and records resize entries;
- `selection_content` uses the existing block-list traversal without measurement or mutation;
- resize entries are collected in canonical block-list order and drained by the viewport after child layout.

### Transcript composition

`TuiTranscriptView::render` (`crates/warp_tui/src/transcript_view.rs`) creates:

1. `TuiBlockListViewportSource`;
2. `TuiViewportedList` with `GrowFromBottom` alignment and `TuiSelectionConfig`;
3. `TuiSelectable` callbacks for cross-surface clearing and copy;
4. the existing `TuiScrollable` wheel driver.

The transcript view owns the selectable region. `TuiTerminalSessionView` owns cross-surface policy and clipboard side effects (`crates/warp_tui/src/terminal_session_view.rs`).

## Event flow

### Mouse down

1. `TuiSelectable` dispatches to interactive descendants first.
2. If unhandled, it delegates to `TuiViewportedList::dispatch_selection_event`.
3. The viewport maps the pointer through its resolved window and starts character, word, or line selection.
4. `Started` causes the session to clear input-editor selection.

### Drag and scrolling

1. The wrapper captures drag/up for the active selection.
2. The viewport scrolls through its own `scroll_by` implementation when the pointer leaves the top or bottom.
3. Focus updates in absolute content coordinates.
4. Normal wheel/page scrolling passes through `TuiScrollable` and preserves selection.

### Render

1. The viewport renders visible children normally.
2. The selectable wrapper calls the viewport's selection paint method.
3. The viewport snapshots unstyled visible cells, validates selected glyphs, and applies reverse-video rectangles.

### Mouse up and copy

1. The viewport ends the gesture and orders anchor/focus.
2. It requests selected row windows through `TuiViewportedElement::selection_content`.
3. Rows are rendered with the viewport's canonical clipping helper, scraped by cell width, trimmed, and joined with newlines.
4. `Completed(Some(text))` dispatches `TranscriptSelectionEnded`.
5. The session writes OSC 52 clipboard and PRIMARY targets and shows the success hint (`crates/warp_tui/src/clipboard.rs`, `crates/warp_tui/src/transient_hint.rs`).

## Content updates and invalidation

- Appending output below selected cells preserves selection.
- Height changes above selection rebase anchors by the cumulative delta.
- Growth below a selected range does not select appended rows.
- Shrink/removal clears only when selected rows are removed; later rows rebase.
- Visible selected glyph changes clear selection.
- Auto-follow and explicit scrolling preserve anchors.
- Horizontal resize clears selection.

Conversation removal obtains the rich-content row range before deletion and applies a shrink-to-zero rebase (`crates/warp_tui/src/transcript_view.rs`).

## Exclusivity

`TuiTerminalSessionView` owns the single-selection-domain invariant:

- transcript selection start clears the input editor through `TranscriptSelectionStarted`;
- non-empty input-editor selection clears `TuiSelectionHandle`;
- typing alone does not clear transcript selection;
- active drag ownership remains with the originating surface until mouse-up.

## Validation

Focused viewport tests in `crates/warpui_core/src/elements/tui/viewported_list_tests.rs` cover:

- linear highlight and copy;
- edge-drag selection into newly scrolled rows;
- first-mouse suppression;
- selection persistence through wheel scrolling;
- reverse-video modifier toggling;
- canonical viewport clamping, bottom alignment, and resolved geometry.

Selection-state tests in `crates/warpui_core/src/elements/tui/selectable/state_tests.rs` cover cumulative resize rebasing.

Warp TUI tests cover:

- read-only transcript row extraction and explicit resize reporting (`tui_block_list_viewport_source_tests.rs`);
- input selection clearing (`input/view_tests.rs`);
- OSC 52 and tmux encoding (`clipboard_tests.rs`);
- transient success hints (`transient_hint_tests.rs`).

Before submission run:

- `./script/format`
- `cargo test -p warpui_core --lib --features tui viewported_list`
- `cargo test -p warpui_core --lib --features tui selectable::state::tests`
- `cargo nextest run -p warp_tui`
- focused Clippy for `warpui_core` and `warp_tui` with warnings denied

## Risks

- Very large selections synchronously materialize and encode many rows; add a bounded copy policy if profiling shows foreground stalls.
- OSC 52 may be disabled by the host terminal; stdout success cannot prove terminal clipboard acceptance.
- Off-screen extraction intentionally reads geometry from the last completed layout and must remain mutation-free.
