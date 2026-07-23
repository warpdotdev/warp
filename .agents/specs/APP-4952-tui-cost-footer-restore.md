---
ticket: APP-4952
repository: warpdotdev/warp
estimate: M
surface: crates/warp_tui plus client restore/hydration
---
# Restore the TUI cost-footer baseline (APP-4952)
## Product
*Summary:* When a user restores a non-empty Agent Mode conversation in the headless TUI, the footer's credits⇄cost entry must not reset the provider-dollar cost to zero. A known historical provider cost is the starting baseline, and each later response adds only its newly reported provider cost. When a legacy or cloud payload does not contain historical provider cost, cost mode must show an explicit unavailable-baseline indicator rather than render a misleading `$0.00`.

*Key design choices:* Persist an optional cumulative provider-cost baseline in the existing client conversation usage metadata; keep the restored baseline separate from the current-session delta so a response is never counted twice; make missing historical cost explicit with an unavailable-baseline indicator.

### Behavior
1. A new conversation with a completed response keeps its current behavior: once the response reports `TokenUsage.cost_in_cents`, cost mode renders that accumulated cost in dollars and credits mode renders cumulative inference plus platform credits.
2. Restoring a conversation whose local persisted usage metadata contains a provider-cost baseline renders that baseline before any new prompt (for example, a persisted 3.2 cents baseline renders `$0.03`, not `$0.00`).
3. After restoration with a known baseline, a follow-up response adds exactly that response's `TokenUsage.cost_in_cents` to the baseline. A 3.2-cent restored baseline followed by a 1.2-cent response therefore renders 4.4 cents (`$0.04`, subject to the existing two-decimal formatter), never only the 1.2-cent increment.
4. Credits mode remains cumulative across restore and follow-up, using the existing inference-plus-platform credit semantics. The fix must not replace or double-count the credits metadata.
5. Both local-database hydration and server-token/local conversation hydration pass the optional baseline through the same `AIConversation::new_restored` / `new_restored_synthesizing_on_empty` semantics. A restore payload that omits the optional field is treated as “historical provider cost unknown,” not as a known zero.
6. For a legacy local row or a fresh cloud transcript that has credits/token metadata but no historical provider cost, cost mode renders an explicit unavailable-baseline indicator (for example, `Cost unavailable`) instead of displaying `$0.00` or an incremental-only dollar total. The indicator is deterministic and applies until a known baseline is available; credits mode is unchanged.
7. The existing usage-update event continues to invalidate the selected TUI session, so the footer reflects the restored baseline and the follow-up delta without requiring a conversation switch or a second prompt.

## Technical
### Current state
- `app/src/ai/agent/conversation.rs:180-194 @ e24f75b2154ffeccc18f988237f45bad42ab613e` defines `ConversationUsageTotals`; its cost field is currently a plain `f32`.
- `app/src/ai/agent/conversation.rs:380-648 @ e24f75b2154ffeccc18f988237f45bad42ab613e` implements new plus strict/lenient restore and initializes `total_request_cost` plus `total_token_usage_by_model` to zero/empty even when `conversation_usage_metadata` is hydrated.
- `app/src/ai/agent/conversation.rs:569-646 @ e24f75b2154ffeccc18f988237f45bad42ab613e` copies persisted usage metadata during restore but does not hydrate provider cost into the in-memory accumulator.
- `app/src/ai/agent/conversation.rs:2134-2206 @ e24f75b2154ffeccc18f988237f45bad42ab613e` adds each live response's `TokenUsage.cost_in_cents` only to the in-memory per-model map.
- `app/src/ai/agent/conversation.rs:3685-3696 @ e24f75b2154ffeccc18f988237f45bad42ab613e` projects credits from cumulative metadata but projects provider cost only by summing that in-memory map.
- `crates/persistence/src/model.rs:1167-1255, 1607-1627 @ e24f75b2154ffeccc18f988237f45bad42ab613e` defines `AgentConversationData` and `ConversationUsageMetadata`; the serialized usage shape has credits, token, tool, and context metadata but no provider-cost field.
- `app/src/ai/agent/api/convert_conversation.rs:65-112 @ e24f75b2154ffeccc18f988237f45bad42ab613e` hydrates cloud conversation metadata into the same persisted usage metadata passed to the restore constructors.
- `crates/graphql/src/api/queries/get_conversation_usage.rs:115-125 @ e24f75b2154ffeccc18f988237f45bad42ab613e` and `crates/warp_graphql_schema/api/schema.graphql:836-880 @ e24f75b2154ffeccc18f988237f45bad42ab613e` expose credits/token usage but no provider-dollar cost. This is why the client must distinguish “unknown baseline” from zero for legacy/fresh cloud payloads.
- `crates/warp_tui/src/terminal_session_view.rs:1420-1436, 2511-2523 @ e24f75b2154ffeccc18f988237f45bad42ab613e` subscribes the footer to usage updates and projects selected-conversation totals.
- `crates/warp_tui/src/usage.rs:35-76 @ e24f75b2154ffeccc18f988237f45bad42ab613e` renders credits or cost and currently formats any zero cost as `$0.00`.
- `app/src/ai/blocklist/history_model.rs:1884-1916 @ e24f75b2154ffeccc18f988237f45bad42ab613e` emits the usage-update event after applying response usage, which the TUI already listens to.

