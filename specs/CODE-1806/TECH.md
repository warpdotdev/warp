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

### Generic selection wrapper

`TuiSelectable<Child>` in `crates/warpui_core/src/elements/tui/selectable.rs` owns the persistent `TuiSelectionHandle` and mouse gesture state machine. It:

- gives normal child interactions first refusal on mouse-down;
- captures drag/up while its selection handle reports an active gesture;
- expands child-resolved points and stores selection spans in absolute content coordinates;
- clears width-invalid state before child layout and applies actual child-reported row resizes afterward;
- dispatches selection-start and copy callbacks;
- asks the child to paint the wrapper-owned selection state after normal rendering.

`TuiSelectableElement` is the child adapter contract. It resolves screen positions into content points, returns rendered row glyphs, materializes text for a supplied span, reports content-row resizes, and renders supplied selection state.

`TuiSelectable` owns semantic expansion, matching GUI `SelectableArea`: it stores a `WordBoundariesPolicy` and optional smart-select function, tries smart selection first, and otherwise expands rendered glyphs according to the configured policy.

`TuiSelectable<Child>` implements `TuiScrollableElement` when its child does, so normal `TuiScrollable` wheel handling remains unchanged.

### Viewport selection adapter

`TuiViewportedList` implements `TuiSelectableElement` (`crates/warpui_core/src/elements/tui/viewported_list.rs`).

Because the viewport owns resolved layout and scrolling, it performs the content-specific work:

- screen-to-content hit-testing using its resolved clamped window;
- edge-drag autoscroll through its canonical row scrolling;
- rendered row-glyph extraction for wrapper-owned unit resolution;
- visible-cell snapshots and glyph validation;
- reverse-video highlight painting;
- off-screen selected-row extraction;
- indexed row-range lookup for rich-content blocks whose measured row height actually changed.

The viewport does not create or retain selection state or word policy. It returns content points, row glyphs, and resize mappings to `TuiSelectable` and renders from the handle passed down by the wrapper.

Mouse-wheel and page scrolling preserve absolute selection anchors. Selected rows stop highlighting while off-screen and highlight again when they return.

### Selection state

`TuiSelectionHandle` persists across per-frame element reconstruction (`crates/warpui_core/src/elements/tui/selectable/state.rs`). The transcript retains the handle and supplies a clone to each rebuilt `TuiSelectable`. It stores:

- anchor and focus spans;
- character/word/line mode;
- active-gesture state;
- render width;
- selected-cell glyph snapshots.

The handle clears on width change before child layout, removed selected rows, or visible selected glyph mutation. It rebases points and glyph snapshots through ordered row-resize batches, skipping batches without selection state and changes entirely below the selected rows.

The transcript configures `TuiSelectable` with `SemanticSelection::word_boundary_policy()` and `SemanticSelection::smart_select_fn()`. Generic glyph-to-word resolution remains in `warpui_core`; Warp-specific settings are converted into the same policy and function used by GUI selection.

### Viewport content contract

`TuiViewportedElement` directly provides two optional selection hooks (`crates/warpui_core/src/elements/tui/viewported_list.rs`):

- `selection_content(window, width, app)` returns arbitrary rows without mutating layout state;
- `take_selection_row_resizes()` drains original row ranges and new heights produced by the latest layout.

Defaults disable off-screen extraction and return no resizes. No extension trait or parallel viewport model exists.

`TuiBlockListViewportSource` implements both hooks:

- normal `visible_items` drains dirty rich content, measures heights, updates the canonical block list, and records resize entries;
- `selection_content` uses the existing block-list traversal without measurement or mutation;
- measured heights use the block list's indexed rich-content positions to emit resize entries only when row count actually changes; entries are reported in canonical order and applied by `TuiSelectable` after child layout.

### Transcript composition

`TuiTranscriptView::render` (`crates/warp_tui/src/transcript_view.rs`) creates:

1. `TuiBlockListViewportSource`;
2. `TuiViewportedList` with `GrowFromBottom` alignment;
3. `TuiSelectable` with the transcript's persistent handle, semantic-selection policy, smart-select function, and transcript-owned selection actions;
4. the existing `TuiScrollable` wheel driver.

The transcript view owns the selectable region and translates element actions into `TuiTranscriptViewEvent`s. `TuiTerminalSessionView` subscribes to those child-view events and owns cross-surface policy and clipboard side effects (`crates/warp_tui/src/terminal_session_view.rs`).

## Event flow

### Mouse down

1. `TuiSelectable` dispatches to interactive descendants first.
2. If unhandled, the wrapper asks the viewport to resolve the pointer through its current window.
3. The wrapper expands the returned content point into a character, word, or line span and starts selection.
4. The transcript handles the selection-start action and emits an event that causes the subscribed session to clear input-editor selection.

### Drag and scrolling

1. The wrapper captures drag/up for its active selection.
2. It asks the viewport to resolve each new pointer position; the viewport scrolls through its own `scroll_by` implementation when the pointer leaves the top or bottom.
3. The wrapper expands the returned point with its configured selection policy and stores the focus span in absolute content coordinates.
4. Normal wheel/page scrolling passes through `TuiScrollable` and preserves selection.

### Render

1. The viewport renders visible children normally.
2. The selectable wrapper passes its selection handle to the viewport's selection paint method.
3. The viewport snapshots unstyled visible cells, validates selected glyphs against the supplied state, and applies reverse-video rectangles.

### Mouse up and copy

1. The wrapper ends the gesture and orders anchor/focus.
2. It asks the viewport to materialize the resolved span; the viewport requests selected row windows through `TuiViewportedElement::selection_content`.
3. Rows are rendered with the viewport's canonical clipping helper, scraped by cell width, trimmed, and joined with newlines.
4. The wrapper dispatches a transcript-owned completion action, and the transcript emits the selected text as a view event.
5. The subscribed session writes OSC 52 clipboard and PRIMARY targets and shows the success hint (`crates/warp_tui/src/clipboard.rs`, `crates/warp_tui/src/transient_hint.rs`).

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

- transcript selection start emits a transcript view event that clears the input editor;
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
