# Trim Trailing Blank Lines from CLI Agent Block Output

## 1. Problem

CLI agents (Codex, Claude CLI, etc.) running in Warp blocks produce trailing blank lines. Output grid height is based on `max_cursor_point` (furthest row cursor visited), but agents frequently emit newlines/clear screen, leaving empty rows below content. Wastes space, especially in shared sessions.

## 2. Approach

Trim trailing blank rows from active CLI agent blocks by capping `BlockGrid::len_displayed()` at `GridHandler::content_len()`. A `trim_trailing_blank_rows` flag on `BlockGrid` gates the cap. `Block` toggles the flag via `set_is_cli_agent_active()`. Gated by `FeatureFlag::TrimTrailingBlankLines` (dogfood-only).

Since `output_grid_displayed_height()` already delegates to `len_displayed()`, all downstream call sites (selection, viewport, seek, rendering) automatically see trimmed values with no per-site changes.

## 3. Implementation

### 3.1 Feature flag

`FeatureFlag::TrimTrailingBlankLines` added to `crates/warp_features/src/lib.rs`, included in `DOGFOOD_FLAGS`.

### 3.2 Backward scan in `GridHandler`

Field: `bottommost_nonempty_row: Option<usize>`
- `None` = not yet computed or grid is all blank
- `Some(row)` = bottommost non-empty row index

`bottommost_nonempty_row_backward()` scans from `max_cursor_point` downward, checking `Cell::is_empty()` per cell. Returns on first non-empty cell found, so cost is O(trailing_blank_rows × cols).

Updated in `on_finish_byte_processing()` (per PTY-read batch). Initialized to `None` in `new()` / `new_for_split()`.

### 3.3 `GridHandler::content_len()`

```rust
pub fn content_len(&self) -> usize {
    match self.bottommost_nonempty_row {
        Some(row) => row + 1,
        None => self.grid.max_cursor_point.row.0 + self.history_size() + 1,
    }
}
```

Returns trimmed row count when cached value exists. Falls back to full `max_cursor_point`-based length when not yet computed or grid is all blank (avoids trimming to 0).

### 3.4 `is_cli_agent_active` flag on `Block`

Boolean field, default `false`. Setter propagates to output grid (see 3.6).

Set from view layer in `TerminalView`'s `CLIAgentSessionsModelEvent` handler:
- `Started` (matching `self.view_id`) → `set_is_cli_agent_active(true)` on active block
- `Ended` (matching `self.view_id`) → `set_is_cli_agent_active(false)` on active block

Both acquire `model.lock()` → `block_list_mut()` → `active_block_mut()`.

### 3.5 `trim_trailing_blank_rows` flag on `BlockGrid`

Boolean field, default `false`. Setter: `set_trim_trailing_blank_rows(bool)`. Cleared in `BlockGrid::finish()`.

When true, `len_displayed()` caps its return at `content_len()`:

```rust
pub fn len_displayed(&self) -> usize {
    let base = if let Some(len_displayed) = self.grid_handler().len_displayed() {
        len_displayed
    } else {
        self.len()
    };
    if self.trim_trailing_blank_rows {
        base.min(self.grid_handler().content_len())
    } else {
        base
    }
}
```

### 3.6 Wiring from `Block`

`Block::set_is_cli_agent_active()` propagates to the output grid, combining the active state with the feature flag:

```rust
pub fn set_is_cli_agent_active(&mut self, active: bool) {
    self.is_cli_agent_active = active;
    self.output_grid.set_trim_trailing_blank_rows(
        active && FeatureFlag::TrimTrailingBlankLines.is_enabled(),
    );
}
```

`Block::finish()` calls `self.output_grid.finish()` which resets `trim_trailing_blank_rows = false`, so trimming is implicitly cleared when the block completes.

## 4. Modified Files

- `crates/warp_features/src/lib.rs` — flag enum + DOGFOOD_FLAGS
- `app/src/terminal/model/grid/grid_handler.rs` — `bottommost_nonempty_row` field, backward scan, `content_len()`
- `app/src/terminal/model/grid/ansi_handler.rs` — cache update in `on_finish_byte_processing()`
- `app/src/terminal/model/blockgrid.rs` — `trim_trailing_blank_rows` flag, updated `len_displayed()`
- `app/src/terminal/model/block.rs` — `is_cli_agent_active` field, `set_is_cli_agent_active()` wiring
- `app/src/terminal/view.rs` — wiring `CLIAgentSessionsModelEvent::Started/Ended` to set flag on active block
- `app/src/terminal/model/grid/grid_handler_test.rs` — 7 unit tests

## 5. Tests

7 unit tests in `grid_handler_test.rs`.

Tests that move the cursor below content use `goto()` (CSI CUP `\e[row;colH`) rather than `linefeed()`. This matches real-world behavior: capturing raw PTY output from Codex CLI (`script -q /tmp/capture.bin codex ...` + `xxd`) showed it positions the cursor via CUP sequences (e.g. `\e[73;2H` to clear row 73), not bare LFs. This matters because `goto` goes through `update_cursor` → `update_max_cursor` (updates max *before* the move), while `linefeed` uses `move_cursor_forward` (skips `update_max_cursor`), producing different `max_cursor_point` values.

- Interspersed blanks preserved (interior blank rows not trimmed)
- Trailing blanks trimmed (cursor moved below via `goto`)
- Cursor below content → trimmed to content (cursor moved via `goto`)
- New output restores height after trimming
- Single trailing newline → no extra blank row
- All-blank grid → `content_len()` falls back to max_cursor_point-based length
- No trailing blanks → `content_len()` equals full length

## 6. Risks and Mitigations

- **Performance**: backward scan O(trailing_blanks × cols). Updated once per PTY-read batch.
- **Intentional blank lines**: only trailing blanks trimmed; interior blanks preserved.
- **Scope**: only active CLI agent blocks. Alt-screen, finished blocks, and non-agent blocks unaffected.
- **Flag lifecycle**: `trim_trailing_blank_rows` cleared in `BlockGrid::finish()`. Feature flag for safe rollback.
- **Single source of truth**: trimming in `len_displayed()` means all call sites automatically correct.

## 7. Follow-Ups

- Remove feature flag after stabilization
- Incremental tracking of last-written row to avoid per-batch scan
- Update `server::Block::embed_pixel_height` to use trimmed height
- Consider broadening to all non-alt-screen blocks
