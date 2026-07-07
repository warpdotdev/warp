# Local-to-Cloud Handoff — Tech Spec
Product spec: `specs/REMOTE-1486/PRODUCT.md`
Linear: [REMOTE-1486](https://linear.app/warpdotdev/issue/REMOTE-1486)
Sub-specs (lower stack branches):
- `TOUCHED_WORKSPACE_TECH.md` — touched-workspace discovery (path extraction, repo grouping, env-overlap pick).
- `SNAPSHOT_UPLOAD_TECH.md` — handoff snapshot upload pipeline and the `SpawnAgentRequest` server-contract additions.
## Context
The product spec describes a chip + `/move-to-cloud` slash command that opens a split cloud-mode pane next to the local agent to hand off the active local Oz conversation to the cloud. The user types the follow-up prompt and submits inside the pane's existing cloud-mode input bar; the cloud agent runs in a fresh sandbox, gets a forked copy of the conversation history, and rehydrates from a workspace snapshot taken on the local machine.
The pieces this builds on already exist:
- **Cloud→cloud handoff and rehydration** (REMOTE-1290): `snapshots/{run_id}/{execution_id}/` GCS layout, the `<system-message>`-wrapped `UserQuery` rehydration prompt injected by `logic/ai/multi_agent/runtime/interceptors/input.go:433` via `ResolveHandoffRehydrationPrompt` in `../warp-server/logic/ai/ambient_agents/handoff_rehydration.go`. Server discovers snapshot files by GCS path convention (`ListSnapshotFiles` in `../warp-server/logic/ai/ambient_agents/attachment_storage.go:281`), no DB column needed.
- **End-of-run snapshot pipeline** (REMOTE-1332): `app/src/ai/agent_sdk/driver/snapshot.rs` reads JSONL declarations and uploads patches + a `snapshot_state.json` manifest. The pipeline is generic over JSONL — it doesn't care who wrote the declarations or where the artifacts go.
- **`task.AgentConversationID` is the load-bearing field**: `RunAgentRequest` already accepts `ConversationID *string` at `../warp-server/router/handlers/public_api/agent_webhooks.go:205`, persisted onto the new task as `AgentConversationID`. The cloud-side resume happens via the `--task-id` chain: the worker passes only `--task-id` (not `--conversation`); the embedded CLI's `--task-id` path fetches the task metadata, reads `conversation_id` off it, and resumes via `get_ai_conversation`. See section 8 for the full trace.
- **Local fork**: `BlocklistAIHistoryModel::fork_conversation` at `app/src/ai/blocklist/history_model.rs:1016` already produces a forked AIConversation by copying tasks. We need a server-side analogue that operates on a `server_conversation_token`.
- **EnvironmentSelector**: existing component at `app/src/ai/blocklist/agent_view/agent_input_footer/environment_selector.rs` reads `CloudAmbientAgentEnvironment::get_all` from `app/src/ai/cloud_environments/mod.rs:114`. Each env carries `github_repos: Vec<GithubRepo>` so overlap with our touched-repo set is computable client-side.
- **Agent input footer chips**: rendered by `app/src/ai/blocklist/agent_view/agent_input_footer/chips.rs`. The chip system is data-driven via `ChipResult` and slot positions (left/right). We add a new chip kind here.
- **Slash commands**: registered in `app/src/search/slash_command_menu/static_commands/commands.rs`. Commands flow through dispatch in `app/src/terminal/input/slash_commands/mod.rs`.
## Diagram
```mermaid
sequenceDiagram
    participant U as User
    participant C as Local Warp Client
    participant LC as Local Conversation
    participant API as warp-server (public API)
    participant DB as Postgres
    participant GCS
    participant Disp as Dispatcher
    participant Wk as Worker
    participant Sand as New Cloud Sandbox
    U->>C: Click "Hand off to cloud" chip
    C->>C: Split a fresh cloud-mode pane next to the local pane
    C->>U: Show handoff pane (standard cloud-mode input)
    C->>LC: Walk action history → touched repos + orphan files (async)
    U->>C: Pick env (or accept default), type prompt, submit
    C->>C: Build declarations programmatically (no script)
    C->>API: POST /agent/handoff/upload-snapshot {files: [{filename, mime_type}]}
    API-->>C: {initial_snapshot_token, expires_at, uploads: [{filename, upload_url}]}
    par Snapshot uploads
        C->>GCS: PUT each file under handoff/{initial_snapshot_token}/...
    end
    C->>API: POST /agent/runs {fork_from_conversation_id, initial_snapshot_token, prompt, config}
    API->>DB: Create forked AI conversation (copy tasks from source)
    API->>DB: Create task with AgentConversationID = forked_conversation_id
    API->>DB: tx { INSERT task, INSERT QUEUED ai_run_executions (input.initial_snapshot_token=<token>) }
    API-->>C: {task_id, run_id}
    C->>C: Pane transitions into live cloud-mode session (queued-prompt + setup affordances)
    C->>U: User stays in the same pane; cloud agent's first turn streams in
    Note over LC: Local conversation continues, user can keep typing
    Disp->>DB: Pop queued task; create new execution (id=1) in PENDING
    Disp->>Wk: Assign task with task.AgentConversationID = <forked_id>
    Wk->>Sand: oz agent run --task-id <new_task_id> --sandboxed (no --conversation flag)
    Sand->>API: GET /agent/runs/<new_task_id> (fetch task metadata)
    API-->>Sand: AmbientAgentTask { conversation_id: <forked_id>, ... }
    Sand->>API: get_ai_conversation(<forked_id>) (resume via the --task-id→conversation_id chain)
    API-->>Sand: ConversationData (forked source's tasks/messages)
    Sand->>Sand: driver_options.resume = ResumeOptions::Oz(Historical{...})
    Sand->>API: GET /agent/runs/<new_task_id>/handoff/attachments
    API->>DB: GetActiveExecutionForRun → Input.InitialSnapshotToken=<token>
    API-->>Sand: presigned download URLs from handoff/<token>/
    Sand->>GCS: Download handoff snapshot files
    Sand->>API: StartFromAmbientRunPrompt (resolves rehydration message)
    API-->>Sand: <system-message>-wrapped rehydration UserQuery + user prompt
    Sand->>Sand: Apply patches via git apply, then handle user prompt
```
## Proposed changes
### 1. Touched-repo derivation (client)
See `TOUCHED_WORKSPACE_TECH.md` for the path-extraction walk, repo grouping, and env-overlap pick. The open path described in §2 calls `extract_paths_from_conversation` and `derive_touched_workspace` on chip click, and applies `pick_handoff_overlap_env` once derivation completes.
### 2. Handoff pane: split-pane bootstrap
There is no dedicated modal view. On chip click or `/move-to-cloud` activation, `Workspace::start_local_to_cloud_handoff` (in `app/src/workspace/view.rs`) drives the open path:
1. Resolve the source conversation from the active session view's `BlocklistAIHistoryModel::active_conversation` (must be non-empty and have a `server_conversation_token`).
2. Call `pane_group.add_ambient_agent_pane(ctx)` to split a new cloud-mode pane next to the active pane (mirrors `Workspace::open_network_log_pane`'s pattern but pre-mounts the cloud-mode chrome).
3. Pre-fill the new pane's prompt editor when the slash command supplied an argument (slash command args do not flow through `PendingHandoff` itself).
4. If the source conversation didn't resolve, return early — the new pane stays as an ordinary fresh cloud-mode pane with no handoff context. Non-eligible clicks are not surfaced as errors.
5. Otherwise, seed `PendingHandoff` onto the new pane's `AmbientAgentViewModel` (see below) and `ctx.spawn` an async block that calls `extract_paths_from_conversation` and then `derive_touched_workspace(...)`. When derivation completes, apply `pick_handoff_overlap_env(...)` to the model's `environment_id` (the env selector's `ensure_default_selection` already runs first; the handoff-aware pick overrides on a real overlap match and is skipped on no-overlap).
#### Handoff context on `AmbientAgentViewModel`
Add a `pending_handoff: Option<PendingHandoff>` field on `AmbientAgentViewModel` (`app/src/terminal/view/ambient_agent/model.rs`):
```rust path=null start=null
pub(crate) struct PendingHandoff {
    pub(crate) source_conversation_id: ServerConversationToken,
    /// `None` until `derive_touched_workspace` completes.
    pub(crate) touched_workspace: Option<TouchedWorkspace>,
    /// Gates `submit_handoff` against double-submits and surfaces inline errors.
    pub(crate) submission_state: HandoffSubmissionState, // Idle | Starting | Failed(String)
}
```
`is_local_to_cloud_handoff()` returns `pending_handoff.is_some()` and is the single source of truth for "this pane is in handoff mode". The new pane needs that predicate true from the moment it opens so the V2-input suppression and the submit-interception logic both fire before the spawn.
#### Suppress `CloudModeInputV2` for handoff panes
Update `Input::is_cloud_mode_input_v2_composing` (`app/src/terminal/input/agent.rs:65`) to also require `!ambient_agent_view_model.is_local_to_cloud_handoff()`. V2 is for fresh cloud-mode runs only; handoff stays on the existing input UI regardless of the flag's state.
#### No banner UI in V0
V0 ships with no dedicated handoff banner. `PendingHandoffChanged` triggers a `ctx.notify()` for future banner work; today the only user-visible effects of derivation completing are (a) the env selector's default updating to the overlap winner and (b) `submit_handoff` being unblocked. Submission errors surface inline via `HandoffSubmissionState::Failed` for future banner work to consume.
### 3. Chip and slash command (client)
- Add a new `AgentToolbarItemKind::HandoffToCloud` variant in `app/src/ai/blocklist/agent_view/agent_input_footer/toolbar_item.rs`. The chip is rendered with the `bundled/svg/upload-cloud-01.svg` icon. Visibility is gated only on `FeatureFlag::OzHandoff && FeatureFlag::HandoffLocalCloud`; conversation eligibility (synced token, non-empty, harness) is enforced via fall-through inside `Workspace::start_local_to_cloud_handoff` rather than at the visibility level. The chip is also hidden from session viewers (`available_to_session_viewer()` returns `!status.is_viewer()`).
- Add the chip to `default_right()` (and `all_available()`) in the same file, gated on the same flags so the user-facing toolbar configurator picks it up.
- The chip's on-click action emits `AgentInputFooterEvent::OpenHandoffPane { initial_prompt: None }`. The terminal `Input` subscriber forwards it to `WorkspaceAction::OpenLocalToCloudHandoffPane`.
- Add `MOVE_TO_CLOUD` to `app/src/search/slash_command_menu/static_commands/commands.rs`:
  ```rust path=null start=null
  pub static MOVE_TO_CLOUD: LazyLock<StaticCommand> = LazyLock::new(|| StaticCommand {
      name: "/move-to-cloud",
      description: "Hand off this conversation to a cloud agent",
      icon_path: "bundled/svg/upload-cloud-01.svg",
      availability: Availability::AGENT_VIEW
          | Availability::ACTIVE_CONVERSATION
          | Availability::AI_ENABLED,
      auto_enter_ai_mode: false,
      argument: Some(Argument::optional()
          .with_hint_text("<optional follow-up prompt>")
          .with_execute_on_selection()),
  });
  ```
  Gate registration on the same flags inside `all_commands()` in that file.
- Wire the slash command's execute path in `app/src/terminal/input/slash_commands/mod.rs` to dispatch `WorkspaceAction::OpenLocalToCloudHandoffPane { initial_prompt: argument.cloned().filter(|s| !s.is_empty()) }`. Like the chip, conversation eligibility is enforced in the workspace handler, so the slash command itself only checks the feature flags.
### 4. Snapshot pipeline: local-mode entry point
See `SNAPSHOT_UPLOAD_TECH.md` for `upload_snapshot_for_handoff`, the `POST /agent/handoff/upload-snapshot` endpoint contract, the `SpawnAgentRequest.fork_from_conversation_id` / `initial_snapshot_token` field additions, and the `task_id` refactor that lets the existing pipeline share its upload helper with the handoff entry point.
### 5. Server-side conversation fork
The existing fork mechanism is client-driven: `BlocklistAIHistoryModel::fork_conversation` (`app/src/ai/blocklist/history_model.rs:1016`) copies tasks locally, then the next request sends `forked_from_conversation_id` + `tasks` together; the server (`router/middleware/set_conversation_info.go:45`) mints a new UUID and records `forked_from_conversation_id` for telemetry only (no DB column persists it). That doesn't fit local→cloud: the cloud sandbox has no source-task in memory, and we need `task.AgentConversationID` to point at a materialized conversation at task-creation time so the local pane can fetch the fork immediately.
We add a server-side helper that materializes the fork synchronously:
```go path=null start=null
// ForkConversation copies an existing conversation's GCS data and metadata into a
// new conversation owned by `principal`. Returns the new conversation_id.
func ForkConversation(
    ctx context.Context,
    db database.SqlQuerier,
    datastores types.Stores,
    sourceConversationID string,
    principal types.Principal,
) (string, error)
```
Location: alongside `UpsertAIConversationMetadata` / `CreateThirdPartyAIConversation` in `../warp-server/logic/ai_conversation_object.go`. Steps:
1. **Authorize.** `GetAIConversationObjectInfo(sourceConversationID)` + require `ViewAction` for `principal` (mirrors `CheckAndRecordConversationAccess` at `ai_conversation_object.go:603`); reject with `NotAuthorizedError` otherwise.
2. **Require persisted source data.** `DoesConversationDataExist(ctx, sourceConversationID)` rejects unsynced conversations before task creation.
3. **Read source metadata.** `AIConversationMetadataStore.GetUsageByConversationIDs([sourceConversationID])` so the fork inherits `title`, `working_directory`, `harness`, `latest_git_branch`.
4. **Mint and copy.** `newID := uuid.NewString()`, then `CopyConversationDataInGCS(ctx, sourceConversationID, newID)` performs a server-side GCS copy of `{conversation_id}.pb`.
5. **Insert metadata + WD object with `has_gcs_data = TRUE`.** `UpsertAIConversationMetadataWithHasGCSData(..., shouldCreateConversationObject: true)` inserts the metadata row, creates the `object_metadata` row owned by `principal`, and marks the fork as GCS-backed in one path.
6. **Return `newID`** and emit a structured log line linking source→fork→principal.
The server-side copy keeps conversation bytes inside GCS even for large conversations. No lineage column is persisted today; if one is added later, the helper can populate it.
### 6. Server-side `RunAgentRequest` extensions
Extend `RunAgentRequest` in `../warp-server/router/handlers/public_api/agent_webhooks.go:199` with two new fields:
```go path=null start=null
type RunAgentRequest struct {
    // existing fields...
    ForkFromConversationID *string `json:"fork_from_conversation_id,omitempty"`
    InitialSnapshotToken   *string `json:"initial_snapshot_token,omitempty"`
}
```
`enqueueAgentRun` is updated:
- If `ForkFromConversationID` is set, call `ForkConversation(...)` to mint `<forked_id>`, then set `req.ConversationID = &<forked_id>` (overriding any caller value). Existing logic at `agent_webhooks.go:381` continues to set `task.AgentConversationID` from `req.ConversationID`.
- If `InitialSnapshotToken` is set, plumb it through `NewTaskParams.InitialSnapshotToken` to `AddTask`. Inside the task transaction, the QUEUED `ai_run_executions` row is inserted with `Input.InitialSnapshotToken = &<initial_snapshot_token>`. Discovery (`handoff_rehydration.go::resolveHandoffSnapshotFilesForRun` and the `/handoff/attachments` handler) reads that field and resolves files at `handoff/{token}/` in place — there's no GCS move, no synthetic ENDED row, and no migration.
- Both new fields are gated behind `local_to_cloud_handoff` (server-side flag, mirroring the client `HandoffLocalCloud`).
- Authorization: user must have view access on `ForkFromConversationID` (existing AI conversation auth checks). The `InitialSnapshotToken` only authorizes uploads back to the prefix that minted it.
- Error handling: fork failure aborts task creation. Per-blob upload failures during the `upload-snapshot` phase are best-effort (logged + reported via `report_error!`); the cloud agent rehydrates against whatever made it to GCS.
### 7. Handoff submission
The client API surface (`AIClient::upload_local_handoff_snapshot` and the `SpawnAgentRequest` field additions) is documented in `SNAPSHOT_UPLOAD_TECH.md`.
The client-side submission lives in `AmbientAgentViewModel::submit_handoff`. It starts one `ctx.spawn`ed future that calls `upload_snapshot_for_handoff` to mint the initial snapshot token, gather repo patches and orphan-file contents, and upload everything to `handoff/{initial_snapshot_token}/`. The actual cloud-agent spawn happens after that future resolves so the existing streaming flow is reused unchanged.
The agent config (env, model, worker_host, computer_use_enabled, harness) is intentionally read at spawn time. By then, the user has already picked an env via the pane's existing env selector chip; `build_default_spawn_config` reads everything else from the model + global preferences.
Failures of the snapshot upload phase set `pending_handoff.submission_state = Failed(msg)`. V0 has no banner; the user retries by re-submitting from the same pane. Failures of the spawn itself surface via the model's existing cloud-mode error rendering.
### 7a. Submit interception in the handoff pane (client)
The handoff pane is a regular cloud-mode pane, so the user's submission flows through the existing input dispatch path. We intercept it when `AmbientAgentViewModel::is_local_to_cloud_handoff()` is true (i.e. `pending_handoff.is_some()`) so the snapshot upload runs *before* the spawn.
#### `AmbientAgentViewModel::submit_handoff`
```rust path=null start=null
pub(crate) fn submit_handoff(
    &mut self,
    prompt: String,
    attachments: Vec<AttachmentInput>,
    ctx: &mut ModelContext<Self>,
);
```
Flow:
1. No-op if `pending_handoff` is absent, derivation hasn't completed (`touched_workspace.is_none()`), or `submission_state` is already `Starting`.
2. Set `submission_state = Starting` and emit `PendingHandoffChanged`.
3. `ctx.spawn` `upload_snapshot_for_handoff` with the model's `touched_workspace`.
4. On success, build a `SpawnAgentRequest` with `fork_from_conversation_id` + `initial_snapshot_token` set and `config = Some(self.build_default_spawn_config(ctx))`, then call `self.spawn_agent_with_request(request, ctx)` — the same helper the regular `spawn_agent` path uses. This flips the model to `WaitingForSession` and emits `DispatchedAgent`.
5. On failure, set `submission_state = Failed(msg)` so the user can retry.
#### Wiring submit interception
The submit dispatch in `Input::handle_input_action` (`app/src/terminal/input.rs`) routes through `submit_handoff` instead of `spawn_agent` when `model.is_local_to_cloud_handoff()` is true. `pending_handoff` is seeded by the chip / slash command's open path (§2) and is not cleared after the spawn — it stays so post-spawn flows that query `is_local_to_cloud_handoff()` (queued-prompt rendering, V2-input suppression) keep behaving consistently.
#### DispatchedAgent + queued-prompt rendering
`DispatchedAgent` (`app/src/terminal/view/ambient_agent/view_impl.rs`) renders the user's prompt via `insert_cloud_mode_queued_user_query_block` (REMOTE-1454's helper) when `is_local_to_cloud_handoff()` is true. The block is removed on the same transitions the non-oz harness path already handles in `handle_ambient_agent_event`: `Failed`, `Cancelled`, `NeedsGithubAuth`, `HarnessCommandStarted`. For the Oz handoff specifically, the first `AppendedExchange` also clears the block (the analogous "harness CLI started" transition for Oz). Each path calls `remove_pending_user_query_block(ctx)` (idempotent). The cloud agent's exchanges flow into the pane via the shared-session replication path that regular cloud-mode runs already use.
### 8. How the conversation reaches the new sandbox (no worker or sandbox CLI changes)
The only invariant the new task needs to satisfy is `task.AgentConversationID = <forked_id>`. From there, the existing `--task-id` chain plumbs the conversation into the cloud agent without any new client-side or worker-side changes:
1. **Worker.** `oz-agent-worker/internal/common/task_utils.go::AugmentArgsForTask` passes only `--task-id <T>` (never `--conversation`). Pinned by `task_utils_test.go:152` ("does not forward --conversation even when AgentConversationID is set").
2. **Embedded CLI.** Inside the sandbox, `setup_and_run_driver` (`app/src/ai/agent_sdk/mod.rs:545`) sees `args.task_id = Some(T)` and `args.conversation = None`. `build_driver_options_and_task` fetches the task via `get_ambient_agent_task(T)` (`mod.rs:1031-1051`); the returned `AmbientAgentTask.conversation_id` (= `task.AgentConversationID`) is merged into `resume_conversation_id`.
3. **Resume.** `load_conversation_information(<forked_id>, HarnessKind::Oz)` (`mod.rs:1105`) calls `get_ai_conversation(<forked_id>)` and produces `ResumeOptions::Oz(ConversationRestorationInNewPaneType::Historical { conversation, ... })`. The terminal driver restores the conversation and the agent starts with the forked history visible.
Our change wires the front of this chain: the client sends `POST /agent/runs` with `fork_from_conversation_id = <local_token>` (and deliberately does *not* set the existing `conversation_id` field, which has resume semantics rather than fork semantics). `enqueueAgentRun` calls `ForkConversation(<local_token>)` to mint `<forked_id>`, sets `req.ConversationID = &<forked_id>`, and the existing line `agent_webhooks.go:381` plumbs it onto `task.AgentConversationID`. Callers should set exactly one of `conversation_id` (resume) and `fork_from_conversation_id` (fork); both live on `SpawnAgentRequest` / `RunAgentRequest` and pick different branches inside `enqueueAgentRun`.
### 9. Sandbox-side: rehydration prompt (no client-side changes)
With the conversation-resume side covered above, the only remaining sandbox-side work is the rehydration prompt that tells the agent to apply the snapshot patches:
- `fetch_and_download_handoff_snapshot_attachments` (`app/src/ai/agent_sdk/driver/attachments.rs:68`) calls `GET /agent/runs/:runId/handoff/attachments`. The server reads the active execution's `Input.InitialSnapshotToken` and lists files at `handoff/{token}/`; if the token is absent (post-first-execution retries, cloud→cloud handoffs) it falls back to the latest ENDED execution's `snapshots/{run_id}/{exec_id}/` upload.
- The runtime's rehydration message construction (`logic/ai/multi_agent/runtime/interceptors/input.go:433` → `resolveHandoffRehydrationMessage`) shares the same two-rule discovery as the `/handoff/attachments` handler. It lists snapshot files at the resolved prefix and prepends the `<system-message>`-wrapped UserQuery to the runtime's first input.
### 10. Feature flags
Add `FeatureFlag::HandoffLocalCloud` in `crates/warp_features/src/lib.rs`. The chip, slash command, client API methods, and server endpoint behavior are all gated on `OzHandoff && HandoffLocalCloud`. Both flags must be enabled for the feature to function.
On the server, mirror with a `local_to_cloud_handoff` flag in `config/features/features.go`. The server feature-flag check happens at the request handler level (returns 404 / `feature not available` when off). This mirrors `HandoffCloudCloudEnabled` which already exists.
## Risks and mitigations
- **Initial snapshot token expires before task creation.** The `initial_snapshot_token` is short-lived (15 min, matching presigned URL lifetime); a stalled handoff past expiry would fail with a "can't find files" error. *Mitigation:* the upload-snapshot endpoint returns the expiry timestamp so the pane can request a fresh token before the deadline; as a backstop, the task-creation handler returns a structured "initial snapshot token expired" error so the client can transparently retry.
- **Fork on a very large conversation.** `ForkConversation` copies the source conversation object inside GCS. *Mitigation:* use the server-side `CopyConversationDataInGCS` path so bytes do not round-trip through the warp-server process.
- **Source conversation isn't fully synced to GCS.** A `server_conversation_token` only proves the metadata row exists; the GCS data (`{conversation_id}.pb`) may still be in flight or never written. *Mitigation:* the fork helper checks `DoesConversationDataExist` before copying and returns a structured `SourceConversationNotPersisted` error; the pane surfaces it via `HandoffSubmissionState::Failed`.
- **Unauthorized cross-user fork.** A caller could try to fork another user's conversation. *Mitigation:* `ForkConversation` step 1 requires `ViewAction` on the source via the existing `auth_types.For(ctx)` engine (same posture as `CheckAndRecordConversationAccess`); the new fork is owned by the requesting principal, not the source's owner.
- **Local-only changes that aren't reproducible in cloud.** Private forks, submodules, large LFS files. `git diff --binary HEAD` and `git ls-files --others --exclude-standard` cover the common cases; submodules are not recursed (same as cloud→cloud). *Mitigation:* acceptable for V0; the rehydration prompt instructs the agent to report apply failures.
- **Worker/server flag drift.** Client flag on, server flag off → endpoint 404. *Mitigation:* standard rollout sequencing (server first); the client surfaces the 404 as `HandoffSubmissionState::Failed`.
- **Snapshot upload tail latency.** Pathological binary diffs hit the existing pipeline's cap (3 retries, exponential backoff, 2-min ceiling). *Mitigation:* same caps as cloud→cloud; the user sees the "Starting…" state for the duration and closing the pane aborts in-flight uploads.
## Testing and validation
Per-branch unit-test coverage (touched-repo helpers, snapshot pipeline, `SessionJoinInfo`) is documented in `TOUCHED_WORKSPACE_TECH.md` and `SNAPSHOT_UPLOAD_TECH.md`.
### Server tests (`../warp-server`)
- `agent_webhooks_test.go::TestHandoff_ForkAndInitialSnapshotToken`: end-to-end inside the test harness. Pre-creates a source conversation, calls the upload-snapshot endpoint, uploads test files, calls `POST /agent/runs` with both new fields, asserts that the new task has `AgentConversationID` pointing at a fresh forked conversation, that `handoff/{initial_snapshot_token}/` retains the uploaded files (no move), and that the QUEUED `ai_run_executions` row's `Input.InitialSnapshotToken` matches the token.
- `agent_webhooks_test.go::TestHandoff_FlagOff`: with `local_to_cloud_handoff=false`, the request fails with the expected error and no task / no fork side effects.
- `agent_webhooks_test.go::TestHandoff_InitialSnapshotTokenWithoutFiles`: `POST /agent/runs` with a `initial_snapshot_token` whose prefix is empty creates the task normally; `/handoff/attachments` returns an empty list at rehydration time.
### Integration / manual
- Starting a handoff with a touched repo containing uncommitted changes, opening the resulting cloud run, and confirming the agent's first turn applies the patches before answering. Verified via the cloud agent's tool calls (`git apply`, `git status`), not by the LLM's chat output.
- After a successful handoff the local conversation accepts new user input and the local agent continues responding. The user can fork it locally too, run other commands, etc.
- The cloud agent's conversation has a different `server_conversation_token` than the local one and that token appears in the cloud agent management view.
- Toggling settings to verify chip availability under various states (no synced server token, `CloudConversations` disabled, etc.).
- Manually break a touched repo's `.git` and confirm the manifest captures it as `gather_failed` and the rest of the snapshot proceeds.
### Feature-flag rollout
- Server flag (`local_to_cloud_handoff`) goes Dogfood first, end-to-end tested with a Warp engineer's local→cloud handoff against a staging worker.
- Client flag (`HandoffLocalCloud`) follows once server flag is stable.
- Promote together to Preview and Stable per the standard `promote-feature` skill.
## Follow-ups
- Extend `/move-to-cloud` to dispatch to non-Oz harnesses (Claude Code, Gemini, etc.). Most of the plumbing (touched-repo derivation, snapshot pipeline, upload-snapshot endpoint, server-side fork) is reusable; the differences are (a) the chip/command gating drops the Oz-only check and instead reads the active conversation's `harness_kind()` to pick the cloud-side resume strategy, (b) for Claude conversations the server handler must also upload the local Claude transcript envelope to the right GCS slot so the cloud Claude run resumes via REMOTE-1373's existing transcript rehydration path.
- A CLI surface for handoff (e.g. `oz agent handoff --conversation <local-id> --env <id> --prompt "..."`). Opens up automation. Out of scope for V0; the public API surface is already CLI-friendly when we get there.
- A "this conversation was handed off to <link>" indicator on the local conversation, persisted on the local conversation metadata. V0 only surfaces the link by auto-opening the new cloud-mode pane; the local pane has no persistent breadcrumb back to its handoff destination.
- Multi-conversation handoff (batch operation) and "handoff with this exact context but a different prompt" (re-launch with the same uploaded snapshot). These would benefit from making the initial snapshot token re-usable across multiple `POST /agent/runs` calls before expiry.
- Snapshot file size cap on the upload-snapshot endpoint. Today the size cap is implicit (presigned URL upload limits + the 100-file cap inherited from `MAX_SNAPSHOT_FILES_PER_RUN`). Worth surfacing more explicitly so the handoff pane can warn the user before they submit.
- Banner UI surfacing touched-repo overlap status, derivation progress, and inline submission errors. V0 ships with no banner; the data is all available on `pending_handoff` but not visualized.
