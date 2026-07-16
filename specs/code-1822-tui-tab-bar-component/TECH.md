# TECH: Reusable TUI Tab-Bar Component
Linear: [CODE-1822 — Orchestration](https://linear.app/warpdotdev/issue/CODE-1822/orchestration)
Product: [specs/code-1822-tui-tab-bar-component/PRODUCT.md](./PRODUCT.md)
Inspected commit: `caa826c2ef395faee32c87c19c533a44ef88d81b`

## Context
The TUI cell-grid library has the layout, styling, retained-geometry, and click primitives needed for a horizontal tab bar, but no component that owns tab packing or paging:
- [`crates/warpui_core/src/elements/tui/mod.rs (31-74) @ caa826c2`](https://github.com/warpdotdev/warp/blob/caa826c2ef395faee32c87c19c533a44ef88d81b/crates/warpui_core/src/elements/tui/mod.rs#L31-L74) — exports the TUI element vocabulary and `TuiElement`.
- [`crates/warpui_core/src/elements/tui/mod.rs (215-292) @ caa826c2`](https://github.com/warpdotdev/warp/blob/caa826c2ef395faee32c87c19c533a44ef88d81b/crates/warpui_core/src/elements/tui/mod.rs#L215-L292) — `layout` must remain side-effect free, while `after_layout` is the settled post-layout side-effect seam.
- [`crates/warpui_core/src/elements/tui/hoverable.rs (40-189) @ caa826c2`](https://github.com/warpdotdev/warp/blob/caa826c2ef395faee32c87c19c533a44ef88d81b/crates/warpui_core/src/elements/tui/hoverable.rs#L40-L189) — `TuiHoverable` requires stable `MouseStateHandle`s across per-frame element reconstruction and limits hit testing to retained painted bounds.
- [`crates/warpui_core/src/elements/tui/flex.rs @ caa826c2`](https://github.com/warpdotdev/warp/blob/caa826c2ef395faee32c87c19c533a44ef88d81b/crates/warpui_core/src/elements/tui/flex.rs) — row composition and child layout.
- [`crates/warpui_core/src/elements/tui/text.rs @ caa826c2`](https://github.com/warpdotdev/warp/blob/caa826c2ef395faee32c87c19c533a44ef88d81b/crates/warpui_core/src/elements/tui/text.rs) — styled terminal text and existing single-element truncation.

The component is intentionally domain-neutral. The orchestration integration that first consumes it is specified separately in `specs/code-1822-tui-orchestration-tab-bar/`.

## Proposed changes
### Public component contract
Add `crates/warpui_core/src/elements/tui/tab_bar.rs` and export the component's public types from `crates/warpui_core/src/elements/tui/mod.rs`.

Use a stable string key rather than indices so dynamic reordering cannot retarget callbacks without parameterizing the component and every supporting layout type. The public data surface contains:
- `TuiTab`: string key, label, and an optional factory for caller-rendered leading content. During each layout pass, the component builds the element once, measures it, and moves that same instance into the rendered tab.
- `TuiTabBarStyles`: caller-supplied bar, leading-label, chrome, normal-tab, focused-selected, and unfocused-selected styles.
- `TuiTabBarConfig`: optional product label and main tab, ordered secondary tabs, selected key, focus presentation, page anchor, selected-tab reveal policy, optional maximum label cells, spacing, and styles. Divider and arrow glyphs are component-owned so every caller uses the same row structure.
- `TuiTabBarNavigationDirection`: `Previous` or `Next`.
- `TuiTabBar`: the reusable component retained by the caller and updated/rendered from config.

The component exposes high-level operations only:
- `render(config, on_event) -> Box<dyn TuiElement>`
- `navigation_target(direction) -> Option<String>`, which resolves against private settled layout.

It does not expose visible indices, visible keys, page boundaries, measured widths, or mouse handles.

### Private retained state
`TuiTabBar` privately retains:
- `HashMap<String, MouseStateHandle>` for currently supplied tabs.
- Previous/next overflow mouse handles.
- The latest lightweight `SettledNavigation` containing only ordered keys, explicit main-tab identity, selected key, and visible secondary keys.

Every config update prunes removed keys before rendering. Existing keys reuse their mouse handles. Settled navigation is invalidated before the next layout publishes replacement data.

The per-frame element receives an internal shared state reference so `after_layout` can publish settled navigation data back to the component without retaining or exposing the render-only page layout. This handle is private to `tab_bar.rs`; it is not part of the public contract described by PRODUCT (7).

### Layout algorithm
Build and measure caller-provided leading elements through the normal `TuiElement::layout` contract, then pass those widths into a pure layout function. The settled row takes the same measured element instances rather than invoking their factories again. The function returns a `TabBarLayout` with named main-tab, previous-anchor, visible-tab, next-anchor, and navigation fields rather than a generic sequence of layout pieces.

The algorithm:
1. Measure fixed caller-supplied leading content, the optional main tab, each tab's caller-rendered leading element, and the component-owned divider.
2. Normalize each label to the optional maximum display-cell width.
3. Resolve the requested page anchor against the ordered secondary keys, clamping a missing anchor.
4. Reserve a previous overflow control when the page does not start at the first secondary tab.
5. Pack secondary tabs from the anchor while reserving a next overflow control whenever later tabs remain.
6. If the final otherwise-visible tab does not fit in full, shrink its label to the remaining display cells while preserving its leading content and required next control.
7. Omit a secondary tab rather than produce invalid or negative-width geometry when the row is too narrow.
8. Build one deterministic page sequence from the beginning. Use that sequence for previous-page anchors and selected-tab reveal so all page progression shares one strictly advancing rule.

Use terminal display-cell width and grapheme-safe truncation. Keep the ellipsis inside the requested width. The component paints from the returned layout rather than independently remeasuring, so rendering, hit testing, overflow callbacks, and navigation share one result.

### Semantic interaction dispatch
Wrap every painted tab and overflow control in `TuiHoverable` with component-owned mouse state:
- A tab click invokes `SelectTab(key)`.
- A previous/next overflow click invokes `PageChanged(anchor_key)` from the settled layout.
- A hovered tab bolds its label, and a hovered overflow control bolds its arrow.
- Neither callback changes focus or application state directly.

`navigation_target(Previous | Next)` uses the private settled layout:
- If the selected key is visible, resolve the adjacent key from the complete supplied sequence of main tab followed by secondary tabs and wrap.
- If the selected key is off-page, resolve `Previous` to the last visible secondary key and `Next` to the first visible secondary key.
- If no target exists, return `None`.

The consuming view remains responsible for binding keys, forwarding directions, and applying the returned semantic target. This keeps keymap and application-selection policy outside the component while keeping width-dependent target resolution inside it.

## Testing and validation
Add `crates/warpui_core/src/elements/tui/tab_bar_tests.rs` using the element render and event-dispatch test harness:
- Optional/absent main tab, empty secondary tabs, caller-rendered leading elements, and fixed chrome — PRODUCT (1-5).
- Private mouse-state reuse and pruning across config changes — PRODUCT (6-11).
- Focused/unfocused selected treatments and missing selection — PRODUCT (12-15).
- ASCII, wide Unicode, and combining-character measurement; configured and final-tab truncation — PRODUCT (16-22).
- Initial, middle, and final pages; anchor clamping; resize; previous/next control visibility; stable selected-tab reveal — PRODUCT (23-32).
- Tab clicks, overflow clicks, hit bounds, cancelled press/release, focus independence, and hover treatments — PRODUCT (33-35, 38-41).
- Visible and off-page previous/next navigation, including complete-order wraparound — PRODUCT (36-37).

Run:
- `cargo nextest run -p warpui_core --features tui tab_bar`
- `cargo test -p warpui_core --features tui tab_bar`
- `./script/format`
- The repository-prescribed Clippy command before submitting the branch.

## Parallelization
Do not split this component across child agents. Its public contract, private retained state, layout result, and event tests are tightly coupled and should land as one coherent PR. Long-running workspace validation can run separately after focused tests pass.

## Risks and mitigations
- **Public geometry leakage:** keep `TabBarLayout` and `SettledNavigation` private to `tab_bar.rs`; expose semantic callbacks only.
- **Layout/event disagreement:** paint and dispatch from one settled layout result.
- **Stale navigation after mutation:** invalidate private layout on config changes and resolve callback keys against the latest supplied key set.
- **Unicode width corruption:** keep one grapheme-safe display-cell truncation helper in the TUI text layer and use it from the tab bar and two-column formatter.
- **Hover-state churn:** key mouse state by stable tab key and prune only removed keys.
