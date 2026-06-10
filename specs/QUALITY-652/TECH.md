# Orchestration Config Card Scrolls With Plan Content
Linear: [QUALITY-652](https://linear.app/warpdotdev/issue/QUALITY-652/orchestration-config-header-should-scroll-away-with-content)
Implementation branch: `harry/quality-652-orchestration-config-view-zone`, stacked on `aloke/code_review_inline`.

## Context
The orchestration config card (`OrchestrationConfigBlockView`) is plan-level UI attached to an AI document. It must scroll away with plan content, but it is not user-authored Markdown: it must never serialize, copy, select, or delete as plan text.

This branch builds on the per-view "view zone" primitive introduced for inline code-review comments: a region of app-supplied UI hosted on a per-view `RenderState` that reserves vertical space at an anchor line while contributing zero buffer characters or lines.

Relevant code:
- `crates/editor/src/render/model/mod.rs` — `ViewZone`, `BlockItem::ViewZone`, `RenderState::set_view_zones`, and the reconcile logic (`reset_view_zones`) that supports anchoring a zone before the first content line (`LineCount::zero()`).
- `app/src/editor/view_zone.rs` — `RenderableViewZone`, the generic hosting renderable: lays out the zone's child element against the zone's content width, writes the measured size back to the owner, and paints/routes events to the child. Configurable x-origin (viewport-pinned for comments, content-space for the card).
- `app/src/ai/document/orchestration_config_block.rs` — the card view. Renders normally (header + toggle, description, details, pickers, validation, modal overlay) and owns all orchestration state.
- `app/src/ai/document/orchestration_config_zone.rs` — `LaidOutOrchestrationConfig`, the zone item hosting the card by window + entity id, and `orchestration_config_zone_size`, the desired reserved size.
- `app/src/ai/ai_document_view.rs` — zone reconciliation (`sync_orchestration_view_zone`) and the height convergence loop.
- `app/src/notebooks/editor/model.rs` — `NotebooksEditorModel::set_view_zones` passthrough to the render state.

## Design
The card is hosted as a view zone anchored at `RenderLineLocation::Current(LineCount::zero())`, which the reconcile logic places before the first content line of the plan document. Because the zone lives in the render tree:
- It reserves real vertical space, so the plan body starts below it and the card scrolls away with content.
- It contributes zero buffer characters, so Markdown serialization, rich/plain copy, selection, deletion, and undo are unaffected by construction — the card is simply not document content.

`AIDocumentView` owns reconciliation. `sync_orchestration_view_zone` sets the editor's complete zone set: one zone when the conversation has an `OrchestrationConfigSnapshot` for this plan, empty otherwise. It runs on construction, editor-model replacement (`set_editor_model`), orchestration config snapshot updates (the `BlocklistAIHistoryModel` subscription), and card layout changes.

## Rendering and event routing
`OrchestrationConfigBlockView` is a normal view: `render()` is the single source of truth for the card's UI, and its typed actions dispatch through the hosted `ChildView`'s responder chain like any other view. `LaidOutOrchestrationConfig::element` resolves the view by window + entity id and returns a `RenderableViewZone` hosting `ChildView::new(&view)` in content-space.

The card root consumes background clicks: view zones hit-test to a clamped text location (`crates/editor/src/render/model/location.rs`), so an unconsumed click on the card would otherwise move the editor cursor to plan offset 0.

## Height convergence
A zone's reserved height must be known before paint (viewporting reads heights from the render tree), but the card's height depends on its state (approval, details expansion, validation messages) and on text wrapping at the editor's current width. Rather than hand-measuring the UI, the reserved height converges through measurement of the real rendered element:
1. Reconcile reserves the card's last measured height (`OrchestrationConfigBlockView::laid_out_size`), with a constant fallback before first measure, at the editor's current content width (viewport width minus zone margins).
2. During the editor's layout pass, `RenderableViewZone::layout` lays out the card's element against the zone's content width and writes the measured size back via `set_laid_out_size`.
3. After element layout, the render state emits `RenderEvent::ViewportUpdated`; `AIDocumentView` re-reconciles only when the desired size differs from the size last reserved (`maybe_resync_orchestration_view_zone`).

The card is always anchored at line zero, so whenever it is relevant it is in-viewport and measured every pass; the loop converges one frame after any state or width change. Card state changes additionally emit `OrchestrationConfigBlockEvent::LayoutChanged` so reconciliation is not gated solely on layout traffic.

## Testing and validation
- `crates/editor/src/render/model/mod_tests.rs` — `view_zone_at_line_zero_reserves_space_before_first_line`: a line-zero zone is the first item in the content tree, reserves height without contributing characters or lines, reports correct zone positions, and is removed by reconciling with an empty set.
- Manual verification: card scrolls away with plan content; expanding/collapsing details and switching Local/Cloud resizes the reserved space; window resize re-wraps and re-reserves; clicking the card background does not move the editor cursor; pickers and the create-environment modal work; plan markdown save/copy contains no card content.
- Validation commands: `cargo check -p warp_editor`, `cargo check -p warp`, `./script/format`, the clippy invocation from `./script/presubmit`, and `cargo nextest run -p warp_editor -E 'test(view_zone)'`.
