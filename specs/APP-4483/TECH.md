# Cloud agent tombstone and followup input behavior — Tech Spec
Product spec: `specs/APP-4483/PRODUCT.md`
## Context
`PRODUCT.md` defines behavior invariants B1–B6. This technical spec implements those invariants when `FeatureFlag::HandoffCloudCloud` is enabled, while preserving existing behavior when that flag is disabled.
The current UI decision points are split across three places:
- `app/src/terminal/view/shared_session/view_impl.rs:729` computes `viewed_ambient_task_id_owned_by_current_user` and uses task creator ownership to decide whether `on_session_share_ended` inserts a tombstone.
- `app/src/terminal/view/shared_session/view_impl.rs:779` enables the followup input only when the current user owns the ambient task.
- `app/src/terminal/view/shared_session/view_impl.rs:811` repeats the same ownership model for `handle_non_running_ambient_agent_task`.
There is a second delayed path for already-loaded ambient tasks:
- `app/src/terminal/view.rs:7123` checks whether a non-running shared ambient task should get a tombstone.
- `app/src/terminal/view.rs:7149` again uses `owned_ambient_agent_task_id` plus `HandoffCloudCloud` to decide whether to show the input instead.
The tombstone currently makes an independent CTA decision:
- `app/src/terminal/view/shared_session/conversation_ended_tombstone_view.rs:207` creates a `Continue` cloud button whenever a task id exists and `HandoffCloudCloud` is enabled.
- `app/src/terminal/view/shared_session/conversation_ended_tombstone_view.rs:221` creates a `Continue locally` button whenever a conversation id exists.
- `app/src/terminal/view/shared_session/conversation_ended_tombstone_view.rs:473` hides local continuation for known non-Oz harnesses, but treats unknown harness metadata as local-continuable.
The data needed for the new product model is already present:
- `ServerAIConversationMetadata` contains `harness` and `permissions` in `app/src/ai/agent/conversation.rs:3849`.
- `AIAgentHarness` distinguishes Oz, Claude Code, Gemini, Codex, and Unknown in `app/src/ai/agent/conversation.rs:3821`.
- `BlocklistAIHistoryModel::get_server_conversation_metadata` already looks up loaded conversation metadata with a fallback to cached conversation metadata in `app/src/ai/blocklist/history_model.rs:1986`.
- `AgentConversationsModel::fetch_ambient_agent_tasks_and_cloud_convo_metadata` fetches ambient tasks and cloud conversation metadata together, including additional metadata for task conversation IDs missing from the first metadata response, in `app/src/ai/agent_conversations_model.rs:675`.
- `AmbientAgentTask` exposes `conversation_id`, active execution state, and whether cloud followup submission is allowed in `app/src/ai/ambient_agents/task.rs:317`.
- `SharingAccessLevel` is ordered as View < Edit < Full in `crates/warp_server_client/src/drive/sharing.rs:8`.
There is an object-access precedent, but it is tied to loaded Warp Drive objects:
- `CloudViewModel::access_level` defaults missing objects to view access in `app/src/cloud_object/model/view.rs:173`.
- `CloudViewModel::object_access_level` grants full access for personal/team-space objects, applies link and guest ACLs for shared-space objects, and upgrades creator access to edit in `app/src/cloud_object/model/view.rs:181`.
The APP-4483 implementation should reuse the same permission semantics where possible, but must operate directly on `ServerAIConversationMetadata.permissions` because AI conversation metadata is not a `CloudObject`.
## Proposed changes
### 1. Add a single continuation UI state resolver
Add a small helper module under `app/src/terminal/view/shared_session/`, for example `cloud_conversation_continuation.rs`, and use it from all tombstone/followup decision paths.
Suggested core types:
- `ContinuationHarness`: `Oz`, `ThirdParty`, or `Unknown`.
- `ConversationAccess`: `Edit`, `ViewOnly`, or `Unknown`.
- `TombstoneCta`: `ContinueLocally`, `ContinueInCloud { task_id }`, or none.
- `CloudConversationContinuationUiState`, containing:
  - whether the ended-state tombstone should be present;
  - whether the inline cloud followup input should be enabled;
  - the task id to use for cloud followup input submission, if any;
  - the tombstone CTA to render, if any.
