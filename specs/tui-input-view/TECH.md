# TUI Input View — Tech Spec (Milestone 1)

Commit ref: `0fb3e4d47af4324f485632aa07d36bf70e500259`

## Context

The TUI runtime (`kevin/tui-presenter-runtime`) introduces a parallel rendering path — ratatui + crossterm instead of WarpUI's GPU renderer — with its own entity lifecycle ([`TuiView`](https://github.com/warpdotdev/warp/blob/0fb3e4d47af4324f485632aa07d36bf70e500259/crates/warpui_core/src/core/view/tui.rs#L19)) and element traits ([`TuiElement`](https://github.com/warpdotdev/warp/blob/0fb3e4d47af4324f485632aa07d36bf70e500259/crates/warpui_core/src/elements/tui/mod.rs#L96)).

Existing TUI elements — [`TuiText`](https://github.com/warpdotdev/warp/blob/0fb3e4d47af4324f485632aa07d36bf70e500259/crates/warpui_core/src/elements/tui/text.rs#L28), [`TuiColumn`](https://github.com/warpdotdev/warp/blob/0fb3e4d47af4324f485632aa07d36bf70e500259/crates/warpui_core/src/elements/tui/column.rs#L26), [`TuiContainer`](https://github.com/warpdotdev/warp/blob/0fb3e4d47af4324f485632aa07d36bf70e500259/crates/warpui_core/src/elements/tui/container.rs#L28), [`TuiEventHandler`](https://github.com/warpdotdev/warp/blob/0fb3e4d47af4324f485632aa07d36bf70e500259/crates/warpui_core/src/elements/tui/event_handler.rs#L32) — are all display-only or generic input hooks. There is no text input widget.

`crates/editor/` provides two relevant traits ([`CoreEditorModel`](https://github.com/warpdotdev/warp/blob/0fb3e4d47af4324f485632aa07d36bf70e500259/crates/editor/src/model.rs#L68), [`PlainTextEditorModel`](https://github.com/warpdotdev/warp/blob/0fb3e4d47af4324f485632aa07d36bf70e500259/crates/editor/src/model.rs#L692)) and an underlying `Buffer` (rope-style text storage with undo/redo, word-boundary movement, and selection). The crate has no GPU or WarpUI rendering coupling — it depends only on `warpui_core` for `AppContext`/`ModelHandle`.

**Architecture**: `CoreEditorModel::move_up`/`move_down` navigate via [`RenderState`](https://github.com/warpdotdev/warp/blob/0fb3e4d47af4324f485632aa07d36bf70e500259/crates/editor/src/render/model/mod.rs#L385), which is pixel-based (`SoftWrapPoint.column: Pixels`, backed by platform font shaping). The `SelectionModel` only calls 3 methods on `RenderState` — `offset_to_softwrap_point`, `softwrap_point_to_offset`, `max_line` — and `column: Pixels` is only actively used in the sticky-goal path of `navigate_line` (move up/down).

Rather than building a parallel `TuiRenderState`, the plan adds an explicit `LayoutMode` enum to `RenderState` itself. In `LayoutMode::CharCell` mode, `RenderState` skips the font engine and `LayoutCache` entirely and computes soft-wrap positions with inline char-cell arithmetic. `SelectionModel` stays non-generic — it holds `ModelHandle<RenderState>` as before. This keeps the full editing model reusable without generics, and positions `RenderState` to handle future TUI rich-text content (diffs, markdown) by extending the `CharCell` branch rather than building a separate block layout system.

This is a prerequisite refactor in `crates/editor/` before the view itself is built. Concrete size: ~158 LOC total (50 for `ColumnUnit` + 108 for the `LayoutMode` changes), well within the threshold for unit-testable refactoring.

**Scope of this milestone**: a functional multi-line selectable editor with Emacs/readline keybindings. Input mode switching, slash command detection, and history navigation are explicitly deferred (see Follow-ups).

## Proposed Changes

### 0. Prerequisite refactor — `ColumnUnit` + `LayoutMode` in `crates/editor/` (~2–3 days)

**Step 1**: Replace `SoftWrapPoint.column: Pixels` with an explicit `ColumnUnit` enum (~50 LOC, 15 call sites):

```rust
// crates/editor/src/render/model/mod.rs
pub enum ColumnUnit {
    Pixels(Pixels),
    Chars(u16),
}

pub struct SoftWrapPoint {
    row: u32,
    column: ColumnUnit,  // was: Pixels
}
```

`ColumnUnit` implements `PartialOrd` only within the same variant; mixed-mode comparison panics. The sticky-goal field in `SelectionModel` becomes `goal_columns: Option<Vec1<ColumnUnit>>`. `navigate_line` at [`selection.rs:868`](https://github.com/warpdotdev/warp/blob/0fb3e4d47af4324f485632aa07d36bf70e500259/crates/editor/src/content/selection.rs#L868) pattern-matches on the variant — comparison logic is identical per-variant. One non-mechanical decision: the `x.max(next_point.column())` expression at `selection.rs:913` needs a same-variant max helper.

Existing GUI call sites: 15 `SoftWrapPoint::new(row, pixels)` → `SoftWrapPoint::new(row, ColumnUnit::Pixels(pixels))` — mechanical.

**Step 2**: Add `LayoutMode` to `RenderState` (~108 LOC across 4 change sites):

```rust
// crates/editor/src/render/model/mod.rs
pub enum LayoutMode {
    Pixels {
        styles: RichTextStyles,
        viewport_width: Pixels,
    },
    CharCell {
        terminal_width: u16,
        line_start_offsets: Vec<CharOffset>,  // rebuilt on each edit
    },
}
```

`RenderState` gains a `layout_mode: LayoutMode` field. Construction: `RenderState::new_internal()` gains a `layout_mode` parameter; 2 production call sites updated.

**`handle_layout_action` branching** ([`mod.rs:2386`](https://github.com/warpdotdev/warp/blob/0fb3e4d47af4324f485632aa07d36bf70e500259/crates/editor/src/render/model/mod.rs#L2386), ~42 lines affected):
- `LayoutAction::BufferEdit` in `CharCell` mode: skip `layout_edit_delta` (font shaping + `LayoutCache`), instead reparse `line_start_offsets` from the edit delta text (~20 LOC new)
- `LayoutAction::LayoutTemporaryBlock` in `CharCell` mode: no-op (~3 LOC)
- `layout_context()` ([`mod.rs:2477`](https://github.com/warpdotdev/warp/blob/0fb3e4d47af4324f485632aa07d36bf70e500259/crates/editor/src/render/model/mod.rs#L2477)) is not called in `CharCell` mode — no changes needed to it

**`offset_to_softwrap_point` / `softwrap_point_to_offset` branching** ([`mod.rs:3053`](https://github.com/warpdotdev/warp/blob/0fb3e4d47af4324f485632aa07d36bf70e500259/crates/editor/src/render/model/mod.rs#L3053), ~21 lines existing):
```rust
pub fn offset_to_softwrap_point(&self, offset: CharOffset) -> SoftWrapPoint {
    match &self.layout_mode {
        LayoutMode::Pixels { .. } => { /* existing impl */ }
        LayoutMode::CharCell { terminal_width, line_start_offsets } => {
            // binary search line_start_offsets + char arithmetic (~15 LOC)
            SoftWrapPoint::new(row, ColumnUnit::Chars(col))
        }
    }
}
```

Same pattern for `softwrap_point_to_offset` (~12 LOC new CharCell branch).

**`SelectionModel` stays non-generic** — holds `ModelHandle<RenderState>` as today. No changes to `CoreEditorModel` trait, no type parameter propagation anywhere.

**Blast radius**: `SoftWrapPoint` construction sites (15 production, mechanical), `RenderState::new_internal` (2 call sites), `handle_layout_action` (2 arms), `offset_to_softwrap_point` / `softwrap_point_to_offset` (new branches). No changes to `navigate_line`, `SelectionModel`, `CoreEditorModel`, or any GUI rendering code.

---

### 1. `TuiInputModel` — new model in `app/src/tui/input/model.rs`

```rust
pub struct TuiInputModel {
    buffer: ModelHandle<Buffer>,
    render_state: ModelHandle<RenderState>,  // constructed with LayoutMode::CharCell
    selection: ModelHandle<SelectionModel>,  // non-generic, holds ModelHandle<RenderState>
    terminal_width: u16,   // updated by view on resize
    scroll_offset: u32,    // first visible visual row (0-indexed)
}
```

**Text storage**: delegates to `Buffer` from `crates/editor/content/buffer.rs` — gives undo/redo, word-boundary movement, and selection for free.

**`RenderState` in CharCell mode**: constructed once at init with `LayoutMode::CharCell { terminal_width, line_start_offsets: vec![] }`. On each buffer edit, `RenderState` updates `line_start_offsets` via the existing `EditDelta` pipeline (the CharCell branch of `handle_layout_action`). On terminal resize, `TuiInputModel` calls a new `RenderState::set_terminal_width(u16)` method.

**Visual cursor position and move_up/down**: fully handled by the existing `SelectionModel` + `RenderState` pipeline. The CharCell branches of `offset_to_softwrap_point`/`softwrap_point_to_offset` return `ColumnUnit::Chars`, and `navigate_line`'s sticky-column logic works correctly without any changes.

**Key methods exposed to the view**:
- `insert(&str)`, `backspace()`, `delete_forward()` — character edits
- `move_left/right(word: bool)`, `move_up/down()`, `move_to_line_start/end()` — cursor movement; `move_up/down` use `visual_row_col` with char-cell math
- `begin_selection()`, `update_selection(cursor)`, `clear_selection()` — selection state via `SelectionModel`
- `undo()`, `redo()` — delegated to `Buffer`'s history
- `kill_to_line_end()`, `kill_to_line_start()`, `yank()` — kill buffer (single-entry; extend to ring later)
- `text() -> &str`, `cursor_offset() -> CharOffset`, `selection_range() -> Option<Range<CharOffset>>`
- `set_terminal_width(u16)` — calls `RenderState::set_terminal_width`; triggers re-layout via the `EditDelta` pipeline
- `visual_line_count() -> u32` — delegated to `RenderState::max_line()` in CharCell mode
- `is_cursor_on_first_visual_row() -> bool` — used later for history trigger
- `submit() -> String` — drains buffer, resets state, returns text

Emits a single `TuiInputModelEvent::Changed` notification on any state change so the view can call `ctx.notify()`.

### 2. `TuiInputView` — new view in `app/src/tui/input/view.rs`

Implements `TuiView`. Holds `ModelHandle<TuiInputModel>`.

**`render(&self, ctx) -> Box<dyn TuiElement>`**:
1. Read `text()`, `cursor_offset()`, `selection_range()`, `scroll_offset`, `visual_line_count()` from model.
2. Compute the set of visible visual rows (`scroll_offset..scroll_offset + visible_rows`).
3. For each visible row, build a `TuiText` span with appropriate styling:
   - Selected text highlighted (reverse video or accent colour).
   - Cursor position returned via `cursor_position()`.
4. Stack rows in a `TuiColumn`.
5. Wrap the column in a `TuiContainer` with a 1-cell left/right inset for the `≫` prompt glyph and padding.

**`cursor_position(&self, area, ctx) -> Option<(u16, u16)>`**:
- Asks the model for `visual_row_col(cursor_offset)`.
- Subtracts `scroll_offset` to convert to view-relative row.
- Returns `(area.x + col + prompt_width, area.y + relative_row)`.

**Height contract**: the view's `layout()` returns `Constraint::Length(cmp::min(visual_line_count, 6).max(1))`. The TUI presenter allocates exactly that many rows.

**`dispatch_event(&mut self, event, area, event_ctx, ctx, app) -> bool`**:

Key dispatch order for Milestone 1 (no overlays yet):

| Key | Action |
|-----|--------|
| `Char(c)` | `model.insert(c)` |
| `Enter` | emit `TuiInputViewEvent::Submit(text)` |
| `Shift+Enter` / `Ctrl+J` / `Alt+Enter` | `model.insert('\n')` |
| `Backspace` / `Ctrl+H` | `model.backspace()` |
| `Delete` / `Ctrl+D` | `model.delete_forward()` |
| `←` / `Ctrl+B` | `model.move_left(word: false)` |
| `→` / `Ctrl+F` | `model.move_right(word: false)` |
| `Alt+←` / `Alt+B` / `Ctrl+←` | `model.move_left(word: true)` |
| `Alt+→` / `Alt+F` / `Ctrl+→` | `model.move_right(word: true)` |
| `↑` / `Ctrl+P` | `model.move_up()` |
| `↓` / `Ctrl+N` | `model.move_down()` |
| `Home` / `Ctrl+A` | `model.move_to_line_start()` |
| `End` / `Ctrl+E` | `model.move_to_line_end()` |
| `Ctrl+W` / `Alt+Backspace` | `model.delete_word_backward()` |
| `Alt+D` / `Ctrl+Delete` | `model.delete_word_forward()` |
| `Ctrl+K` | `model.kill_to_line_end()` |
| `Ctrl+U` | `model.kill_to_line_start()` |
| `Ctrl+Y` | `model.yank()` |
| `Ctrl+Z` | `model.undo()` |
| `Shift+←/→/↑/↓` | `model.begin_selection()` + movement |
| `Ctrl+A` (select all) | `model.select_all()` |
| Resize event | `model.set_terminal_width(new_width)` |

All handled keys return `true` (consumed). Unknown keys return `false`.

**Scroll management**: after each event that changes cursor position, the view updates `scroll_offset` to keep the cursor's visual row in `scroll_offset..scroll_offset + 6`. This is a pure view concern; the model stores `scroll_offset` as a field updated via `set_scroll_offset`.

**Event emission**: `TuiInputView` emits a `TuiInputViewEvent` enum: `Submit(String)` and `Changed`. The parent view (the TUI app shell) subscribes to `Changed` if it needs to react to content changes, and to `Submit` to route the query.

### 3. Module layout

```
app/src/tui/
    input/
        mod.rs          — pub use; TuiInputViewEvent
        model.rs        — TuiInputModel, TuiInputModelEvent
        view.rs         — TuiInputView : TuiView
        kill_buffer.rs  — KillBuffer (single-entry for M1)
```

No new files in `crates/editor/` — all changes are additive to existing files (`mod.rs`, `selection.rs`). The `tui/` module under `app/src/` parallels the existing `app/src/terminal/input/` structure and will eventually share `BlocklistAIInputModel`, `SlashCommandModel`, and `InputSuggestionsModeModel` — but those are explicitly out of scope for M1.

### 4. Dependency notes

- `crates/editor` is already a dependency of `app/`; no new crate dependencies, no new files.
- `TuiInputModel` uses `Buffer`, `SelectionModel`, `RenderState` (CharCell mode) — all existing types.
- `TuiInputView` uses `TuiText`, `TuiColumn`, `TuiContainer`, `TuiEventHandler` — all already in `crates/warpui_core`.

## Diagram

```
TuiInputView: TuiView
│
├── render() ──────────────────────────────────────────────────┐
│   reads: text, cursor_offset, selection_range, scroll_offset │
│   produces: TuiColumn([TuiText, TuiText, ...])               │
│   cursor_position() → (col, row) in terminal cells           │
│                                                               └─► ratatui Buffer
│
├── dispatch_event() ──► TuiInputModel
│      keybinding table │
│      (handled above)  ├── Buffer (crates/editor)
│                       │     ├── text storage (rope)
│                       │     ├── undo/redo history
│                       │     └── word-boundary ops
│                       ├── SelectionModel (non-generic, existing type)
│                       │     └── navigate_line() sticky-column logic unchanged
│                       └── RenderState (LayoutMode::CharCell)
│                             ├── line_start_offsets: Vec<CharOffset>
│                             ├── offset_to_softwrap_point → ColumnUnit::Chars
│                             └── skips LayoutCache, font engine, TextFrame
│
└── emits TuiInputViewEvent::{Submit(String), Changed}
    consumed by parent TuiView (app shell)
```

## Testing and Validation

**Unit tests on `ColumnUnit` + `RenderState` CharCell mode** (extend `crates/editor/src/render/mod_tests.rs`):
- `offset_to_softwrap_point` returns `ColumnUnit::Chars` (not `Pixels`) when `LayoutMode::CharCell`
- `offset_to_softwrap_point` / `softwrap_point_to_offset` round-trip at various offsets and `terminal_width` values
- Correct row/col with embedded `\n` (logical line boundaries reset the column)
- Wide-char handling (CJK characters count as 2 columns via `unicode_width`)
- `max_line()` in CharCell mode matches expected visual row count
- `RenderState` in `LayoutMode::Pixels` (GUI) still produces `ColumnUnit::Pixels` — no regression
- `line_start_offsets` is correctly updated after a sequence of edits via `EditDelta`

**Unit tests on `TuiInputModel`** (`app/src/tui/input/model_tests.rs`):
- `move_up`/`move_down` with soft-wrapped and multi-logical-line buffers
- Sticky-column preserved across consecutive up/down presses
- `move_up` from visual row 0 stays at offset 0 (no panic)
- `kill_to_line_end` + `yank` round-trip
- Undo/redo after a sequence of inserts and deletes
- Selection range after `begin_selection` + `move_right(word: true)`
- `visual_line_count` matches `terminal_width`-aware line wrapping

**Integration test using the `warp-integration-test` skill**:
- Type a multi-line prompt via `Shift+Enter`; assert visual height grows to N rows
- Type past 6 visual rows; assert height is capped at 6 and scroll_offset advances
- Navigate with arrow keys; assert cursor position reflects correct visual row/col
- Submit with `Enter`; assert `TuiInputViewEvent::Submit` fires with correct text

**Manual smoke**:
- Run `cargo run` on the TUI branch; verify the input box renders at bottom
- Verify block cursor renders at correct column
- Verify `Ctrl+K` / `Ctrl+Y` round-trip

## Risks and Mitigations

**`Buffer` initialization overhead**: `crates/editor`'s `Buffer` was designed for document-scale use. Profile that creating one per TUI session doesn't add measurable startup latency; if so, consider a simpler `VecDeque<char>`-backed store.

**Shift+Enter terminal support**: crossterm receives `Shift+Enter` as a `KeyModifiers::SHIFT | KeyCode::Enter` event only in terminals that support the Kitty keyboard protocol. In others it arrives as bare `Enter`. The `Ctrl+J` fallback must always work; `Shift+Enter` should be opportunistic.

**Selection rendering**: ratatui's `Buffer` doesn't have a built-in selection highlight primitive — the view must apply `Style::reversed()` to selected char spans manually when building `TuiText`. Test this path with both empty and non-empty selections.

## Follow-ups

The following are intentionally out of scope for M1 and should each become their own spec task:

- **Input mode (Agent / Shell)**: wire `BlocklistAIInputModel` to `TuiInputModel`; update placeholder text and submit routing per mode
- **Slash command menu**: wire `SlashCommandModel` to the TUI input buffer; render a `TuiContainer` overlay on `Composing` state
- **History (up-arrow)**: implement `is_cursor_on_first_visual_row()` hook; open a TUI history overlay via `InputSuggestionsModeModel`
- **Vim mode**: add a `VimState` to `TuiInputModel`, gate on user setting
- **Kill ring**: extend single-entry `KillBuffer` to a ring (multi-entry yank, `Alt+Y` to cycle)
- **Clipboard integration**: `Ctrl+V` paste from system clipboard via `arboard` or `cli-clipboard`
