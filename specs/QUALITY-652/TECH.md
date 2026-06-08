# Orchestration Config Header Scrolls With Plan Content
Linear: [QUALITY-652](https://linear.app/warpdotdev/issue/QUALITY-652/orchestration-config-header-should-scroll-away-with-content)
Current implementation branch: `harry/quality-652-orchestration-config-header-should-scroll-away-with-content`
## Context
The orchestration config block is part of the AI document plan UI. The issue is that this block should behave like plan content: when a user scrolls a long plan, the config chrome should scroll away before the plan body continues scrolling.
Relevant current code:
- `app/src/ai/ai_document_view.rs:317` reacts to `OrchestrationConfigUpdated`, lazily creates the config block view, and refreshes the editor scroll header.
- `app/src/ai/ai_document_view.rs:480` creates the initial `OrchestrationConfigBlockView` when opening an AI document with an existing config.
- `app/src/ai/ai_document_view.rs:541` adapts the AI document config block into a rich-text editor scroll-header renderer.
- `app/src/notebooks/editor/view.rs:93` defines `RichTextWithScrollHeaderElement`, which measures optional header chrome and passes it to the rich-text renderer as a scroll prefix.
- `crates/editor/src/render/element/mod.rs:461`, `crates/editor/src/render/model/mod.rs:1896`, and `crates/editor/src/render/model/viewport.rs:155` keep one vertical scroll coordinate while deriving content-space offsets after the header prefix is consumed.
## Proposed changes
Keep the orchestration config block owned by `AIDocumentView`, but render it inside the embedded `RichTextEditorView` as an optional scroll header instead of as a sibling above the editor.
The editor exposes `ScrollHeaderRenderer`, a minimal callback that returns optional header chrome for the current render pass. `AIDocumentView::update_editor_scroll_header` supplies a renderer when an orchestration config block exists and clears it otherwise.
`RichTextWithScrollHeaderElement` wraps the existing `RichTextElement` and implements `Element` plus `ScrollableElement`:
- During layout, measure the header and call `RichTextElement::set_scroll_prefix_height` with that measured height.
- During paint, draw rich text in the normal viewport and draw the header shifted upward by the combined scroll offset consumed within the prefix.
- For scrollbar data and wheel input, delegate to `RichTextElement`, whose scroll data now includes the prefix height.
`RenderState` and `ViewportState` own the scroll ordering:
- `ViewportState::scroll_top` remains the single vertical scroll coordinate and includes the header prefix.
- Rich-text content rendering, hit testing, viewporting, selection autoscroll, and cursor/selection positioning derive content-space offsets with `content_offset`, `content_scroll_top`, and `visible_content_height` in `crates/editor/src/render/model/viewport.rs:155`, `crates/editor/src/render/model/location.rs:127`, and `crates/editor/src/render/model/mod.rs:2111`.
- When the measured prefix height changes, `RenderState::apply_element_update` at `crates/editor/src/render/model/mod.rs:2551` always scrolls to the top when a header first appears so the new header is visible. For subsequent prefix changes, it preserves the header-relative position while the viewport is still inside the header, and preserves content position once content scrolling has started.
The existing AI document render tree remains otherwise unchanged: `AIDocumentView::render` now renders only the editor container in the pane body, while the config block is injected through the editor header path.
## Testing and validation
The integration test coverage is the main validation because the behavior depends on layout, scroll wheel events, and render-state offsets:
- `app/src/integration_testing/ai_document.rs:40` adds helpers to create an AI document, attach an approved orchestration config, send precise scroll events, and assert header/content offsets.
- `crates/integration/src/test/ai_document.rs:29` verifies the full sequence: no header before config, header starts at top, header partially hides before content scrolls, content scrolls after the header is hidden, content scrolls back before the header reappears, and both return to top.
- `crates/integration/src/test.rs:6`, `crates/integration/src/bin/integration.rs:377`, and `crates/integration/tests/integration/ui_tests.rs:238` register the test in the integration harness.
Implementation validation should run `./script/format`, `cargo check -p warp_editor`, `cargo check -p warp --features integration_tests`, `cargo check -p integration`, `cargo test -p warp_editor --lib --no-run`, and `git --no-pager diff --check`. Full `nextest`/presubmit is not required for this focused branch unless preparing the PR.
## Parallelization
No sub-agents are proposed. The change is localized to a shared editor wrapper, one AI document call site, and one integration test, so parallel implementation would add coordination cost around the same scroll state rather than reduce wall-clock time.
