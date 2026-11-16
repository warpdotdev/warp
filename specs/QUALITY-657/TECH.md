# Associate Orchestration Config with Plan ID — Client Tech Spec

## Problem
The Warp desktop client stores orchestration config as a **single value per conversation**. When the server switches to per-plan snapshots (append-only, one per plan — see `warp-server/specs/QUALITY-657/TECH.md`), the client needs to:
1. Hydrate multiple orchestration configs from conversation history, indexed by `plan_id`.
2. Render a config block on each plan card showing that plan's config.
3. Thread `plan_id` through the `RunAgents` request so the auto-launch match check is plan-scoped.
4. Send per-plan dirty events back to the server.

## Companion spec
Server-side changes are documented in `warp-server/specs/QUALITY-657/TECH.md`. The proto changes (`plan_id` on `RunAgents` field 9, append-only `OrchestrationConfigSnapshot` messages) land in `warp-proto-apis` before this work begins. This spec covers only the Warp desktop client (Rust).

## Relevant code

### Hydration and model
- `app/src/ai/document/ai_document_model.rs (1192-1319)` — `handle_history_event_for_orchestration_config()` and `scan_conversation_for_orchestration_config()`. Currently calls `.last()` on all snapshot messages to find the single config.
- `app/src/ai/document/ai_document_model.rs (195-199)` — `dirty_orchestration_events: HashMap<AIConversationId, DirtyOrchestrationEvent>`. One dirty event per conversation.

### Conversation state
- `app/src/ai/agent/conversation.rs (882-909)` — `orchestration_config()`, `orchestration_status()`, `orchestration_plan_id()`, `set_orchestration_config()`. Single config/status/plan_id stored per conversation.

### Plan card config block
- `app/src/ai/document/orchestration_config_block.rs (109-186)` — `OrchestrationConfigBlockView`. Keyed by conversation, not by plan.
- `app/src/ai/ai_document_view.rs (1061-1085)` — Renders a single config block if the conversation has an orchestration config.

### Auto-launch / match check
- `crates/ai/src/agent/orchestration_config.rs (52-96)` — `matches_active_config()`. Compares a `RunAgentsRequest` against a single `OrchestrationConfig`.
- `app/src/ai/blocklist/inline_action/run_agents_card_view.rs (189-268)` — `should_auto_launch()`. Receives `active_config: Option<(OrchestrationConfig, OrchestrationConfigStatus)>` — no `plan_id` context.

### RunAgents request
- `crates/ai/src/agent/action/mod.rs (187-195)` — `RunAgentsRequest` struct. No `plan_id` field.

### Dirty sync
- `app/src/ai/blocklist/controller.rs (746-788)` — Takes one dirty event per conversation from `AIDocumentModel`, appends to `inputs`.
- `app/src/ai/agent/api/convert_to.rs (438-450)` — Converts `AIAgentInput::OrchestrationConfigUpdate` to proto, already includes `plan_id`.

### Dispatch
- `app/src/ai/blocklist/action_model/execute/run_agents.rs (77-245)` — `dispatch_run_agents()`. No `plan_id` handling. Needs to thread `plan_id` from `RunAgentsRequest` into per-child dispatch calls (for logging/telemetry context, not for child execution).

## Current state
The client treats orchestration config as a conversation-level singleton:
- **Hydration**: Scans all messages, takes the `.last()` snapshot, stores one config on the conversation.
- **Plan card**: One config block per conversation. All plan cards share it.
- **Auto-launch**: `should_auto_launch()` receives the single active config. No plan_id filtering.
- **RunAgentsRequest**: No `plan_id` field. The request can't express which plan it's executing.
- **Dirty sync**: One dirty event per conversation. Editing the config block queues one event.

## Proposed changes

### 1. `RunAgentsRequest`: add `plan_id`
Add `plan_id` to the request struct in `crates/ai/src/agent/action/mod.rs`:
```rust
pub struct RunAgentsRequest {
    pub summary: String,
    pub base_prompt: String,
    pub skills: Vec<SkillReference>,
    pub model_id: String,
    pub harness_type: String,
    pub execution_mode: RunAgentsExecutionMode,
    pub agent_run_configs: Vec<RunAgentsAgentRunConfig>,
    pub plan_id: String,  // NEW
}
```
Update the proto-to-struct conversion (wherever `RunAgentsRequest` is built from the `RunAgents` proto) to read `plan_id` from the proto field 9.

Note: `RunAgentsResult::Launched` does not need a `plan_id` field — the client already has `plan_id` from the `RunAgentsRequest` and does not need it repeated on the result.

### 2. Conversation state: per-plan config map
Replace the single config on `AIAgentConversation` with a map:
```rust
// Before (conversation.rs):
orchestration_config: Option<OrchestrationConfig>,
orchestration_status: OrchestrationConfigStatus,
orchestration_plan_id: Option<String>,

// After:
orchestration_configs: HashMap<String, (OrchestrationConfig, OrchestrationConfigStatus)>,
```
Keyed by `plan_id`. Snapshots with empty `plan_id` are ignored (legacy conversations).

