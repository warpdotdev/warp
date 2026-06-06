# Conversation renaming client spec
## Context
This spec covers the `warp-4` client side of GitHub issue #8642. User-facing behavior is defined in `PRODUCT.md`. The server-side rename API, metadata persistence, `title_is_user_provided`, generated-title guards, task-title sync, and stored `ConversationData` updates are documented in `../warp-server-4/specs/gh-8642/TECH.md`.
On the client, a Warp-native conversation title is derived from the root task description. The client should keep that model: a successful rename updates the local root task description rather than creating a parallel display-title override.
Relevant client code:
- `app/src/search/slash_command_menu/static_commands/commands.rs` registers static slash commands.
- `app/src/terminal/input/slash_commands/mod.rs` handles slash command execution and toasts.
- `app/src/server/server_api/ai.rs` owns the `AIClient` public API plumbing.
- `app/src/ai/agent/conversation.rs` derives conversation titles from root task descriptions.
- `app/src/ai/blocklist/history_model.rs` owns local conversation mutation, SQLite persistence, generic metadata updates, and the title-specific `UpdatedConversationTitle` event.
- `app/src/terminal/view.rs`, `app/src/ai/agent_conversations_model.rs`, and `app/src/workspace/view/conversation_list/view_model.rs` refresh title-bearing surfaces.
## Proposed changes
1. Add `/rename-conversation <title>`:
   - Active only when Agent View, AI, and an active conversation are available.
   - Required title argument with existing slash-command UI patterns.
   - Trim leading/trailing whitespace, preserve internal whitespace, and reject empty or over-500-character titles before network.
2. Add server API plumbing:
   - `RenameConversationRequest` and `RenameConversationResponse`.
   - `build_rename_conversation_url`.
   - `AIClient::rename_conversation`.
   - Implement using `post_public_api` against `POST /agent/conversations/{conversation_id}/rename`.
3. Execute rename without optimistic local mutation:
   - Resolve the selected conversation id.
   - Require a server conversation token/id; if missing, show an error toast and do not call the server.
   - Spawn the server request.
   - On error, show an error toast and leave local state unchanged.
   - On success, use the returned normalized title for local state.
4. Update local canonical title on success:
   - Update the selected conversation's root task description.
   - Persist the updated task list via existing multi-agent conversation persistence.
   - Update cached conversation/server metadata titles.
   - Emit `UpdatedConversationTitle` for rename-specific UI refreshes. Keep `UpdatedConversationMetadata` for non-title metadata/capability changes such as server tokens and permissions.
   - Do not add or persist a separate client-side title override field.
5. Refresh all title-bearing client surfaces:
   - Pane title and tab title refresh through `TerminalView` handling of `UpdatedConversationTitle`.
   - Vertical tabs and workspace chrome refresh from the title-specific history event.
   - Command palette and conversation search read fresh `ConversationNavigationData`.
   - `AgentConversationsModel` maps `UpdatedConversationTitle` to `ConversationUpdateKind::TitleChanged`.
   - Conversation list and management panel rebuild on `ConversationUpdateKind::TitleChanged` for renames, while still using `ConversationUpdateKind::MetadataChanged` for non-title metadata changes.
   - Task-backed rows in `AgentConversationsModel` update cached `AmbientAgentTask.title` from the title event for the renamed conversation so list and management surfaces do not wait for a poll.
6. Preserve fork/handoff semantics:
   - Forks naturally inherit the renamed root task description.
   - Explicit fork or handoff title overrides still take precedence where supplied.
## Testing and validation
Client tests should cover:
1. Slash command registration and availability.
2. Title validation through command-level coverage if added: trim, empty rejection, Unicode scalar limit. Do not keep helper-only unit tests just to cover extracted validation.
3. Rename success updates root task description only after server success.
4. Rename failure leaves local title unchanged.
5. Local persistence/restoration derives the renamed title from root task description.
6. Conversation list and management panel refresh cached task-backed titles on `UpdatedConversationTitle` / `ConversationUpdateKind::TitleChanged`.
7. Pane title refreshes immediately after `UpdatedConversationTitle`.
Validation commands:
- `./script/format`.
- `cargo check -p warp`.
- `cargo check -p warp --tests`.
- `cargo clippy -p warp --tests -- -D warnings`.
- `git diff --check`.
Do not run nextest or presubmit for this client validation pass.
