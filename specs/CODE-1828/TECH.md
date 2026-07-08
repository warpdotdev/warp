# Token / cost transparency in the TUI — TECH

Parent: [CODE-1828](https://linear.app/warpdotdev/issue/CODE-1828/token-cost-transparency). Sub-issues: [CODE-1831](https://linear.app/warpdotdev/issue/CODE-1831/tui-footer-token-usage-entry-with-click-to-toggle-cost) (footer entry) and [CODE-1832](https://linear.app/warpdotdev/issue/CODE-1832/tui-token-usage-next-to-the-loading-indicator-end-of-response-summary) (loading indicator + end-of-response summary row, blocked on PR [#13442](https://github.com/warpdotdev/warp/pull/13442)).

## Context

Per the [Figma mocks](https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=323-17499&t=ZINACTiPr1Rk74Dn-0) (frames `323:17499` tokens, `323:17553` hover, `323:17607` cost):

- The footer's right side shows a token entry between the branch and diff stats: `… ↬ main • 4 tok • +31 -12`. Clicking it toggles tokens ⇄ dollar cost (`$0.03`); no third state exists in the mocks.
- A completed agent response ends with a dim (`bright.black`) summary row: `∷ 1s • 4 tokens`. While streaming, the token count accompanies the `⋮ Warping (Ns)` indicator added by PR #13442 (unmerged; also adds the TUI animation machinery).

Current state (verified live on master and in source):

- `crates/warp_tui/src/terminal_session_view.rs:317-364` — `render_footer` shows only the ctrl-c hint (left) and model name + cwd (right). No mouse handling, no tokens/cost.
- `crates/warp_tui/src/agent_block.rs`, `agent_block_sections.rs` — agent block sections are Input / PlainText / ToolCall / Thinking only; no per-exchange usage or duration row.
- No token-count or dollar formatting helper exists anywhere in the TUI or app crate (`format_credits` in `app/src/ai/blocklist/view_util.rs:145` formats credits, not tokens/dollars).

Data plumbing that already exists app-side (unused by the TUI):

- `app/src/ai/agent/conversation.rs` — `update_cost_and_usage_for_request` (line 1960) accumulates per-model `TokenUsage { total_input, output, input_cache_read, input_cache_write, cost_in_cents }` into `total_token_usage_by_model`; accessors `total_token_usage()` / `total_request_cost()` (lines 3489-3496) are `#[allow(dead_code)]` today. Dollar cost comes from `cost_in_cents` (`RequestCost` is credits, not dollars).
- `BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { conversation_id }` (`app/src/ai/blocklist/history_model.rs:1866`) fires on every usage update; the GUI usage footer subscribes to exactly this (`ConversationUsageView::new_footer_with_rollup`, `app/src/ai/blocklist/usage/conversation_usage_view.rs:156`).
- Per-exchange precedent for stream-derived metadata: `set_exchange_time_to_first_token` (`history_model.rs:1935`).
- The TUI consumes app types only through `app/src/tui_export.rs`.

## Proposed changes

### 1. App-side: conversation usage totals + exports (`app/`)

- New `ConversationUsageTotals { total_tokens: u64, cost_in_cents: f64 }` plus `AIConversation::usage_totals()` summing tokens and `cost_in_cents` across `total_token_usage_by_model`. The token count is `(total_input - input_cache_read) + output`: the server's `total_input` includes cached reads (see `ExactTokenUsage` construction in warp-server), which re-count the entire discounted context every request — ballooning the number (e.g. `100k tok` next to `$0.05`) while barely moving the cost.
- `update_conversation_cost_and_usage_for_request` now also emits `ConversationUsageMetadataUpdated` for token-only updates (previously only for request-cost/metadata updates).
- Export `ConversationUsageTotals` through `tui_export.rs` — `BlocklistAIHistoryEvent` is already exported.
- Per-exchange capture (`token_usage`/`cost_in_cents` on `AIAgentExchange`, mirroring `time_to_first_token_ms`) is **deferred to PR 2** with its consumer: it touches ~14 `AIAgentExchange` construction sites and its request→exchange attribution semantics are best decided alongside the summary row.

### 2. Shared TUI component (`crates/warp_tui/src/usage.rs`, new)

The reusable piece both sub-issues consume:

- `format_token_count(tokens, Style)` → `4 tok` (footer, short) / `4 tokens` (transcript, long), with `k`/`M` abbreviation above 4 digits.
- `format_cost(cost_in_cents)` → `$0.03` (two decimals).
- `TokenCostToggle` — shared tokens⇄cost state owned by `TuiTerminalSessionView`, plus a render helper that wraps the footer entry in the hover/click element (`TuiHoverable`/`TuiEventHandler` from `crates/warpui_core/src/elements/tui/`). The `MouseStateHandle` must be owned by the view, not created inline during render.
- Styles come from `TuiUiBuilder` (`dim_text_style`/`muted_text_style`), matching the mock's `#8e8e8e`.
- The entry shows a pointing-hand mouse pointer while hovered. This is new TUI-core plumbing mirroring the GUI `Hoverable`'s `with_cursor`: `TuiPointerShape` + `TuiHoverable::with_pointer_shape` request a shape into `TuiEventContext` during mouse-move dispatch (first request wins, so the innermost hovered element prevails), and `TuiScreen` syncs the host terminal via the OSC 22 pointer-shape sequence — emitted only on changes, reset to `default` when nothing requests a shape and on terminal restore (`CrosstermModeControl::leave`). Terminals without OSC 22 support (it's honored by kitty/WezTerm/foot/xterm-class emulators) ignore the sequence.

