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
    pub terminal_width: u16,
    line_starts: RefCell<Vec<usize>>, // 0-based char index of each logical line start
    total_chars: Cell<usize>,
}
```

Construction and APIs:
- `RenderState::new_internal` gains a `layout_mode` parameter; existing pixel constructors pass `LayoutMode::Pixels`.
- New `RenderState::new_tui(terminal_width, styles, ctx)` constructs a `CharCell` `RenderState`. Callers supply a stub `RichTextStyles` (the field is never read for char-cell layout).
- `is_char_cell_mode() -> bool`.
- `update_char_cell_text(&str)` rebuilds `line_starts` / `total_chars` from the current buffer text (O(n) char scan).
- `set_char_cell_terminal_width(u16)` updates the width.

Layout behaviour in `CharCell` mode:
- `handle_layout_action`'s `BufferEdit` arm is a no-op — the async font-shaping channel and `LayoutCache` are bypassed entirely.
- `offset_to_softwrap_point`, `softwrap_point_to_offset`, and `max_line` branch on `layout_mode` and delegate to free functions: `char_cell_offset_to_softwrap_point`, `char_cell_softwrap_point_to_offset`, `char_cell_max_line`. These operate on `line_starts` + `terminal_width` with a 0-based soft-wrap API (callers pass `cursor_offset - 1` and re-add 1, matching the existing convention so `navigate_line` stays layout-mode-agnostic).
- `char_cell_softwrap_point_to_offset` treats the final (unbounded) logical line as spanning all remaining rows (`u32::MAX`) so a target row past the end resolves there rather than overflowing `usize → u32`.

**Blast radius**: `SoftWrapPoint` construction sites (mechanical), `RenderState::new_internal` (signature + call sites), `handle_layout_action` (CharCell arm), `offset_to_softwrap_point` / `softwrap_point_to_offset` / `max_line` (new branches). No changes to GUI rendering behaviour.

### 1. `CodeEditorModel::new_tui` — char-cell editor (no separate model)

There is **no** `TuiInputModel`. The TUI input is backed directly by the existing `CodeEditorModel` (`app/src/code/editor/model.rs`), constructed in char-cell mode:

- `CodeEditorModel::new_tui(terminal_width, ctx)` builds the model with a `CharCell` `RenderState`. It shares all sub-model wiring (buffer, `BufferSelectionModel`, `SyntaxTreeState`, `DiffModel`, `HiddenLinesModel`, `SelectionModel`, comments) with the GUI `new()` via a common `from_content(..)` helper; the only differences are the `RenderState` constructor and a few flags (`show_current_line_highlights = false`, lazy layout disabled).
- Syntax colours come from the `Appearance` singleton via the same `syntax_highlighting_color_map(ctx)` path as the GUI, so callers must register `Appearance` (a real one at runtime; `Appearance::mock()` in tests/examples). The `RichTextStyles` handed to `RenderState::new_tui` is a local stub (`tui_stub_text_styles()`), since char-cell layout never reads it.
- `CodeEditorModel::set_tui_terminal_width(width, ctx)` calls `RenderState::set_char_cell_terminal_width` then `update_char_cell_text` with the current buffer text.
- **Keeping char-cell layout in sync**: `CodeEditorModel`'s `CoreEditorModel::on_buffer_version_updated` override checks `is_char_cell_mode()` and, when true, synchronously calls `update_char_cell_text(text)`. Because the async font-shaping pipeline is bypassed in `CharCell` mode, this guaranteed-synchronous post-edit hook is what keeps `max_line` / `offset_to_softwrap_point` correct within the same frame as each edit.

`app/src/editor/mod.rs` re-exports `CodeEditorModel` / `CodeEditorModelEvent` so the `warp_tui` crate can construct and subscribe to it.

### 2. `TuiInputView` — view in `crates/warp_tui/src/input/view.rs`

`TuiInputView` implements `TuiView` + `TypedActionView`. It holds `ModelHandle<CodeEditorModel>` plus all **TUI-specific session state** (deliberately kept on the view, not the model):

```rust
pub struct TuiInputView {
    model: ModelHandle<CodeEditorModel>, // char-cell mode
    kill_buffer: KillBuffer,             // single-entry (Ctrl+K/U/W + Ctrl+Y)
    scroll_offset: u32,                  // first visible visual row (0-indexed)
    terminal_width: u16,
    max_visible_rows: u32,               // = 6
}
```

**Rendering** (`render(&self, ctx) -> Box<dyn TuiElement>`):
1. Reads plain text, cursor offset, and selection range from the model.
2. Builds visible rows with the pure helper `build_visual_rows_with_offsets(text, width)` and computes the cursor's `(row, col)` with `char_cell_cursor_pos(text, cursor_offset, width)`. These pure functions operate directly on the plain text (independent of the `RenderState` SumTree).
3. Returns a `TuiInputElement`, which stacks the rows in a `TuiColumn`, applies `Modifier::REVERSED` to selected spans, and reports the block cursor via `cursor_position()`.

`visual_line_count()` reads `render_state().max_line()` (the `RenderState` char-cell path), and `scroll_to_cursor()` uses `render_state().offset_to_softwrap_point()` — so the editor-crate char-cell layout drives line-count and scrolling while the view-local helpers drive the actual cell rendering. Height is effectively capped at `max_visible_rows` (6) via the scroll logic.

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

### 4. Dependency notes

- `crates/warp_tui` depends on `warp` (with the `tui` feature) for `CodeEditorModel`, on `warp_editor` for the editing traits/types, and on `warpui_core` (with `tui`) for the TUI elements/runtime.
- `app`'s `tui` feature enables `warpui_core/tui`; `CodeEditorModel::new_tui` / `set_tui_terminal_width` are not feature-gated.
- `warp_tui` dev-dependencies enable `warp_core`'s `test-util` feature so tests/examples can register `Appearance::mock()`.

## Diagram

```
TuiInputView : TuiView + TypedActionView
│  state: kill_buffer, scroll_offset, terminal_width, max_visible_rows
│
├── render() ──────────────────────────────────────────────────┐
│   reads: plain_text, cursor_offset, selection_range          │
│   build_visual_rows_with_offsets() + char_cell_cursor_pos()  │
│   → TuiInputElement (TuiColumn rows, REVERSED selection,     │
│      cursor_position())                                       └─► ratatui Buffer
│
├── dispatch_event() → TuiInputAction → handle_action() ──► CodeEditorModel (LayoutMode::CharCell)
│      keybinding table                                     ├── Buffer (rope, undo/redo, word ops)
│                                                           ├── SelectionModel (non-generic)
│                                                           │     └── navigate_line() sticky-column (ColumnUnit)
│                                                           ├── on_buffer_version_updated → update_char_cell_text
│                                                           └── RenderState (CharCell)
│                                                                 ├── line_starts: Vec<usize>
│                                                                 ├── offset_to_softwrap_point → ColumnUnit::Chars
│                                                                 ├── max_line (drives visual_line_count)
│                                                                 └── skips LayoutCache / font engine
│
└── emits TuiInputViewEvent::Submitted(String)  (consumed by parent TuiView)
```

## Testing and Validation

**Editor char-cell unit tests** (`crates/editor/src/render/model/mod_tests.rs`, module `char_cell` — 12 tests):
- `char_cell_max_line` for empty, short, wrapping, multi-logical-line, and empty-logical-line content.
- `char_cell_offset_to_softwrap_point` for single/wrapping/multi-line content, returning `ColumnUnit::Chars`.
- `offset → point → offset` round-trips across offsets and `terminal_width` values (single line and wrapping).
- Explicit checks that the char-cell path returns `ColumnUnit::Chars` (not `Pixels`) and that offset 0 maps to row 0 / col 0.

**`TuiInputView` tests** (`crates/warp_tui/src/input/view_tests.rs` — 12 tests): drive a real `CodeEditorModel` (char-cell) behind a real `TuiInputView` (registering `Appearance::mock()`), covering cursor placement on empty/multi-line buffers, empty-line handling, up/down navigation across blank lines, selection text, and `Ctrl+K` / `Ctrl+U` / `Ctrl+Y` kill-yank behaviour.

**Examples (manual smoke)**:
- `crates/warp_tui/examples/tui_input_demo.rs` — interactive editor-backed input demo. Run: `cargo run -p warp_tui --example tui_input_demo`.
- `crates/warpui_core/examples/tui_file_viewer.rs` — validates the TUI runtime/rendering pipeline independently (scrollable file viewer, no editor dependency). Run: `cargo run -p warpui_core --example tui_file_viewer --features tui -- <path>`.

## Risks and Mitigations

**Two char-cell implementations**: the view renders with its own pure helpers (`build_visual_rows_with_offsets`, `char_cell_cursor_pos`, kill-range helpers) while line-count/scroll use the `RenderState` char-cell path. They must agree on wrapping semantics; the round-trip and view tests guard the overlap. A future cleanup could collapse the view helpers onto the `RenderState` API.

**Shift+Enter terminal support**: crossterm only delivers `Shift+Enter` distinctly in terminals supporting the Kitty keyboard protocol; elsewhere it arrives as bare `Enter`. The `Ctrl+J` fallback always inserts a newline.

**Selection rendering**: ratatui has no selection-highlight primitive, so `TuiInputElement` applies `Modifier::REVERSED` to selected cell spans manually. Tested with empty and non-empty selections.

**`Appearance` dependency**: `new_tui` reads syntax colours from the `Appearance` singleton (shared with the GUI). Contexts that build the model must register one — a real `Appearance` at runtime, `Appearance::mock()` in tests/examples. When the input view is wired into the `warp-tui` runtime, that runtime will need to register `Appearance`.

## Follow-ups

Intentionally out of scope for M1; each should become its own task:

- **Wire into the `warp-tui` runtime**: render `TuiInputView` in the `warp_tui` binary (today only auth runs in `app/src/tui/mod.rs`); register `Appearance` there.
- **Input mode (Agent / Shell)**: wire `BlocklistAIInputModel`; placeholder text and submit routing per mode.
- **Slash command menu**: render an overlay on the `Composing` state.
- **History (up-arrow)**: open a TUI history overlay; add an "is cursor on first visual row" trigger.
- **Vim mode**: gate the editor's vim navigation on a user setting.
- **Kill ring**: extend the single-entry `KillBuffer` to a multi-entry ring (`Alt+Y` to cycle).
- **Clipboard integration**: `Ctrl+V` paste from the system clipboard.
