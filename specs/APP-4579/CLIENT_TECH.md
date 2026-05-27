# APP-4579 — Client tech spec
Implements the client-side surface of `specs/APP-4579/PRODUCT.md`. Server persistence and hidden-prompt injection are specified in `../warp-server/specs/APP-4579/TECH.md`.
## Problem
Local-to-cloud handoff must be available for conversations involved in orchestration, and the handoff spawn request must identify those conversations without exporting their topology. The server needs one fact only: whether it should inject the universal hidden orchestration-handoff message on the first cloud turn.
## Current handoff path
- `app/src/settings/ai.rs` supplies global and conversation-level cloud-handoff eligibility used by `&`, `/handoff`, footer-chip, and auto-handoff surfaces.
- `app/src/workspace/view.rs (13950-14112)` selects the environment, creates the forked server conversation, uploads snapshot state, computes the marker, and constructs the pending handoff.
- `app/src/terminal/view/ambient_agent/model.rs (144-163, 625-656, 1118-1145)` keeps `PendingHandoff` while environment setup completes, constructs the eventual handoff `SpawnAgentRequest`, and omits the marker for fresh cloud launches.
- `app/src/server/server_api/ai.rs (200-254)` defines the public spawn request payload sent to the server.
- `app/src/ai/conversation.rs` exposes whether the source conversation has a parent agent; `app/src/ai/agent_history/model.rs` exposes locally-known children.
## Client changes
### 1. Permit handoff for orchestrated local conversations
Remove only the orchestration-specific gating from the existing handoff eligibility and workspace initiation paths. The global handoff setting, cloud conversation storage requirement, feature flags, sync-token requirement, and long-running-command protection remain unchanged.
All existing surfaces become available under the same global rules for a local conversation with a parent agent, locally-known children, or both:
- `&` input prefix.
- `/handoff`.
- The footer handoff chip.
- Workspace action and auto-handoff initiation.
### 2. Compute one universal marker at handoff construction time
`complete_local_to_cloud_handoff_open` (`app/src/workspace/view.rs:14043`) already holds the source conversation and can query locally-known children. It computes the marker from either kind of orchestration relationship:
```rust
let orchestration_handoff = (source_conversation.has_parent_agent()
    || !history_model
        .as_ref(ctx)
        .child_conversation_ids_of(&source_conversation.id())
        .is_empty())
    .then_some(true);
```
The marker deliberately does not distinguish parent, child, or mixed roles. Any orchestrated source receives the same server-injected universal prompt.
### 3. Carry the optional marker through the pending handoff request
In `app/src/terminal/view/ambient_agent/model.rs (144-163, 625-656)`, retain the computed marker until the spawn request is built:
```rust
struct PendingHandoff {
    // existing fields...
    orchestration_handoff: Option<bool>,
}
```
`build_handoff_spawn_request` forwards this value directly. Fresh cloud launches set it to `None`, because no local handoff occurred.
### 4. Send the minimal public request shape
In `app/src/server/server_api/ai.rs (200-254)`, extend `SpawnAgentRequest` with:
```rust
pub struct SpawnAgentRequest {
    // existing fields...
    /// True only when a local-to-cloud handoff source participated in orchestration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orchestration_handoff: Option<bool>,
}
```
Wire contract:
- Orchestrated local-to-cloud handoff: `"orchestration_handoff": true`.
- Non-orchestrated local-to-cloud handoff: field absent.
- Fresh cloud launch: field absent.
The client never sends local run identifiers, parent/child identifiers, relationship direction, or local execution state through this field.
## End-to-end client flow
1. A local conversation starts handoff through an existing surface after global eligibility checks pass.
2. The client captures and uploads its task-context snapshot as in the existing handoff flow.
3. The client forks the synced source conversation with the existing fork RPC.
4. The client evaluates whether the source has a parent agent or locally-known children.
5. If it does, the pending handoff and `SpawnAgentRequest` contain `orchestration_handoff: Some(true)`; otherwise they contain `None`.
6. The client spawns the cloud run using the forked conversation id as `conversation_id`.
7. On cloud-start failure, existing snapshot cleanup and local recovery behavior remain unchanged.
## Tests
- Eligibility tests verify orchestrated conversations can use handoff after the orchestration-specific gate is removed, while existing global and operational blockers continue to apply.
- `app/src/terminal/view/ambient_agent/model_tests.rs` verifies a pending marker propagates to `SpawnAgentRequest` and serializes as `orchestration_handoff: true`.
- The same tests verify `None` omits the field for non-orchestrated handoff and fresh cloud launch paths.
## Validation
- Run `cargo fmt` for Rust formatting.
- Run focused compile/test coverage for modified APP-4579 request and handoff code without launching the application.