### 3. Footer entry (CODE-1831, `terminal_session_view.rs`)

- In `new`: subscribe to `BlocklistAIHistoryModel`; on `ConversationUsageMetadataUpdated` for this surface's selected conversation (`conversation_selection.selected_conversation_id`), `ctx.notify()`. Add the new event arm explicitly — no wildcard matches (repo convention).
- In `render_footer`: after the cwd, render `• ` + the toggle component using the selected conversation's totals; hide the entry until the first usage event (mock shows it only with data). Click flips `TokenCostToggle` and notifies. Branch (`↬ main`) and `+31 -12` diff stats remain out of scope.

### 4. Transcript row + streaming counter (CODE-1832, after PR #13442 lands)

- App-side per-exchange capture moves here (deferred from PR 1): add `token_usage: Option<u64>` / `cost_in_cents: Option<f64>` to `AIAgentExchange`, populated from each request's `stream_finished` usage (attribution decided with the row's semantics).
- New `TuiAIBlockSection::UsageSummary { duration, tokens }` in `agent_block.rs`, extracted when the exchange output is `Finished` and `token_usage` is present; rendered by a new function in `agent_block_sections.rs` as `∷ {duration} • {N} tokens` using the long-form formatter (static text, not clickable, per mocks). Duration from exchange start→finished timestamps (`format_elapsed_seconds` is already exported).
- Streaming: append `• {N} tokens` to the Warping indicator row using the same formatter, updating as usage events arrive mid-stream. Integration point is the indicator element #13442 adds between transcript and input; defer wiring details until it merges.

## Testing and validation

- Unit tests in sibling `_tests.rs` files per repo convention (`usage_tests.rs`, extend `terminal_session_view` and `agent_block_tests.rs`): formatter edge cases (0/singular/abbreviation/rounding), footer hidden before first usage event, footer entry renders token form then cost form after a click event dispatch, summary section extracted only for finished exchanges with usage, dim styling sourced from theme.
- Pointer-shape tests in `warpui_core`: `hoverable_tests.rs` (hovered moves request the configured shape, first request wins) and `runtime/mod_tests.rs` (mouse moves over a hand-pointer hoverable emit OSC 22 once per change and reset on leave, against the in-memory terminal).
- App-side tests in `conversation_tests.rs`: per-exchange `token_usage`/cost captured from `stream_finished`, totals accumulate across requests.
- Commands: `cargo nextest run -p warp_tui`, `cargo nextest run -p warp` (touched test files), `cargo clippy -p warp_tui --all-targets -- -D warnings`, `./script/format` — all must pass before each PR (presubmit requirement).
- Manual: `./script/run-tui`; send a prompt; verify footer count appears and updates, click toggles `4 tok` ⇄ `$0.03` and back, summary row appears after completion; compare against Figma frames `323:17499`/`323:17607`.

## Parallelization

Parallel child agents are not proposed: CODE-1832 is hard-blocked on PR #13442, both surfaces share the new `usage.rs` component and the app-side capture, and the touched files overlap heavily (`terminal_session_view.rs`, `tui_export.rs`). Sequential, stacked delivery is cleaner:

1. PR 1 (now): app-side conversation totals + exports + `usage.rs` + footer entry (CODE-1831), branch `ian/code-1831-tui-footer-token-usage-entry-with-click-to-toggle-cost`.
2. PR 2 (after #13442 merges): per-exchange usage capture + transcript summary row + streaming counter (CODE-1832), branch `ian/code-1832-tui-token-usage-next-to-the-loading-indicator-end-of`, stacked on PR 1 with graphite.

## Risks and mitigations

- Small responses show tiny counts (`4 tok`) while real conversations reach thousands — abbreviation is part of the formatter from day one so the footer width stays stable.
- The hand pointer requires host-terminal OSC 22 support. Warp's own terminal doesn't parse OSC 22 today, so the pointer shows in kitty/WezTerm/foot/xterm but not in Warp until terminal-side support lands (tracked separately; the TUI-side emission is unit-tested).
- Restored/persisted conversations predate per-exchange capture, so old exchanges have no summary row — acceptable; render nothing when `token_usage` is `None`. Persisting per-exchange usage is a follow-up if product wants history parity.
- Footer click is the TUI's first mouse-interactive footer element; keep the hit target to the entry's cells only so text selection elsewhere in the footer is unaffected.
