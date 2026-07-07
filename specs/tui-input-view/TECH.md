# TUI Input View — Tech Spec (Milestone 1)

Commit ref: `724c54771e2a06766257bc20f0053c6737a7d1b8`

> This spec documents the **as-built** Milestone 1 implementation. Where the
> original plan diverged during implementation, this reflects what actually
> landed.

## Context

The TUI runtime introduces a parallel rendering path — ratatui + crossterm instead of WarpUI's GPU renderer — with its own entity lifecycle (`TuiView`) and element traits (`TuiElement`) in `crates/warpui_core`. Existing TUI elements (`TuiText`, `TuiColumn`, `TuiContainer`, `TuiEventHandler`) are display-only or generic input hooks; there was no text input widget.

`crates/editor/` provides the reusable editing core: the `CoreEditorModel` / `PlainTextEditorModel` traits, the `Buffer` (rope-style text storage with undo/redo, word-boundary movement, selection), `SelectionModel`, and `RenderState`. The crate has no GPU coupling — it depends only on `warpui_core` for `AppContext` / `ModelHandle`.

**Key architectural decision**: rather than build a parallel `TuiRenderState` or a bespoke `TuiInputModel`, the TUI input view reuses the existing `CodeEditorModel` (the app's plain-text editor model) in a new **char-cell layout mode**. `RenderState` gains a `LayoutMode` enum; in `LayoutMode::CharCell` it skips the font engine and computes soft-wrap positions with monospace character-count arithmetic. `SelectionModel` stays non-generic (it still holds `ModelHandle<RenderState>`). This makes the full editing model — vim-capable navigation, syntax, diff, hidden lines — reusable by the TUI for free, and positions `RenderState` to handle future TUI rich-text content by extending the `CharCell` branch rather than building a separate layout system.

**Scope of this milestone**: a functional multi-line, selectable editor with Emacs/readline keybindings, living in the `warp_tui` crate and exercised by unit tests and an interactive example. Wiring it into the `warp-tui` binary's runtime, input-mode switching, slash-command detection, and history navigation are explicitly deferred (see Follow-ups).

## Proposed Changes

### 0. Prerequisite refactor — `ColumnUnit` + `LayoutMode` in `crates/editor/`

**`ColumnUnit`** (`crates/editor/src/render/model/mod.rs`): the horizontal component of `SoftWrapPoint` becomes an explicit enum instead of a bare `Pixels`, so the GUI (proportional, pixel) and TUI (monospace, char-cell) coordinate spaces are distinguished at the type level:

```rust
pub enum ColumnUnit {
    Pixels(Pixels), // GPU-rendered GUI path
    Chars(u16),     // TUI char-cell path
}

pub struct SoftWrapPoint {
    row: u32,
    column: ColumnUnit, // was: Pixels
}
```

Mixing variants in a comparison or arithmetic expression is a bug; helper methods (`pixels_zero`, `chars_zero`, `col_max`, `as_pixels`, `as_chars`) `debug_assert!` on mismatch and fall back gracefully in release. The sticky-goal field in `SelectionModel` becomes `goal_xs: Option<Vec1<ColumnUnit>>` and `NavigationResult.goal_x` becomes `Option<ColumnUnit>`. `navigate_line`'s sticky-column logic is unchanged in shape — it threads `ColumnUnit` through instead of `Pixels`. The ~15 existing GUI construction sites become `SoftWrapPoint::new(row, ColumnUnit::Pixels(px))` (mechanical).

**`LayoutMode`** (`crates/editor/src/render/model/mod.rs`): `RenderState` gains a `layout_mode: LayoutMode` field alongside its existing `styles: RichTextStyles` field (styles are retained for API compatibility but unused in char-cell mode):

```rust
pub enum LayoutMode {
    Pixels,                // font-aware pixel layout (GUI)
    CharCell(CharCellState),
}

pub struct CharCellState {
    pub terminal_width: Cell<u16>,    // interior-mutable: pushed during element layout
    line_starts: RefCell<Vec<usize>>, // 0-based char index of each logical line start
    char_widths: RefCell<Vec<u8>>,    // per-char display width (derived; NOT a text copy)
}
```

`char_widths` deliberately stores only each character's display width (0/1/2), **not** the buffer text. The render-state query methods are `&self` with no `AppContext` and `RenderState` does not hold the `Buffer`, so the per-char data layout needs must live here; storing widths (1 byte/char of derived metadata, like `line_starts`) instead of a `Vec<char>` avoids duplicating the text. The view builds row strings from the live buffer text directly, so it never needs this copy.

Construction and APIs:
- `RenderState::new_internal` gains a `layout_mode` parameter; existing pixel constructors pass `LayoutMode::Pixels`.
- New `RenderState::new_tui(terminal_width, styles, ctx)` constructs a `CharCell` `RenderState`. Callers supply a stub `RichTextStyles` (the field is never read for char-cell layout).
- `RenderState::char_cell() -> Option<&CharCellState>` is the single gateway to char-cell state. It returns `Some` only in `CharCell` mode, so the char-cell ops below are simply unreachable in pixel mode (no implicit "CharCell-only" runtime contract on `RenderState`). On `CharCellState`: `terminal_width()` / `set_terminal_width(u16)` (interior-mutable, so the element can push width during its layout pass with only a shared `&AppContext`) and `update_text(&str)` (rebuilds `line_starts` + per-char `char_widths` from the buffer text, O(n) char scan).
- Public per-line primitives are the single source of truth for the wrapping rule, shared by both the editor conversions and the `warp_tui` view, and all operate on a line's per-char display widths (`&[u8]`): `char_cell_display_width(char)` (terminal cell width via `unicode-width`, used to build the width slices), `char_cell_line_row_starts(widths, terminal_width)` (char indices where each visual row begins), and `char_cell_line_gap_position(widths, terminal_width, char_in_line)` (`(row, display_col)` of a cursor gap).

Layout behaviour in `CharCell` mode:
- `handle_layout_action`'s `BufferEdit` arm is a no-op — the async font-shaping channel and `LayoutCache` are bypassed entirely.
- `offset_to_softwrap_point`, `softwrap_point_to_offset`, and `max_line` branch on `layout_mode` and delegate to free functions (`char_cell_offset_to_softwrap_point`, `char_cell_softwrap_point_to_offset`, `char_cell_max_line`) built on the per-line primitives above. They use a 0-based soft-wrap API (callers pass `cursor_offset - 1` and re-add 1, matching the existing convention so `navigate_line` stays layout-mode-agnostic).
- Wrapping is **display-width aware**: wide CJK/emoji occupy two columns and wrap to the next row when they don't fit; zero-width/combining marks share their base character's column. ASCII (all width-1) layout is identical to simple `idx / width` arithmetic, so existing behaviour is unchanged.
- `char_cell_softwrap_point_to_offset` resolves the target row within its logical line and clamps the column to that line's end (the final line is bounded by the buffer length), so it never returns an offset past the end of the buffer even when the target column is beyond a shorter final line.

**Blast radius**: `SoftWrapPoint` construction sites (mechanical), `RenderState::new_internal` (signature + call sites), `handle_layout_action` (CharCell arm), `offset_to_softwrap_point` / `softwrap_point_to_offset` / `max_line` (new branches). No changes to GUI rendering behaviour.

### 1. `CodeEditorModel::new_tui` — char-cell editor (no separate model)

There is **no** `TuiInputModel`. The TUI input is backed directly by the existing `CodeEditorModel` (`app/src/code/editor/model.rs`), constructed in char-cell mode:

- `CodeEditorModel::new_tui(terminal_width, ctx)` builds the model with a `CharCell` `RenderState`. It shares all sub-model wiring (buffer, `BufferSelectionModel`, `SyntaxTreeState`, `DiffModel`, `HiddenLinesModel`, `SelectionModel`, comments) with the GUI `new()` via a common `from_content(..)` helper; the only differences are the `RenderState` constructor and a few flags (`show_current_line_highlights = false`, lazy layout disabled).
- Syntax colours come from the `Appearance` singleton via the same `syntax_highlighting_color_map(ctx)` path as the GUI, so callers must register `Appearance` (a real one at runtime; `Appearance::mock()` in tests/examples). The `RichTextStyles` handed to `RenderState::new_tui` is a local stub (`tui_stub_text_styles()`), since char-cell layout never reads it.
- The terminal width is pushed during the element's layout pass (see §2) via `render_state.char_cell()?.set_terminal_width(..)` (interior-mutable). `line_starts`/`char_widths` don't depend on the width, so no text rebuild is needed on resize.
- **Keeping char-cell layout in sync**: `CodeEditorModel`'s `CoreEditorModel::on_buffer_version_updated` override calls `render_state.char_cell()` and, when `Some`, synchronously calls `update_text(text)` on it. Because the async font-shaping pipeline is bypassed in `CharCell` mode, this guaranteed-synchronous post-edit hook is what keeps `max_line` / `offset_to_softwrap_point` correct within the same frame as each edit.

`app/src/editor/mod.rs` re-exports `CodeEditorModel` / `CodeEditorModelEvent` so the `warp_tui` crate can construct and subscribe to it.

### 2. `TuiInputView` — view in `crates/warp_tui/src/input/view.rs`

`TuiInputView` implements `TuiView` + `TypedActionView`. It holds `ModelHandle<CodeEditorModel>` plus all **TUI-specific session state** (deliberately kept on the view, not the model):

```rust
pub struct TuiInputView {
    model: ModelHandle<CodeEditorModel>, // char-cell mode
    kill_buffer: KillBuffer,             // single-entry (Ctrl+K/U/W + Ctrl+Y)
    scroll_offset: u32,                  // first visible visual row (0-indexed)
    max_visible_rows: u32,               // = 6
}
```

**Rendering**: `render(&self, ctx) -> Box<dyn TuiElement>` only gathers width-*independent* state (plain text, cursor offset, selection range, scroll offset) plus a model-handle clone into a `TuiInputElement`. All width-dependent work happens in `TuiInputElement::layout(constraint, ctx, app)` — the first point that knows the terminal width (from the constraint), mirroring the GUI where the element computes geometry in `layout`. There it: pushes the width onto the model (`char_cell().set_terminal_width`, interior-mutable) so event-time navigation/scroll read it; builds the visible rows with `build_visual_rows_with_offsets(text, width)` and the cursor `(row, col)` with `char_cell_cursor_pos(...)`; then assembles the `TuiColumn`, applies `Modifier::REVERSED` to selected spans, and reports the block cursor via `cursor_position()`. These pure helpers operate directly on the plain text (independent of the `RenderState` SumTree).

`visual_line_count()` reads `render_state().max_line()` and `scroll_to_cursor()` uses `render_state().offset_to_softwrap_point()` — both reading the width the element pushed during the previous layout. Height is effectively capped at `max_visible_rows` (6) via the scroll logic.

**Input** (`TypedActionView`): key events are mapped to a `TuiInputAction` enum inside `TuiInputElement::dispatch_event` (matching on `keystroke` ctrl/alt/shift + key, and printable `chars`), then dispatched via `event_ctx.dispatch_typed_action`. `handle_action` applies each action to the model and finally runs `scroll_to_cursor` + `ctx.notify()`.

Keybinding table (Milestone 1):

| Key(s) | `TuiInputAction` → model |
|--------|--------------------------|
| `Char(c)` | `InsertChar` → `user_insert` |
| `Shift+Enter` / `Ctrl+J` / `Alt+Enter` | `InsertNewline` → `user_insert("\n")` |
| `Enter` | `Submit` → emits `TuiInputViewEvent::Submitted(text)` |
| `Backspace` / `Ctrl+H` | `Backspace` |
| `Delete` / `Ctrl+D` | `DeleteForward` |
| `←` / `Ctrl+B`, `→` / `Ctrl+F` | `MoveLeft` / `MoveRight` |
| `Alt+←/→`, `Alt+B/F`, `Ctrl+←/→` | `MoveWordLeft` / `MoveWordRight` |
| `↑` / `Ctrl+P`, `↓` / `Ctrl+N` | `MoveUp` / `MoveDown` |
| `Home` / `Ctrl+A`, `End` / `Ctrl+E` | `MoveToLineStart` / `MoveToLineEnd` |
| `Shift+←/→/↑/↓` | `SelectLeft/Right/Up/Down` |
| `Ctrl+Shift+←/→`, `Alt+Shift+←/→` | `SelectWordLeft` / `SelectWordRight` |
| `Ctrl+Shift+A` / `Meta+A` | `SelectAll` |
| `Ctrl+W` / `Alt+Backspace` / `Ctrl+Backspace` | `DeleteWordBackward` |
| `Alt+D` / `Alt+Delete` / `Ctrl+Delete` | `DeleteWordForward` |
| `Ctrl+K`, `Ctrl+U` | `KillToLineEnd` / `KillToLineStart` |
| `Ctrl+Y` | `Yank` |
| `Ctrl+Z`, `Ctrl+Shift+Z` | `Undo` / `Redo` |

**Kill/yank**: kill ranges are computed with pure text helpers (`visual_line_end_exclusive`, `visual_line_start_idx`) and applied via `Buffer` edits; the killed text is stored in the single-entry `KillBuffer`, and `Yank` re-inserts it.

**Events**: `TuiInputView` emits `TuiInputViewEvent::Submitted(String)` on `Enter`. (No separate `Changed` event — parents that need content updates subscribe to the model's `CodeEditorModelEvent::ContentChanged`.)

**Resize**: there is no dedicated resize hook. The presenter lays out against the current terminal size every frame and `TuiElement::layout` receives the `AppContext`, so `TuiInputElement::layout` re-derives the width from the constraint and pushes it onto the model — wrapping and cursor math stay correct as the terminal resizes.

### 3. Module layout

```
crates/warp_tui/src/
    input/
        mod.rs          — pub use TuiInputView, TuiInputViewEvent
        view.rs         — TuiInputView (TuiView + TypedActionView), TuiInputAction,
                          TuiInputElement, pure char-cell helpers
        view_tests.rs   — cursor/coordinate/kill regression tests
        kill_buffer.rs  — KillBuffer (single-entry for M1)
```

The editor-crate refactor is additive to existing files (`render/model/mod.rs`, `selection.rs`). The `new_tui` constructor lives on the existing `CodeEditorModel` in `app/src/code/editor/model.rs`. `app/src/tui/mod.rs` remains the auth-only headless entry point; the input view is not yet wired into the `warp-tui` runtime (next step).

**Framework change (resize)**: `TuiElement::layout` takes an `app: &AppContext` parameter (mirroring the GUI `Element::layout`), threaded through the presenter and every element impl. This lets an element refresh viewport-dependent model state during layout, so terminal resizes flow through the normal layout pass — no `TuiView::on_resize` hook is needed. `TuiRuntime::draw_if_dirty` marks the window dirty on a size change; the presenter then lays out against the new size, and each element's `layout` sees it.

### 4. Dependency notes

- `crates/warp_tui` depends on `warp` (with the `tui` feature) for `CodeEditorModel`, on `warp_editor` for the editing traits/types, and on `warpui_core` (with `tui`) for the TUI elements/runtime.
- `app`'s `tui` feature enables `warpui_core/tui`; `CodeEditorModel::new_tui` is not feature-gated.
- `warp_tui` dev-dependencies enable `warp_core`'s `test-util` feature so tests/examples can register `Appearance::mock()`.

## Diagram

```
TuiInputView : TuiView + TypedActionView
│  state: kill_buffer, scroll_offset, max_visible_rows
│
├── render() gathers plain_text, cursor_offset, selection_range
│   → TuiInputElement
│        └─ layout(constraint, ctx, app):
│             push width → model (char_cell().set_terminal_width)
│             build_visual_rows_with_offsets() + char_cell_cursor_pos()
│             → TuiColumn rows, REVERSED selection, cursor_position() ─► ratatui Buffer
│
├── dispatch_event() → TuiInputAction → handle_action() ──► CodeEditorModel (LayoutMode::CharCell)
│      keybinding table                                     ├── Buffer (rope, undo/redo, word ops)
│                                                           ├── SelectionModel (non-generic)
│                                                           │     └── navigate_line() sticky-column (ColumnUnit)
│                                                           ├── on_buffer_version_updated → char_cell().update_text
│                                                           └── RenderState (CharCell)
│                                                                 ├── line_starts + char_widths (derived; no text copy)
│                                                                 ├── offset_to_softwrap_point → ColumnUnit::Chars
│                                                                 ├── max_line (drives visual_line_count)
│                                                                 └── skips LayoutCache / font engine
│
└── emits TuiInputViewEvent::Submitted(String)  (consumed by parent TuiView)
```

## Testing and Validation

**Editor char-cell unit tests** (`crates/editor/src/render/model/mod_tests.rs`, module `char_cell` — 18 tests):
- `char_cell_max_line` for empty, short, wrapping, multi-logical-line, and empty-logical-line content.
- `char_cell_offset_to_softwrap_point` for single/wrapping/multi-line content, returning `ColumnUnit::Chars`.
- `offset → point → offset` round-trips across offsets and `terminal_width` values (single line and wrapping).
- Explicit checks that the char-cell path returns `ColumnUnit::Chars` (not `Pixels`) and that offset 0 maps to row 0 / col 0; that the final shorter line clamps without exceeding the buffer.
- Unicode display width: wide CJK chars occupy two columns and wrap when they don't fit (with round-trips), zero-width/combining marks don't advance the column, and `char_cell_line_row_starts` breaks on wide-char boundaries.

**`TuiInputView` tests** (`crates/warp_tui/src/input/view_tests.rs` — 14 tests): drive a real `CodeEditorModel` (char-cell) behind a real `TuiInputView` (registering `Appearance::mock()`), covering cursor placement on empty/multi-line buffers, empty-line handling, up/down navigation across blank lines, selection text, `Ctrl+K` / `Ctrl+U` / `Ctrl+Y` kill-yank behaviour, and display-width cursor positioning for wide (CJK) and zero-width characters.

**Examples (manual smoke)**:
- `crates/warp_tui/examples/tui_input_demo.rs` — interactive editor-backed input demo. (Since removed — the input view is exercised by the real `warp-tui` binary via `./script/run-tui`.)
- `crates/warpui_core/examples/tui_file_viewer.rs` — validates the TUI runtime/rendering pipeline independently (scrollable file viewer, no editor dependency). Run: `cargo run -p warpui_core --example tui_file_viewer --features tui -- <path>`.

## Risks and Mitigations

**Two char-cell layout call sites**: the view builds row strings/cursor with its own helpers (`build_visual_rows_with_offsets`, `char_cell_cursor_pos`, kill-range helpers) while line-count/scroll use the `RenderState` char-cell path — but both now delegate to the same shared per-line primitives (`char_cell_line_row_starts` / `char_cell_line_gap_position`), so they apply one wrapping rule. The round-trip and view tests guard the overlap; a future cleanup could route row-string building through the editor API too.

**Unicode display width**: cell widths come from `unicode-width` (Unicode East Asian Width). Widths are summed per `char`, so multi-`char` grapheme clusters (e.g. ZWJ emoji sequences) can mismeasure; full grapheme-cluster segmentation is a future refinement.

**Shift+Enter terminal support**: crossterm only delivers `Shift+Enter` distinctly in terminals supporting the Kitty keyboard protocol; elsewhere it arrives as bare `Enter`. The `Ctrl+J` fallback always inserts a newline.

**Selection rendering**: ratatui has no selection-highlight primitive, so `TuiInputElement` applies `Modifier::REVERSED` to selected cell spans manually. Tested with empty and non-empty selections.

**`Appearance` dependency**: `new_tui` reads syntax colours from the `Appearance` singleton (shared with the GUI). Contexts that build the model must register one — a real `Appearance` at runtime, `Appearance::mock()` in tests/examples. When the input view is wired into the `warp-tui` runtime, that runtime will need to register `Appearance`.

## Follow-ups

Intentionally out of scope for M1; each should become its own task:

- **Wire into the `warp-tui` runtime**: render `TuiInputView` in the `warp_tui` binary (today only auth runs in `app/src/tui/mod.rs`); register `Appearance` there. (Resize is already handled through the layout pass — `TuiElement::layout` receives the `AppContext` — so this remaining item is just mounting the view in the binary.)
- **Input mode (Agent / Shell)**: wire `BlocklistAIInputModel`; placeholder text and submit routing per mode.
- **Slash command menu**: render an overlay on the `Composing` state.
- **History (up-arrow)**: open a TUI history overlay; add an "is cursor on first visual row" trigger.
- **Vim mode**: gate the editor's vim navigation on a user setting.
- **Kill ring**: extend the single-entry `KillBuffer` to a multi-entry ring (`Alt+Y` to cycle).
- **Clipboard integration**: `Ctrl+V` paste from the system clipboard.
