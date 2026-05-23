# TECH.md — URL detection across TUI-rendered row breaks

Issue: https://github.com/warpdotdev/warp/issues/11609
Product spec: `specs/GH11609/product.md`

## Context

`GridHandler::url_at_point` uses `GraphemeCursor::Wrap::Soft`, which traverses rows only when the terminal grid marks the prior row as wrapped (`WRAPLINE` flag set, `row_wraps() == true`). Rows terminated by `\r\n` from upstream TUI renderers have no `WRAPLINE` flag — the cursor stops at these boundaries, truncating URLs that span them.

`TerminalModel::link_at_range` calls `bounds_to_string` → `visible_rows_to_string` → `line_to_string`, which appends `\n` to rows where `WRAPLINE` is absent and `cols.end >= columns - 1`. A URL range spanning a hard-wrap boundary therefore contains an embedded `\n`.

## Relevant files

| File | Role |
|------|------|
| `app/src/terminal/model/grid/grid_handler.rs` | `url_at_point` (line ~649) — main fix site |
| `app/src/terminal/model/grid/grid_handler_tests.rs` | Existing URL tests + new tests |
| `app/src/terminal/model/terminal_model.rs` | `link_at_range` (line ~1825) — companion fix |

## Proposed changes

### Change 1 — `url_at_point` continuation probe (`grid_handler.rs`)

After the forward scan loop completes, insert a continuation probe:

```rust
if !url.is_empty {
    let url_end = *url.range.end();
    let last_col = self.columns().saturating_sub(1);
    let near_right_edge = url_end.col + 4 >= last_col;
    let row_hard_wrapped = !self.row_wraps(url_end.row);
    let next_row_exists = url_end.row
        .checked_add(1)
        .map_or(false, |r| self.row(r).is_some());

    if near_right_edge && row_hard_wrapped && next_row_exists {
        let next_row_start = Point { row: url_end.row + 1, col: 0 };
        let mut continuation_cursor =
            self.grapheme_cursor_from(next_row_start, grapheme_cursor::Wrap::None);
        let mut is_first_char = true;
        while let Some(item) = continuation_cursor.current_item() {
            if item.point().row != url_end.row + 1 { break; }
            let cell = item.cell();
            if is_at_boundary(cell) || cell.c.is_whitespace() { break; }
            if is_first_char && cell.c.is_uppercase() { break; }
            is_first_char = false;
            url.extend_link(item.point());
            continuation_cursor.move_forward();
        }
    }
}
```

**Guards (all O(1)):**
- `near_right_edge`: URL ends within 4 cols of row boundary (accommodates trailing `.,:;?!` chars)
- `row_hard_wrapped`: `row_wraps()` is a single `match` on storage row index
- `next_row_exists`: `self.row(row + 1)` is a storage lookup

Common case (short URL, soft-wrap, or URL not near edge): early-exit at `near_right_edge` with zero additional allocations.

### Change 2 — `link_at_range` newline stripping (`terminal_model.rs`)

```rust
pub fn link_at_range<T: RangeInModel>(...) -> String {
    let text = self.string_at_range(item, respect_obfuscated_secrets);
    let trimmed = text.trim_matches(['\u{200B}', ' ', '\n', '\r', '\t']);
    trimmed.replace(['\n', '\r'], "")
}
```

All existing callers are URL-specific (guarded by `GridHighlightedLink::Url` at call sites). RFC 3986 URLs cannot contain raw newlines — stripping is always correct. No-op for single-row URLs.

## Tests

| Test | Input | Expected |
|------|-------|----------|
| `test_url_extends_across_hard_wrap_boundary` | `"https://example.com/\r\npath"` (20-col grid) | `url_at_point((0,0))` → range `(0,0)..=(1,3)` |
| `test_url_hard_wrap_no_extend_uppercase` | `"https://example.com/\r\nFor more"` | range stays `(0,0)..=(0,19)` |
| `test_url_hard_wrap_no_extend_whitespace` | `"https://example.com/\r\n  more"` | range stays `(0,0)..=(0,19)` |
| `test_url_hard_wrap_backward_returns_none` | hover on `(1,0)` | `None` (documented V1 limitation) |

Existing `test_find_url_line_wrapping` (soft-wrap) is unaffected — `row_wraps(0) == true` → probe does not fire.

## Risks and mitigations

| Risk | Mitigation |
|------|-----------|
| Uppercase path segment false negative | Documented known limitation; V2 relaxes guard after `/` |
| Multi-row wrap not handled | Explicitly out of V1 scope |
| `url_end.row + 1` overflow | `checked_add(1)` used |
| Performance regression on hover | Three O(1) guards; common case exits immediately |

## Follow-ups

- **V2 multi-row**: Iterative loop across successive hard-wrap boundaries (depth limit ~3)
- **V2 backward scan**: Extend backward scan to cross hard-wrap boundaries
- **OSC 8 (#4194)**: Takes precedence over this heuristic when it ships