New accessors:
```rust
pub fn orchestration_config_for_plan(&self, plan_id: &str)
    -> Option<(&OrchestrationConfig, OrchestrationConfigStatus)>

pub fn set_orchestration_config_for_plan(
    &mut self,
    plan_id: String,
    config: OrchestrationConfig,
    status: OrchestrationConfigStatus,
) -> bool
```

Remove the old `orchestration_config()`, `orchestration_status()`, `orchestration_plan_id()`, `set_orchestration_config()` accessors.

### 3. Hydration: scan and index by `plan_id`
Change `scan_conversation_for_orchestration_config()` in `ai_document_model.rs`:

**Before**: Finds the `.last()` `OrchestrationConfigSnapshot` message, stores a single config.

**After**: Scans backward through all messages. For each `OrchestrationConfigSnapshot` with a non-empty `plan_id`, stores the first one found during the backward scan (i.e. the most recent) per `plan_id`. Result is a map of `plan_id → (config, status)` set on the conversation.

```rust
fn scan_conversation_for_orchestration_config(messages: &[Message]) -> HashMap<String, (OrchestrationConfig, OrchestrationConfigStatus)> {
    let mut configs = HashMap::new();
    for msg in messages.iter().rev() {
        if let Some(snapshot) = msg.orchestration_config_snapshot() {
            let plan_id = snapshot.plan_id();
            if !plan_id.is_empty() && !configs.contains_key(plan_id) {
                configs.insert(plan_id.to_string(), (
                    OrchestrationConfig::from_proto(snapshot.config()),
                    OrchestrationConfigStatus::from_proto(snapshot.status()),
                ));
            }
        }
    }
    configs
}
```

Also update `handle_history_event_for_orchestration_config()` to process incremental snapshot messages the same way — when a new snapshot arrives (via `UpdatedConversationStatus` or `AppendedExchange`), insert/overwrite the entry for that `plan_id` in the map.

### 4. Plan card config block: per-plan rendering
Change `OrchestrationConfigBlockView` to be keyed by `(conversation_id, plan_id)` instead of just `conversation_id`.

In `ai_document_view.rs`, each `AIDocumentView` knows its plan's `document_id` (which is the `plan_id`). Pass it to the config block constructor:
```rust
// Before:
OrchestrationConfigBlockView::new_with_conversation_id(conversation_id, ctx)

// After:
OrchestrationConfigBlockView::new(conversation_id, plan_id, ctx)
```

The config block reads its config from `conversation.orchestration_config_for_plan(plan_id)` instead of `conversation.orchestration_config()`.

A plan card only shows a config block if a config exists for its `plan_id`. Plans without configs show no block (rather than sharing a global config).

### 5. Auto-launch: plan-scoped match check
Change `RunAgentsCardView` construction to look up the config by `plan_id` from the request:

**Before** (line 250-254 of `run_agents_card_view.rs`):
```rust
let active_config = conversation.orchestration_config()
    .map(|c| (c.clone(), conversation.orchestration_status()));
```

**After**:
```rust
let active_config = if !state.plan_id.is_empty() {
    conversation.orchestration_config_for_plan(&state.plan_id)
        .map(|(c, s)| (c.clone(), s))
} else {
    None
};
```

`should_auto_launch()` signature stays the same — it already receives `active_config: &Option<(OrchestrationConfig, OrchestrationConfigStatus)>`. The plan-scoping happens at the call site.

`matches_active_config()` in `orchestration_config.rs` is unchanged — it compares request fields against a config. The plan_id filtering is done before calling it.

### 6. Dirty sync: per-plan dirty events
Change the dirty event queue from `HashMap<AIConversationId, DirtyOrchestrationEvent>` to `HashMap<(AIConversationId, String), DirtyOrchestrationEvent>` where the second key element is `plan_id`.

In `controller.rs`, the current `take_dirty_orchestration_event(&conversation_id)` becomes `take_dirty_orchestration_events(&conversation_id)` which returns all dirty events for the conversation (one per plan that was edited). Each is appended as a separate `AIAgentInput::OrchestrationConfigUpdate`.

The proto conversion in `convert_to.rs` is unchanged — each `OrchestrationConfigUpdate` already carries its own `plan_id`.

### 7. Config block editing: plan-scoped dirty events
When the user edits a field in `OrchestrationConfigBlockView`, the `apply_field_change()` method currently calls:
```rust
model.set_orchestration_config(config, status, plan_id);
model.set_dirty_orchestration_event(conversation_id, dirty_event);
```

After the change, this becomes:
```rust
model.set_orchestration_config_for_plan(plan_id, config, status);
model.set_dirty_orchestration_event(conversation_id, plan_id, dirty_event);
```

Each config block edit only affects its own plan's config and queues a dirty event for that plan.

## End-to-end flow

### Hydration (restore)
1. Client opens a conversation with history.
2. `scan_conversation_for_orchestration_config()` scans backward, builds `HashMap<plan_id, (config, status)>`.
3. Each plan card's `AIDocumentView` checks if its `plan_id` has an entry → renders config block if so.

