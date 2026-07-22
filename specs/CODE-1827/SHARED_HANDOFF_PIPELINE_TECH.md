# CODE-1827 PR 2: Shared Handoff Pipeline and GUI Migration

## Context

This middle PR extracts local-to-cloud handoff policy and sequencing from GUI workspace/pane state, then migrates the GUI as the shared pipeline’s only caller. It adds no TUI command or handoff card. References are pinned to warp commit `c4cc7be9477897c75c34bdc75c1a324c25b12f27`.

- [`app/src/terminal/input.rs (4244-4617)`](https://github.com/warpdotdev/warp/blob/c4cc7be9477897c75c34bdc75c1a324c25b12f27/app/src/terminal/input.rs#L4244-L4617) owns GUI compose activation and prompt/attachment dispatch.
- [`app/src/workspace/view.rs (15540-15941)`](https://github.com/warpdotdev/warp/blob/c4cc7be9477897c75c34bdc75c1a324c25b12f27/app/src/workspace/view.rs#L15540-L15941) owns eligibility, cancellation, server fork, local fork/pane materialization, and snapshot startup.
- [`app/src/terminal/view/ambient_agent/model.rs (110-187)`](https://github.com/warpdotdev/warp/blob/c4cc7be9477897c75c34bdc75c1a324c25b12f27/app/src/terminal/view/ambient_agent/model.rs#L110-L187) stores GUI handoff state.
- [`app/src/terminal/view/ambient_agent/model.rs (690-807)`](https://github.com/warpdotdev/warp/blob/c4cc7be9477897c75c34bdc75c1a324c25b12f27/app/src/terminal/view/ambient_agent/model.rs#L690-L807) owns prompt substitution and queued submission.
- [`app/src/ai/blocklist/handoff/touched_repos.rs (81-422)`](https://github.com/warpdotdev/warp/blob/c4cc7be9477897c75c34bdc75c1a324c25b12f27/app/src/ai/blocklist/handoff/touched_repos.rs#L81-L422) already provides frontend-neutral path/repository derivation.
- [`app/src/ai/agent_sdk/driver/snapshot.rs (752-869)`](https://github.com/warpdotdev/warp/blob/c4cc7be9477897c75c34bdc75c1a324c25b12f27/app/src/ai/agent_sdk/driver/snapshot.rs#L752-L869) uploads handoff snapshots without GUI rendering.
- [`app/src/ai/ambient_agents/spawn.rs (102-136)`](https://github.com/warpdotdev/warp/blob/c4cc7be9477897c75c34bdc75c1a324c25b12f27/app/src/ai/ambient_agents/spawn.rs#L102-L136) combines run creation with GUI joinable-session polling.

## Proposed changes

Add a shared pipeline module under `app/src/ai/blocklist/handoff/` and move domain state out of `AmbientAgentViewModel`.

### `prepare_handoff`

`prepare_handoff` receives concrete shared handles/facts rather than a source trait:

- Terminal surface/view identity.
- Shared history, controller, and AI context handles.
- Current working directory and snapshot target information.
- Long-running-command state.
- Whether the selected environment is required by the caller.
- Optional prompt and entry metadata.

It exclusively owns this order:

1. Resolve the selected source conversation.
2. Apply empty-source/no-prompt and long-running-command guardrails.
3. Reject an orchestrator before cancellation if any loaded child is in-progress or blocked.
4. Capture source active/orchestration state.
5. Transfer pending attachments into shared pending state.
6. Cancel an active source response.
7. Resolve initial model/environment defaults.
8. Emit initiation telemetry.
9. Return `PendingHandoff`.

`PendingHandoff` keeps fields private and exposes only presentation snapshots, environment/model setters, explicit-selection precedence, current valid-environment refresh, validation, and exactly-once prompt/attachment restoration. Environment validation permits no selection for GUI and can require one for future callers. It exposes no fork, snapshot, spawn, or arbitrary phase-transition methods.

Preparation captures attachment payloads before cancellation but clears them from `BlocklistAIContextModel` only after an active source has been cancelled and its server token has been accepted. Guard failures and missing-token failures leave the shared context attachments untouched; an active missing-token source is still cancelled before the error is returned.

### `commit_handoff`

`commit_handoff` is a normal function that consumes `PendingHandoff`, takes `&AppContext`, and synchronously revalidates current handoff enablement, valid environments, and model cloud compatibility. It returns an owned async future that does not borrow `AppContext`; invalid state resolves that future to the original pending state without external work, while valid state starts committed execution.

Committed execution exclusively owns:

1. Server conversation fork or fresh-launch selection.
2. Optional `materialize_handoff_target`.
3. Parent plus terminal-child safe-path collection.
4. Snapshot derivation/upload.
5. Non-fatal snapshot degradation.
6. Empty-prompt substitution.
7. Final `SpawnAgentRequest` construction.
8. `AIClient::spawn_agent`.
9. Shared startup classification and created outcome with task ID, run ID, and URL.

Use module-private staged values for forked, snapshot-settled, and spawn-ready data so callers cannot reorder the pipeline.

`materialize_handoff_target` is the only frontend callback. It runs for both forked and fresh launches after server fork selection and before snapshot/spawn. The GUI callback creates the target pane, restores the local fork when one exists, and binds the server token for forked handoffs.

Use owned clients, handles, and data across async boundaries. Never hold app/view/model contexts or terminal-model guards across an await.

### Orchestration-aware handoff

- Resolve loaded descendants through shared topology/history APIs.
- Block before parent cancellation when any loaded child is in-progress or blocked.
- Permit terminal child states.
- Fork only the selected conversation.
- Mark participating sources with `orchestration_handoff`.
- Include safe write paths and working directories from all terminal loaded descendants in snapshot collection.

Do not reproduce the orchestration tree in cloud.

### GUI migration

- Preserve GUI `&`, chip, slash command, environment modal, pane targeting, and rendering.
- Replace workspace/model-owned policy and sequencing with `prepare_handoff` and `commit_handoff`.
- Move prompt substitution and snapshot outcome semantics out of `AmbientAgentViewModel`.
- Supply `materialize_handoff_target` from `Workspace`.
- Split post-spawn polling into a helper that monitors an already-created task; keep `spawn_task` as a wrapper for unaffected callers.
- Feed the created outcome back into the GUI model and start its existing session-attachment monitoring.

No independent GUI-only fork/snapshot/request policy should remain after this PR.

### Telemetry and exports

- Add a GUI/TUI surface dimension to `AmbientAgent.Handoff.Initiated`; PR 2 emits GUI.
- Keep `HandoffSnapshotPrepared` semantics.
- Keep startup error classification and run URL construction shared.
- Re-export the pipeline and required plain data through `app/src/tui_export.rs` for the future TUI caller, without importing TUI types into `app`.

## Testing and validation

Add app-layer tests with fake clients/dependencies:

- Empty-source branching and cancellation order.
- Long-running and active/blocked-child guardrails before cancellation.
- Attachment transfer and exactly-once restoration.
- Environment/model defaults and validation.
- Fork versus fresh launch.
- Parent/terminal-child safe-path collection.
- Snapshot success, empty workspace, and silent failure degradation.
- Complete empty-prompt substitution matrix.
- Request fields, privacy flag, orchestration marker, and exactly one spawn call.
- Duplicate commit and stale completion protection.

Retain or migrate existing GUI tests in `ambient_agent/model_tests.rs`, `handoff/touched_repos_tests.rs`, and `workspace/auto_handoff_tests.rs`. Add parity assertions for GUI request construction and prompt behavior.

Run:

- `./script/format`
- Focused `cargo nextest run -p warp` tests.
- GUI handoff regression tests.
- The applicable app clippy command from `./script/presubmit`.

## Risks and mitigations

- Keep GUI migration isolated from the TUI product surface so parity can be reviewed independently.
- Preserve GUI compose/pane state outside the shared domain layer.
- Consume pending state on commit and gate late callbacks to prevent duplicate runs.
- Treat snapshot failure as a settled no-token result.
- Avoid logging prompts, paths, image contents, or environment secrets.

## Parallelization

Do not parallelize implementation. The pipeline API, GUI migration, polling split, and parity tests should evolve together before a second frontend becomes a caller.
