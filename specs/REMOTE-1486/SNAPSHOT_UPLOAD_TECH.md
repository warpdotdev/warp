# Cloud Handoff Snapshot Upload — Tech Spec
Part of the local-to-cloud Oz handoff feature ([REMOTE-1486](https://linear.app/warpdotdev/issue/REMOTE-1486)). Full feature behavior in `PRODUCT.md`; the orchestrator that wires this together lives in `TECH.md`.
## Context
The handoff flow stages the local agent's workspace into GCS *before* the cloud task exists, so the cloud sandbox can rehydrate from those files on its first turn. There's no `task_id` to scope the upload to at that point — only a server-minted initial snapshot token and a `handoff/{token}/` GCS prefix.
The existing end-of-run snapshot pipeline (REMOTE-1332) is generic enough to reuse for the gather + upload phase, but the entry point and the URL-allocation step both need new variants that don't depend on a task. The `task_id` parameter that the existing helpers thread through purely for log context becomes a liability in the new entry point, so we drop it and re-extract a `task_id`-free upload helper that both paths share.
This branch contains only the upload + server-contract pieces; nothing in-tree calls `upload_snapshot_for_handoff` yet, so the function and the unused `InitialSnapshotToken::as_str` accessor are gated with `#[allow(dead_code)]` until the parent stack branch wires them up.
Relevant code:
- `app/src/ai/agent_sdk/driver/snapshot.rs` — existing end-of-run pipeline (`run_pipeline`, `gather_snapshot_entries`, `upload_gathered_snapshot`, `apply_per_run_cap`, `upload_entry`, `repo_metadata`, `build_repo_patch`).
- `app/src/server/server_api/ai.rs` — `AIClient` trait, `SpawnAgentRequest`, the existing `HarnessSupportClient::get_snapshot_upload_targets` URL allocation we don't reuse here.
- `app/src/ai/ambient_agents/spawn.rs` — `SessionJoinInfo::from_task`, the cloud-mode session join helper we tighten for the new fork-via-handoff server contract.
## Proposed changes
### `upload_snapshot_for_handoff`
A sibling entry point in `app/src/ai/agent_sdk/driver/snapshot.rs` that reuses the existing gather + upload internals but skips the JSONL declarations file and the cloud-side `run_declarations_script`:
```rust path=null start=null
pub(crate) async fn upload_snapshot_for_handoff(
    repo_paths: Vec<PathBuf>,
    orphan_file_paths: Vec<PathBuf>,
    client: Arc<dyn AIClient>,
    http: &http_client::Client,
) -> Result<Option<InitialSnapshotToken>>;
```
Translates the input paths into the same internal `Vec<DeclarationEntry>` that `parse_declarations` produces today (repos → `EntryKind::Repo`, orphan files → `EntryKind::File`), then:
1. Calls `gather_snapshot_entries` to build the manifest stubs and upload blobs.
2. Applies the existing per-run cap (`MAX_SNAPSHOT_FILES_PER_RUN = 100`).
3. Calls `AIClient::upload_local_handoff_snapshot` with the planned filenames + mime types to mint an initial snapshot token and presigned upload URLs scoped to `handoff/{token}/` (rather than going through `HarnessSupportClient::get_snapshot_upload_targets`, which requires a task).
4. Builds the upload `target_map` by zipping the requested filenames with the positionally-aligned server `uploads` array; missing targets are marked `skipped` downstream.
5. Routes the actual blob + manifest uploads through the new `upload_prepared_snapshot_files` helper.
Returns:
- `Ok(Some(initial_snapshot_token))` when a token was minted **and the `snapshot_state.json` manifest landed in GCS**. Individual blob uploads may still have failed; the manifest catalogues their status so the cloud side rehydrates against whatever did land, matching the cloud→cloud best-effort posture.
- `Ok(None)` when the workspace was empty **or** when the manifest itself failed to upload. Without the manifest the snapshot prefix is unusable, so callers spawn the cloud agent without an initial snapshot token instead of pointing it at incomplete state.
- `Err(_)` only for hard failures of `upload_local_handoff_snapshot` itself (auth, etc.).
Manifest-upload failures (whether the manifest serialization aborted the pipeline or its presigned PUT failed) also route through `report_error!` so on-call alerting catches the silent regression.
### Refactor: drop `task_id` from the existing helpers, extract `upload_prepared_snapshot_files`
The existing `upload_snapshot_from_declarations_file`, `run_pipeline`, `gather_snapshot_entries`, `upload_gathered_snapshot`, `gather_repo` / `gather_file`, `apply_per_run_cap`, `fold_upload_results`, `upload_entry`, `parse_declarations`, and `read_and_parse_declarations` previously took `&AmbientAgentTaskId` only for log context. The new handoff entry point has no task at this stage, so each helper drops the parameter and the corresponding log lines lose the `(task X)` suffix. The outer `upload_snapshot_from_declarations` (which `AgentDriver` calls at end-of-run) still has a task id and passes it to `resolve_declarations_path` for the per-run JSONL file path; that's the only remaining task-aware helper.
`upload_prepared_snapshot_files` is extracted out of `upload_gathered_snapshot` as a private helper. Both the existing `run_pipeline` path (declarations → server `get_snapshot_upload_targets` → upload) and the new `upload_snapshot_for_handoff` path (touched-workspace input → `upload_local_handoff_snapshot` → upload) terminate in the same blob + manifest upload logic.
### Server contract: `upload_local_handoff_snapshot` + new types
Adds to `app/src/server/server_api/ai.rs`:
- `InitialSnapshotToken(String)` — opaque token the server returns from `upload_local_handoff_snapshot` and the client passes back via `SpawnAgentRequest.initial_snapshot_token`.
- `UploadLocalHandoffSnapshotRequest { files: Vec<SnapshotUploadFileInfo> }` and `SnapshotUploadFileInfo { filename, mime_type }`.
- `UploadLocalHandoffSnapshotResponse { initial_snapshot_token, expires_at, uploads: Vec<UploadTarget> }`; the response field deserializes from the public wire key `initial_snapshot_token`.
- `AIClient::upload_local_handoff_snapshot(...)` trait method (POSTs to `agent/handoff/upload-snapshot`) and its `ServerApi` implementation.
The server-side handler mints a UUID-v4 initial snapshot token, authorizes against the user, and generates URLs scoped to `handoff/{token}/` via the existing presigned-URL helper. No DB writes — discovery happens later by GCS prefix. Server-side details are covered in the parent feature's `TECH.md`.
### `SpawnAgentRequest` field additions
Two new optional fields on `SpawnAgentRequest`:
- `fork_from_conversation_id: Option<String>` — instructs the server to fork the named conversation and use the resulting fork id as `task.AgentConversationID`. The actual fork is server-side; this field is the client's signal.
- `initial_snapshot_token: Option<InitialSnapshotToken>` — references the GCS prefix uploaded above so the server can bind the token to the new run's queued execution and the cloud sandbox can list / download files from it on first turn. This serializes as the public API wire key `initial_snapshot_token`.
Both fields use `#[serde(skip_serializing_if = "Option::is_none")]` so they're backwards-compatible against older server builds. Existing constructor sites in `agent_sdk/ambient.rs`, `agent_sdk/mcp_config_tests.rs`, `pane_group/pane/terminal_pane.rs`, `ambient_agents/spawn_tests.rs`, and `view/ambient_agent/model.rs::spawn_agent` set both to `None`; the parent stack branch's `submit_handoff` is the only call site that populates them.
### `SessionJoinInfo::from_task` strictness
`SessionJoinInfo::from_task` (`app/src/ai/ambient_agents/spawn.rs`) is rewritten to require a parseable `session_id` and return `None` otherwise. Previously a task with a `session_link` but no `session_id` returned a join info with `session_id: None`; that path is no longer actionable for the cloud-mode pane.
The new behavior is needed because the GET task handler now overwrites `session_link` with a conversation link for tasks that have synced conversation data (e.g. the local-to-cloud handoff fork) — so a `session_link` alone is no longer a reliable signal that a real session exists. `session_link` falls back to `shared_session::join_link(&session_id)` when the server didn't provide one. Matching `spawn_tests.rs` test updates ship in this branch.
## Testing and validation
- `snapshot_tests.rs` is updated to drop the `&fake_task_id()` arguments from every helper call, keeping the existing `run_pipeline` coverage intact under the simplified signature.
- `spawn_tests.rs` covers the new `SessionJoinInfo::from_task` invariants: `requires_session_id` (no session_id returns `None`), `prefers_server_session_link_when_session_id_is_present`, `constructs_link_from_session_id_when_link_missing`.
- End-to-end coverage of `upload_snapshot_for_handoff` (mockito for the upload-snapshot endpoint, asserting manifest shape and per-blob upload outcomes) lands on the parent stack branch where the function actually has a caller.