### Agent calls `run_agents` with `plan_id`
1. Server emits `SetRunAgentsToolCall` with `plan_id` and resolved defaults.
2. Client parses `RunAgentsRequest` including `plan_id`.
3. `RunAgentsCardView` construction looks up `conversation.orchestration_config_for_plan(plan_id)`.
4. If config found + approved + fields match → auto-launch (no card shown).
5. If no config or mismatch → show confirmation card.

### User edits config on plan B's card
1. User toggles a field on plan B's config block.
2. `apply_field_change()` updates `orchestration_configs["plan-B"]` on the conversation.
3. Dirty event queued for `(conversation_id, "plan-B")`.
4. On next outbound request, dirty event piggybacked as `OrchestrationConfigUpdate { plan_id: "plan-B", ... }`.
5. Server appends a new `OrchestrationConfigSnapshot` message with `plan_id = "plan-B"`.

### Two plans, independent configs
- Plan A has `local` config. Plan B has `remote` config.
- `run_agents(plan_id="A")` → inherits local. `run_agents(plan_id="B")` → inherits remote.
- Editing plan A's config does not affect plan B's.

## Coordination with server changes

### Proto dependency
The `plan_id` field on `RunAgents` (field 9) must land in `warp-proto-apis` before the client work begins. The `OrchestrationConfigSnapshot` proto already has `plan_id` (field 1) — no proto change needed for that.

### Backward compatibility
- **New client + old server**: Server sends `OrchestrationConfigSnapshot` with empty `plan_id` (singleton model). Client ignores empty `plan_id` snapshots → no config hydrated → every `run_agents` call shows a confirmation card. Functionally correct, just no auto-launch.
- **Old client + new server**: Server appends per-plan snapshots. Old client's `.last()` scan picks up whichever snapshot was appended most recently → may show the wrong plan's config. Acceptable during rollout since the old client doesn't use `plan_id` for match-checking anyway.
- **New client + new server**: Full per-plan behavior.

### Rollout order
1. Proto PR (`warp-proto-apis`): adds `plan_id` field 9 to `RunAgents`.
2. Server PR (`warp-server`): implements per-plan append-only snapshots, `plan_id` on `create_orchestration_config` and `run_agents`.
3. Client PR (`warp`): implements per-plan hydration, config blocks, auto-launch, dirty sync.

Server and client PRs can land in either order after the proto — backward compatibility is maintained in both directions.

## Risks and mitigations

**Risk: Conversations with many plans accumulate snapshot messages.** Client scans backward through all messages on hydration.
*Mitigation:* The backward scan is O(n) over messages but short-circuits per plan_id (first match wins). For typical conversations this is negligible.

**Risk: Old singleton snapshots (empty `plan_id`) are orphaned.** After the client upgrade, they're never hydrated.
*Mitigation:* Desired behavior. The agent will call `create_orchestration_config` with a `plan_id` on its next interaction, creating a proper per-plan snapshot.

**Risk: Behavioral change — `run_agents` without `plan_id` no longer auto-launches.** Today, any `run_agents` call can auto-launch against the singleton config. After this change, `run_agents` without `plan_id` always shows a confirmation card because `active_config` is `None` when `plan_id` is empty. This is intentional — inheritance now requires the agent to specify which plan it's executing — but it changes the default experience for agents that orchestrate without plans.

**Risk: Config block flicker during hydration.** Plan cards may briefly render without config blocks until hydration completes.
*Mitigation:* Hydration runs synchronously in `scan_conversation_for_orchestration_config()` before the plan card view is built. No async gap.

## Testing and validation

### Unit tests

**`orchestration_config.rs` (crate-level):**
- `matches_active_config()` — unchanged; existing tests still pass.

**`ai_document_model.rs`:**
- Hydration with multiple snapshots for different `plan_id`s → map contains one entry per plan.
- Hydration with multiple snapshots for the same `plan_id` → most recent wins.
- Hydration with empty `plan_id` snapshots → ignored.
- Incremental snapshot arrival → updates the correct plan's entry.

**`run_agents_card_view.rs`:**
- `should_auto_launch()` with matching plan config → true.
- `should_auto_launch()` with no config for plan → false.
- `should_auto_launch()` with config for a different plan → false.

**`conversation.rs`:**
- `orchestration_config_for_plan()` returns correct config per plan.
- `set_orchestration_config_for_plan()` doesn't affect other plans.

**`controller.rs` (dirty sync):**
- Editing plan A queues one dirty event; editing plan B queues another.
- Both are sent on the next outbound request.
- Events are cleared after send.

### Manual validation
- Create two plans in the same conversation with different orchestration configs.
- Verify each plan card shows its own config block.
- Verify `run_agents` for each plan inherits from its own config.
- Verify editing one plan's config doesn't affect the other.
- Verify auto-launch works per-plan.
- Verify disapproving one plan's config doesn't block the other.

## Follow-ups
- **Config block visibility without explicit create.** If the user wants to add a config to a plan that doesn't have one, the plan card currently shows nothing. A future enhancement could add an "Add orchestration config" affordance.
- **Garbage collection of stale snapshots.** If a plan is deleted, its snapshot messages remain. Not harmful (they're never matched), but could be cleaned up in a future pass.
