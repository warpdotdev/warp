# REMOTE-1674: Tech Spec — Agent view entry for web-started 3p harness shared sessions

## Context
REMOTE-1674 makes live shared-session viewers enter agent view for third-party harness conversations started outside the Warp desktop flow, such as from the web. This extends the REMOTE-1459 viewer-entry work to cases where the desktop viewer may not have an `AmbientAgentViewModel` with a resolved harness yet.
Linear: https://linear.app/warpdotdev/issue/REMOTE-1674/make-sure-that-we-enter-the-agent-view-for-3p-harness-conversations
Current relevant flow:
- `app/src/terminal/shared_session/shared_handlers.rs (356-435)` applies remote `CLIAgentSessionState`. It creates a pane-scoped `CLIAgentSession`, shows the CLI agent footer, manages rich input, and now has enough signal to infer a 3p harness even without an ambient model.
- `app/src/terminal/shared_session/viewer/event_loop.rs (184-259)` processes ordered viewer terminal events. `CommandExecutionStarted` inserts the remote command into the viewer terminal model, then the viewer can sync agent view around that newly started block.
- `app/src/terminal/shared_session/viewer/terminal_manager.rs (686-766)` applies the initial `universal_developer_input_context` before calling `enter_viewing_existing_session`, so CLI-agent state can arrive before the ambient model has the task harness.
- `app/src/terminal/view/ambient_agent/model.rs (816-861)` fetches the ambient task, reads `agent_config_snapshot.harness`, calls `set_harness`, and emits `ViewerHarnessUpdated` after the server-resolved harness is applied.
- `app/src/terminal/view/ambient_agent/view_impl.rs (291-315)` handles `ViewerHarnessUpdated` by syncing agent view, updating pane configuration, and notifying the view.
- `app/src/terminal/view/ambient_agent/view_impl.rs (489-565)` owns the shared 3p viewer sync helper. It checks `is_third_party_cloud_agent_viewer`, enters agent view with `AgentViewEntryOrigin::ThirdPartyCloudAgent` only if not already active, then retags existing blocks and rich content onto the vehicle conversation.
- `app/src/terminal/model/terminal_model.rs (1426-1444)` identifies shared ambient-agent sessions via `SessionSourceType::AmbientAgent`.
- `app/src/terminal/cli_agent_sessions/mod.rs (289-315)` stores pane-scoped CLI agent sessions. A session on a shared ambient viewer is the fallback signal for a web-started 3p harness.
- `app/src/terminal/model/block.rs (1405-1435)` hides blocks in fullscreen agent view unless their `AgentViewVisibility` includes the active conversation.
- `app/src/terminal/model/blocks.rs (1586-1621)` tags the current unfinished block when agent view activates, `app/src/terminal/model/blocks.rs (1666-1694)` attaches existing non-startup blocks to a conversation, and `app/src/terminal/model/blocks.rs (2545-2588)` tags newly-created blocks with the active conversation.
- `app/src/terminal/view/agent_view.rs (374-396)` retags rich content with an agent-view conversation id.

## Implementation
Shared 3p viewer entry is centralized in `TerminalView::sync_agent_view_for_shared_third_party_viewer`.
The helper:
- Detect a 3p harness with `is_third_party_cloud_agent_viewer`. This checks both `AmbientAgentViewModel::is_third_party_harness()` and `CLIAgentSessionsModel::session(view_id).is_some()` to determine if we are in a 3p harness convo.
The former gets set when we receive task data from the server, the latter is set when we receive join info in a shared session that contains an actively running CLIAgent.
- Require `is_shared_ambient_agent_session()` so local harness picker changes do not enter agent view.
- Enter agent view only when not already active, using `AgentViewEntryOrigin::ThirdPartyCloudAgent`.
- Return immediately with the active conversation id if the viewer is already in agent view. This keeps duplicate sync calls cheap.
- Return the active vehicle conversation id for tests and idempotency checks.
- On first entry, retag non-startup terminal blocks via `attach_non_startup_blocks_to_conversation`.
- On first entry, retag terminal-mode rich content with no conversation id via `set_rich_content_agent_view_conversation_id`.

Call the helper from three signal points:
- `ViewerHarnessUpdated` in `app/src/terminal/view/ambient_agent/view_impl.rs (305-315)` for the desktop live-viewer path where the ambient model resolves the harness asynchronously. This event fires after `enter_viewing_existing_session` fetches task config, even when `set_harness` is a no-op because the local harness already matches the server harness.
- `apply_cli_agent_state_update` in `app/src/terminal/shared_session/shared_handlers.rs (356-435)` for web-started or otherwise pre-harness viewer paths where the remote side broadcasts a CLI agent session.
- `CommandExecutionStarted` in `app/src/terminal/shared_session/viewer/event_loop.rs (184-259)` after the viewer model starts the block, so late-arriving command blocks are attached to the vehicle conversation immediately.

This keeps the implementation local to viewer synchronization. It does not change transcript viewer entry, Oz harness behavior, CLI agent serialization, or shared-session protocol fields.

## Testing and validation
Unit tests in `app/src/terminal/view_tests.rs (922-1139)` cover the important invariants:
- `shared_third_party_viewer_sync_enters_agent_view_and_retags_existing_block` verifies a shared ambient viewer with an ambient model and `Harness::Claude` enters agent view, is idempotent, uses `ThirdPartyCloudAgent`, and keeps the existing harness block visible.
- `shared_third_party_viewer_syncs_from_viewer_harness_updated_when_harness_unchanged` verifies `ViewerHarnessUpdated` enters agent view and retags the harness block even when `set_harness` does not emit `HarnessSelected` because the harness is unchanged.
- `shared_third_party_viewer_syncs_from_cli_agent_state_without_ambient_model` verifies a plain terminal viewer with shared ambient session metadata enters agent view after `apply_cli_agent_state_update`, even with no ambient model.
Additional validation:
- `cargo test -p warp shared_third_party_viewer_sync --lib` passes.
- Manually start a Claude/Codex/Gemini cloud task from web, open the desktop shared-session viewer, and confirm it lands in agent view with the harness block visible.
- Repeat a desktop-started 3p shared-session viewer to ensure the `ViewerHarnessUpdated` path works.
- Repeat an Oz shared-session viewer to confirm no new 3p sync path activates.
- Confirm a non-shared local harness picker change does not enter agent view.
## Risks and mitigations
- `CLIAgentSessionsModel::session(view_id).is_some()` is broader than an explicit harness enum. The shared ambient-session guard and `AgentHarness` flag keep the fallback scoped to 3p shared viewer contexts.
- `apply_cli_agent_state_update` can run before `enter_viewing_existing_session` finishes fetching task config and setting `AmbientAgentViewModel.harness`. The CLI-session fallback covers that race.
- `set_harness` intentionally no-ops when the harness is already selected. `ViewerHarnessUpdated` is the viewer-specific signal that still runs shared-viewer sync after server task hydration.
- Sync can run before and after command blocks exist. First entry retags existing blocks. Later calls are cheap if agent view is already active, and subsequent blocks inherit the active conversation from `BlockList::create_new_block`.
- Fullscreen agent view filtering is strict. Tests assert both `should_hide_block` and `AgentViewVisibility::Terminal.conversation_ids` so regressions are visible.
