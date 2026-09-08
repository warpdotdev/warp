# TuiFlex cross-axis sizing + `CrossAxisAlignment` — Tech Spec

Branch: `harry/tui-flex-alignment`. Consumed by the stacked shell-command-execution work ([`specs/CODE-1805/TECH.md`](../CODE-1805/TECH.md)).

## Motivation

`TuiFlex` previously reported its cross-axis size as the full extent it was offered: a row offered 80×20 claimed 20 rows tall regardless of content. Callers that needed a thinner flex capped the *offer* with a `TuiConstrainedBox` (e.g. the one-row footer in `crates/warp_tui/src/terminal_session_view.rs`), which only works when the height is known up front.

The shell-mode input breaks that assumption. It composes as `row[ "!" gutter, editor ]`, where the editor is 1–6 rows tall depending on how its text wraps — a height only known *during* layout, so there is nothing to pre-cap the row at. With fill-the-offer sizing, the row claims the whole offered height and destroys the bottom-docked input layout. The row must size to its tallest child.

This is also exactly the GUI `Flex`'s policy (Flutter's): cross-axis size is the max of the children's cross extents (`cross_axis_max` in `crates/warpui_core/src/elements/gui/flex/mod.rs (229-345)`), with filling as an opt-in alignment. The TUI flex was the divergence.

## Changes

All in `crates/warpui_core/src/elements/tui/flex.rs` plus two call-site consequence fixes.

### Content-sized cross axis (new default)

`TuiFlex::layout` now reports its cross axis as the largest child's cross extent, clamped to the constraint — so a tight cross constraint still forces the flex to fill it, matching how the GUI flex respects `constraint.min`. Children are still *offered* the full cross extent (loose); flex children are tight only along the main axis. Rendering is unchanged for `Start`: each child keeps its full-cross-extent slot, so paint regions and hit areas don't shift.

### `with_cross_axis_alignment(CrossAxisAlignment)`

Reuses the GUI's `CrossAxisAlignment` enum directly (`crates/warpui_core/src/elements/gui/flex/mod.rs:733`; already re-exported at `crate::elements::` alongside `Axis`, so no hoist was needed) with the same builder name as the GUI `Flex`:

- `Start` (default): children anchored at the cross start with full-slot rects — the pre-change behavior minus the greedy self-sizing.
- `Center` / `End`: each child's measured cross extent is positioned within its slot. One helper (`child_rect_for`, `flex.rs:193`) computes the rect and is shared by `render`, `cursor_position`, and `dispatch_event`, so painting, cursor placement, and hit-testing always agree.
- `Stretch`: children get a tight cross constraint (`child_cross_min`, `flex.rs:168`) and the flex reports the full offered cross extent (`reported_cross`, `flex.rs:178`) — the GUI `Stretch` semantics, used where a child must span the flex (e.g. a full-width background).

### Call-site consequence fixes

Two places relied on the old fill behavior:

- The transcript input banner (`crates/warp_tui/src/agent_block_sections.rs:30`) opts into `Stretch` so its highlighted background spans the full row, not just the text width.
- `TuiInputElement::layout` (`crates/warp_tui/src/input/view.rs (1302-1306)`) reports its full wrap width rather than its inner column's content width, since the cursor/selection geometry assumes the element spans the width it wrapped at.

## GUI parity summary

Default cross-axis sizing (content, clamped to constraint), the `CrossAxisAlignment` enum and builder method, and `Stretch`'s tighten-children behavior now all match the GUI `Flex`. Remaining intentional difference: under `Start` the TUI gives children full-cross-extent slots for paint/hit-testing (cells are cheap and this preserves existing row-wide click targets), whereas the GUI paints children at their own size.

## Testing

`crates/warpui_core/src/elements/tui/flex_tests.rs`: content-sized rows (`row_sizes_cross_axis_to_its_tallest_child`), tight-constraint fill (`tight_cross_axis_constraint_forces_fill`), `Stretch` reporting + child tightening (`stretch_fills_offered_cross_extent_and_tightens_children`), and `Center`/`End` positioning via rendered output (`center_positions_child_along_cross_axis`, `end_positions_child_along_cross_axis`). Existing flex, input-view, and agent-block suites pin the no-regression behavior at the call sites.
