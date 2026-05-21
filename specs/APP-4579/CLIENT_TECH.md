# APP-4579 — Client tech spec
Implements the client-side surface of `specs/APP-4579/PRODUCT.md`. The server-side prompt injection is handled in `../warp-server/specs/APP-4579/TECH.md`.

## Problem
Three client-side gates currently block local-to-cloud handoff for orchestrated conversations, plus the client needs to forward orchestration role to the server so the cloud agent's first turn can carry the right hidden system message:
1. `AISettings::is_cloud_handoff_enabled_for_conversation` (`app/src/settings/ai.rs:1722-1730`) returns `false` when `is_orchestration_conversation(conversation, app)` returns `true`. This gate feeds the `&` prefix check, the `/handoff` slash command, the footer chip, the workspace action's safety net, and the auto-handoff controller.
2. `is_orchestration_conversation` (`app/src/settings/ai.rs:2057-2062`) returns `true` when the conversation has a parent agent (`has_parent_agent()`) or has at least one locally-known child (`BlocklistAIHistoryModel::child_conversation_ids_of` non-empty).
3. `Workspace::start_local_to_cloud_handoff_from_source` (`app/src/workspace/view.rs:13822-13842`) re-checks the per-conversation gate immediately after computing the source conversation and, on `show_user_feedback` paths, shows the toast "Cloud handoff isn't available for orchestrated agent conversations."
4. `SpawnAgentRequest` (`app/src/server/server_api/ai.rs:205-252`) carries the handoff payload but has no `orchestration_handoff` field today. The two bits the server needs (`had_parent`, `had_children`) are derived locally from the same `has_parent_agent()` and `child_conversation_ids_of` calls used by gate (2), and need to be plumbed through to spawn time.
The client makes no other use of orchestration-vs-non-orchestration to differentiate the handoff flow. The fork RPC, snapshot upload, and pane wiring are all orchestration-agnostic today.
## Current state
- `is_cloud_handoff_enabled(app)` evaluates the global gates (feature flags, cloud-conversations storage, AI enabled, user setting). `is_cloud_handoff_enabled_for_conversation` layers the orchestration gate on top.
- `is_ampersand_handoff_enabled_for_conversation` (`app/src/settings/ai.rs:1745-1752`) and `is_ampersand_handoff_enabled_for_terminal_view` (`app/src/settings/ai.rs:1754-1761`) both delegate to `is_cloud_handoff_enabled_for_conversation`, so removing the orchestration gate there transitively removes it from the `&` prefix check (`app/src/terminal/input.rs:3919`).
- The footer chip's gating (`app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs:2066`) uses `is_cloud_handoff_enabled_for_conversation`.
- The `/handoff` slash command's `is_supported` callback (`app/src/terminal/input/slash_commands/mod.rs:902`) uses `is_cloud_handoff_enabled_for_conversation`.
- The workspace action's safety net (`app/src/workspace/view.rs:13794`) uses `is_cloud_handoff_enabled(ctx)` (not the per-conversation variant), so it stays unchanged. The toast-emitting branch at line 13822 is the per-conversation re-check.
- `AutoCloudHandoffEligibility` (`app/src/workspace/auto_handoff.rs:44-58`) reads `is_cloud_handoff_enabled_for_conversation` to set `can_handoff_to_cloud`. Today this skips orchestrated conversations entirely. After this change, auto-handoff becomes eligible for orchestrated conversations using the same logic.
- `SpawnAgentRequest` (`app/src/server/server_api/ai.rs:205-252`) is the cross-cutting payload sent to `POST /agent/runs`. `build_handoff_spawn_request` (`app/src/terminal/view/ambient_agent/model.rs:621-649`) is the single chokepoint where the request is constructed for the handoff path, using state stashed on `PendingHandoff` (`app/src/terminal/view/ambient_agent/model.rs:144-161`) by `complete_local_to_cloud_handoff_open` (`app/src/workspace/view.rs:13994-14139`).
- `complete_local_to_cloud_handoff_open` owns the `source_conversation: AIConversation` and an `&mut ViewContext<Workspace>` (which can access `BlocklistAIHistoryModel`), so it is the natural place to compute `had_parent`/`had_children` once per handoff and stash on `PendingHandoff`.
- The forked conversation that the cloud agent will receive comes from `ai_client.fork_conversation(...)` (`app/src/workspace/view.rs:13945`). Inside the fork helper on the server, the source `.pb` is copied verbatim, so the cloud agent will see the source's `parent_agent_id` and any child-related transcript content. The new `orchestration_handoff` field is the explicit "both relationships are severed" annotation that the server uses to inject the hidden first-turn message.
## Proposed changes
### 1. Delete the per-conversation / per-terminal-view handoff helpers
In `app/src/settings/ai.rs`:
- Delete `is_cloud_handoff_enabled_for_conversation` (lines 1722-1730), `is_cloud_handoff_enabled_for_terminal_view` (lines 1732-1740), `is_ampersand_handoff_enabled_for_conversation` (lines 1745-1752), and `is_ampersand_handoff_enabled_for_terminal_view` (lines 1754-1761). Once the orchestration gate is gone, each is a passthrough to its non-suffixed sibling (`is_cloud_handoff_enabled` / `is_ampersand_handoff_enabled`) that never touches its conversation/terminal-view parameter.
- Delete the now-unused `is_orchestration_conversation` private function (lines 2057-2062). Its single call site is going away.
- Replace every call site with the equivalent non-suffixed call:
  - `app/src/workspace/auto_handoff.rs:177-178` → `AISettings::as_ref(ctx).is_cloud_handoff_enabled(ctx)`
  - `app/src/workspace/view.rs:13822` → removed entirely as part of §2 (the surrounding `if` block is the orchestration toast we're dropping)
  - `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs:2066` → `is_cloud_handoff_enabled(app)`
  - `app/src/terminal/input/slash_commands/mod.rs:902` → `is_cloud_handoff_enabled(app)`
  - `app/src/terminal/input.rs:3919` (and any other `&`-prefix gate) → `is_ampersand_handoff_enabled(app)`
  - The settings UI page (`app/src/settings_view/ai_page.rs:6845`) and any tests under `app/src/settings/ai_tests.rs` → the equivalent non-suffixed call.
We collapse rather than wrap because the conversation/terminal-view arguments were only ever there to evaluate `is_orchestration_conversation`. Without that, they're dead weight, and dead arguments invite future drift. If a per-conversation gate is ever needed again, it can be re-introduced at that point.
### 2. Drop the orchestration toast from `start_local_to_cloud_handoff_from_source`
In `app/src/workspace/view.rs:13822-13842`, remove the entire per-conversation gate block. The remaining flow already handles the relevant invariants:
- The global gate at line 13794 (`is_cloud_handoff_enabled(ctx)`) is unaffected and continues to short-circuit handoff when the user/org has it off.
- The "no server_conversation_token" branch at line 13918 continues to surface the "hasn't synced" toast for orchestrated and non-orchestrated conversations alike.
- The "long-running command in flight" branch at line 13878 continues to surface its toast for orchestrated and non-orchestrated conversations alike.
The toast string itself is removed; there is no replacement copy.
### 3. Add `orchestration_handoff` to `SpawnAgentRequest`
In `app/src/server/server_api/ai.rs`:
- Add a new struct next to `InitialSnapshotToken`:
  ```rust
  /// Records the orchestration relationships the source conversation had at the
  /// moment of a local-to-cloud handoff. Populated by `build_handoff_spawn_request`
  /// from the source conversation. The server stamps this onto the new task's
  /// `AgentConfigSnapshot` and the runtime reads it at first-turn time to inject
  /// a hidden system message telling the cloud agent that those prior
  /// relationships no longer reach it.
  #[derive(Debug, Clone, serde::Serialize)]
  pub struct OrchestrationHandoffInfo {
      #[serde(skip_serializing_if = "std::ops::Not::not")]
      pub had_parent: bool,
      #[serde(skip_serializing_if = "std::ops::Not::not")]
      pub had_children: bool,
  }
  ```
  Using `skip_serializing_if = "std::ops::Not::not"` per-field matches the server's `omitempty` semantics, keeping the wire-level JSON minimal (`{}` when both are false) and identical to what the server expects.
- Extend `SpawnAgentRequest` (lines 205-252) with:
  ```rust
  /// Records that the source conversation was part of an orchestration tree at
  /// handoff time. Only set on local-to-cloud handoff spawns; absent otherwise.
  /// The server uses this to inject a hidden first-turn message into the cloud
  /// agent's conversation telling it that prior orchestration relationships are
  /// unreachable.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub orchestration_handoff: Option<OrchestrationHandoffInfo>,
  ```
  The field is on `SpawnAgentRequest` (not `AgentConfigSnapshot`) because the server-side request type also keeps it on `RunAgentRequest` rather than on the snapshot wire shape — the server is the one that copies request → snapshot. Matching that layout keeps both sides symmetric.
### 4. Compute the bits in `complete_local_to_cloud_handoff_open` and stash on `PendingHandoff`
In `app/src/terminal/view/ambient_agent/model.rs:144-161`, extend `PendingHandoff` with:
```rust
/// Orchestration relationships the source conversation had at handoff time.
/// `None` when the source had no orchestration; `Some(_)` when at least one bit
/// is true. Forwarded verbatim to the server as `SpawnAgentRequest.orchestration_handoff`.
pub(crate) orchestration_handoff: Option<OrchestrationHandoffInfo>,
```
In `app/src/workspace/view.rs::complete_local_to_cloud_handoff_open` (lines 13994-14139), compute the bits once — right before constructing `PendingHandoff` at line 14107 — using the source conversation and the history model that is already in scope:
```rust
let orchestration_handoff = {
    let history = BlocklistAIHistoryModel::as_ref(ctx);
    let had_parent = source_conversation.has_parent_agent();
    let had_children = !history.child_conversation_ids_of(&source_conversation.id()).is_empty();
    (had_parent || had_children).then_some(OrchestrationHandoffInfo { had_parent, had_children })
};
```
Populate `PendingHandoff.orchestration_handoff` with this value. The same computation is done in `Workspace::start_fresh_cloud_launch` (when present) only if it ever begins to honor handoff state — today it is a fresh launch with no source conversation, so `orchestration_handoff` stays `None` there. Tests cover both paths.
### 5. Wire through `build_handoff_spawn_request`
In `app/src/terminal/view/ambient_agent/model.rs::build_handoff_spawn_request` (lines 621-649), pull the stashed bits off `self.pending_handoff` and pass them to the request:
```rust
orchestration_handoff: self.pending_handoff.as_ref().and_then(|h| h.orchestration_handoff.clone()),
```
`spawn_agent` (the fresh-launch entry point, lines 1110-1139) keeps `orchestration_handoff: None` — fresh cloud launches have no source orchestration to sever. `spawn_agent_with_request` (lines 1142-1173) is a passthrough and does not need to compute anything; callers that already constructed a request pass it through verbatim.
### 6. Tests
- `app/src/settings/ai_tests.rs` already covers `is_cloud_handoff_enabled_for_conversation`. The test that asserts orchestration conversations are gated out flips to asserting that they are eligible. Add tests for both the "had parent" and "had children" cases so any future re-introduction of an orchestration gate breaks loudly.
- `app/src/workspace/auto_handoff_tests.rs` exercises `AutoCloudHandoffEligibility::skip_reason`. Add a test that an orchestrated, in-progress, synced conversation with no long-running command returns `None` (i.e. is eligible).
- `app/src/terminal/view/ambient_agent/model_tests.rs` (or a new colocated test for `build_handoff_spawn_request`) — table-driven: pending handoff with no orchestration info → request omits `orchestration_handoff`; with `{had_parent: true, had_children: false}` → request carries that exact value; both true → request carries both; both false → normalized to `None`. JSON-serialize the request and confirm the wire shape matches the server's expected snake_case (`orchestration_handoff`, `had_parent`, `had_children`).
- `app/src/workspace/view.rs` does not have direct tests for `complete_local_to_cloud_handoff_open`, but the bit computation is small enough to extract into a free function (`derive_orchestration_handoff_info(source: &AIConversation, history: &BlocklistAIHistoryModel) -> Option<OrchestrationHandoffInfo>`) and table-test directly. Recommend this extraction.
## Risks and mitigations
**Auto-handoff fires on an orchestrated parent during macOS sleep, leaving locally-running children orphaned.** The cloud parent will now run; the local children will keep running until they finish on their own. The hidden orchestration message (server side) tells the cloud parent the local children are unreachable, so it should not block waiting for them. Local children that try to message the (now-cloud) parent will write to the server-side messaging inbox; the cloud parent technically receives those messages but is instructed by the orchestration prompt to ignore the prior relationships. We accept this minor leakage; the alternative (cancelling local children on auto-handoff) is out of scope.
**Loss of the toast removes user signal that orchestration is in play.** Today the toast doubles as a confused-user breadcrumb. After this change, the handoff proceeds silently for orchestration conversations, which is exactly what we want — but it means users may not realize they have severed orchestration. The server-side cloud agent message handles the cloud agent's behavior; the user-facing surface intentionally does not call attention to this.
**Client/server disagreement on what the bits mean.** The client computes `had_parent` from `AIConversation::has_parent_agent()` (which is `parent_conversation_id.is_some() || parent_agent_id.is_some()`) and `had_children` from `BlocklistAIHistoryModel::child_conversation_ids_of` (the locally-known children index). The server treats both bits as opaque facts and does not cross-check. A buggy client that lies produces a wrong informational prompt and nothing else — the prompt has no authority over routing or tool execution.
**Race: children spawn or finish between gate check and request send.** `complete_local_to_cloud_handoff_open` is `&mut ViewContext<Workspace>` and runs on the main thread, so the computation is atomic with respect to other UI events. Background async child spawns can theoretically race the fork RPC, but the worst-case outcome is the prompt missing a child that was spawned after the computation — again, informational only.
## Testing and validation
- `cargo check -p warp` and `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings` after the change.
- Run `app/src/settings/ai_tests.rs`, `app/src/workspace/auto_handoff_tests.rs`, and the new `model_tests.rs` cases.
- Manual dogfood end-to-end: start a local orchestration parent, have it spawn at least one local child, then handoff via `&`, `/handoff`, and the footer chip in three separate runs. Verify each opens the handoff pane without a toast, sends a `POST /agent/runs` with `orchestration_handoff: {had_children: true}` in the payload (visible in the network log), and produces a cloud agent that does not try to message the local children. Repeat with the child as the source (handoff a child while it's running) — expect `orchestration_handoff: {had_parent: true}` on the request and a cloud child that runs to completion without waiting for the parent.
## Parallelization
This task is too small for parallel sub-agents. The client diff is ~80 LOC across ~5 files plus tests. The server-side work in `../warp-server/specs/APP-4579/TECH.md` is independent at the implementation level — the wire-level JSON shape is defined once in the server spec and both sides target it. Either order is fine; neither blocks the other behind a wire-incompatible change.
