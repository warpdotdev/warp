# Tech spec: roll up orchestration credit usage in the agent-mode footer

Linear: https://linear.app/warpdotdev/issue/QUALITY-671
Companion: `specs/QUALITY-671/PRODUCT.md`

## Context

The agent-mode footer this feature extends is rendered in two layers.

- Collapsed footer (credits + chevron):
  - `app/src/ai/blocklist/block/view_impl/output.rs:3243` — `render_usage_button` builds the inline footer pill. It reads `conversation.credits_spent()`, `conversation.credits_spent_for_last_block()`, `conversation.token_usage()`, and `conversation.tool_usage_metadata().total_tool_calls()`. If all are empty, the button is suppressed (`output.rs:3252-3258`). Per PRODUCT invariant 11, the pill's headline credit number is replaced with the rollup total when one applies; the existing `(+N)` last-response delta keeps using the orchestrator's own `credits_spent_for_last_block`.
- Expanded footer (full usage summary):
  - `app/src/ai/blocklist/usage/conversation_usage_view.rs` — `ConversationUsageView` owns the expanded layout.
  - `ConversationUsageInfo` (`conversation_usage_view.rs:28-40`) carries `credits_spent`, `credits_spent_for_last_block`, `tool_calls`, `models: Vec<ModelTokenUsage>`, `context_window_usage`, `files_changed`, `lines_added`, `lines_removed`, `commands_executed`.
  - `render_unified_layout` (`conversation_usage_view.rs:127`) emits the "USAGE SUMMARY", "TOOL CALL SUMMARY", and "LAST RESPONSE TIME" sections via `render_section_header`, `render_label_text`, `render_value_text` helpers.
  - The "Credits spent (total)" row is rendered at `conversation_usage_view.rs:155-159` (non-last-block path) and `:146-159` (last-block-aware path). The rollup work modifies this row.

Per-conversation usage data is populated on `AIConversation`:
- `AIConversation.conversation_usage_metadata` (`app/src/ai/agent/conversation.rs:160`). Populated by `StreamFinished` events for live conversations and by `get_conversation_usage` GraphQL (`crates/graphql/src/api/queries/get_conversation_usage.rs`) on init / hydration. Every locally-loaded child has its own populated metadata.

Orchestration topology lives in `BlocklistAIHistoryModel`:
- `child_conversation_ids_of(&parent_id)` (`history_model.rs:455`) — direct children from the `children_by_parent` index. The index is maintained by `start_new_child_conversation` / `set_parent_for_conversation` (`history_model.rs:397-450`).
- A transitive walker already exists for the pill bar: `descendant_conversation_ids_in_spawn_order` / `collect_descendant_conversation_ids_in_spawn_order` (`app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs:133-151`). Lift this to a shared module — do not duplicate.

