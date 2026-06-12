# Tech Spec: Double-click the hidden-section bar to fully expand

**Issue:** [warpdotdev/warp#11622](https://github.com/warpdotdev/warp/issues/11622)
**Product spec:** `specs/GH11622/product.md`

## Context

The diff editor collapses long runs of unchanged context into hidden sections. Each section paints a full-width **bar** in the content area and a separate **gutter expand button** (`▲`/`▼`/`▲▼`) in the left margin. The product spec moves "expand the whole section" onto a **double-click of the bar**, leaves single-click chunked reveal on the gutter button exactly as it is today, and makes a single click on the bar a no-op.

References are pinned to the pre-feature baseline `a44fbf16` (the parent of the discarded implementation — see note below). Paths are repo-root-relative.

**The bar today.** `RenderableHiddenSection` (`crates/editor/src/render/element/hidden_section.rs`) paints a full-width `Container` (background `fg_overlay_1`) wrapping an empty `Flex` row. Its `dispatch_event` (`hidden_section.rs:72-80`) just delegates to that inert container, which returns `false` ("not consumed"). Because nothing consumes the press, `RichTextElement::dispatch_event` (`crates/editor/src/render/element/mod.rs:1190-1204`) routes the `LeftMouseDown` to `handle_left_mouse_down`, which clamps to a text offset and starts a selection. That fall-through is exactly the unwanted "click the bar selects text" behavior the product spec calls out.

**The expansion path we reuse (unchanged).** Gutter single-click → `CodeEditorViewAction::HiddenSectionExpansion { line_range, expansion_type }` → handler in `app/src/code/editor/view/actions.rs` → `CodeEditorView::expand_hidden_section` (`app/src/code/editor/view.rs:896`) → `model.set_visible_line_range(..)` → `HiddenLinesModel::set_visible_line_range` (`crates/editor/src/content/hidden_lines_model.rs:133`). Passing the **full** section range with `ExpansionType::Both` already reveals an entire section in one transition, so the bar can reuse this path verbatim.

**The precedent for "block click → app action."** The bar lives in `crates/editor`, which cannot name the app's `CodeEditorViewAction`. `RenderableTaskList` (`crates/editor/src/render/element/task_list.rs:30-98`) solves the same problem: it is generic `new<V: EditorView>(.., mouse_state: MouseStateHandle, parent_view: WeakViewHandle<V>)`, wraps its content in a `Hoverable` with `.on_click(|ctx, app, _| { if let Some(a) = V::Action::task_list_clicked(offset, &parent_view, app) { ctx.dispatch_typed_action(a) } })`, `.with_cursor(Cursor::PointingHand)`, and a hover-background closure. The `RichTextAction<V>` trait (`crates/editor/src/render/element/mod.rs:307`, with `task_list_clicked` at `:366`) is the decoupling seam — the editor crate produces an opaque `V::Action`; the app supplies the concrete action.

Supporting APIs:
- `Hoverable` (`crates/warpui_core/src/elements/hoverable.rs`): `on_click` (`:249`), `on_double_click` (`:279`), `with_cursor` (`:346`), `on_hover` (`:240`). A `Hoverable` that registers a handler **consumes** the press, which is what suppresses the text-selection fall-through.
- `RenderState::line_range(&dyn RenderableBlock) -> Option<Range<LineCount>>` (`crates/editor/src/render/model/mod.rs:3258`) gives a block's full line span — the bar's full hidden range.
- `RenderableHiddenSection::new` is constructed at `crates/editor/src/render/element/mod.rs:908`, inside `renderable_blocks`, where `self.parent_view: WeakViewHandle<V>` is already in scope (it is handed to `RenderableTaskList` at `:857-863`).
- `BlockItem::Hidden`'s payload `HiddenBlockConfig` (`crates/editor/src/render/model/mod.rs:1598`) carries `line_count`/`content_length`/`block_location` but — unlike `BlockItem::TaskList` — **no** `MouseStateHandle`, which a `Hoverable` needs to persist hover state across frames.

> **Discarded prior work.** The earlier gutter-button + debounce implementation (commit `a3187b1f`: `warpui_core::DeferredCall` and its tests, the platform `double_click_interval` shim, gutter `click_count`/`pending_click_count` plumbing, `full_line_range` on `GutterRange`/`GutterElementType`, and the `click_count` field + defer branch on the action) is dropped in full. The separate-targets design needs none of it: gutter single-click stays instant, and double-click detection comes from `Hoverable` on the bar.

## Proposed changes

### 1. Make `RenderableHiddenSection` an interactive bar (editor crate)

In `crates/editor/src/render/element/hidden_section.rs`, make the struct generic the way `RenderableTaskList` is: `new<V: EditorView>(viewport_item, mouse_state: MouseStateHandle, parent_view: WeakViewHandle<V>, app: &AppContext)`. Wrap the existing background `Container` in a `Hoverable`:

- `.on_double_click(move |ctx, app, _| { if let Some(a) = V::Action::hidden_section_double_clicked(block_offset, &parent_view, app) { ctx.dispatch_typed_action(a) } })` — full expand.
- `.on_click(|_, _, _| {})` — an empty single-click handler. Its only job is to make the `Hoverable` consume the press so a single click on the bar no longer falls through to selection (invariant #3).
- `.with_cursor(Cursor::PointingHand)` — pointer affordance (invariant #4).
- A hover-state background in the `Hoverable` build closure (e.g. brighten `fg_overlay_1` when `state.is_hovered()`) — hover highlight (invariant #4).

`dispatch_event` delegates to the `Hoverable` (replacing the current delegate to the inert container) and returns its consumed flag. `layout`/`paint` keep painting the wrapped element.

### 2. Give the hidden block a `MouseStateHandle`

`Hoverable` needs a frame-stable `MouseStateHandle`. Add one to `HiddenBlockConfig` (`crates/editor/src/render/model/mod.rs:1598`), populated where `BlockItem::Hidden` is built, mirroring how `BlockItem::TaskList` carries `mouse_state`. Pass `config.mouse_state.clone()` into `RenderableHiddenSection::new` at `mod.rs:908`. Without a persisted handle the hover highlight would flicker each layout.

### 3. New `RichTextAction` method

Add to the `RichTextAction<V>` trait (`crates/editor/src/render/element/mod.rs:307`), alongside `task_list_clicked`:

```
fn hidden_section_double_clicked(
    block_offset: CharOffset,
    parent_view: &WeakViewHandle<V>,
    ctx: &AppContext,
) -> Option<Self>;
```

This keeps the editor crate decoupled from the app's action enum. (Passing `block_offset` mirrors `task_list_clicked`; the app resolves it to a line range in change #4. Alternative: resolve the range inside `RenderableHiddenSection::dispatch_event` via `model.line_range(self)` — the model is available there — and pass a `Range<LineCount>` into the trait method instead. Prefer the offset form for consistency with the existing precedent unless offset→range resolution proves awkward app-side.)

### 4. App-side trait impl reuses the existing action

In the app's `impl RichTextAction<CodeEditorView> for CodeEditorViewAction`, implement `hidden_section_double_clicked` to resolve `block_offset` to the section's full `Range<LineCount>` (using the same model offset→line mapping `RenderState::line_range` relies on — `content.block_at_offset(offset)` for the line count plus the block's start line) and return the **existing** variant:

```
CodeEditorViewAction::HiddenSectionExpansion {
    line_range: full_range,
    expansion_type: ExpansionType::Both,
}
```

No new action variant and no handler change: the baseline handler already calls `expand_hidden_section(line_range, ExpansionType::Both, ctx)`, which fully reveals the range in one `set_visible_line_range` call.

### 5. Tooltip on the bar

`Hoverable` has no `with_tooltip`; tooltips in this codebase are a `Button` affordance (`with_tooltip`, e.g. `app/src/code_review/code_review_view.rs:207`) backed by the tooltip overlay system. The bar is a raw `Hoverable`/`Container`, so the tooltip is the one piece without a drop-in API. Reuse the same tooltip overlay the button path uses, driven by the bar's hover state. This is the highest-uncertainty sub-task — see Risks; if it cannot reuse the overlay cleanly it is the natural candidate to split into a fast-follow while shipping cursor + hover-state discoverability first.

## Testing and validation

Invariants below refer to the numbered **Behavior** list in `specs/GH11622/product.md`.

| Invariant | Verification |
|-----------|--------------|
| #1 (double-click bar fully expands, one transition) | Integration test in `crates/integration/src/test/code_review.rs`: fixture with a multi-hundred-line hidden section; drive a double-click on the bar (or dispatch the resolved `HiddenSectionExpansion { full_range, Both }`); assert the section is gone in one model update. |
| #2 (gutter single-click chunks, instant) | Existing chunked-expansion path is untouched and no longer deferred; covered by existing gutter test. Add an assertion that a single gutter click reveals exactly one chunk with no pending timer. |
| #3 (bar single-click is a no-op, no selection) | Test: single-click the bar; assert no selection/cursor change and no visible-range change. Structurally guaranteed by the `Hoverable` consuming the press. |
| #4 (discoverable: cursor, hover, tooltip) | Cursor (`with_cursor`) and hover background are review + manual-screenshot verifiable; tooltip is manual. |
| #5 (triple+ click ≡ double-click) | `Hoverable::on_double_click` fires once; after the section expands the bar is gone, so later clicks have no target. Covered by code review + the #1 test (assert a single expansion). |
| #6 (double-click gutter button = two chunks, no special behavior) | Gutter path is unchanged and has no double-click handling; review + a test that two gutter clicks reveal two chunks. |
| #7 (gestures independent, no debounce/cancel) | Structural: no `DeferredCall` exists; the two gestures hit different elements/handlers. Code review. |
| #8 (small section double-click fully reveals, no regression) | Integration test variant with a section that one chunk would fully reveal; assert double-click result equals the single-chunk result. |
| #9 (adjacent gutter interactions unaffected) | Sliver toggle / add-as-context / revert / gutter no-op arms are untouched; existing tests cover them. |
| #10 (cross-OS identical, detection via framework) | Double-click detection is `Hoverable`/framework-level with no per-feature platform code, so behavior is uniform across macOS/Linux/Windows. Code review (no multi-OS CI test). |

### Manual test plan

A short matrix in the PR description covers: double-click expand on large and small sections; single-click bar produces no selection; pointer cursor + hover highlight + tooltip appear; gutter single-click still chunks instantly; double-click on the gutter button reveals two chunks; and adjacent gutter controls (sliver, `+`, revert) still work.

## Parallelization

Not worth splitting. The change is small and tightly coupled across a single compile boundary: the new `RichTextAction` method (editor crate), its app-side impl, and the bar wiring must all land together to build at all, and they touch only a handful of files. A single agent on one checkout is faster than coordinating worktrees here.

## Risks and mitigations

- **Tooltip has no drop-in API on `Hoverable`.** Highest-uncertainty piece (change #5). Mitigation: reuse the button tooltip overlay; if integration is non-trivial, ship cursor + hover-state discoverability and split the tooltip into a fast-follow rather than block the feature.
- **Bar consuming the press could swallow legitimate selection.** A drag-select that *starts* on the bar would no longer place a cursor. This matches the product intent (the bar is not text), but verify a drag that *crosses* the bar from real content still selects normally.
- **`MouseStateHandle` plumbing.** Hover state requires the handle to be created where the hidden block is built and persisted in `HiddenBlockConfig`; a freshly-allocated handle per layout would flicker. Mirror `BlockItem::TaskList`.
- **Bar vs. gutter hit-testing.** Confirm the bar's `Hoverable` bounds do not overlap the gutter button's hit region, so double-clicking near the arrow does not ambiguously trigger both.

## Follow-ups

- If the tooltip is deferred, track it as the one remaining discoverability item from invariant #4.
- Consider hoisting the offset→`Range<LineCount>` resolution into a small reusable model helper if a second caller appears.
