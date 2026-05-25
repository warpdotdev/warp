# APP-4267: Mermaid render failures show an explicit callout — Tech Spec
Product spec: `specs/APP-4267/PRODUCT.md`
Linear issue: https://linear.app/warpdotdev/issue/APP-4267/show-failure-callout-when-mermaid-diagram-rendering-gets-stuck
## Context
Mermaid rendering is implemented as an async image asset layered on top of the rich-text markdown renderer.
- `crates/editor/src/content/text.rs (721-729)` classifies fenced Mermaid blocks as `CodeBlockType::Mermaid` when `FeatureFlag::MarkdownMermaid` is enabled.
- `crates/editor/src/content/edit.rs (671-726)` converts styled Mermaid code blocks into `LayoutTask::MermaidDiagram` and calls `mermaid_diagram_layout`.
- `crates/editor/src/content/edit.rs (1031-1067)` converts that layout task into `BlockItem::MermaidDiagram` while preserving the source block content length.
- `crates/editor/src/content/mermaid_diagram.rs:20` defines the current default pending height as 10 line-heights.
- `crates/editor/src/content/mermaid_diagram.rs (28-44)` creates an `AssetSource::Async` whose fetch future calls `mermaid_to_svg::render_mermaid_to_svg`.
- `crates/editor/src/content/mermaid_diagram.rs (47-66)` computes Mermaid block dimensions from the loaded SVG and falls back to the default placeholder height while the asset is not loaded.
- `crates/editor/src/render/element/mermaid.rs (33-66)` renders a Mermaid block by wrapping `Image::new(...).contain().before_load(...)` with the “Rendering Mermaid diagram…” placeholder.
- `crates/editor/src/render/element/mermaid.rs (68-112)` paints the code-block-like rounded background/border and then paints the image element into the content rect.
- `crates/warpui_core/src/elements/image.rs (316-358)` currently paints `before_load_element` for `Loading`, `Evicted`, and `FailedToLoad`; the shared image element has no separate failed-state element.
- `crates/warpui_core/src/assets/asset_cache.rs (284-330)` stores new async assets as `Loading` and spawns the fetch future once per async asset ID.
- `crates/warpui_core/src/assets/asset_cache.rs (415-486)` promotes async assets to `Loaded` or `FailedToLoad` only when the background future resolves.
There is already an attached implementation branch, `origin/oz-agent/APP-4267/mermaid-failure-callout`, that adds an `Image::on_load_failure` element and a compact Mermaid failure notice for `AssetState::FailedToLoad`. That direction fits the current architecture, but it should be tightened to cover the product-level timeout behavior: a truly stuck render can remain `AssetState::Loading` forever because the asset cache only transitions when the fetch future resolves.
## Proposed changes
### 1. Add a Mermaid-specific failed height
In `crates/editor/src/content/mermaid_diagram.rs`, add a compact failed-height multiplier next to the existing pending-height multiplier:
- `DEFAULT_MERMAID_HEIGHT_LINE_MULTIPLIER` remains the pending/success fallback height.
- `FAILED_MERMAID_HEIGHT_LINE_MULTIPLIER` should be 2 line-heights for known permanent failures.
Update `mermaid_diagram_layout` so it distinguishes:
- `Loaded` SVG asset: use intrinsic SVG aspect ratio, as today.
- `FailedToLoad`: use `max_width` and `base_line_height * FAILED_MERMAID_HEIGHT_LINE_MULTIPLIER`.
- `Loading` or `Evicted`: keep the existing default pending height.
Keep the helper focused on layout sizing; do not add render-element state to the content model.
### 2. Add explicit failed-load rendering to `Image`
In `crates/warpui_core/src/elements/image.rs`, add an optional failed-state element to `Image`:
- Store `failed_to_load_element: Option<Box<dyn Element>>`.
- Add `pub fn on_load_failure(mut self, element: Box<dyn Element>) -> Self`.
- During `layout` and `after_layout`, lay out both the before-load element and failed-load element when present.
- During `paint`, render `failed_to_load_element` for `AssetState::FailedToLoad(_)`; if none is provided, preserve current behavior by falling back to `before_load_element`.
This keeps all existing image callers behavior-compatible while allowing Mermaid to provide a distinct error UI.
### 3. Add a loading-timeout path for Mermaid
`AssetState::FailedToLoad` only covers futures that resolve with an error. To satisfy `PRODUCT.md` Behavior 2 for stuck renders, add a Mermaid render timeout without making every image in Warp time out.
Preferred shape:
- Add an optional timeout API to `Image`, for example `pub fn on_load_timeout(mut self, timeout: Duration, element: Box<dyn Element>) -> Self`.
- Track load start time by stable asset-source key inside the image element module. Do not reuse the existing animation `started_at`, and do not store the timeout start only on a single `Image` instance because rich-text layout can rebuild the element before the timeout fires.
- When `paint` sees `AssetState::Loading`, initialize `load_started_at` if needed, schedule a repaint for the timeout deadline, and paint the before-load element until the timeout expires.
- Once the timeout expires, paint the timeout element instead of the before-load element while the asset remains `Loading`.
- If the asset later becomes `Loaded`, paint the image normally and clear backup elements as the existing loaded path does.
- If the asset later becomes `FailedToLoad`, prefer the failed-load element.
If a generic `Image` timeout API feels too broad during implementation, keep the timeout state in `RenderableMermaidDiagram` instead. The important invariant is user-visible: Mermaid cannot show the loading text indefinitely. Avoid wrapping `render_mermaid_to_svg` with `warpui::r#async::FutureExt::with_timeout` as the only timeout mechanism; that helper only times out while the wrapped future yields, and Mermaid rendering is currently a synchronous call inside the async fetch body.
### 4. Wire the Mermaid failure and timeout callouts
In `crates/editor/src/render/element/mermaid.rs`:
- Build the existing loading placeholder as today, using `model.styles().placeholder_color`.
- Build a failure callout text element with the exact string `Failed to render Mermaid diagram`, the code text font family, font size, line-height ratio, and theme-derived placeholder or secondary text color.
- Create the `Image` with:
  - `.contain()`
  - `.before_load(loading_placeholder)`
  - `.on_load_failure(failure_callout)`
  - the Mermaid timeout API from §3, using a 10 second timeout and the same visible failure callout text.
