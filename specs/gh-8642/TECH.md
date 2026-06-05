# Conversation renaming
## Context
This spec implements the behavior in `PRODUCT.md` for GitHub issue #8642. The feature spans the Warp client and `warp-server`: the client owns the slash command and local refresh, while the server owns title persistence for cloud conversation metadata and agent run/task rows.
Client code inspected at `dd274743ab595172f5a5ce873430135ec18f7aff`:
- [`app/src/search/slash_command_menu/static_commands/commands.rs (129-144) @ dd274743](https://github.com/warpdotdev/warp/blob/dd274743ab595172f5a5ce873430135ec18f7aff/app/src/search/slash_command_menu/static_commands/commands.rs#L129-L144) defines `/rename-tab`; the new command should follow the same `StaticCommand` pattern.
- [`app/src/search/slash_command_menu/static_commands/commands.rs (547-624) @ dd274743](https://github.com/warpdotdev/warp/blob/dd274743ab595172f5a5ce873430135ec18f7aff/app/src/search/slash_command_menu/static_commands/commands.rs#L547-L624) registers static slash commands. `/rename-conversation` should be active-conversation scoped, not always available like `/rename-tab`.
- [`app/src/terminal/input/slash_commands/mod.rs (200-380) @ dd274743](https://github.com/warpdotdev/warp/blob/dd274743ab595172f5a5ce873430135ec18f7aff/app/src/terminal/input/slash_commands/mod.rs#L200-L380) dispatches slash-command handlers and shows command error toasts. The new command should use the existing handler/toast pattern.
- [`app/src/terminal/input/slash_commands/mod.rs (889-984) @ dd274743](https://github.com/warpdotdev/warp/blob/dd274743ab595172f5a5ce873430135ec18f7aff/app/src/terminal/input/slash_commands/mod.rs#L889-L984) shows active-conversation command handling for `/fork` and `/continue-locally`, including selected-conversation lookup.
- [`app/src/ai/agent/conversation.rs (1437-1479) @ dd274743](https://github.com/warpdotdev/warp/blob/dd274743ab595172f5a5ce873430135ec18f7aff/app/src/ai/agent/conversation.rs#L1437-L1479) derives the local display title from the root task description, falling back to the initial query and then fallback title. Rename should add a narrow manual-title override path instead of rewriting transcript task data.
- [`app/src/ai/agent/task.rs (535-583) @ dd274743](https://github.com/warpdotdev/warp/blob/dd274743ab595172f5a5ce873430135ec18f7aff/app/src/ai/agent/task.rs#L535-L583) exposes task descriptions. These should remain transcript/task content for this feature rather than becoming the durable manual title store.
- [`app/src/ai/blocklist/history_model.rs (634-652) @ dd274743](https://github.com/warpdotdev/warp/blob/dd274743ab595172f5a5ce873430135ec18f7aff/app/src/ai/blocklist/history_model.rs#L634-L652) persists conversation state and emits `UpdatedConversationMetadata`; the rename path should emit this after mutating local state.
- [`app/src/server/server_api/ai.rs (202-219) @ dd274743](https://github.com/warpdotdev/warp/blob/dd274743ab595172f5a5ce873430135ec18f7aff/app/src/server/server_api/ai.rs#L202-L219) already has public API request/response structs for conversation fork. Rename should add analogous request/response types.
- [`app/src/server/server_api/ai.rs (1083-1094) @ dd274743](https://github.com/warpdotdev/warp/blob/dd274743ab595172f5a5ce873430135ec18f7aff/app/src/server/server_api/ai.rs#L1083-L1094) defines `AIClient::fork_conversation`; rename should add an `AIClient::rename_conversation` method.
- [`app/src/server/server_api/ai.rs (1906-1929) @ dd274743](https://github.com/warpdotdev/warp/blob/dd274743ab595172f5a5ce873430135ec18f7aff/app/src/server/server_api/ai.rs#L1906-L1929) implements the fork request over the public API; rename should use the same `post_public_api`/URL-builder style.
Server code inspected at `ee625e8c1968c80c3e94e2f46c8afae0918c786a`:
- [`router/handlers/public_api/agent_conversations.go (247-279) @ ee625e8](https://github.com/warpdotdev/warp-server/blob/ee625e8c1968c80c3e94e2f46c8afae0918c786a/router/handlers/public_api/agent_conversations.go#L247-L279) implements `POST /agent/conversations/{conversation_id}/fork`, including title normalization. Rename should be a sibling conversation-scoped endpoint.
- [`router/handlers/public_api/agent_conversations.go (314-333) @ ee625e8](https://github.com/warpdotdev/warp-server/blob/ee625e8c1968c80c3e94e2f46c8afae0918c786a/router/handlers/public_api/agent_conversations.go#L314-L333) has view-only auth helpers. Rename needs `EditContentAction` on the conversation object, not view-only access.
- [`authz/types/action.go (13-25) @ ee625e8](https://github.com/warpdotdev/warp-server/blob/ee625e8c1968c80c3e94e2f46c8afae0918c786a/authz/types/action.go#L13-L25) defines `EditContentAction` as editing object content/name.
- [`model/conversation_metadata.go (59-69) @ ee625e8](https://github.com/warpdotdev/warp-server/blob/ee625e8c1968c80c3e94e2f46c8afae0918c786a/model/conversation_metadata.go#L59-L69) currently overwrites `ai_conversation_metadata.title` on every metadata upsert.
- [`model/conversation_metadata.go (129-135) @ ee625e8](https://github.com/warpdotdev/warp-server/blob/ee625e8c1968c80c3e94e2f46c8afae0918c786a/model/conversation_metadata.go#L129-L135) has `setTitleByConversationIDQuery`, which updates the metadata title but does not mark it as manually owned.
- [`model/types/conversation_metadata.go (95-98) @ ee625e8](https://github.com/warpdotdev/warp-server/blob/ee625e8c1968c80c3e94e2f46c8afae0918c786a/model/types/conversation_metadata.go#L95-L98) exposes `SetTitleByConversationID`; rename should replace or extend this with a manual-title-aware store method.
- [`model/ai_tasks.go (1362-1380) @ ee625e8](https://github.com/warpdotdev/warp-server/blob/ee625e8c1968c80c3e94e2f46c8afae0918c786a/model/ai_tasks.go#L1362-L1380) updates `ai_tasks.title` by `agent_conversation_id`, covering the run management surface.
- [`logic/ai/multi_agent/gcs/conversation_data.go (36-40) @ ee625e8](https://github.com/warpdotdev/warp-server/blob/ee625e8c1968c80c3e94e2f46c8afae0918c786a/logic/ai/multi_agent/gcs/conversation_data.go#L36-L40) exposes GCS read/write APIs for stored conversation data. Rewriting this data just to update display titles is avoidable and should not be part of the first pass.
- [`router/handlers/maa_usage.go (161-169) @ ee625e8](https://github.com/warpdotdev/warp-server/blob/ee625e8c1968c80c3e94e2f46c8afae0918c786a/router/handlers/maa_usage.go#L161-L169) derives generated conversation titles from the root task description at stream finish.
- [`router/handlers/maa_usage.go (317-324) @ ee625e8](https://github.com/warpdotdev/warp-server/blob/ee625e8c1968c80c3e94e2f46c8afae0918c786a/router/handlers/maa_usage.go#L317-L324) writes that title into `ConversationUsage`.
## Proposed changes
Server first, then client local update:
1. Add a public API endpoint in `warp-server`:
   - Route: `POST /agent/conversations/{conversation_id}/rename`.
   - Request: `{ "title": string }`.
   - Response: `{ "title": string }` with the normalized title the server accepted.
   - Reject missing, empty-after-trim, null-byte-only, and over-500-character titles with `InvalidRequest`.
   - Require an authenticated principal and `EditContentAction` on the conversation object.
2. Update `public_api/openapi.yaml` and regenerate `public_api/types/types.gen.go` via the existing OpenAPI generator from `public_api/types/generate.go` or `script/codegen`. The new endpoint should be feature-gated consistently with the existing conversation API routes in `RegisterAgentWebhookRoutes`.
3. Add a server logic function, e.g. `logic.RenameConversation`, that updates every server-side title surface in one request:
   - `ai_conversation_metadata.title`.
   - `ai_tasks.title` for the task whose `agent_conversation_id` matches the conversation id.
   - existing conversation metadata/object content that records whether the title is a manual override.
4. Make manual titles sticky:
   - Do not add a top-level `title_is_manual` table column. Keep title-guard state minimal and colocated with the conversation object/metadata payload that already describes the conversation.
   - Prefer adding a small nested display-title metadata field to existing `ConversationUsageMetadata`, for example `DisplayTitle { Source, Value }` or `TitleSource`, so the table continues to expose only the denormalized `title` column plus existing `usage_metadata`.
   - Replace or extend `SetTitleByConversationID` with a manual-title-aware method, e.g. `SetManualTitleByConversationID`, that sets the denormalized `title`, records the manual source in metadata/object content, and bumps `last_updated`.
   - Update ordinary metadata upserts so generated titles do not overwrite `title` when the stored display-title metadata says the title is manual.
   - Ensure the upsert result uses the actual stored title, not the newly generated title, when a manual title is preserved.
5. Do not rewrite stored GCS `ConversationData` for title-only renames:
   - Rewriting transcript bytes is more invasive than the feature needs, creates extra race surfaces with the async final conversation-data write, and conflates display title with recorded task content.
   - Instead, the server returns/fetches the manual display title through conversation metadata, and the client treats that server-provided manual display title as the title override when materializing cloud conversations.
   - If a future UI exposes task-level renaming or transcript rewriting, that should be a separate feature with its own product behavior.
6. Add server tests:
   - Public route auth: unauthenticated request fails; view-only access fails; edit access succeeds.
   - Request validation: empty and too-long titles fail.
   - Rename success updates metadata title, records the manual title source in conversation metadata/object content, and updates an associated task title.
   - Ordinary `UpsertMetadata` after a manual rename preserves the manual title while still updating usage, working directory, git branch, harness, and last-updated metadata as intended.
   - Stream-finish metadata upsert preserves the manual title when a manual title exists, without relying on GCS transcript mutation.
7. Add client server API plumbing:
   - `RenameConversationRequest` and `RenameConversationResponse` in `app/src/server/server_api/ai.rs`.
   - `build_rename_conversation_url`.
   - `AIClient::rename_conversation(conversation_id: String, title: String)`.
   - Implementation using `post_public_api`, matching the fork endpoint style.
8. Add client local-history mutation after server success:
   - Add a narrow manual display title override to `AIConversation`/persisted conversation data, or reuse server metadata if it can reliably carry the manual-title source.
   - Update `AIConversation::title()` to prefer the manual display title override when present, then fall back to the existing root-task-description, initial-query, and fallback-title order.
   - Add a `BlocklistAIHistoryModel` method, e.g. `rename_conversation_after_server_success(conversation_id, title, ctx)`, that updates the local manual display title override, persists conversation state, updates cached metadata, and emits `UpdatedConversationMetadata`.
   - Keep this method local-only; it must not send the server request itself.
9. Add `/rename-conversation` to the slash-command registry:
   - `Availability::AGENT_VIEW | Availability::ACTIVE_CONVERSATION | Availability::AI_ENABLED`.
   - Required argument with a hint such as `<new title>`.
   - Use an icon consistent with rename/edit actions, likely the same pencil icon as `/rename-tab`.
10. Implement command execution in `app/src/terminal/input/slash_commands/mod.rs`:
   - Resolve selected conversation id.
   - Validate title locally for empty and length before network.
   - Resolve the server conversation token/id from the selected conversation; if missing, show a toast and do not update locally.
   - Spawn the server rename request.
   - Only in the success callback, update `BlocklistAIHistoryModel` locally with the title returned by the server and show a success toast if consistent with existing patterns.
   - On error, show an error toast and leave local state unchanged.
11. Generated-title and handoff interactions:
   - Fork/handoff title override paths should continue to work. Forks of a manually renamed conversation should inherit the current display title unless a fork-specific title override is provided.
   - Future title generation should not clear the manual-title metadata; only another successful rename updates the manual title.
12. Changelog:
   - This is user-visible and should likely use `CHANGELOG-IMPROVEMENT:` when a PR is opened.
## Testing and validation
Map tests to `PRODUCT.md` behavior numbers:
1. Client slash-command unit tests for detection/registration and required argument behavior cover Behavior 1-5.
2. Client model tests for the post-success local mutation cover Behavior 6, 8, 13, 14, and 19.
3. Client slash-command handler tests with a mocked `AIClient` cover Behavior 6-11: local title unchanged before success, title updated after success, no update on server failure, no update when server token is missing, and selected-conversation-only targeting.
4. Server public API tests cover Behavior 9, 12, 15, and 18: failed rename leaves stored title unchanged, in-progress-safe server persistence, task/title surfaces converge, and edit permission is required.
5. Server metadata-store tests cover Behavior 12 and 13 by verifying generated metadata upserts preserve manual titles.
6. Cloud-load tests cover Behavior 11-13, 16, and 19 by verifying a cloud conversation whose stored root task description is unchanged still displays the manual title from server metadata after load.
7. Manual validation after implementation:
   - Start or load a conversation, run `/rename-conversation My Project Debugging`, and confirm the visible title changes only after the request succeeds.
   - Run `/rename-conversation` with no argument and confirm the toast/error and unchanged title.
   - Rename a conversation while a response is streaming, let it finish, then confirm the manual title remains in the active pane, conversation list, and after app restart.
   - If a linked cloud run exists, confirm the run management surface shows the same title.
## Parallelization
Use two child agents after spec approval, with the orchestrator integrating and validating the combined branch.
- `server-rename-api` owns `../warp-server-4`: the public API endpoint, permissions, OpenAPI/types regeneration, metadata/object-content manual-title storage, generated-title guard behavior, task-title update, and server tests. This agent should work directly in `/Users/harryalbert/warp-server-4`.
- `client-rename-command` owns `/Users/harryalbert/warp-4`: slash-command registration/handler, `AIClient` plumbing, local post-success title override, local persistence/cache refresh, and client tests. This agent should work directly in `/Users/harryalbert/warp-4`.
- Neither child agent should create a worktree, commit changes, push branches, or open a pull request. Their output should be uncommitted file changes plus a concise report of changed files and validation commands/results.
Ordering:
1. Launch both agents after spec approval. The server agent should report the final endpoint shape and generated request/response names as soon as they are available.
2. The client agent can initially code against the spec shape, then adjust to the server agent's final OpenAPI-generated names if needed.
3. The orchestrator merges or ports both agents' work into the primary checkout, resolves API naming drift, and runs final formatting/checks.
4. Validation should include server tests first, then client `cargo check`/`cargo fmt`-level validation. For Warp client validation, child agents should not run nextest or presubmit; cargo check and cargo fmt are sufficient.