### Design alternatives
- *Seed the existing `total_token_usage_by_model` map with a synthetic restored row.* This keeps the public totals shape unchanged, but introduces a fake model identity, contaminates token reporting/replay, and makes it easy to double-count the first live response. Do not use it.
- *Recompute historical provider cost from restored token counts and client model pricing.* The persisted/server payload has token counts but no authoritative historical pricing, and pricing can vary by provider, model, cache category, or billing policy. This would produce guessed totals and is not acceptable for a cost display.
- *Extend the server/GraphQL usage contract immediately.* That would make fresh cloud restores exact, but expands APP-4952 into a coordinated warp-server/proto/schema rollout. The current ticket is scoped to the warp client and the client already has a durable local conversation record. Keep the client field backward-compatible and use the explicit unavailable-baseline indicator when cloud/legacy data omits provider cost; a future server field can populate the same optional field without changing the TUI projection.
- *Chosen: add an optional client-persisted cumulative provider-cost field and a dedicated restore-aware accumulator.* A new conversation starts with a known zero baseline; a restored conversation with the field gets that baseline; a restored conversation without it remains unknown. `ConversationUsageTotals` should expose whether cost is known (prefer `Option<f32>` for `cost_in_cents`), and `UsageToggle` should render an unavailable-baseline indicator when cost is unknown. This preserves existing local data, avoids false dollars, and keeps all restore constructors on one path.

### Proposed changes
1. Add `provider_cost_in_cents: Option<f32>` to `persistence::model::ConversationUsageMetadata` with serde default/skip behavior so old local JSON and current cloud GraphQL payloads continue to deserialize. The field means cumulative provider cost for the entire conversation, not the last request.
2. Add restore-aware state to `AIConversation` (a known baseline plus current-session provider-cost delta, or an equivalent representation) and initialize it from the optional persisted field in both restore constructors. New conversations must initialize a known zero baseline; restored payloads without the field must preserve “unknown,” not coerce to zero.
3. In `update_cost_and_usage_for_request`, continue aggregating per-model token counts for existing consumers, but add each response's `TokenUsage.cost_in_cents` exactly once to the current-session delta. Update the persisted optional cumulative field only when the baseline is known; never overwrite a known baseline with an absent incoming metadata field.
4. Change `usage_totals` to return `Some(baseline + delta)` only when provider cost is known. Keep `credits_spent` sourced from `conversation_usage_metadata.credits_spent + platform_credits_spent`.
5. Update `write_updated_conversation_state`/the persisted `AgentConversationData` path and any fork/restore helpers that copy usage metadata so the cumulative field survives local restart and server-token continuation. Preserve `None` for legacy/fresh cloud payloads.
6. Update `crates/warp_tui/src/usage.rs` so cost mode formats a known dollar value exactly as today and renders a stable unavailable-baseline indicator when cost is `None`. Keep the click toggle, setting persistence, hover state, separators, and shell-mode suppression unchanged.
7. Update the conversation projection, persistence, conversion, and TUI render tests named below. Do not change server APIs in this PR; document the optional field as a client-compatible extension point for a future server payload.