Keep the existing rounded background, border, selection overlay, and cursor painting in `RenderableMermaidDiagram::paint`.
### 5. Preserve source and selection semantics
Do not change `BlockItem::MermaidDiagram` content length, markdown serialization, copy behavior, hidden block handling, or editor selection semantics. The failure callout is a render-state presentation for the diagram block, not new document content.
### 6. Logging
Do not add new user-visible toasts. Existing `AssetCache` warning logs for fetch/conversion failures are sufficient for actual failed loads. If timeout logging is added, use at most a debug-level log so a pathological document with many invalid diagrams does not create noisy client logs.
## Testing and validation
Use `cargo nextest run --no-fail-fast --workspace <test identifier>` for focused Rust tests in this repo.
### Unit tests
- Add `crates/editor/src/content/mermaid_diagram_tests.rs` and import it from `mermaid_diagram.rs` with a `#[cfg(test)]` path module. Test that a failed Mermaid asset uses the compact failed height while loading still uses the default pending height.
- Extend or add `crates/warpui_core/src/elements/image_tests.rs` coverage for:
  - `AssetState::FailedToLoad` renders the failed-load element when one is provided.
  - `AssetState::FailedToLoad` falls back to `before_load_element` when no failed-load element is provided.
  - A loading image switches from before-load element to timeout element after the configured timeout.
  - Timeout start time survives rebuilding an `Image` element for the same asset source.
- Keep the existing `test_layout_mermaid_block_uses_loaded_svg_aspect_ratio` coverage in `crates/editor/src/content/edit_tests.rs` passing for successful diagrams.
### Integration/manual validation
- In a markdown viewer or editable plan, render a valid Mermaid diagram and confirm it still becomes an SVG.
- Render invalid Mermaid syntax and confirm the block shows `Failed to render Mermaid diagram` instead of staying on the loading placeholder.
- Simulate a stuck Mermaid render by using a test-only async asset source that never resolves, or a temporary local patch in `mermaid_asset_source`, and confirm the loading text is replaced after 10 seconds.
- Confirm editing the Mermaid source creates a new render attempt and does not permanently preserve the previous failure state.
- Confirm two Mermaid diagrams in the same document can independently show success and failure states.
- Confirm selecting/copying/exporting the document still preserves the authored fenced Mermaid markdown, not the failure callout text.
### Commands
- Focused editor tests: `cargo nextest run --no-fail-fast --workspace mermaid`
- Focused image tests: `cargo nextest run --no-fail-fast --workspace image`
- Compile check after implementation: `cargo check`
## Risks and mitigations
### Timeout does not cancel underlying work
A UI-level timeout prevents an indefinite placeholder, but it does not necessarily cancel a synchronous Mermaid render already running on the background executor. Keep the timeout scoped to presentation for this issue. If stuck renders consume executor capacity in practice, follow up with renderer-level cancellation or process isolation.
### Shared `Image` behavior regresses other callers
The new failure and timeout behavior must be opt-in. Existing callers without `on_load_failure` or timeout configuration should behave exactly as they do today.
### Layout height for timed-out loading assets
Known `FailedToLoad` assets can use compact layout on the next layout pass. A UI timeout while the asset remains `Loading` may still occupy the pending placeholder height unless implementation adds model-level timeout state and requests relayout. The first iteration prioritizes replacing the indefinite loading text; compact timeout layout can be a follow-up if needed.
## Parallelization
- One agent can implement the shared `Image` failed-load and timeout behavior plus unit tests.
- Another agent can wire Mermaid-specific layout and render behavior plus Mermaid-focused tests.
- Manual UI validation should wait until both code paths are integrated.
## Follow-ups
- Add a retry affordance for failed diagrams if users need to retry the same source without editing or reopening the document.
- Add a “copy Mermaid source” affordance if the failure state needs an explicit raw-source escape hatch.
- Investigate renderer-level cancellation or isolation if stuck Mermaid renders are confirmed to consume background executor capacity.