The resolver should accept the terminal view id, optional ambient task id, whether a live shared session is still active, and `AppContext`. It should derive:
- active execution from the ambient task when present;
- harness from `ServerAIConversationMetadata.harness`, not from tombstone display metadata;
- edit access from `ServerAIConversationMetadata.permissions`;
- unknown state when task or conversation metadata is unavailable.
Mapping:
- B1: Oz + edit + no active execution returns no tombstone and followup input enabled for that task.
- B2: Oz + view-only + no active execution returns tombstone with `ContinueLocally`.
- B3: third-party + edit + no active execution returns tombstone with `ContinueInCloud`.
- B4: third-party + view-only + no active execution returns tombstone with no CTA.
- B5: any harness/access + active execution or live shared session returns no ended-state tombstone and no followup input transition.
- B6: unknown harness or unknown access + no active execution returns tombstone with no CTA.
When `FeatureFlag::HandoffCloudCloud` is disabled, keep the existing pre-APP-4483 behavior path: ended ambient sessions may show the tombstone, but the permission-aware cloud followup state should not be used.
### 2. Resolve conversation metadata by server token
Add a `BlocklistAIHistoryModel` helper that returns `ServerAIConversationMetadata` by `ServerConversationToken`, for example:
- check `server_token_to_conversation_id`;
- if found, reuse `get_server_conversation_metadata`;
- otherwise scan `all_conversations_metadata` for matching `server_conversation_token`;
- return `None` if metadata is not loaded.
This avoids forcing callers to manufacture or resolve an `AIConversationId` before they can inspect task-linked conversation permissions. `AmbientAgentTask::conversation_id` returns the token string at `app/src/ai/ambient_agents/task.rs:317`, so the resolver can construct a `ServerConversationToken` from that value and ask the history model for metadata.
If metadata is missing, do not fetch synchronously from the UI resolver. Treat the state as B6 and let the existing `AgentConversationsModel` fetch path populate metadata asynchronously.
### 3. Compute edit access from `ServerPermissions`
Add a pure helper near the resolver or in a small permission utility that computes the current user's effective `SharingAccessLevel` from `ServerAIConversationMetadata`.
Rules:
- If the current user is missing or logged out, start at `SharingAccessLevel::View`.
- If `permissions.space` is `Owner::User` for the current user, return at least `Full`.
- If `permissions.space` is `Owner::Team` and the team appears in `UserWorkspaces::team_from_uid_across_all_workspaces`, return at least `Full`.
- Apply `permissions.anyone_link_sharing` as a baseline when present.
- Apply user guest ACLs when `ServerGuestSubject::User { firebase_uid }` matches the current user.
- Apply team guest ACLs when the current user belongs to the guest team according to `UserWorkspaces`.
- Ignore pending-user ACLs for this UI decision.
- If `metadata.creator_uid` matches the current user, upgrade to at least `Edit`, matching the creator fallback in `CloudViewModel::object_access_level`.
- Return `ConversationAccess::Edit` for `Edit` or `Full`; return `ViewOnly` for `View`.
If team membership data is not loaded and access is only knowable through a team owner/guest ACL, do not assume edit access. The safe UI state remains view-only/unknown until workspace metadata is available.
### 4. Replace ownership-based UI branching
Replace the creator-owned task gate in these call sites with the resolver:
- `on_session_share_ended` in `app/src/terminal/view/shared_session/view_impl.rs:727`.
- `handle_non_running_ambient_agent_task` in `app/src/terminal/view/shared_session/view_impl.rs:811`.
- `maybe_insert_tombstone_for_non_running_shared_ambient_task` in `app/src/terminal/view.rs:7123`.
Add a shared method on `TerminalView`, for example `refresh_non_running_cloud_agent_continuation_ui`, that:
- exits early for disabled `CloudModeSetupV2`, active replay, existing pending cloud followup submission, or disabled `HandoffCloudCloud`;
- asks the resolver for the current state;
- removes an existing tombstone when the new state is B1 and enables the cloud followup input;
- inserts or updates the tombstone when the new state is B2, B3, B4, or B6;
- keeps input selectable/read-only for viewers when no continuation input should be shown.
`enable_owned_cloud_followup_input` should be renamed or wrapped with a permission-neutral name, such as `enable_cloud_followup_input`, because it will now be used for users with edit access who may not be the task creator.
Update `try_submit_pending_cloud_followup` in `app/src/terminal/view.rs:20026` so it does not fall back to `owned_ambient_agent_task_id`. Submission should use an explicit `pending_cloud_followup_task_id` or a task id that the resolver stored when it enabled the inline input. This prevents creator ownership from remaining a hidden permission bypass.
### 5. Make tombstone CTAs data-driven
Change `ConversationEndedTombstoneView::new` to accept a CTA decision from the resolver rather than constructing both buttons from `task_id` and `conversation_id`.
The tombstone should render:
- `Continue locally` only for `TombstoneCta::ContinueLocally`.
- `Continue` only for `TombstoneCta::ContinueInCloud { task_id }`.
- no button when the CTA is absent.
Keep `ConversationEndedTombstoneEvent::ContinueInCloud` and `start_cloud_followup_from_tombstone` for B3. Keep `ContinueLocally` behavior for B2. Keep `TombstoneDisplayData::enrich_from_task` for display metadata only; it should no longer decide CTA visibility after an async task fetch.
This removes the current mismatch where `Continue` is shown for any task id and `Continue locally` is shown for unknown harness metadata.
### 6. Recompute when metadata changes
Permissions and metadata can arrive after the tombstone is first inserted. Recompute the continuation UI state when:
- `AgentConversationsModelEvent::TasksUpdated` updates task active-execution state or task conversation id;
- `AgentConversationsModelEvent::ConversationsLoaded` merges cloud conversation metadata;
- `BlocklistAIHistoryEvent::UpdatedConversationMetadata` updates server metadata for a live conversation.
If recomputation transitions:
- from B6 to B1, remove the tombstone and enable the input;
- from B6 to B2/B3/B4, update or reinsert the tombstone with the correct CTA;
- from B1 to a non-edit state, clear/disable the input and show the tombstone state for the latest access.
Use the resolver as the only source of truth for these transitions.
## Testing and validation
Add focused unit tests for the resolver and update existing shared-session tests so validation maps directly to `PRODUCT.md` B1–B6:
- B1: Oz metadata + edit access + ended execution produces no tombstone, editable followup input, and cloud submission uses the same task id even when task creator is someone else.
- B2: Oz metadata + view-only access produces a tombstone with `Continue locally`, no inline followup input, and no cloud CTA.
- B3: Claude Code/Codex/Gemini metadata + edit access produces a tombstone with `Continue`, and clicking it enters the existing cloud followup flow.
- B4: third-party metadata + view-only access produces a tombstone with no continue CTA.
- B5: active execution or live shared session does not insert an ended-state tombstone regardless of harness/access.
- B6: missing metadata, unknown harness, or unknown access produces a tombstone with no mutation CTA.
Permission helper tests should cover:
- user owner;
- team owner with current-user team membership;
- user guest view vs edit;
- team guest view vs edit;
- link sharing view only;
- creator fallback to edit;
- missing current user defaults to non-edit.
Update or replace creator-based assertions in `app/src/terminal/view/shared_session/view_impl_tests.rs`, especially the tests currently named around “owned” ambient sessions. Add tombstone CTA tests in `app/src/terminal/view/shared_session/conversation_ended_tombstone_view_tests.rs` once CTA state is data-driven.
Suggested targeted commands:
- `cargo test -p warp --lib terminal::view::shared_session::view_impl_tests`
- `cargo test -p warp --lib terminal::view::shared_session::conversation_ended_tombstone_view_tests`
- `cargo test -p warp --lib terminal::view::shared_session::cloud_conversation_continuation`
Before PR/update, run the repository-required format and clippy checks from the PR workflow. Do not use `cargo fmt --all` or file-specific `cargo fmt`.
## Parallelization
Do not split this implementation across sub-agents. The changes are tightly coupled across one UI state resolver, terminal view lifecycle transitions, tombstone CTA rendering, and existing shared-session tests. Parallel edits would likely touch the same files and increase merge overhead more than they reduce wall-clock time.
