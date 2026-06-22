# APP-4460: Suppress Cloud Mode setup input sync during follow-up setup commands
## Context
APP-4460 covers Cloud Mode follow-up executions that run setup commands before the next agent exchange. The desired behavior is to keep the follow-up input visible and interactive for the viewer, but prevent the ambient-agent sharer's setup-command text from syncing into the viewer's input while that setup phase is active.
Cloud Mode panes are backed by `AmbientAgentViewModel`. Initial runs call `spawn_internal`, set `Status::WaitingForSession { kind: InitialRun }`, and emit `DispatchedAgent` in `app/src/terminal/view/ambient_agent/model.rs (1224-1248)`. Follow-ups call `submit_cloud_followup`, set `pending_followup_prompt`, transition to `Status::WaitingForSession { kind: Followup }`, and emit `FollowupDispatched` in `app/src/terminal/view/ambient_agent/model.rs (898-923)`. `TerminalView::handle_ambient_agent_event` responds to `FollowupDispatched` by starting a new setup-command group when `CloudModeSetupV2` is enabled in `app/src/terminal/view/ambient_agent/view_impl.rs (168-176)`.
Setup-command visibility and lifetime live in `SetupCommandState` in `app/src/terminal/view/ambient_agent/block/setup_command_text.rs (23-91)`. The state has a current group, tracks whether that group has executed at least one setup command, and records the currently running group. `maybe_insert_setup_command_blocks` marks `did_execute_a_setup_command` when the first startup command block appears and inserts the setup summary and per-command rich content in `app/src/terminal/view/ambient_agent/view_impl.rs (373-453)`. The group is finished and collapsed when the first Oz exchange arrives or a third-party harness command starts.
Viewer input updates from the ambient sharer arrive through the shared-session CRDT path. `NetworkEvent::InputUpdated` is handled in `app/src/terminal/shared_session/viewer/terminal_manager.rs`, and previously applied remote operations directly with `Input::process_remote_edits`. There is already broad startup suppression for `ambient_agent::is_cloud_agent_pre_first_exchange`, but follow-up setup groups can continue after the viewer has an ambient model and visible input, so the setup command text can still be applied to the viewer input unless the receiver suppresses it.
## Proposed changes
Keep the input visibility behavior unchanged for Cloud Mode setup-v2 follow-ups: the viewer input should remain visible and interactive while setup commands run.
Add a narrow setup-v2 sync predicate on `TerminalView` that checks:
- `FeatureFlag::CloudModeSetupV2` is enabled;
- the pane has an `AmbientAgentViewModel`;
- the ambient model's current setup-command group is a non-initial follow-up group;
- that current group is still running.
Derive input-sync suppression from the running follow-up setup group rather than from whether a setup command block has rendered, so the first setup command is suppressed even if its shared-session input update arrives before `maybe_insert_setup_command_blocks` marks the group as having executed a command.
Route viewer shared-session input updates through `TerminalView::apply_viewer_shared_session_input_update`. That method should return without applying CRDT operations while the narrow predicate is true, and otherwise call `Input::process_remote_edits` as before. This receiver-side suppression keeps local viewer drafts intact during the follow-up setup phase and automatically resumes normal shared-session input sync once the setup group finishes.
Do not change setup-command rendering, setup-command grouping, follow-up submission routing, tombstone behavior, or broad pre-first-exchange suppression. The fix is specific to shared-session input editor updates from the ambient-agent sharer during an active follow-up setup-command group.
## Testing and validation
Add unit coverage next to existing Cloud Mode terminal-view tests. The key regression test should:
- enable `AgentView`, `CloudMode`, `CloudModeSetupV2`, and `HandoffCloudCloud`;
- create a Cloud Mode terminal;
- seed an existing ambient task with `enter_viewing_existing_session`;
- start a follow-up setup-command group by handling `FollowupDispatched`;
- assert `is_input_box_visible` remains `true`;
- assert incoming shared-session input operations are not applied while the setup group is running, including before the first setup command block has rendered;
- finish the setup group and assert incoming shared-session input operations apply again.
Run the focused test by name with `cargo nextest run -p warp -E 'test(<new test name>)'`. Run `cargo fmt --manifest-path /workspace/warp/Cargo.toml -p warp --check` and a targeted `cargo clippy --manifest-path /workspace/warp/Cargo.toml -p warp --tests -- -D warnings` before updating the PR.
## Parallelization
Parallelization is not beneficial for this revision. The implementation is a small, tightly coupled change across one setup-state helper, one shared-session input receiver path, and one regression test; splitting it across agents would add coordination overhead and merge risk without reducing wall-clock time.
