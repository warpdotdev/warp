# APP-4486: Use server forking endpoint for local forks

## Context

When a user forks a conversation (via `/fork`, context menu, conversation details, etc.), the forked conversation is only created locally. It has no `server_conversation_token` until the user sends a new query and receives a `StreamInit` response. This means the forked conversation isn't immediately stored in the cloud and doesn't have a server-side identity.

The server-side fork endpoint already exists and is used by the local-to-cloud handoff flow. The goal is to also call it during regular forking so forked conversations immediately get cloud storage and server tokens.

### Relevant code

**Server endpoint**: `../warp-server/logic/ai_conversation_fork.go` — `ForkConversation()` copies GCS data, creates a new metadata row, returns a new conversation ID. No changes needed.

**Client API**: `app/src/server/server_api/ai.rs:942-946` — `AIClient::fork_conversation()` calls `POST /agent/conversations/{id}/fork`. No changes needed.

**Handoff fork flow (pattern to follow)**: `app/src/workspace/view.rs:13356-13431` — `start_local_to_cloud_handoff` calls `ai_client.fork_conversation()` first, then on success calls `complete_local_to_cloud_handoff_open` which creates the local fork and binds the server token via `set_server_conversation_token_for_conversation`.

**Regular fork flow (code to change)**: `app/src/workspace/view.rs:11718-11941` — `fork_ai_conversation()` loads conversation data (async), then synchronously creates a local fork via `history_model.fork_conversation()` or `fork_conversation_at_exchange()`, and restores it into a pane. Both local fork methods set `server_conversation_token: None` (`history_model.rs:1155`, `history_model.rs:1312`).

**Fork entry points**: All funnel through `WorkspaceAction::ForkAIConversation` → `fork_ai_conversation()` (`view.rs:22030`). `ContinueConversationLocally` also calls it (`view.rs:22042`).

**Cloud storage check**: `PrivacySettings::as_ref(ctx).is_cloud_conversation_storage_enabled` (`settings/privacy.rs:153`).

## Proposed changes

Modify `fork_ai_conversation` in `app/src/workspace/view.rs` to call the server-side fork endpoint before creating the local fork, following the handoff pattern.

### Approach: server fork first, then local fork

1. After the existing async conversation data load completes, check two conditions:
   - `PrivacySettings::as_ref(ctx).is_cloud_conversation_storage_enabled` is true
   - The source conversation has a `server_conversation_token`
2. If both are met, spawn `ai_client.fork_conversation()` with the source's server token and the source conversation's title.
3. On success, proceed with the existing local fork logic, then bind the returned `forked_conversation_id` to the local fork via `history_model.set_server_conversation_token_for_conversation()`.
4. On failure, log a warning and fall through to the existing local-only fork — no user-visible impact.
5. If the conditions in step 1 are not met, skip the server call and go straight to the existing local fork logic.

### Implementation detail

Extract the existing local-fork-and-restore logic into a helper (e.g. `complete_fork_ai_conversation`) that takes an optional `server_forked_conversation_id: Option<String>`. The outer `fork_ai_conversation` method handles the conditional async server call and then delegates to this helper regardless of the outcome.

`preserve_task_ids` stays `false` for regular forks — unlike handoff, no cloud agent will execute against the fork, so the local fork can mint new task IDs freely.

## Testing and validation

- **Unit tests**: Add a test in `history_model_tests.rs` verifying that when `server_forked_conversation_id` is provided, the forked conversation's `server_conversation_token` is set to that value.
- **Manual verification**: Fork a conversation with cloud storage enabled and confirm the forked conversation appears in the cloud conversation list immediately (without sending a new query). Fork again with cloud storage disabled and confirm the fork still works locally without errors.
- **Error path**: Temporarily force the server fork to fail (e.g. use an invalid conversation ID) and verify the local fork still succeeds with a warning logged.

## Parallelization

This is a single-file change (~50 lines of new logic in `workspace/view.rs`). Parallelization via child agents would not provide meaningful benefit.