Agent identity for the per-agent list comes from helpers the pill bar already uses:
- `pill_avatar_color` / `pill_initial` (`orchestration_pill_bar.rs:85-99`) for child avatars.
- `render_orchestrator_avatar_disc` / `render_agent_avatar_disc` (`orchestration_pill_bar.rs:100-131`) for the actual disc element.
- Display name: `AIConversation.agent_name` for children, "Orchestrator" (or the orchestrator's existing user-facing label, TBD during implementation by walking the code that titles the orchestrator pill).

## Proposed changes

### 1. Aggregation helper

Add a new module `app/src/ai/blocklist/usage/rollup.rs` exposing:

```rust path=null start=null
pub struct OrchestrationCreditRollup {
    /// Sum of credits across orchestrator + all locally-known descendants.
    pub total_credits: f32,
    /// Per-agent rows for the breakdown list, sorted by credits descending,
    /// ties broken by spawn order (earlier first). Excludes agents with
    /// zero credits.
    pub per_agent: Vec<PerAgentCreditEntry>,
}

pub struct PerAgentCreditEntry {
    pub conversation_id: AIConversationId,
    pub display_name: String,
    pub avatar: AgentAvatar, // enum: Orchestrator | Child { color, initial }
    pub credits_spent: f32,
}

pub fn compute_orchestration_rollup(
    parent_id: AIConversationId,
    history: &BlocklistAIHistoryModel,
) -> Option<OrchestrationCreditRollup>;
```

Implementation notes:
- Returns `None` if the orchestrator has no loaded descendants OR if every eligible agent (orchestrator + descendants) has zero credits. PRODUCT invariants 1, 7.
- Walks descendants via the existing helper extracted from `orchestration_pill_bar.rs` (move it to `app/src/ai/blocklist/orchestration_topology.rs` or similar shared module; re-export from `usage/rollup.rs`).
- Sums each contributor's `credits_spent` for `total_credits`.
- Builds `per_agent` from the orchestrator + each loaded descendant, filters out zero-credit rows, sorts by `credits_spent` descending with spawn-order tie-break.
- Unknown / unloaded descendants are silently skipped (PRODUCT invariant 10 — no footnote, no warning).
- Pure function — no I/O, no GraphQL — runs synchronously on the local model.

### 2. Render the "Credits spent (total)" row with toggle and list

Modifications in `conversation_usage_view.rs`:

- Extend `ConversationUsageView` with:
  - `rollup: Option<OrchestrationCreditRollup>` — passed in by the caller (None in `DisplayMode::Settings`).
  - `details_expanded: bool` — local UI state, defaults `false`.
  - `show_all_clicked: bool` — local UI state, defaults `false`.

- In `render_unified_layout`, when `rollup.is_some()` and `DisplayMode::Footer`:
  - The "Credits spent (total)" value uses `rollup.total_credits` (instead of `usage_info.credits_spent`).
  - Append a "View details" / "Hide details" toggle element to the value row. On click, flip `details_expanded` and `notify` the view to re-render.
  - When `details_expanded`, emit one row per `PerAgentCreditEntry`, rendered via a new helper `render_per_agent_row(entry, appearance)`:
    - Leading avatar disc (12–16 px) via `render_agent_avatar_disc` / `render_orchestrator_avatar_disc`.
    - Display name (label slot) + credits value (value slot), using the existing label/value helpers.
  - When `per_agent.len() > 5` and `!show_all_clicked`, render only the first 5 entries followed by a "Show N more" link row (N = `per_agent.len() - 5`). On click of the link, set `show_all_clicked = true` and re-render.

- In `DisplayMode::Settings` and the non-rollup `DisplayMode::Footer` paths, the existing "Credits spent (total)" rendering is preserved unchanged.

- "Credits spent (last response)" rendering is untouched (PRODUCT invariant 3).

### 3. Wire the rollup into the footer construction

The rollup is consumed in two places:

- **Expanded footer.** The caller that constructs `ConversationUsageView::new(...)` for `DisplayMode::Footer` lives near `render_usage_footer` in `output.rs` (locate via `ConversationUsageView::new` call sites; `render_usage_button` at `output.rs:3243` is adjacent). The caller has access to the `BlocklistAIHistoryModel` because the surrounding view already uses it.
   - The footer constructor always uses `ConversationUsageView::new_footer_with_rollup`, which holds the parent conversation id and calls `compute_orchestration_rollup` at render time.
   - `compute_orchestration_rollup` returns `None` whenever the orchestrator has no loaded descendants or no eligible credits, so the rollup-aware UI naturally collapses to today's exact behavior.
   - Settings-mode (`DisplayMode::Settings`) continues to use `ConversationUsageView::new`, which leaves `parent_conversation_id` unset; `rollup()` then short-circuits to `None`.
- **Collapsed pill** (PRODUCT invariant 11). In `render_usage_button` (`output.rs:3243`):
   - Call `compute_orchestration_rollup(conversation.id(), history)` unconditionally.
   - When the rollup is `Some(_)`, use `rollup.total_credits` instead of `conversation.credits_spent()` for the pill's headline number and for the "has any usage" suppression check (`output.rs:3252-3258`).
   - The `(+N)` last-block annotation block (`output.rs:3271-3291`) is unchanged — it continues to read `conversation.credits_spent_for_last_block()` and compare to the headline number. With rollup active, headline ≫ last block, so the annotation appears whenever the orchestrator has had a recent response. This is the intended behavior per PRODUCT invariant 11a.
   - Self-gating: when the conversation has no descendants the helper returns `None` and the pill renders exactly as today.

### 4. Reset UI state on footer collapse / reopen

The expanded footer view is created fresh each time `is_usage_footer_expanded` flips to true (the `ConversationUsageView` is constructed in `render_usage_footer`, not held across collapse). This means `details_expanded` and `show_all_clicked` naturally reset on collapse + reopen because the view instance is rebuilt — satisfying PRODUCT invariant 6.

- Verify this assumption during implementation. If the view instance is cached across collapse cycles, add explicit reset logic on the open transition, or hoist the bools to props that the parent rebuilds.

### 5. Self-gating (no feature flag)

There is no dedicated `OrchestrationCreditRollup` feature flag. The rollup activates whenever `compute_orchestration_rollup` returns `Some(_)` and falls through to today's UI whenever it returns `None`:

- Conversations with no locally-loaded descendants → `None` (no children walked, no rollup UI).
- Conversations where every loaded descendant has zero credits → `None` (zero-credit filter empties `per_agent`).
- Settings-mode views → `rollup()` short-circuits on `display_mode != Footer`.

The upstream ability to spawn child agents is still gated by `FeatureFlag::OrchestrationV2`, and the expanded footer surface itself is gated by `FeatureFlag::AgentView`, so the rollup is reachable only when both already permit the user to create and view an orchestration. Adding a third flag on top of those would only have offered a kill-switch; the self-gating data check provides the same safety net (today's UI is preserved whenever the rollup has nothing to add) without the cleanup burden.

### 6. Live updates from child usage changes

The expanded footer must re-render when any contributing agent's `conversation_usage_metadata` changes. Audit during implementation:

- Confirm there is a `BlocklistAIHistoryEvent` (or per-conversation event) that fires when any conversation's `conversation_usage_metadata` is mutated (via `StreamFinished` handling). If yes, ensure the orchestrator's footer view subscription covers all descendants — most likely already true via the existing history-level subscription used by `orchestration_pill_bar.rs` / `child_agent_status_card.rs`.
- If subscriptions cover descendants only via the parent's own view-bound observers, add a coarse "history changed" observation in `render_usage_footer` so the parent re-renders when any descendant updates.
- Worst case: introduce a fine-grained `ChildUsageUpdated { conversation_id }` event emitted on metadata write in `AIConversation` and subscribe from the parent view.

### 7. Tradeoffs and alternatives

- **Client-side aggregation (chosen).** Each loaded descendant's metadata is already on the client. Walk + sum is O(n) with n bounded by the locally-loaded orchestration tree (small in practice). Limitation: remote-only descendants are silently invisible (PRODUCT invariant 10) — acceptable for v1 given the gap is small in practice.
- **Server-side rollup (rejected for v1).** Add a `conversationUsageRollup(parentId)` GraphQL field that walks the run tree on the server. Pros: covers remote-only descendants. Cons: new query lifecycle, second source of truth that can drift from per-conversation metadata, server work to scope. Recommended follow-up if discrepancies prove confusing.
- **Separate "ORCHESTRATION TOTAL" section (rejected, was the prior strawman).** Adding a new section under USAGE SUMMARY duplicates the credit number and visually fragments the footer. The Figma mock integrates the rollup into the existing "Credits spent (total)" row — cleaner and matches the design.
- **Roll up other metrics too (deferred, follow-up).** PRODUCT v1 scope is credits only. The helper can be widened later to include summed `tool_calls`, `files_changed`, etc.
- **Collapsed pill: orchestration total vs self total.** Chose orchestration total (PRODUCT invariant 11) so the true cost is visible at a glance without expanding. The `(+N)` delta stays as orchestrator's own last-block credits because that is the only meaningful per-response delta available without tracking inter-agent timing.

## Testing and validation

Unit tests in `app/src/ai/blocklist/usage/rollup_tests.rs` (mod-included via `#[cfg(test)] #[path = "rollup_tests.rs"] mod tests;`):

- Orchestrator with no loaded descendants → `compute_orchestration_rollup` returns `None`. (PRODUCT invariant 1)
- Orchestrator + 1 child with credits → rollup `total_credits` = parent + child; `per_agent` has 2 entries sorted descending. (invariants 2a, 5a, 5b)
- Orchestrator + 3 children, mixed credits including a zero-credit child → zero-credit child is excluded; 3 entries returned sorted descending. (invariants 5b, 5d)
- Parent → child → grandchild — rollup includes all three transitively. (invariant 2a)
- 6 contributors → `per_agent.len() == 6`; caller logic verified separately in the renderer test (5 shown + "Show 1 more"). (invariants 5e, 5f)
- 1 contributor with zero credits → returns `None`. (invariant 7)
- Spawn-order tie-break: two children with equal credits → child spawned earlier sorts first. (invariant 5b)
- Unloaded descendant id present in topology but missing from `conversations_by_id` → silently skipped, no contribution to the rollup. (invariant 10)

Renderer tests in `conversation_usage_view_tests.rs`:

- `DisplayMode::Footer` + `Some(rollup)` renders the "Credits spent (total)" value as `rollup.total_credits` and shows "View details ▼". Clicking expands and shows "Hide details ▲". (invariants 2a–2d)
- Per-agent list with 5 rows renders all 5, no "Show N more". (invariant 5e)
- Per-agent list with 6 rows renders first 5 plus "Show 1 more"; clicking the link reveals the 6th and removes the link. (invariant 5f)
- Footer collapse + reopen rebuilds the view with `details_expanded == false` and `show_all_clicked == false`. (invariant 6)
- `DisplayMode::Settings` renders no toggle and no per-agent list even if a rollup is passed (defensive). (invariant 17)
- Conversation with no descendants (`rollup() == None`): row renders exactly as today (no toggle, no breakdown). (invariant 13)
- "Credits spent (last response)" row is unchanged regardless of rollup state. (invariant 3)

Collapsed-pill tests in `output_tests.rs` (or equivalent next to `render_usage_button`):

- Conversation with a non-empty rollup: pill headline number equals `rollup.total_credits`. (invariant 11)
- Conversation with rollup `None` (no descendants, or only zero-credit descendants): pill headline number equals `conversation.credits_spent()` (today's behavior). (invariants 11, 13)
- `(+N)` annotation renders using `credits_spent_for_last_block` regardless of rollup state. (invariant 11a)
- "Hide button entirely" suppression is evaluated against rollup total when applicable. (invariant 11b)

Manual verification:

- Start a local orchestration with three children (`oz-local` or in-app orchestrator), wait for each child to consume credits:
  - The collapsed footer pill shows the orchestration total, not just the orchestrator's self credits. (invariant 11)
  - Expand the footer: the "Credits spent (total)" row shows the same orchestration total. (invariants 2a, 7)
  - "View details" reveals the orchestrator + children sorted by credits descending. (invariants 5a, 5b)
- Trigger a child to finish a response mid-view; confirm both the pill number and the expanded rollup update without re-expanding. (invariants 7, 11)
- Spawn 7+ children; confirm the list shows 5 + "Show N more"; click and confirm all rows visible and link gone. (invariant 5f)
- Collapse + reopen the footer; confirm the list is back to truncated (and "View details" is closed). (invariant 6)
- Open the same expanded footer on a non-orchestrator conversation (or one with no loaded descendants) and confirm the row renders exactly as it does today, with no "View details" affordance. (invariant 13)

Run `./script/presubmit` before pushing. Formatting, clippy, and tests must pass.

## Parallelization

Single change, single repo, single owner. The aggregation helper, view extension, and feature-flag wiring touch the same handful of files and require a shared mental model. A parallel split would manufacture coordination overhead with no real wall-clock savings. This is best done by one local agent in this checkout (`/Users/matthew/src/rollup-orch-credit-usage/warp` on branch `matthew/rollup-orch-credit-usage`). No `run_agents` proposed for v1.

## Risks and mitigations

- **Tree walk cost on every render.** Orchestration trees observed in practice are small (≪100 nodes); the walk is O(n) with negligible constants. The collapsed pill renders every paint, so memoize the rollup on `BlocklistAIHistoryModel` (or cache on the parent view) if profiling shows it matters; invalidate on the same event used to drive live updates.
- **Collapsed-pill number jumps when children spawn.** Users watching only the pill might be surprised by sudden growth as children start spending. Acceptable trade for surfacing real cost at a glance — the existing `(+N)` delta still shows the orchestrator's own most-recent response so users can attribute the jump.
- **Display name fallback.** Some children may have `agent_name == None` (e.g. v1 orchestration or pre-naming spawn). Fallback to a short label like "Child agent" or the conversation's `task_id` short form. Pick during implementation; document in `DECISIONS.md` if it matters.
- **Pre-rollup metadata.** Forked or restored conversations may briefly have stale `conversation_usage_metadata` until the next `StreamFinished` or GraphQL hydrate completes. Acceptable: the next render corrects the rollup.

## Follow-ups

- Roll up other metrics (tool calls, code edits) — v1 scope is credits only.
- Server-side rollup query so remote-only descendants are always included (PRODUCT invariant 10).
- Make per-agent rows clickable (open the agent's conversation) — invariant 5g currently rules this out for v1.
- Show a rollup credit chip on the orchestrator pill in the orchestration pill bar (`orchestration_pill_bar.rs` hover card has room at lines 1044-1314).
