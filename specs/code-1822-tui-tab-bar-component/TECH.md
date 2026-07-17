# TECH: Reusable TUI Tab-Bar View
Linear: [CODE-1822 — Orchestration](https://linear.app/warpdotdev/issue/CODE-1822/orchestration)
Product: [specs/code-1822-tui-tab-bar-component/PRODUCT.md](./PRODUCT.md)

## Context
The TUI separates retained views from per-frame elements:
- `crates/warpui_core/src/core/view/tui.rs (19-75)` defines `TuiView`, whose `render` method produces an element tree from retained state.
- `crates/warpui_core/src/elements/gui/size_constraint_switch.rs (45-153)` is the GUI responsive-layout precedent: it selects a normal child from the current layout constraint and delegates subsequent lifecycle passes to that child.
- `crates/warpui_core/src/elements/tui/mod.rs (205-292)` defines the TUI element lifecycle.
- `crates/warpui_core/src/elements/tui/flex.rs (263-424)`, `container.rs`, `text.rs`, and `hoverable.rs` provide the generic composition, styling, text, and pointer primitives needed by a tab bar.

The reusable tab abstraction is a retained view in the `warp_tui` front-end, not a tab-specific element. The core element library supplies the same discrete size-constraint switch pattern as the GUI.

## Implementation
### GUI-parity size switching
`crates/warpui_core/src/elements/tui/size_constraint_switch.rs` adds `TuiSizeConstraintSwitch` and `TuiSizeConstraintCondition`.

Like the GUI `SizeConstraintSwitch`, it accepts a default prebuilt child plus ordered conditional children. During layout it selects the first child whose width, height, or combined-size condition matches. Every later lifecycle pass delegates to that same selected child.

The switch contains no tab, paging, or application semantics. It is exported from `crates/warpui_core/src/elements/tui/mod.rs`.

### Generic text ellipsis
`crates/warpui_core/src/elements/tui/text.rs` adds `TuiText::truncate_with_ellipsis`. The text element truncates inside its assigned display-cell width, preserves grapheme boundaries and span styles, and uses as much of `...` as fits. Tab rendering therefore does not construct pre-truncated strings.

### Retained tab-bar view
`crates/warp_tui/src/tab_bar.rs` defines:
- `TuiTab`: stable string key, label, and optional styled leading text.
- `TuiTabBarStyles`: caller-supplied bar, leading-label, chrome, normal-tab, focused-selected, and unfocused-selected styles.
- `TuiTabBarConfig`: optional product label and main tab, ordered secondary tabs, selected key, focus presentation, page anchor, selected-tab reveal policy, optional maximum label cells, spacing, and styles.
- `TuiTabBarEvent`: semantic `SelectTab` and `PageChanged` outcomes.
- `TuiTabBarNavigationDirection` and `TuiTabBarSecondaryEdge`: semantic keyboard target requests.
- `TuiTabBarView`: retained view state and responsive rendering.

The view is registered as a typed-action TUI view. Click handlers on generic `TuiHoverable` elements dispatch private component actions; `TuiTabBarView::handle_action` converts those actions into public view events for its owner.

### State ownership
`TuiTabBarView` retains:
- `HashMap<String, MouseStateHandle>` for currently supplied tab keys.
- One mouse handle for each overflow arrow.
- The latest caller-supplied `TuiTabBarConfig`.

`set_config` replaces semantic inputs, prunes removed mouse handles, creates handles for new keys, and notifies the view. Application selection, focus, and page anchors remain caller-owned.

### Responsive row composition
`TuiTabBarView::render` prebuilds one row alternative for each distinct visible-tab count. `TuiSizeConstraintSwitch` selects the row during layout. Each row is composed only from:
- `TuiFlex` for row ordering;
- `TuiFlex::with_spacing` for gaps between leading text and labels, tabs, and overflow controls;
- `TuiText` for labels, divider, and arrows;
- `TuiConstrainedBox` for configured maximum label and tab widths;
- `TuiContainer` for tab and divider padding plus backgrounds;
- `TuiHoverable` for hover and click behavior.

The static threshold calculation:
1. Measures known text and padding in terminal display cells.
2. Reserves the optional caller label, fixed main tab, and divider.
3. Resolves the requested secondary page anchor, falling back to the first page.
4. Computes the minimum row width for each possible visible-tab count from both the requested anchor and, when reveal is enabled, the selected tab.
5. Reserves a previous control only when the page starts after the first secondary tab.
6. Reserves a next control only when the page ends before the last secondary tab.
7. Gives the final visible tab the remaining flex width only when that width can show at least one label glyph plus the ellipsis; otherwise the tab becomes the next-page anchor.
For each width alternative, the requested anchor wins while the selected tab remains visible. An off-page selected tab moves to the deterministic page containing it only when reveal is enabled. The view derives a non-overlapping page sequence from the first secondary tab: next-page anchors begin after the final visible tab, and previous-page anchors target the preceding sequence entry rather than subtracting the current page size. This preserves stable in-page selection and whole-page navigation without exposing layout geometry to the caller.

`crates/warpui_core/src/elements/tui/text_helpers.rs` continues to provide shared display-cell measurement and string truncation for non-element formatting such as `crates/warp_tui/src/tui_column_layout.rs`.

### Navigation
Keyboard target methods depend only on semantic tab order:
- Previous/next navigation uses the optional main tab followed by all secondary tabs and wraps.
- First/last-secondary navigation reads the edges of the secondary list.

The caller applies returned keys, updates its authoritative selection/page models, and resynchronizes the view config.

## Testing and validation
`crates/warpui_core/src/elements/tui/size_constraint_switch_tests.rs` covers default, ordered width/height, and combined-size selection.

`crates/warpui_core/src/elements/tui/text_tests.rs` covers constraint-aware ellipsis, grapheme preservation, and span styling.

`crates/warp_tui/src/tab_bar_tests.rs` covers:
- page anchors and selected-tab reveal;
- stable pages when selection changes within the visible range, including the all-tabs-fit case;
- strictly increasing visible-count thresholds;
- narrow-width ellipsis and next-control preservation;
- start, middle, and end overflow-control visibility;
- semantic navigation and secondary edges;
- selected and leading-text styles rendered through generic elements; and
- retained mouse-state reuse and removed-key pruning.

`crates/warpui_core/src/elements/tui/text_helpers_tests.rs` covers display-cell measurement and grapheme-safe truncation.

Validation commands:
- `cargo test -p warpui_core --features tui size_constraint_switch`
- `cargo test -p warpui_core --features tui ellipsis`
- `cargo test -p warp_tui tab_bar`
- `cargo test -p warpui_core --features tui text_helpers`
- `cargo test -p warp_tui tui_column_layout`
- `./script/format`
- `cargo clippy -p warpui_core --features tui --tests -- -D warnings`
- `cargo clippy -p warp_tui --tests -- -D warnings`

## Risks and mitigations
- **Responsive policy leaking into the element library:** the switch knows only about conditions and child lifecycle delegation, matching the GUI primitive.
- **Layout/event disagreement:** each prebuilt row owns its matching visible tabs and overflow callbacks; the switch delegates all passes to one selected row.
- **Variant growth:** alternatives are created only when the visible-tab count changes, not for every terminal column.
- **Stale pointer state:** `set_config` keys mouse handles by stable tab identity and prunes removed keys.
- **Unicode width corruption:** `TuiText` measures terminal display width and truncates only at grapheme boundaries.
- **Application state divergence:** the view emits semantic events and never mutates caller-owned selection, focus, or page models.
