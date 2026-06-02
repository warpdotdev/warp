# Cloud-to-cloud handoff resumable completed cloud panes
## Context
`PRODUCT.md` defines the target behavior: completed cloud conversations should restore into Cloud Mode panes seeded with prior conversation history, keep the completed transcript/tombstone presentation, and become resumable in cloud mode when cloud-to-cloud handoff is available.
The follow-up execution path mostly exists. `ConversationEndedTombstoneView` creates a cloud `Continue` action when it has an ambient task id and `FeatureFlag::HandoffCloudCloud` is enabled in `app/src/terminal/view/shared_session/conversation_ended_tombstone_view.rs (202-235)`, then emits `ConversationEndedTombstoneEvent::ContinueInCloud` from `app/src/terminal/view/shared_session/conversation_ended_tombstone_view.rs (631-640)`. `TerminalView::start_cloud_followup_from_tombstone` sets `pending_cloud_followup_task_id` and focuses the existing input in `app/src/terminal/view/shared_session/view_impl.rs (810-842)`. `TerminalView::try_submit_pending_cloud_followup` routes the next prompt through `AmbientAgentViewModel::submit_cloud_followup` in `app/src/terminal/view.rs (19979-20075)`. The ambient model emits `FollowupSessionReady` after polling for a fresh session in `app/src/terminal/view/ambient_agent/model.rs (686-744)`, and the deferred Cloud Mode manager attaches it through `TerminalManager::attach_followup_session` in `app/src/terminal/view/ambient_agent/mod.rs (60-118)` and `app/src/terminal/shared_session/viewer/terminal_manager.rs (310-384)`.
The follow-up submit trigger should be explicit Cloud Mode behavior, not a side effect of shared-session viewer submission. Fresh Cloud Mode prompts already bypass `Input::submit_ai_query` while the ambient model is in `Composing` and call `AmbientAgentViewModel::spawn_agent` directly. Follow-up prompts should follow the same pattern: before generic AI submission and before generic shared-session viewer permission checks, `Input` should detect a disconnected Cloud Mode follow-up composer and emit a dedicated Cloud follow-up event handled by `TerminalView::try_submit_pending_cloud_followup`. Both cases must use this same path: a pane whose initial cloud execution just ended, and a pane restored from ambient conversation history.
Disconnected Cloud Mode panes should also stop representing themselves as shared-session viewers. `SharedSessionStatus::ViewPending`, `ActiveViewer`, and `FinishedViewer` should mean a live or ended collaborative viewer state with shared-session permission semantics. A fresh, restored, or between-executions Cloud Mode pane that has no attached session should instead use `SharedSessionStatus::NotShared` plus the existing Cloud Mode/ambient model state. `connect_session` and `attach_followup_session` should transition into `ViewPending` only when a real shared-session id is available.
The broken path is pane construction. `TerminalView::new` creates `ambient_agent_view_model` only when `is_cloud_mode` is true in `app/src/terminal/view.rs (3003-3030)`. Historical transcript panes are currently created through `MockTerminalManager::create_model` in `app/src/pane_group/mod.rs (5675-5728)` and loaded through `PaneGroup::load_data_into_transcript_viewer` in `app/src/pane_group/mod.rs (3825-3928)`. Those panes can have `ConversationTranscriptViewerStatus::ViewingAmbientConversation(task_id)`, but that is a workaround for treating a generic transcript viewer like an ambient session. It still does not give the pane the ambient model or deferred viewer manager needed to submit and attach a cloud follow-up.
`WorkspaceAction::OpenConversationTranscriptViewer` already carries `ambient_agent_task_id`, but the action handler discards it and calls `load_cloud_conversation_into_new_transcript_viewer(conversation_id, ctx)` in `app/src/workspace/view.rs (21677-21698)`. The loader then creates a generic transcript-loading tab in `app/src/workspace/view.rs (3998-4054)`. By the time cloud conversation data is loaded, `PaneGroup` can recover an ambient task id from metadata, but it still mutates a non-cloud transcript viewer rather than creating a resumable cloud viewer.
The desired invariant is that viewing any ambient agent conversation, including a restored completed one, puts the user in a Cloud Mode pane. The fetched conversation history is seeded UI state for scrollback and completed-tombstone presentation, not the server-side agent context for the next follow-up request. Generic transcript-viewer state should remain only for non-ambient conversations and loading/fallback paths.
## Proposed changes
### Preserve ambient task identity through restore loading
Change `Workspace::load_cloud_conversation_into_new_transcript_viewer` to accept `ambient_agent_task_id: Option<AmbientAgentTaskId>`, and pass the action’s task id through from `WorkspaceAction::OpenConversationTranscriptViewer`.
When `load_conversation_from_server` returns `CloudConversationData`, resolve the effective task id as:
1. the task id from the action, if present;
2. the task id from cloud conversation metadata, if present;
3. `None`.
Use the effective task id to choose the pane construction path, seed the ambient model, register active ambient views, and decide whether the tombstone can offer cloud `Continue`.
### Route ambient restores into Cloud Mode pane construction
Add a PaneGroup helper that creates a restored ambient Cloud Mode pane when all of these are true:
- `FeatureFlag::HandoffCloudCloud` is enabled;
- an effective ambient task id exists;
- the loaded conversation is a cloud/ambient conversation, not a purely local transcript.
A concrete shape:
- `PaneGroup::create_restored_ambient_cloud_mode_pane(conversation, task_id, resources, initial_size, ctx)`
- `PaneGroup::replace_loading_pane_with_restored_ambient_cloud_mode_pane(loading_pane_id, cloud_conversation, task_id, ctx)`
- or a branch in the cloud-conversation loading callback that replaces the loading pane with a Cloud Mode pane instead of calling the generic transcript restoration path.
The helper should use `terminal::view::ambient_agent::create_cloud_mode_view` or the existing `PaneGroup::create_cloud_mode_terminal` wrapper, then restore the fetched conversation history into that view. This gives the view `ambient_agent_view_model`, the `FollowupSessionReady` subscription, and a deferred viewer manager that can later call `attach_followup_session`.
Do not create a `MockTerminalManager` transcript viewer and then retrofit Cloud Mode behavior onto it. Also do not set `ConversationTranscriptViewerStatus::ViewingAmbientConversation(task_id)` for the new path. The pane should be ambient because it is a Cloud Mode pane, not because a transcript-viewer marker carries a task id.
### Seed the Cloud Mode pane with historical UI state
After creating the restored Cloud Mode pane:
- restore the fetched conversation into the terminal view so prior blocks and rich content render as historical UI state;
- insert the completed-conversation tombstone at the end of the restored history;
- call `AmbientAgentViewModel::enter_viewing_existing_session(task_id, ctx)` so the model stores the stable task id and fetches run config metadata;
- if the loaded conversation has a local `AIConversationId`, call `set_conversation_id(Some(id))`;
- register `ActiveAgentViewsModel::register_ambient_session(terminal_view.id(), task_id, ctx)`;
- enter the same agent-view/header/details state as a normal Cloud Mode pane for that task.
This should mirror existing remote-child restoration in `app/src/pane_group/mod.rs (3161-3258)` and the existing ambient Cloud Mode construction path in `app/src/pane_group/mod.rs (3249-3276)`. The restored history is only the pane’s initial UI state. `submit_cloud_followup` should still send the follow-up prompt and task/session identity through the existing cloud-to-cloud follow-up API rather than replaying the transcript as prompt context.
If the current `AmbientAgentViewModel::Status::AgentRunning` is too strong for a completed task with no live session, introduce or reuse a between-executions state that represents “viewing an existing ambient task with no active session.” The important behavior is that the pane looks like the completed Cloud Mode state produced after a fresh Cloud Mode execution ends and the VM/session is no longer active.
When an ambient shared session ends, owner panes should transition back to this disconnected Cloud Mode follow-up composer state and use `SharedSessionStatus::NotShared`. Non-owner panes should remain read-only ended viewer surfaces and must not expose editable Cloud follow-up input.
### Use one Cloud follow-up submission path
Do not maintain separate follow-up submission paths based on whether the pane was previously attached to a shared session. The post-session-ended case and restored-from-history case should converge before submission:
- the pane is in disconnected Cloud Mode state;
- it has an ambient task id in `AmbientAgentViewModel`;
- the next non-empty prompt is submitted through `TerminalView::try_submit_pending_cloud_followup`;
- `try_submit_pending_cloud_followup` calls `AmbientAgentViewModel::submit_cloud_followup`;
- the ambient model emits `FollowupDispatched` and later `FollowupSessionReady`;
- the deferred Cloud Mode manager attaches the fresh shared session through `attach_followup_session`.
`Input::submit_ai_query` should no longer be the mechanism that routes follow-up prompts through `submit_viewer_ai_query` and `InputEvent::SendAgentPrompt`. That path is still appropriate for a true live shared-session viewer sending a prompt to the sharer, but it is not the semantic model for starting a new cloud follow-up execution.
### Stop using ambient transcript-viewer status as an ambient identity source
Remove `ConversationTranscriptViewerStatus::ViewingAmbientConversation` from the new design. If the variant can be deleted cleanly, replace its call sites with Cloud Mode predicates:
- `TerminalModel::ambient_agent_task_id` should derive ambient identity from `shared_session_source_type` and/or the `AmbientAgentViewModel`, not from transcript-viewer status.
- `TerminalPane::snapshot` should snapshot restored ambient panes as `LeafContents::AmbientAgent`, because they are Cloud Mode panes.
- tab and pane-header ambient indicators should use `is_shared_ambient_agent_session`, `TerminalView::ambient_agent_view_model`, or another explicit Cloud Mode ambient predicate instead of checking transcript-viewer status.
- transcript share-link behavior should remain on true transcript viewers; restored ambient panes should use the Cloud Mode/shared-session share/open behavior available while idle between executions.
It is acceptable to keep `ConversationTranscriptViewerStatus::Loading` and `ViewingLocalConversation` for generic local/non-ambient transcript viewers. The key invariant is that ambient restored conversations do not become read-only because they are marked as transcript viewers.
### Keep generic transcript behavior as the fallback
Continue using the current `MockTerminalManager` transcript path when:
- `HandoffCloudCloud` is disabled;
- no effective task id exists;
- the conversation data is not cloud/ambient;
- the pane is opened on a platform or flow that cannot continue cloud runs.
This preserves `PRODUCT.md` invariants 22-24 and keeps the fix scoped to resumable cloud tasks.
### Align tombstone capability with pane capability
`ConversationEndedTombstoneView::new` currently infers cloud Continue eligibility from task id plus feature flag. Prefer adding a capability argument from `TerminalView::insert_conversation_ended_tombstone`, for example `CloudContinueCapability::Available(AmbientAgentTaskId)` vs `Unavailable`, so the button reflects whether the surrounding view can actually submit a cloud follow-up.
If restored ambient panes are always Cloud Mode panes with an ambient model, the first implementation can keep the existing constructor logic and add a regression test. The capability argument is safer because it prevents a stale task id on a generic transcript viewer from showing a cloud `Continue` action that can only fall back to the generic toast.
## Testing and validation
Map tests to `PRODUCT.md` behavior invariants instead of duplicating product requirements:
- Invariants 1-5 and 22-24: add PaneGroup/workspace tests that opening a completed cloud task with `HandoffCloudCloud` enabled creates a Cloud Mode pane with an `AmbientAgentViewModel` and no ambient transcript-viewer status, while disabled/no-task/local cases use the existing transcript path.
- Invariants 6-12 and 26-28: add terminal/shared-session tests that tombstone Continue in a restored ambient Cloud Mode pane sets `pending_cloud_followup_task_id`, focuses input, submits through `AmbientAgentViewModel`, and does not hit the generic toast path.
- Add an input/terminal test that a disconnected Cloud Mode follow-up prompt emits the dedicated Cloud follow-up event before generic shared-session viewer gating. Cover both restored panes and panes whose initial shared session just ended so they share the same submission path.
- Add a state test that fresh/restored/between-executions Cloud Mode panes with no live session use `SharedSessionStatus::NotShared`, then transition to `ViewPending` only when `connect_session` or `attach_followup_session` starts joining a real session.
- Invariants 13-17 and 25: add tests that the view remains registered for the same ambient task, repeated `FollowupSessionReady` attaches through the deferred manager, and follow-up completion returns the pane to a between-executions Cloud Mode tombstone state.
- Invariants 18-21: extend existing follow-up error tests to cover transcript-originated follow-ups, including prompt restoration before request acceptance and Cloud Mode error state after accepted-but-failed startup.
Targeted checks:
- `cargo nextest run -p warp terminal::view::shared_session::view_impl_test terminal::view_test`
- `cargo nextest run -p warp pane_group::mod_tests ai::agent_conversations_model_tests`
- `cargo check -p warp --features handoff_cloud_cloud`
Use the repo-standard formatting command when preparing the PR; do not run `cargo fmt --all` or file-specific `cargo fmt`.
## Risks and mitigations
### Duplicate transcript restoration
Creating a Cloud Mode view and then loading historical blocks could duplicate content if a follow-up session replays prior blocks. Keep follow-up joins on `SharedSessionInitialLoadMode::AppendFollowupScrollback` and rely on the existing block-id dedupe in `TerminalModel::append_followup_shared_session_scrollback`.
### Completed Cloud Mode state does not currently have a precise model status
`AmbientAgentViewModel::enter_viewing_existing_session` currently sets `Status::AgentRunning`, which may not accurately describe a completed task with no live session. If this causes incorrect setup/progress/footer behavior, add a between-executions/viewing-existing status instead of falling back to transcript-viewer state.
### Read-only transcript state blocks input
`TerminalModel::is_read_only` returns true when `conversation_transcript_viewer_status` is set in `app/src/terminal/model/terminal_model.rs (1534-1554)`. Restored ambient panes should avoid setting transcript-viewer status entirely so cloud follow-up input is not blocked. Generic transcript viewers can keep the existing read-only behavior.
### Manager/view mismatch
Retrofitting a `MockTerminalManager` transcript viewer with cloud follow-up subscriptions would duplicate Cloud Mode setup logic. Replace the loading pane with a deferred shared-session viewer manager for restored ambient conversations instead.
### Authorization and stale task state
The client may think a task is resumable when the server rejects follow-up creation or returns stale no-execution data. Treat the follow-up API as authoritative and surface errors through existing Cloud Mode error/auth/capacity/credits states.
## Parallelization
This fix is small enough to implement sequentially because the key change is one coherent pane-construction path. If split, one agent can own workspace/pane-group creation and restore behavior while another owns tombstone/input/read-only tests. They converge at the restored ambient Cloud Mode pane helper and should avoid simultaneous edits to the same `PaneGroup` creation functions.