### Open questions resolved
- The requester confirmed the GUI behavior is out of scope and the fix should remain in the warp client/TUI path.
- The ticket's confirmed code-path probe observed original cost 3.2 cents → restored cost 0.0 → follow-up cost 1.2 cents. The probe could not execute in this sandbox because Cargo's `warp` test build was SIGKILLed after dependencies compiled; implementation must rerun it in a resource-capable environment.
- Historical provider cost is not present in the current server GraphQL usage metadata. The spec therefore treats missing cost as an explicit unknown state and renders an unavailable-baseline indicator rather than fabricating a dollar value. Exact historical dollars for fresh cloud-only transcripts require a future server contract change and are out of scope for APP-4952.
- “Server-token restoration” means a conversation restored through the client/local persisted record that carries a server token; the same optional field and restore constructors must be used. A cloud payload that carries no provider-cost field follows the unknown/unavailable-indicator rule above.
- Cost values remain US cents represented as `f32`, matching `TokenUsage.cost_in_cents` and the existing formatter; the optional wrapper is for availability, not a unit change.

### Risks / blast radius
Changing the persisted metadata shape touches local JSON round-tripping, conversation forks, cloud-to-client conversion, and TUI formatting. Mitigate with serde defaults, a restore test for `Some` and `None`, a no-double-count follow-up test, and render-to-lines assertions for known cost, unavailable cost, credits mode, shell mode, and new conversations. Do not derive cost from credits or token counts.

### Validation & verification criteria
All criteria below must pass before merge:
1. Reproduce the ticket's confirmed sequence in a focused test: create a conversation with a persisted provider-cost baseline of 3.2 cents and cumulative credits, restore it through `AIConversation::new_restored` (and the empty-task constructor where applicable), assert the cost projection is 3.2 cents before a new prompt, apply one follow-up `TokenUsage` with `cost_in_cents = 1.2`, and assert the projection is 4.4 cents. The pre-fix implementation must fail this test by producing 0.0 then 1.2.
2. Add/maintain a named regression test in `app/src/ai/agent/conversation_tests.rs` (for example `restored_usage_totals_preserve_provider_cost_baseline_and_add_follow_up`) that covers both a known baseline and the no-baseline legacy case; it must fail before the change and pass after.
3. Verify `ConversationUsageMetadata` and `AgentConversationData` serde compatibility in `crates/persistence/src/model_tests.rs`: a payload with `provider_cost_in_cents` round-trips exactly, and a legacy payload that omits it deserializes to `None` without changing credits/token fields.
4. Verify the TUI projection in `crates/warp_tui/src/usage_tests.rs` and `crates/warp_tui/src/terminal_session_view_tests.rs`: known cost mode renders the expected dollars, unknown cost mode renders the stable unavailable-baseline indicator (never `$0.00`), credits mode remains unchanged, and shell mode still omits usage.
5. Verify restore/update redraw behavior by exercising `ConversationUsageMetadataUpdated` for the selected conversation and asserting the rendered footer changes from the known baseline to baseline-plus-delta without switching conversations. Use the existing render-to-lines helpers; no GUI integration test is required.
6. Verify no collateral regression for new conversations: the existing `usage_totals_reads_gui_credits_and_accumulates_provider_cost` test (updated for optional cost) continues to pass, including cumulative credits and two live request deltas.
7. Run the focused tests with a writable Cargo home in a resource-capable environment: `CARGO_HOME=/tmp/cargo-home cargo nextest run -p warp restored_usage_totals --no-fail-fast` (or the repository's equivalent filter), `CARGO_HOME=/tmp/cargo-home cargo nextest run -p warp_tui --no-fail-fast`, and the relevant persistence tests.
8. Run the repository checks required by `warp/AGENTS.md`: `./script/format`; `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`; `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`; and `cargo test --doc`. If the sandbox resource limit still SIGKILLs Cargo, record the exact failure and leave the PR unverified rather than claiming success.
9. Verify the user-visible headless TUI path in a resource-capable terminal using the repository's TUI verification/render-to-lines workflow (`./script/run-tui` or the corresponding `tui-verify-change` procedure): restore a conversation with a known baseline, select cost mode, observe the baseline before a prompt, submit one prompt, and observe baseline-plus-increment. Also exercise a legacy/cloud payload without a provider-cost field and confirm the unavailable-baseline indicator; no `$0.00` placeholder is permitted.
