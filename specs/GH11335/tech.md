# Tech Spec: Find in collapsed reasoning traces

**Issue:** [warpdotdev/warp#11335](https://github.com/warpdotdev/warp/issues/11335)
**Product spec:** [product.md](product.md)
**Research revision:** [`a7a8f1ec792d9eed1702cac2dc21b8fdcf589979`](https://github.com/warpdotdev/warp/commit/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979)

## Context

The issue's original root-cause analysis predates merged PR [#11205](https://github.com/warpdotdev/warp/pull/11205). That PR already gave async Find a unified terminal/AI focused-match resolution and connected it to rich-content highlighting, block-level scrolling, and reasoning auto-expansion. The implementation should preserve that foundation rather than reintroduce a second focus path.

At the researched revision:

- [`app/src/terminal/find/model/async_find.rs:270-318`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/find/model/async_find.rs#L270-L318) stores AI matches by rich-content view and represents the current AI focus with `AsyncFocusedAiMatch`.
- [`app/src/terminal/find/model/async_find.rs:608-740`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/find/model/async_find.rs#L608-L740) resolves the global focused index to either a terminal-grid match or an AI match in visual traversal order.
- [`app/src/terminal/find/model.rs:296-363`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/find/model.rs#L296-L363) exposes path-agnostic focused block-list and rich-content match accessors for both sync and async Find.
- [`app/src/ai/blocklist/block/find.rs:13-47`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/ai/blocklist/block/find.rs#L13-L47) maps every rich-content match ID to a `TextLocation`, character range, and optional `MessageId`; it has no per-message count API for a collapsed header.
- [`app/src/ai/blocklist/block/find.rs:82-140`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/ai/blocklist/block/find.rs#L82-L140) records the owning message ID while scanning rendered output, which is the grouping key needed for a reasoning-trace indicator.
- [`app/src/ai/blocklist/block.rs:1224-1230`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/ai/blocklist/block.rs#L1224-L1230) intentionally treats `RanFind` as a repaint and reserves expansion for `UpdatedFocusedMatch`. This already matches the product distinction between typing and explicit navigation.
- [`app/src/ai/blocklist/block.rs:4349-4381`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/ai/blocklist/block.rs#L4349-L4381) resolves the focused match back through `FindState` and expands its `collapsible_block_states` entry.
- [`app/src/ai/blocklist/block/view_impl/output.rs:3796-3853`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/ai/blocklist/block/view_impl/output.rs#L3796-L3853) renders the collapsible reasoning header but has no Find-result indicator.
- [`app/src/ai/blocklist/block/view_impl/output.rs:3890-3987`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/ai/blocklist/block/view_impl/output.rs#L3890-L3987) renders reasoning and summarization through the same collapsible-text helper, so the new indicator must be explicitly limited to reasoning messages.
- [`app/src/terminal/view.rs:19686-19713`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/view.rs#L19686-L19713) scrolls after both query runs and explicit match navigation. [`app/src/terminal/block_list_viewport.rs:1111-1118`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/block_list_viewport.rs#L1111-L1118) documents that rich-content scrolling stops at the containing AI block and cannot position the exact match inside its nested scrollable.
- [`app/src/terminal/find/model/async_find.rs:757-800`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/find/model/async_find.rs#L757-L800) resets controller results when a query starts, while [`app/src/terminal/find/model/async_find.rs:1258-1271`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/find/model/async_find.rs#L1258-L1271) rejects messages from stale async generations. The AI view's own `FindState` is only cleared when that view is scanned, so a previous query's header count could otherwise survive briefly.

The remaining scope is therefore the per-trace indicator, immediate rich-content state invalidation on query changes, exact nested scrolling, and regression coverage for the behavior already supplied by #11205.

## Proposed changes

### Track current-query counts by message

Extend `FindState` in `app/src/ai/blocklist/block/find.rs` with a derived `HashMap<MessageId, usize>` and a read-only `match_count_for_message(&MessageId) -> usize` accessor. Increment the count when `run_find` records an output match with a `MessageId`, and clear the count map together with the match-location map. Keeping the derived index avoids scanning every match once per collapsed header on every render.

The match ID remains the identity used by the existing focus plumbing. The `MessageId` index is display metadata only; do not add message IDs to `AsyncFindController` or create a second traversal order.

### Clear stale rich-content state at query boundaries

Add a private async-controller helper that calls `clear_matches` on every registered rich-content view without clearing the new query's configuration. Invoke it when a valid full scan or refinement scan starts, before work is enqueued. The no-query path continues to use `clear_results`, which already clears registered rich-content views.

This makes indicators and highlights disappear immediately when the query changes. The existing generation check remains the authority that prevents a cancelled scan from repopulating state for an older query.

### Render a reasoning-only collapsed indicator

Pass the current message's count from `FindContext.state` into the collapsible header in `app/src/ai/blocklist/block/view_impl/output.rs`. Distinguish reasoning from summarization at the two `render_collapsible_block` call sites with a small exhaustive kind enum (or an equivalently explicit parameter) so conversation summaries do not inherit reasoning-specific UI accidentally.

When the kind is reasoning, the state is collapsed, and the count is greater than zero, append a compact textual badge to the existing clickable header. Use singular/plural copy and existing theme tokens; the visible text must include the count so color is not the only signal. Expanded reasoning continues to expose the existing inline highlights and does not need the collapsed badge.

### Reveal the exact focused location inside the trace

Keep `TerminalFindModel::focused_rich_content_match_id` and `AIBlock::handle_find_match_focus_change` as the single focus-to-message route introduced by #11205. Extend the latter to use the already resolved `FindMatchLocation.text_location` as an inner-scroll target after expanding the owning `CollapsibleElementState`.

Define a stable position ID from `(MessageId, TextLocation)` and expose matching anchors from every reasoning-section renderer. For formatted text, use `FormattedTextElement::with_saved_glyph_position` with the focused character offset so the target resolves inside the combined formatted-text element instead of assuming parsed lines are separate elements. Plain-text and code sections can anchor at the matching glyph or containing section, and table matches can anchor at the matching rendered row. Image and Mermaid matches currently search non-visible markdown source; for those variants, anchor the rendered image or diagram section as the explicit fallback required by Product Behavior 8. Keep the match over source text in the count and traversal order, but do not synthesize a nonexistent glyph highlight.

Queue `ClippedScrollStateHandle::scroll_to_position` with `ScrollToPositionMode::FullyIntoView` for the focused anchor. The outer `TerminalView::scroll_to_match` continues to bring the AI block into the conversation viewport, while this request moves the nested reasoning viewport. Only `UpdatedFocusedMatch` queues this request, so query typing and async result delivery do not expand or move a collapsed trace.

Do not auto-collapse a trace when focus leaves it or Find closes. Manual collapse continues through `CollapsibleElementState::toggle_expansion`; a later explicit navigation into that trace may expand it again.

## Testing and validation

- Add `FindState` unit tests in `app/src/ai/blocklist/block/find_tests.rs` covering per-message counts, multiple matches in one message, isolation between messages, and clearing/re-running with a new query (Behavior 1, 3-6, 12-13).
- Add focused transition tests in `app/src/ai/blocklist/block_tests.rs` (extracting a small state-transition helper if constructing a full `AIBlock` is impractical) proving that a `RanFind` repaint does not expand a trace, an explicit focused-match update expands only the owning message, and manual collapse persists until a later navigation targets that trace (Behavior 2, 7, 9-11).
- Preserve and extend the mixed terminal/AI focus tests in [`app/src/terminal/find/model/async_find_tests.rs:813-950`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/app/src/terminal/find/model/async_find_tests.rs#L813-L950) to assert forward, backward, and wraparound traversal continues to resolve the correct AI match after result replacement (Behavior 5-9).
- Add renderer-level tests alongside `view_impl` for the indicator's zero/one/many copy, collapsed-only visibility, reasoning-only scope, and non-color count label (Behavior 3-4, 13-14).
- Add a real-display GUI integration case under [`crates/integration/src/test/agent_mode.rs`](https://github.com/warpdotdev/warp/blob/a7a8f1ec792d9eed1702cac2dc21b8fdcf589979/crates/integration/src/test/agent_mode.rs) with a long collapsed reasoning fixture. Assert or capture that typing shows the indicator without expansion, Enter/next and previous expand the correct trace, and a match outside the nested viewport is scrolled into view with focused highlighting (Behavior 2, 7-10, 14).
- Manually test both input-mode sort directions with multiple collapsed traces and mixed terminal/AI results. Attach before/after screenshots for the collapsed indicator and a narrated recording showing query edits, forward/backward navigation, manual re-collapse, query clearing, and Find dismissal.
- Before review, run `./script/format`, the focused `cargo nextest` targets for the affected Warp tests, the new GUI integration case, and the repository clippy command required by `AGENTS.md`.

## Risks and mitigations

- **Nested scroll timing:** the anchor is created during the render triggered by expansion. Use WarpUI's position-target mechanism, which can resolve the queued target during layout, rather than reading geometry synchronously in the focus event.
- **Volatile async match IDs:** match IDs are regenerated on rescans. Consume the focused ID and location only inside the focus-change handler; persist neither across `RanFind` events nor async generations.
- **Scope leakage:** reasoning and summarization share a renderer. An exhaustive block-kind distinction and renderer tests prevent the badge from appearing on unrelated collapsible content.
- **User-scroll interference:** queue inner scrolling only for explicit focus changes, not for ordinary repaints, streaming updates, or query edits.

## Parallelization

Do not split the implementation across parallel coding agents. Count lifecycle, header rendering, focus expansion, and nested scrolling all share `FindState`, `AIBlock`, and the collapsible renderer, so parallel edits would collide and make the state contract harder to verify. Use one local agent in a dedicated `../warp-gh11335` worktree on a `<handle>/gh11335-collapsed-find` branch and land the implementation plus tests in one PR. Independent test commands may run concurrently after the implementation is complete.
