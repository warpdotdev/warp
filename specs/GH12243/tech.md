# GH12243: Tech Spec — Prevent RowIterator crash after clear resize truncates wide characters
Product spec: `specs/GH12243/product.md`
Issue: https://github.com/warpdotdev/warp/issues/12243
Code references inspected at commit: `55b411ec694a5c16a01929bcaef1d8f971677ca2`
## Context
The issue is a producer-side row-invariant break that later appears as a consumer-side panic. The observed crash is in flat-storage row materialization, but the malformed row is produced earlier when the active terminal grid is resized without reflow under full-grid clear behavior.
Relevant current code:
- [`CONTRIBUTING.md:90-107 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/CONTRIBUTING.md#L90-L107) — spec PR requirements for `specs/GH<issue-number>/product.md` and `tech.md`.
- [`app/src/terminal/view.rs:12935-12955 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/app/src/terminal/view.rs#L12935-L12955) — terminal view handles CLI-agent OSC notifications that start the clear-style redraw behavior.
- [`app/src/terminal/model/block.rs:1100-1102 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/app/src/terminal/model/block.rs#L1100-L1102) — block-level entry point enables full-grid clear behavior on the output grid.
- [`app/src/terminal/model/grid/grid_handler.rs:330-340 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/app/src/terminal/model/grid/grid_handler.rs#L330-L340) — `FullGridClearBehavior` distinguishes in-place redraw behavior from normal scrollback preservation.
- [`app/src/terminal/model/grid/grid_handler.rs:490-492 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/app/src/terminal/model/grid/grid_handler.rs#L490-L492) — `enable_full_grid_clear_behavior` switches a grid handler to the clear path.
- [`app/src/terminal/model/grid/resize.rs:57-82 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/app/src/terminal/model/grid/resize.rs#L57-L82) — `GridHandler::resize_storage` takes the alt-screen / `FullGridClearBehavior::Clear` early path and delegates to `self.grid.resize(false, ...)`, then syncs flat-storage columns.
- [`app/src/terminal/model/grid/resize.rs:98-158 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/app/src/terminal/model/grid/resize.rs#L98-L158) — the normal resize path pushes rows into flat storage, changes flat-storage width, then materializes rows back with `pop_rows`.
- [`app/src/terminal/model/grid/ansi_handler.rs:1500-1534 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/app/src/terminal/model/grid/ansi_handler.rs#L1500-L1534) — existing wide-character boundary helpers reset both halves when an overwrite or clear boundary splits a wide-character pair.
- [`app/src/terminal/model/grid/ansi_handler.rs:1536-1582 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/app/src/terminal/model/grid/ansi_handler.rs#L1536-L1582) — `write_at_cursor` resets the paired cell before writing when the cursor lands on either half of a wide-character pair.
- [`app/src/terminal/model/grid/grid_storage/resize.rs:308-363 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/app/src/terminal/model/grid/grid_storage/resize.rs#L308-L363) — `GridStorage::shrink_cols` calls `row.shrink(columns)` and, when `reflow` is false, pushes the shortened row without processing wrapped cells.
- [`app/src/terminal/model/grid/grid_storage/resize.rs:365-390 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/app/src/terminal/model/grid/grid_storage/resize.rs#L365-L390) — the `reflow=true` branch already has special wide-character handling: a trailing `WIDE_CHAR` is replaced with `LEADING_WIDE_CHAR_SPACER` and moved into wrapped content.
- [`crates/warp_terminal/src/model/grid/row.rs:66-92 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/crates/warp_terminal/src/model/grid/row.rs#L66-L92) — `Row::shrink` splits off cells beyond the new column count and returns non-empty discarded cells. It is a low-level primitive and does not know whether the caller will discard overflow or reflow it.
- [`crates/warp_terminal/src/model/grid/flat_storage/mod.rs:185-217 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/crates/warp_terminal/src/model/grid/flat_storage/mod.rs#L185-L217) — flat storage skips wide-character spacer cells when serializing rows and records leading-spacer metadata separately.
- [`crates/warp_terminal/src/model/grid/flat_storage/mod.rs:124-140 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/crates/warp_terminal/src/model/grid/flat_storage/mod.rs#L124-L140) — `FlatStorage::pop_rows` materializes stored rows through `rows_from`.
- [`crates/warp_terminal/src/model/grid/flat_storage/row_iterator.rs:86-133 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/crates/warp_terminal/src/model/grid/flat_storage/row_iterator.rs#L86-L133) — `RowIterator::next` fills row cells from grapheme runs and marks `row[idx + 1]` as `WIDE_CHAR_SPACER` for width-2 graphemes. If `idx` is the final row cell, this panics.
- [`app/src/terminal/model/grid/grid_handler_tests.rs:1511-1530 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/app/src/terminal/model/grid/grid_handler_tests.rs#L1511-L1530) — existing helper asserts there are no orphaned `WIDE_CHAR` or `WIDE_CHAR_SPACER` flags in a visible row.
- [`app/src/terminal/model/grid/grid_handler_tests.rs:1533-1563 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/app/src/terminal/model/grid/grid_handler_tests.rs#L1533-L1563) — existing tests assert that overwriting either half of a wide-character pair clears the paired cell.
- [`app/src/terminal/model/grid/grid_handler_tests.rs:1769-1774 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/app/src/terminal/model/grid/grid_handler_tests.rs#L1769-L1774) — existing test covers finished primary-grid behavior with full-grid clear enabled.
- [`app/src/terminal/model/grid/grid_handler_tests.rs:2001-2022 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/app/src/terminal/model/grid/grid_handler_tests.rs#L2001-L2022) — existing wide-character wrap test protects `LEADING_WIDE_CHAR_SPACER` semantics.
- [`app/src/terminal/model/grid/grid_handler_tests.rs:2208-2247 @ 55b411ec`](https://github.com/warpdotdev/warp/blob/55b411ec694a5c16a01929bcaef1d8f971677ca2/app/src/terminal/model/grid/grid_handler_tests.rs#L2208-L2247) — existing full-grid clear resize tests guard the earlier flat-storage column-sync regression.
The important ownership boundary is that `RowIterator::next` is the panic site, not the best primary fix site. The producer creates a row that violates the wide-character invariant described in `product.md`; flat storage then faithfully serializes and rematerializes that malformed row until the missing spacer becomes an out-of-bounds write.
## Proposed changes
### 1. Repair the confirmed producer path in `GridStorage::shrink_cols`
In `app/src/terminal/model/grid/grid_storage/resize.rs`, keep the fix localized to the branch where `GridStorage::shrink_cols` has just called `row.shrink(columns)`.
After `row.shrink(columns)` returns, handle the no-reflow split-wide-character case before the non-reflow branch pushes the shortened row into `new_raw`:
- If `!reflow`, `columns > 0`, and the retained final cell has `WIDE_CHAR`, reset that cell to an empty cell with the same background.
- Continue to pass the discarded cells returned by `row.shrink(columns)` into the existing `reflow=true` branch unchanged.
This satisfies `product.md` behavior 2-6 by repairing rows that would otherwise end with an orphaned `WIDE_CHAR`. Resetting the retained leading cell matches the existing overwrite/clear behavior for wide-character boundary operations and avoids pretending that a width-2 glyph has a valid one-cell representation.
This intentionally discards the boundary glyph. That is consistent with this no-reflow path: right-side overflow cells are already discarded, and once the spacer half is not present in the retained row the retained leading half is no longer independently renderable. The reset preserves the cell background so resize does not create a visual hole in applications that paint non-default backgrounds.
### 2. Do not move this fix into `Row::shrink`
`Row::shrink` only knows that cells were split off. It does not know whether the caller will discard those cells, reflow them into wrapped rows, or transform them with leading-spacer semantics. Its return value should preserve the original discarded cell flags so callers can decide how to handle a split wide-character pair. A generic `Row::shrink` cleanup that removes `WIDE_CHAR` whenever the first discarded cell is a spacer would also affect callers that still intend to preserve or reflow the wide character.
Keeping the fix in `GridStorage::shrink_cols` preserves the caller-specific distinction:
- `reflow=false`: overflow is discarded, so a retained final `WIDE_CHAR` must be reset rather than materialized as a one-cell glyph.
- `reflow=true`: overflow is wrapped, and the existing code moves a trailing `WIDE_CHAR` into wrapped content while placing a `LEADING_WIDE_CHAR_SPACER` in the retained row.
This directly protects `product.md` behavior 7.
### 3. Do not use `RowIterator::next` as the primary repair
`RowIterator::next` should not silently paper over this producer bug by dropping or narrowing width-2 graphemes whenever `idx + 1 == row.len()`. That would prevent this specific panic but would make corrupted flat-storage rows harder to diagnose and could hide unrelated producers.
If implementation review identifies a producer path that cannot be repaired before flat-storage materialization and requires extra consumer hardening, keep it secondary and explicit:
- It must log enough context to identify the producer path.
- It must have a dedicated test that proves the fallback does not corrupt valid rows.
- It must not replace the producer-side regression test.
Do not add consumer fallback for this issue by default.
### 4. Regression test in `grid_handler_tests.rs`
Add a focused test next to the existing full-grid clear resize tests:
`test_full_grid_clear_shrink_cols_does_not_orphan_wide_char_at_boundary`
The test should:
1. Create a `GridHandler` with a wider initial column count than the final resized width.
2. Enable `FullGridClearBehavior::Clear`.
3. Build a valid wide-character pair exactly at the shrink boundary, preferably through the normal grid input path so `WIDE_CHAR` and `WIDE_CHAR_SPACER` are produced by terminal writing rather than hand-set flags.
4. Resize through the real `grid.resize(SizeInfo::new_without_font_metrics(...))` API so the test exercises `GridHandler::resize_storage` and `GridStorage::shrink_cols`.
5. Use `assert_no_orphaned_wide_chars` to assert the row invariant.
6. Assert the exact boundary postcondition: the final retained cell is reset to an empty cell preserving the original background, and no retained cell contains an orphaned `WIDE_CHAR` or `WIDE_CHAR_SPACER`.
7. Push the post-resize retained row through a `FlatStorage` whose column count matches the resized grid width, then call `flat_storage.pop_rows(1)` and assert one row materializes without panic and keeps the boundary cell reset.
This is slightly stronger than only asserting "does not panic" because it proves the producer invariant before flat storage gets involved.
### 5. Add a resize-specific `reflow=true` guard
Add a focused `GridStorage` / `GridHandler` resize regression for the ordinary reflow path:
`test_shrink_cols_reflow_preserves_split_wide_char_as_wrapped_content`
The test should:
1. Build a valid wide-character pair at the shrink boundary in a normal reflowing resize path, without enabling `FullGridClearBehavior::Clear`.
2. Resize narrower through the real resize API so `GridStorage::shrink_cols(reflow=true, ...)` handles the split pair.
3. Assert the retained row uses the existing `LEADING_WIDE_CHAR_SPACER` representation rather than narrowing the retained cell.
4. Assert the wrapped content still contains the original `WIDE_CHAR` cell followed by its `WIDE_CHAR_SPACER`.
5. Assert the no-reflow boundary reset rule from change 1 does not run when `reflow=true`.
This protects `product.md` behavior 7 and proves the producer-side fix is scoped to discarded overflow, not ordinary wrapped wide-character content.
### 6. Preserve existing regression coverage
Do not remove or weaken the existing clear-resize tests. In particular:
- `test_full_grid_clear_resize_then_scroll_does_not_panic_on_row_iteration`
- `test_full_grid_clear_resize_narrower_then_scroll_does_not_panic`
- `test_full_grid_clear_resize_then_bounds_to_string_does_not_panic`
- `test_resize_finished_primary_with_full_grid_clear_behavior_uses_scrollback`
- `test_wide_char_wrap_preserves_own_leading_spacer`
Those tests protect the earlier flat-storage column-sync behavior, finished primary-grid routing, and normal wrapped wide-character semantics. They also prevent this issue from being conflated with #10305-style width mismatches.
## Testing and validation
Map tests to `product.md` behavior:
- Behavior 1, 8, 9: run the existing full-grid clear resize and routing tests:
  ```bash
  cargo nextest run --package warp terminal::model::grid::grid_handler::tests::test_full_grid_clear_resize_then_scroll_does_not_panic_on_row_iteration
  cargo nextest run --package warp terminal::model::grid::grid_handler::tests::test_full_grid_clear_resize_narrower_then_scroll_does_not_panic
  cargo nextest run --package warp terminal::model::grid::grid_handler::tests::test_full_grid_clear_resize_then_bounds_to_string_does_not_panic
  cargo nextest run --package warp terminal::model::grid::grid_handler::tests::test_resize_finished_primary_with_full_grid_clear_behavior_uses_scrollback
  ```
- Behavior 2-6: add and run:
  ```bash
  cargo nextest run --package warp terminal::model::grid::grid_handler::tests::test_full_grid_clear_shrink_cols_does_not_orphan_wide_char_at_boundary
  ```
  This test should fail before the producer fix by detecting an orphaned `WIDE_CHAR` at the final retained column or by panicking during flat-storage materialization. It is the accepted deterministic proof for the original crash, which is difficult to reproduce manually because it depends on command timing, clear-hook handling, resize timing, and a wide-character boundary.
- Behavior 4, 7: add and run the resize-specific `reflow=true` regression, and keep running the existing wrap guard:
  ```bash
  cargo nextest run --package warp terminal::model::grid::tests::test_shrink_cols_reflow_preserves_split_wide_char_as_wrapped_content
  cargo nextest run --package warp terminal::model::grid::grid_handler::tests::test_wide_char_wrap_preserves_own_leading_spacer
  ```
- Behavior 4, 6, 7: run the existing wide-character editing and wrapping tests around `assert_no_orphaned_wide_chars`. At minimum, run the full grid-handler test module if time permits:
  ```bash
  cargo nextest run --package warp terminal::model::grid::grid_handler::tests
  ```
  If a narrower subset is needed, include the tests whose names mention wide char, spacer, wrap, erase, delete, insert, and clear.
- General formatting and linting:
  ```bash
  ./script/format --check
  cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
  ```
- PR-level validation before final review:
  ```bash
  ./script/presubmit
  ```
  If unrelated local failures appear, record the failing test names and explain why they are unrelated to grid storage / row materialization.
Manual validation is secondary to the deterministic regression tests, but the implementation PR should still exercise the known clear-resize repro locally: emit an OSC 777 CLI-agent session start, print multiple rows whose full-width glyph targets adjacent shrink widths, open find on a repeated character, slowly shrink the pane or window, then finish the command if needed. The fixed build should not log a `row_iterator.rs:132` panic during resize, find rerun, or command-finished block serialization.
## Parallelization
Parallel sub-agents are not proposed for the implementation itself. The code change is narrow and the implementation/test files are tightly coupled:
- `app/src/terminal/model/grid/grid_storage/resize.rs`
- `app/src/terminal/model/grid/grid_handler_tests.rs`
- `app/src/terminal/model/grid/tests.rs`
Splitting these across agents would likely create more coordination overhead than saved time. A useful parallel review pattern is possible after the first implementation draft: one reviewer can inspect the producer/consumer tradeoff while another runs the focused regression tests, but the patch should be authored as one coherent change.
## Risks and mitigations
### Risk: accidentally changing reflow semantics
A too-low-level fix in `Row::shrink` could remove `WIDE_CHAR` before the `reflow=true` branch has a chance to preserve wrapped wide-character semantics.
Mitigation: keep the mutation in `GridStorage::shrink_cols` and gate it on `!reflow`.
### Risk: hiding malformed rows in `RowIterator`
A broad consumer guard in `RowIterator::next` could stop panics while making future producer bugs invisible.
Mitigation: repair the confirmed producer and rely on deterministic invariant tests. Add consumer hardening only if it is explicitly justified and separately tested.
### Risk: overclaiming coverage for related RowIterator issues
Public issues such as #11471 and #12459 share a `RowIterator::next` crash shape, but their public reports do not prove the same no-reflow clear-resize producer.
Mitigation: describe this spec as fixing the deterministic producer in #12243. Do not claim it fixes every RowIterator bounds-check crash unless further evidence ties those reports to the same producer.
### Risk: resetting the boundary glyph is user-visible
When no-reflow resize discards the spacer cell, the retained leading cell can no longer be represented as a valid two-cell wide character in that row. Resetting that leading cell means the boundary glyph is not displayed.
Mitigation: this only happens at the discard boundary in a path that already discards right-side overflow rather than restoring it after resize. Valid pairs inside the retained width are unchanged, and ordinary reflowing resize still preserves split wide characters through `LEADING_WIDE_CHAR_SPACER`.
## Follow-ups
- If more RowIterator crashes appear with different producer paths, consider a broader audit of all row producers that can create or mutate wide-character pairs.
- Consider adding debug-only invariant checks around row transitions into flat storage if future failures show more malformed-row producers.
