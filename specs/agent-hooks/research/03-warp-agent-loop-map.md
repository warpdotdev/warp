# 03 - Warp agent loop map (for agent hooks scoping)

Repo: `warp` @ `393e97a30a5ed85cf062b75e88326f83746dbecb` (master). All paths below are relative to the repo root unless noted. Line numbers are from this commit.

Legend: **[verified]** = read in code. **[inferred]** = reasonable conclusion from code read, not directly observed. **not found** = searched, nothing located.

---

## Section 1 - Loop topology

### 1.1 One-paragraph summary

Warp's built-in agent ("Oz" / Agent Mode) is a **client-driven tool loop with a server-hosted model loop**. The client never calls an LLM directly: every turn is a `POST {server}/ai/multi-agent` SSE request carrying the *entire* task/message history (`task_context.tasks`) plus the new inputs; warp-server runs the model (plus server-side subagents, web search, summarization) and streams protobuf `ResponseEvent`s back. Tool calls arrive as streamed `ToolCall` messages; the client converts them to `AIAgentAction`s, queues them **after the stream finishes**, runs them locally through per-tool executors (subject to the permission model), then sends the `AIAgentActionResult`s back as the inputs of the next request. A "turn" ends when a stream finishes with no actions that require a follow-up. **[verified]**

### 1.2 Where the loop lives

All of the agent loop is in the `warp` app crate (`app/`), under `app/src/ai/`. Shared pieces live in `crates/ai` (action/result types) and `crates/ai_types` (ids). The TUI (`crates/warp_tui`) reuses the exact same models; it only supplies its own views and event bridges (see 1.6). **[verified]**

| Concern | Type / fn | Location |
|---|---|---|
| Per-surface controller: builds requests, owns response streams, triggers follow-ups | `BlocklistAIController` | `app/src/ai/blocklist/controller.rs:335` |
| Request construction to server | `BlocklistAIController::send_request_input` | `controller.rs:2981-3245` |
| User prompt entry | `send_query` → `input_for_query` | `controller.rs:759-983`, `controller.rs:4196-4260` |
| Follow-up after tool results | `send_follow_up_for_conversation` | `controller.rs:2089-2204` |
| Streaming event application | `apply_response_event` / `handle_response_stream_event` / `handle_response_stream_finished` | `controller.rs:3550-3628`, `3670-3935`, `3988-4180` |
| Action queue + confirmation gating | `BlocklistAIActionModel` | `app/src/ai/blocklist/action_model.rs:225` |
| Tool dispatcher (one executor per tool) | `BlocklistAIActionExecutor::try_to_execute_action` | `app/src/ai/blocklist/action_model/execute.rs:599-854` |
| Per-tool executors | `execute/{shell_command,read_files,request_file_edits,call_mcp_tool,read_mcp_resource,grep,file_glob,run_agents,start_agent,ask_user_question,wait_for_events,...}.rs` | `app/src/ai/blocklist/action_model/execute/` |
| Permission decisions | `BlocklistAIPermissions` | `app/src/ai/blocklist/permissions.rs:134` |
| Conversation / transcript model | `AIConversation`, `Task`, `AIAgentExchange` | `app/src/ai/agent/conversation.rs:311`, `app/src/ai/agent/task.rs`, `app/src/ai/agent/mod.rs:3471` |
| Global conversation registry + persistence | `BlocklistAIHistoryModel` | `app/src/ai/blocklist/history_model.rs` |
| HTTP/SSE transport to warp-server | `warp_multi_agent_client::generate_multi_agent_output` | `crates/warp_multi_agent_client/src/lib.rs:48-124` (endpoint at `:135-150`) |
| Proto request assembly | `generate_multi_agent_output` | `app/src/ai/agent/api/impl.rs:16-178` |
| Request params (model ids, autonomy level, MCP context, redaction flag) | `RequestParams::new` | `app/src/ai/agent/api.rs:254-464` |
| Action / result type definitions | `AIAgentActionType`, `AIAgentActionResultType` | `crates/ai/src/agent/action/mod.rs:38-214`, `crates/ai/src/agent/action_result/mod.rs` |
| Proto → action conversion | `From<api::message::tool_call::*> for AIAgentActionType` | `crates/ai/src/agent/action/convert.rs` |

### 1.3 Step-by-step: prompt → model → tool → result → end of turn **[verified]**

1. **Prompt submission.** `BlocklistAIController::send_user_query_in_conversation_internal` (`controller.rs:1307-1449`) promotes pending context blocks, picks the task, and calls `send_query` (`controller.rs:759`). `send_query` cancels any in-flight stream for the active conversation (`FollowUpSubmitted`), drains already-finished action results so they are prepended as `AIAgentInput::ActionResult`, builds context via `input_context_for_request` (`controller/input_context.rs:47-98`), and builds the `AIAgentInput::UserQuery` via `input_for_query` (`controller.rs:4196`). Slash commands (`/compact`, `/init`) are routed to `SlashCommandRequest` (`controller/slash_command.rs:52-66`) instead.
2. **Request build.** `send_request_input` (`controller.rs:2981`) reads the conversation's `compute_active_tasks()` (full proto task history), builds `api::RequestParams::new` (`api.rs:254`), creates a `ResponseStream` model (`controller/response_stream.rs:374`), registers it in `PendingResponseStreams`, appends a new `AIAgentExchange` to the conversation (`history_model.update_conversation_for_new_request_input`, `history_model.rs:1204`), sets status `InProgress`, emits `BlocklistAIControllerEvent::SentRequest` (`controller.rs:3222`).
3. **Model call (server).** `ResponseStream::spawn_request` → `api::impl::generate_multi_agent_output` (`api/impl.rs:16`) which optionally redacts secrets from inputs (`api/impl.rs:55-57`, `agent/redaction.rs:13`), converts to proto, and POSTs to `/ai/multi-agent` (`crates/warp_multi_agent_client/src/lib.rs:48-124`). The server owns the model loop, system prompt, rules/Warp Drive context, web search, summarization, and server-side subagents (CLI subagent for long-running commands, research, advice, computer-use).
4. **Streaming.** `ResponseEvent::Init` → `history_model.initialize_output_for_response_stream` (`history_model.rs:1515`) assigns the server conversation token. `ResponseEvent::ClientActions` → `history_model.apply_client_actions` (`history_model.rs:2044`) → `AIConversation::apply_client_action` (`conversation.rs:2875-3561`): `AddMessagesToTask`, `AppendToMessageContent`, `UpdateTaskMessage` mutate the task's proto message list *and* the exchange's client output (`Task::add_messages` / `upsert_message` / `append_to_message_content`, `task.rs:630-789`); each converts proto messages via `api/convert_from.rs:205-251` into `AIAgentOutputMessageType::{Text, Reasoning, Action, Subagent, Summarization, ...}` (`agent/mod.rs:1901-1943`). Every change emits `BlocklistAIHistoryEvent::UpdatedStreamingExchange`.
5. **Stream end.** `ResponseEvent::Finished` → `handle_response_stream_finished` (`controller.rs:3988`) records cost/usage and marks the exchange `Finished{Success|Error}` based on `StreamFinished.reason`. Then `ResponseStreamEvent::AfterStreamFinished` (`controller.rs:3788-3933`) **collects all `AIAgentAction`s from the stream's new exchanges and calls `action_model.queue_actions(...)` (`controller.rs:3887-3891`)**. This is the single place tool calls enter the client's execution queue; actions are therefore dispatched only after the whole model response has streamed.
6. **Tool dispatch.** `BlocklistAIActionModel::queue_actions` (`action_model.rs:968-999`) first runs per-action `preprocess_action` futures (notably file-edit diff computation, `execute/request_file_edits.rs:214-270`), then `handle_preprocess_actions_results` (`action_model.rs:1020-1068`) pushes to `pending_actions` and calls `try_to_execute_available_actions` (`action_model.rs:479-520`). Each action goes through `start_pending_action_by_id` (`action_model.rs:898-953`) → `BlocklistAIActionExecutor::try_to_execute_action` (`execute.rs:599`): permission check (`should_autoexecute`, `execute.rs:975-1073`), then either `NotExecuted{NeedsConfirmation}` (conversation set to `ConversationStatus::Blocked`, `action_model.rs:848-871`) or executor `execute(...)` returning `ActionExecution::{Sync, Async}`. Read-only tools (`ReadFiles`, `SearchCodebase`, `ReadSkill`, sometimes `Grep`/`FileGlob`) may run in parallel (`execute.rs:452-474`, `action_model.rs:212-223`).
7. **Tool completion.** Executor emits `BlocklistAIActionExecutorEvent::FinishedAction` (`execute.rs:821-835` async, `:838-852` sync) → `BlocklistAIActionModel::handle_action_result` (`action_model.rs:1343-1452`): the single choke point for every terminal outcome (success/failure/cancel). It stores the result in `finished_action_results`, emits `BlocklistAIActionEvent::FinishedAction`, and starts the next pending action.
8. **Result back to model.** `BlocklistAIController` subscribes to `BlocklistAIActionEvent::FinishedAction` (`controller.rs:492-631`). When no actions remain for the conversation and any result `should_trigger_request_upon_completion()`, it calls `send_follow_up_for_conversation` (`controller.rs:2089`), which drains finished results into `RequestInput::for_actions_results` (`controller.rs:248-276`; each becomes `AIAgentInput::ActionResult{result, context}`), optionally piggybacks pending orchestration events (`controller.rs:2161-2187`) and a steered queued prompt (`steer_head_prompt_for_request`), and calls `send_request_input` again (step 2).
9. **Turn end.** If results don't trigger a follow-up (the only common case is a passive code-diff) the status is set from the results (`controller.rs:573-617`). Otherwise the turn ends when a stream finishes with **no** actions: `mark_response_stream_completed_successfully` (`history_model.rs:2136`) → `AIConversation::mark_request_completed` sets `ConversationStatus::Success`; `FinishedReceivingOutput` is emitted at `controller.rs:3924`. Error reasons map to `ConversationStatus::Error` at `controller.rs:4028-4172`.

### 1.4 Client vs server responsibilities **[verified]**

- **Always on the client:** tool execution for every `AIAgentActionType` variant (`ConversationDriver::Native.executes_tool_calls_locally() == true`, `app/src/ai/agent/conversation_driver.rs:33-41`), permission decisions, the transcript (full proto task list is held client-side and re-sent each request: `api.rs:433` `tasks: conversation.tasks`), local persistence (SQLite via `ModelEvent`, `conversation.rs:3981-4075`), MCP server processes (`app/src/ai/mcp/`), prompt redaction (`api/impl.rs:55`).
- **Always on the server:** the LLM call (BYO API keys are *forwarded* to the server in `settings.api_keys`, `api/impl.rs:95`), system prompt + rules + Warp Drive context (`rules_enabled`, `warp_drive_context_enabled` flags, `api/impl.rs:78-79`), web search/fetch, **context summarization** (client only receives `Message::Summarization` and the `MoveMessagesToNewTask` client action, `conversation.rs:3501-3551`), server-side subagents (`SubagentType::{Cli,Research,Advice,ComputerUse,Summarization,...}`, `agent/mod.rs:1847-1864`), orchestration event bus / messaging between agents, child-agent run creation for remote children.
- **Hybrid:** long-running commands in "interact" mode: the client runs the command in its PTY and returns a `LongRunningCommandSnapshot`; the server then drives a CLI subagent task (`AgentType::Cli`) that issues `WriteToLongRunningShellCommand` / `ReadShellCommandOutput` / `TransferShellCommandControlToUser` tool calls, which the client executes (`execute/shell_command.rs:414-520`).

### 1.5 Response stream wire format **[verified]**

`crates/warp_multi_agent_client/src/lib.rs:48-124`: SSE over HTTPS, each event a base64 protobuf `warp_multi_agent_api::ResponseEvent`. Proto source lives outside this repo in `warp-proto-apis` (pinned at `Cargo.toml:351`; local checkout `~/.cargo/git/checkouts/warp-proto-apis-2098b3a955068931/c5aa183/apis/multi_agent/v1/*.proto`).

### 1.6 How the topology differs per front-end / runtime

The user's hypothesis (differences mostly affect *which configs load from where*, not parsing/execution) is **largely confirmed for GUI vs TUI vs Oz cloud agent**, with two notable exceptions: (a) **remote orchestration children run no loop at all on the parent's client**, and (b) **the headless/autonomous modes change the permission outcome** (not just config location). Details:

**GUI app (`app/`)** **[verified]**
- `TerminalView::new` constructs `BlocklistAIContextModel`, `BlocklistAIInputModel`, `BlocklistAIActionModel`, `BlocklistAIController` (`app/src/terminal/view.rs:3541-3610`).
- GUI-specific bridges: `handle_shell_command_executor_event` runs the command in the PTY (`view.rs:7856-8040`); `handle_start_agent_executor_event` → `Event::StartAgentConversation` → `dispatch_start_agent_conversation` in `app/src/pane_group/pane/terminal_pane.rs:1533-1640` (hidden child panes); confirmation UI is `AIBlock` + `CodeDiffView` (`app/src/ai/blocklist/block.rs:3244-3308`); diff storage is `CodeDiffView` (`inline_action/code_diff_view.rs:345,2965`).

**TUI (`crates/warp_tui`)** **[verified]**
- `TuiTerminalSessionView::new` constructs the *same four models* (`crates/warp_tui/src/terminal_session_view.rs:1513-1598`) with a TUI `ConversationSelection` and `TuiInputModePolicy`.
- TUI-specific bridges: `handle_shell_command_executor_event` (`terminal_session_view.rs:5136-5213`); `TuiTerminalSessionEvent::StartAgentConversation` handled in `crates/warp_tui/src/session_registry.rs:327` (background sessions); permission cards `TuiPermissionPrompt` used by `tui_shell_command_view.rs:247-272`, `tui_file_edits_view.rs:735`, `tui_generic_tool_call_view.rs:173`; diff storage `TuiDiffStorage` writes straight through `FileModel` with no review editor (`crates/warp_tui/src/tui_diff_storage.rs:212-322`).
- Shared code: everything under `app/src/ai/blocklist/{controller,action_model,permissions,history_model}` and `app/src/ai/agent/`.

**Oz cloud agent (remote run / `oz agent run`)** **[verified]**
- Runs the *same app crate* in `ExecutionMode::Sdk` (`app/src/lib.rs:570` `LaunchMode::CommandLine → ExecutionMode::Sdk`; `crates/warp_core/src/execution_mode.rs:10-19`). `AgentDriver` (`app/src/ai/agent_sdk/driver.rs:704`) creates a **headless `TerminalView`** via `TerminalDriver` (`agent_sdk/driver/terminal.rs:149`, `open_new_with_workspace_source`) and drives the normal `BlocklistAIController` through `execute_run` (`driver.rs:3441`), sending `AIAgentInput::StartFromAmbientRunPrompt` and subscribing to `BlocklistAIHistoryEvent`s to stream output to stdout (`driver.rs:3567-3640`).
- Differences that *do* change behavior beyond config loading:
  - `RequestParams.autonomy_level = Unsupervised` when `is_autonomous()` (`api.rs:369-373`); `isolation_level = Sandbox` when sandboxed (`api.rs:375-379`). These are sent to the server and (inferred) change server-side prompting/tooling.
  - Default permission profile is `create_default_cli_profile` with every action `AlwaysAllow`, `ask_user_question: Never`, `run_agents: AlwaysAllow` (`crates/cloud_object_models/src/ai_execution_profile.rs:463-503`, selected at `app/src/ai/execution_profiles/profiles.rs:369-371`). An explicit profile can be set per run (`AgentDriver::configure_terminal`, `driver.rs:3396-3418`).
  - If an action still needs confirmation in autonomous mode, denylisted commands short-circuit to `RequestCommandOutputResult::Denylisted` (`execute.rs:632-656`); anything else sets `ConversationStatus::Blocked` and the driver fails the run with `AgentDriverError::ConversationBlocked` (`driver.rs:832-834`, `:941-942`).
  - Third-party harnesses (Claude Code etc.) inside Oz use `ConversationDriver::ExternalHarness`, where the client **does not** execute tools (`conversation_driver.rs:33-41`); those are out of scope for native-agent hooks but relevant for "Claude Code hook formats later".

**Orchestration child agents (`run_agents`)** **[verified]**
- *Local child* (`StartAgentExecutionMode::Local{harness_type: None}`): the parent's front-end creates a **second full agent surface** (GUI: hidden pane via `launch_local_no_harness_child`, `terminal_pane.rs:1558-1570`; TUI: background session via `session_registry.rs:327`). The child has its own `BlocklistAIController` + action model + executor, i.e. a second complete loop in the same process. Permissions and model are copied from the parent surface by `inherit_child_agent_settings` (`app/src/ai/blocklist/child_agent_launch.rs:98-124`). A server task row is created first (`prepare_local_oz_child_launch`, `child_agent_launch.rs:34-67`).
- *Local child with 3p harness* (`harness_type: Some(..)`): `launch_local_harness_child` (`terminal_pane.rs:1572-1586`); the child is an `ExternalHarness` conversation — no native tool execution.
- *Remote child*: `launch_remote_child` (`terminal_pane.rs:1603-1640`, request prep in `app/src/ai/orchestration/remote_child.rs`) asks warp-server to start a cloud run. Locally there is only a `ConversationDriver::RemoteChild` placeholder (`conversation_driver.rs:14`, `history_model.ensure_remote_child_conversation`, `history_model.rs:604`). **No hook can observe that child's tool calls from the parent's machine; the child's own Oz runtime (above) would have to run the hooks.**
- Child results reach the parent as orchestration events (server SSE) via `OrchestrationEventService` (`app/src/ai/blocklist/orchestration_events.rs:99`) and are injected into the parent's next request either piggybacked on a follow-up (`controller.rs:2180-2187`) or as a standalone idle injection (`inject_pending_events_for_request`, `controller.rs:2447-2518`) as `AIAgentInput::MessagesReceivedFromAgents` / `EventsFromAgents` (`agent/mod.rs:3096-3102`).

**Shared-session viewer / CLI transcript** are view-only drivers (`conversation_driver.rs:10-12`) and never execute tools; hooks are N/A there.

---

## Section 2 - Identifiers and state

| Cursor concept | Warp equivalent | Type | Created / set where |
|---|---|---|---|
| conversation / session id (client) | `AIConversationId` (UUID) | `crates/ai_types/src/agent.rs:9` | `AIConversation::new` (`conversation.rs:464`), via `BlocklistAIHistoryModel::start_new_conversation` (`history_model.rs:1421-1448`) |
| conversation id (server) | `ServerConversationToken` | `app/src/ai/agent/api.rs:48` | Set from `StreamInit` on first response (`history_model.rs:1515-1561`, `ConversationServerTokenAssigned` event) |
| run id (Oz task / orchestration identity) | `AmbientAgentTaskId` (`AIConversation.task_id`), string form `run_id()` | `conversation.rs:357` | From `StreamInit.run_id`, or `assign_run_id_for_conversation` (`history_model.rs:1567`) |
| generation / turn id | `AIAgentExchangeId` (UUID per request/response pair); `ResponseStreamId` per HTTP stream; `ServerOutputId` per server response | `agent/mod.rs:3440`, `controller/response_stream.rs:226`, `agent/mod.rs:65` | Exchange created in `update_conversation_for_new_request_input` (`history_model.rs:1204`); `ServerOutputId` from `StreamInit` (`conversation.rs:4989`). A *user-visible turn* spans multiple exchanges: `AIConversation::turn_exchange_ids` (`conversation.rs:3620-3638`) groups from one `UserQuery` to the next. |
| tool call id | `AIAgentActionId` (wraps the server `tool_call_id` string) on `AIAgentAction.id` | `crates/ai_types` (re-exported `agent/mod.rs:30`), `AIAgentAction` at `agent/mod.rs:1151-1166` | Converted from proto `ToolCall` in `convert_from.rs:239-251`; also `AIAgentAction.task_id: TaskId` |
| workspace root / project dir | **No first-class project root.** Candidates: session cwd (below); `AIAgentContext::Codebase{path}` for indexed repos (`input_context.rs:100-119`); `AIAgentContext::ProjectRules{root_path}` (`agent/mod.rs:2351`); `TerminalView::current_repo_path` (GUI, `view.rs:7806`); Oz run `AgentDriverOptions.working_dir` (`driver.rs:646`) | — | — |
| current cwd | `ActiveSession::current_working_directory()`; snapshotted into `RequestInput.working_directory` (`controller.rs:312-315`) and `AIAgentExchange.working_directory` (`agent/mod.rs:3495`); `SessionContext` (`controller.rs:100-171`) also carries shell + session type (local vs `WarpifiedRemote{host_id}`) | `app/src/terminal/model/session/active_session.rs` | per request |
| model name | `LLMId`; `RequestInput.model_id/coding_model_id/cli_agent_model_id/computer_use_model_id` (`controller.rs:214-217`), actual used model in `OutputModelInfo.model_id` from `Message::ModelUsed` (`conversation.rs:3226-3247`) | `app/src/ai/llms.rs` | `LLMPreferences::get_active_base_model` (`controller.rs:295-311`) |
| transcript | `AIConversation.task_store: TaskStore` of `Task`s; each `Task` holds proto `api::Task.messages` (wire truth) and `Vec<AIAgentExchange>` (client view) | `conversation.rs:316`, `task.rs` | persisted via `write_updated_conversation_state` (`conversation.rs:3981`) → SQLite `ModelEvent`; restored by `restore_conversations` (`history_model.rs:1248`) |
| user identity | `warp_server_auth::User { local_id: UserUid, metadata: UserMetadata{email, display_name}, principal_type, ... }` | `crates/warp_server_auth/src/user.rs:84-120` | Managed by `AuthManager` (`app/src/auth/auth_manager.rs`); exact accessor for "current user" not pinned down in this pass |
| team scope | `TeamContext` / `RequestTeamScope`, pinned at send (`controller.rs:3103-3105`) | `app/src/workspaces/user_workspaces.rs` | — |
| telemetry join ids | `AIIdentifiers { client_conversation_id, client_exchange_id, server_output_id, server_conversation_id, model_id }` | `app/src/ai/agent/telemetry.rs:46-71` | built per stream at `controller.rs:3123-3129` |
| execution mode | `AppExecutionMode { mode: App|Tui|Sdk|RemoteServerDaemon, is_sandboxed }` | `crates/warp_core/src/execution_mode.rs:57-61` | set at launch (`app/src/lib.rs:570-574`) |
| per-conversation autonomy override | `AIConversationAutoexecuteMode::{RespectUserSettings, RunToCompletion}` ("auto-approve") | `conversation.rs:5029-5039` | `toggle_autoexecute_override` (`conversation.rs:4149`) |
| conversation status | `ConversationStatus::{InProgress, Success, Error, TransientError, Cancelled, Blocked{blocked_action}, WaitingForEvents}` | `conversation.rs:5068-5092` | `update_conversation_status` (`history_model.rs:1464`) |

---

## Section 3 - Interception points

For each: location, data available, blockable (can the flow `await` an external process?), modifiable, how it surfaces, shared across front-ends.

General notes that apply everywhere **[verified]**:
- The whole loop runs on the warpui model thread; long work is done with `ctx.spawn(future, on_complete)`. Any point that already returns a future (executor `ActionExecution::Async`, `preprocess_action` `BoxFuture`) can trivially await a hook process. Points that are synchronous `&mut self` methods (e.g. `send_query`, `should_autoexecute`) cannot block without restructuring into a spawn + continuation.
- There is already a precedent for "mutate the request right before it leaves": `redaction::redact_inputs(&mut params.input)` in `api/impl.rs:55-57` (operates on `Vec<AIAgentInput>` *inside the async send path*, so it runs off the model thread and could await).

### a. Session start / end

**Start (create):**
- `BlocklistAIHistoryModel::start_new_conversation` (`history_model.rs:1421-1448`) creates `AIConversation`, emits `BlocklistAIHistoryEvent::StartedNewConversation{new_conversation_id, terminal_surface_id}`. Called from `BlocklistAIController::start_new_conversation_for_request` (`controller.rs:2944-2968`) and the front-end `ConversationSelection::try_start_new_conversation` impls (`app/src/ai/blocklist/conversation_selection.rs:86,167`; `agent_view/conversation_selection.rs:160`). Note the client `AIConversationId` exists before any server token; the server token arrives on first `StreamInit` (`ConversationServerTokenAssigned`, `history_model.rs:1555-1560`).
- Data: `AIConversationId`, `terminal_surface_id`, autoexecute override; cwd via the surface's `ActiveSession`.
- **Resume:** `restore_conversations` (`history_model.rs:1248`) for local restore; `BlocklistAIController::resume_conversation` (`controller.rs:2563`) sends `AIAgentInput::ResumeConversation`; Oz resume via `AgentDriverOptions.resume` (`driver.rs:666-669`).
- Blockable: no (sync, emits events). Modifiable: n/a.
- Shared: yes (history model).

**End (close):** there is no single "session end" event. Candidates:
- Turn completion: `ConversationStatus::Success|Error|Cancelled` via `update_conversation_status` (`history_model.rs:1464-1491`) → `BlocklistAIHistoryEvent::UpdatedConversationStatus`.
- Surface close: `clear_conversations_for_terminal_surface` / `clear_conversations_for_closed_terminal_surface` (`history_model.rs:2268-2315`), `delete_conversation` (`history_model.rs:2328`).
- Oz run exit: `AgentDriver::execute_run`'s `IdleTimeoutSender` (`driver.rs:3449-3525`), `cleanup` (`driver.rs:4450`).
- Blockable: `AgentDriver::cleanup` is async **[verified]**; GUI/TUI close paths are sync.

### b. Prompt submission (before send)

- **Best single point:** `BlocklistAIController::send_query` (`controller.rs:759-983`), specifically after `input_for_query` builds the `AIAgentInput::UserQuery{query, context, referenced_attachments, user_query_mode, running_command, intended_agent, base}` (`controller.rs:920-937`) and before `send_request_input` (`controller.rs:952`). All inputs for the request (`inputs: Vec<AIAgentInput>`, including prepended action results and queued passive results) are in a local `Vec` at this moment.
- Alternative async point covering *every* request kind (follow-ups, slash commands, orchestration injections): `generate_multi_agent_output` in `api/impl.rs:16-57`, right where `redact_inputs` already mutates `params.input`. This is inside an `async fn`, so it can await a hook; it has `RequestParams` (model ids, session context, tasks, conversation token, autonomy level, MCP context) but **not** the warpui `AppContext`.
- Data: query text, `referenced_attachments: HashMap<String, AIAgentAttachment>` (blocks, files, drive objects, images via `AIAgentContext::Image`), `context: Arc<[AIAgentContext]>` (cwd `Directory`, `ExecutionEnvironment{os, shell_name, shell_version}`, `CurrentTime`, `Codebase`, `Skills`, `Git`, `Repository`, `PullRequest`, `Block`), `conversation_id`, `task_id`, model ids (via `RequestInput`).
- Blockable: `send_query` is sync (`&mut self, ctx`) → no; `api/impl.rs` path → yes.
- Modifiable: yes at either point (`query: String` and `inputs` are owned; `redact_inputs` proves the pattern).
- Surfaces to user: the exchange's input is what gets rendered in the AI block/transcript, so a rewritten query would be visible unless kept in `base`/display fields (see `display_query`, `agent/mod.rs:3426`).
- Shared: yes.
- Caveat: attachments by `@file` reference are `AIAgentAttachment::FilePathReference` (`controller.rs:4267`) and the **server** reads/requests their content (inferred); block attachments carry content client-side.

### c. Generic tool call before / after (including failure)

- **Before (all tools, after permission):** `BlocklistAIActionExecutor::try_to_execute_action` (`execute.rs:599-854`). `action: AIAgentAction{id, task_id, action, requires_result}`, `conversation_id`, `is_user_initiated`. The permission result is computed at `:618` (`should_autoexecute`), the dispatch `match` at `:660-792`. Inserting a hook *between* `:657` and `:660` gives "permission already granted, about to run" semantics; inserting before `:618` gives "decide allow/deny/ask". `action` is owned (`AIAgentAction`) so arguments can be rewritten before dispatch (precedent: `handle_requested_command_accepted` rewrites `command` in place, `action_model.rs:1312-1341`).
- **Before (earlier, pre-permission, async-friendly):** `BlocklistAIActionModel::queue_actions` (`action_model.rs:968-999`) runs `preprocess_action` futures for every action and only enqueues after `join_all`. A hook awaited inside the preprocess phase can block without restructuring; but at that point there is no permission decision yet and actions have not been shown as "pending" to the user.
- **After (all tools, all outcomes):** `BlocklistAIActionModel::handle_action_result` (`action_model.rs:1343-1452`) — documented as the choke point for success, failure, and cancellation. Data: `Arc<AIAgentActionResult{id, task_id, result: AIAgentActionResultType}>`, `conversation_id`, `cancellation_reason: Option<CancellationReason>`. Failure detection: `AIAgentActionResultType::is_failed()` / `is_cancelled()` (`action_model.rs:129-141`). Result is behind `Arc` → not modifiable there; the modifiable spot is the executor's `on_complete` closure (`execute.rs:821-835`) or, for everything at once, `send_follow_up_for_conversation` where `finished_results: Vec<AIAgentActionResult>` is owned (`controller.rs:2105-2107`) before `RequestInput::for_actions_results`.
- Blockable after: `handle_action_result` is sync; the `on_complete` in `ctx.spawn` is sync too, but the future it awaits (`execute_future`) can be wrapped to await a hook before resolving. For a shared implementation, wrapping the future at `execute.rs:807-836` (one place for all async executors) is the natural spot; sync executors (`ActionExecution::Sync`, e.g. `InitProject`, denylisted command) would need separate handling.
- Shared: yes (all of this is in `app/src/ai/blocklist`).

### d. Shell command (`run_shell_command` = `RequestCommandOutput`)

- Action fields: `command, is_read_only, is_risky, wait_until_completion, uses_pager, rationale, citations` (`crates/ai/src/agent/action/mod.rs:41-64`). **Risk classification gap:** the proto has `RiskCategory risk_category = 7` with READ_ONLY/TRIVIAL_LOCAL_CHANGE/NONTRIVIAL_LOCAL_CHANGE/EXTERNAL_CHANGE/RISKY (`task.proto:660-662`, `:2116-2123` in the warp-proto-apis checkout) but the client conversion `From<RunShellCommand>` only maps `is_read_only`/`is_risky` (`crates/ai/src/agent/action/convert.rs:21-40`). A hook wanting the fine-grained category needs that field plumbed through. **[verified]**
- Permission: `ShellCommandExecutor::should_autoexecute` (`execute/shell_command.rs:203-288`) → `BlocklistAIPermissions::can_autoexecute_command` (`permissions.rs:912-1031`). See Section 4.
- Before execution: `ShellCommandExecutor::execute` (`execute/shell_command.rs:307-413`): checks terminal busy / displaced, decorates pager commands (`:358-363`), emits `ShellCommandExecutorEvent::ExecuteCommand{action_id, command: decorated_command}` (`:372-375`). The front-end bridge actually runs it: GUI `view.rs:7863-8001` (PowerShell wrapping at `:7909-7919`, `Event::ExecuteCommand` at `:7960`), TUI `terminal_session_view.rs:5143-5172`. cwd = the session's cwd at that moment (`ActiveSession`); not passed in the event.
- Interact mode: `wait_until_completion == false` ⇒ `action_result_future` resolves with `ActionResult::LongRunningCommandSnapshot{grid_contents, cursor, is_alt_screen_active, activity}` (`shell_command.rs:994-1010`) and `handle_action_result` cancels other pending command actions (`action_model.rs:1376-1385`); the server's CLI subagent then drives `WriteToLongRunningShellCommand{block_id, input: Bytes, mode}` (`shell_command.rs:414-474`, PTY write via `ShellCommandExecutorEvent::WriteToPty`), `ReadShellCommandOutput`, `TransferShellCommandControlToUser`. The `interact_task` text never reaches the client (server-side).
- After execution: result `RequestCommandOutputResult::Completed{command, block_id, output, exit_code, start_ts, completed_ts}` built in `action_result_for_requested_command` (`shell_command.rs:975-1029`); also `ShellRecovered`, `TerminalBusy`, `CancelledBeforeExecution`, `Denylisted`. Output comes from the terminal block (`block.output_with_secrets_unobfuscated()` for the finished-block path, `shell_command.rs:429`).
- Blockable before: `execute` is sync but returns `ActionExecution::Async` whose future (`action_result_future`) is awaited; the *command start* happens on the emitted event, so blocking before emission requires making the pre-emit step async (e.g., via `preprocess_action`, which is currently a no-op for shell commands at `shell_command.rs:948-954`).
- Modifiable: command string yes (owned `String`; permission card already does it). Output yes in `on_complete`/follow-up (see c).
- Shared: executor shared; PTY bridge per front-end.

### e. MCP tool call / resource read

- Actions: `CallMCPTool{server_id: Option<Uuid>, name, input: serde_json::Value}`, `ReadMCPResource{server_id, name, uri}` (`action/mod.rs:102-116`).
- Permission: `CallMCPToolExecutor::should_autoexecute` (`execute/call_mcp_tool.rs:41-76`) → `BlocklistAIPermissions::can_call_mcp_tool` / `can_read_mcp_resource` / `can_use_mcp_server` (`permissions.rs:796-888`). Server identity resolved to the template UUID via `TemplatableMCPServerManager` (`app/src/ai/mcp/`).
- Before: `CallMCPToolExecutor::execute` (`call_mcp_tool.rs:79-170`) — arguments are an owned `serde_json::Map` (already mutated by `coerce_integer_args`, `:127-131`, so argument rewriting has precedent), `operation_id: Uuid` for log correlation (`:147`), then `reconnecting_peer.call_tool(...)` awaited in the async future (`:151-160`).
- After: `handle_call_tool_result(res, server_output_id, name, ctx)` in the `on_complete` (`:161-167`) → `CallMCPToolResult`. `ReadMCPResourceExecutor` is the analogous file `execute/read_mcp_resource.rs`.
- Blockable: yes (async future). Modifiable: args and result yes.
- Shared: yes. Not available on wasm (`cfg`).
- Note: MCP tool/resource *catalog* sent to the server is built in `RequestParams::new` (`api.rs:267-335`), which is where a hook that wants to hide tools from the model would act.

### f. File read

- `ReadFiles(ReadFilesRequest{locations: Vec<FileLocations{name, lines}>})`. Permission: `ReadFilesExecutor::should_autoexecute` (`execute/read_files.rs:~30-79`) → `can_read_files_with_conversation` (`permissions.rs:698-714`). Note `ActionPermission::AgentDecides` currently always allows reads (`permissions.rs:747-750`).
- Execute: `ReadFilesExecutor::execute` (`read_files.rs:81-269`): resolves absolute paths with `host_native_absolute_path(name, shell, cwd)`; local path → `read_local_file_context` (`execute.rs:1145-1279`, 1 MB per-file cap, binary/image handling); remote session → `RemoteServerManager` RPC (`read_files.rs:142-234`). Result: `ReadFilesResult::Success{files: Vec<FileContext{file_name, content: AnyFileContent::{StringContent|BinaryContent}, line_range, last_modified, line_count}>, failed_files}`.
- Contents can be replaced/redacted: yes — in `on_complete` (`read_files.rs:228-232`, `:264-267`) the `ReadFilesResult` is owned before it becomes the action result. Blockable: yes (async).
- **Other paths that put file contents in front of the model** (each would need its own hook point):
  - `SearchCodebase` executor (`execute/search_codebase.rs`) and `get_files` (`execute/get_files.rs`) use `read_local_file_context` too (`execute.rs:1122-1125` comment).
  - `Grep` / `FileGlob` return paths/lines (`execute/grep.rs`, `execute/file_glob.rs`).
  - User `@file` attachments (`AIAgentAttachment::FilePathReference`, `controller.rs:4267`) — content fetched server-side **[inferred]**; `AIAgentContext::File(FileContext)` and `ProjectRules{active_rules}` carry content client-side.
  - `ReadSkill` (`execute/read_skill.rs`) returns skill markdown.
  - Passive suggestions attach `relevant_files` (`redaction.rs:52-60`).
  - Shared-session / remote reads route through `remote_server` and `warp_files::FileModel`.
- Shared: yes.

### g. File edit / create / delete

- `RequestFileEdits{file_edits: Vec<FileEdit::{Edit(ParsedDiff::{StrReplaceEdit, V4AEdit}), Create{file, content, allow_overwrite}, Delete{file}}>, title}` (`action/mod.rs:81-84`, conversion `convert.rs:79-128`). `edit_files` and `create_file` from the model both arrive as this one action.
- Timeline **[verified]**:
  1. `queue_actions` → `RequestFileEditsExecutor::preprocess_action` (`execute/request_file_edits.rs:214-270`) → `ApplyDiffModel::apply_diffs` computes in-memory results; `on_diffs_applied` (`:272-337`) builds `Vec<FileDiff{base: DiffBase{content: original, file_path}, diff_type: DiffType::{Create, Update{deltas}, Delete}}>` (`app/src/ai/blocklist/diff_types.rs:12-66`) and hands them to the surface via `RegisteredDiffStorage::set_candidate_diffs` (`diff_storage.rs:124-135`). **Nothing is written to disk yet.** Old content = `base.content`; new content derivable (`final_content_from_op` in TUI storage, editor buffers in GUI); unified diff is computed later in `accept_and_save` (`diff_storage.rs:100-111`).
  2. Permission: `should_autoexecute` (`request_file_edits.rs:68-123`) → `can_write_files` (`permissions.rs:771-793`; MCP config paths always require approval, `:1245-1257`). If not auto-approved the action sits as `NeedsConfirmation` while the user reviews/edits in `CodeDiffView` (GUI) or the TUI card.
  3. On approval (`CodeDiffViewEvent::TryAccept` → `action_model.execute_action`, `block.rs:3290-3302`; TUI `tui_file_edits_view.rs:735`) or auto-approval, `execute` (`request_file_edits.rs:143-212`) calls `storage.accept_and_save(ctx)` → `DiffStorageHelper::accept_and_save` (`diff_storage.rs:83-115`): snapshots `FileSnapshot{updated: UpdatedFileState{path, changed_lines, final_content, was_edited}, deleted_paths, diff_base, diff_new}`, then `start_saving` performs the actual writes (GUI through editor buffers; TUI via `FileModel`, `tui_diff_storage.rs:254-281`).
  4. Result `RequestFileEditsResult::Success{updated_files, ...}` or `DiffApplicationFailed{error}`; failed diff application auto-"executes" so the LLM can recover (`request_file_edits.rs:102-112`).
- Hook points: **pre-write** = inside `accept_and_save` before `start_saving` (has both old/new content and path; async future already) or at `execute` before calling it; **pre-review** = `on_diffs_applied` (`request_file_edits.rs:272-337`) where `FileDiff`s are owned and could be rewritten/filtered before the user sees them. **post-write** = `on_complete` in `execute` (`:188-211`) or generic (c).
- Blockable: yes (futures at both preprocess and execute). Modifiable: yes (owned `FileDiff`s pre-review; `FileSnapshot` pre-write is per-surface).
- Other write paths: Warp Code editor saves (`app/src/code/global_buffer_model.rs:806`, `local_code_editor.rs:1759`) are user-driven, not agent-driven; `CreateDocuments`/`EditDocuments` are AI *plan documents*, not files (`execute/create_documents.rs`, `edit_documents.rs`).
- Shared: executor + `diff_storage.rs` shared; storage impl per front-end.

### h. Child agents (`run_agents`) start / stop

- Start: `RunAgentsExecutor::execute` (`execute/run_agents.rs:402-456`) → `prepare_request_for_execution` policy checks (`:551`) → `dispatch_children_for_prepared_request` (`:238-400`) → per child `StartAgentExecutor::dispatch` (`execute/start_agent.rs:251-287`) which emits `StartAgentExecutorEvent::CreateAgent(StartAgentRequest{name, prompt, execution_mode, metadata, parent_conversation_id, parent_run_id, request_team_scope})`. Front-ends create the child (see 1.6). The `RunAgentsResult::Launched{agents: [{name, Launched{agent_id}|Failed{error}}]}` resolves when each child gets its server id (`start_agent.rs:102-133`, via `ConversationServerTokenAssigned`).
- Permission: `should_autoexecute` (`run_agents.rs:458-500`): autonomous mode or child conversations always auto-run; otherwise approved orchestration config or `run_agents: AlwaysAllow` profile setting.
- Data at start: full child prompt (`compose_run_agents_child_prompt`, `:740`), model, harness, execution mode (local/remote env), skills, metadata. Blockable: yes (`ActionExecution::new_async` over `receiver.recv()`). Modifiable: `request` is an owned clone at `:411`.
- Stop / per-child results: there is **no synchronous "child finished" callback in the parent executor**. Children report via the server event bus; the parent receives `OrchestrationEventServiceEvent::EventsReady` → `handle_pending_events_ready` (`controller.rs:2522`) → `inject_pending_events_for_request` (`controller.rs:2447-2518`), or piggybacked at `controller.rs:2180-2187`. The injected inputs are `AIAgentInput::MessagesReceivedFromAgents{messages}` / `EventsFromAgents{events: Vec<AgentEvent>}` (owned `Vec` at `controller.rs:2456-2462` and `:2180-2186` — a hookable, modifiable point). `wait_for_events` (`execute/wait_for_events.rs`) is the tool the parent uses to block until such events arrive. Child status changes are also visible as `BlocklistAIHistoryEvent::UpdatedConversationStatus` for local children, and `OrchestrationEventStreamerEvent::ChildStatusChanged` (`controller.rs:724-726`).
- Local child lifecycle end: `Event::StopAgentConversation` / `KillAgentConversation` (`terminal_pane.rs:1504-1512`).
- Shared: executors shared; child creation per front-end; remote children run on a different machine.

### i. Context compaction / summarization

- **Triggered server-side.** The client cannot observe "about to compact" before it happens. Evidence: summarization arrives as `Message::Summarization{summary_type: ConversationSummary|ToolCallResultSummary, finished_duration}` (`convert_from.rs:401-448`) and the client action `MoveMessagesToNewTask{source_task_id, new_task, first_message_id, last_message_id, replacement_messages}` which splices the proto message list (`conversation.rs:3501-3551`, `task.rs:849`). Settings flag `supports_summarization_via_message_replacement` is sent in the request (`api/impl.rs:104-105`). The server's trigger heuristics (context window %) live in warp-server (not in this repo); `context_window_limit` is sent per request (`api.rs:423-426`, `impl.rs:75`).
- **User-triggered:** `/compact [prompt]` → `SlashCommandRequest::Summarize{prompt}` → `AIAgentInput::SummarizeConversation{prompt, context}` (`controller/slash_command.rs:58-63`, `:278-280`). This is a request send and therefore observable/blockable at the prompt-submission point (b).
- What is available before it runs: for `/compact`, the full `AIConversation` (all tasks/messages) and the optional user prompt. For automatic compaction: only the pre-summarization transcript as it existed when the previous request was sent (the client cannot inject a "pre-compact" instruction mid-stream).
- After: `UpdatedStreamingExchange` with the `Summarization` output message; usage metadata `was_summarized` (`conversation.rs:270`).
- Shared: yes.

### j. Agent turn stop

- Decision point: `handle_response_stream_finished` (`controller.rs:3988-4180`) sets exchange finished state from `StreamFinished.reason` (`Done | Other | ContextWindowExceeded | QuotaLimit | LlmUnavailable | InvalidApiKey | ChatgptSubscriptionError | InternalError | MaxTokenLimit`), then `AfterStreamFinished` (`controller.rs:3788-3933`) either queues actions (loop continues) or leaves the conversation at `Success`. The "stop reason" available to a hook is therefore: `StreamFinished.reason` + `actions_to_queue.is_empty()` + `CancellationReason` for cancellations (`agent/mod.rs:89-128`) + final `ConversationStatus`.
- Final assistant message: the last exchange's `AIAgentOutput.messages` (`agent/mod.rs:448-476`); text extraction helper `text_from_agent_output()` is used for notifications (`view.rs:8113-8125`).
- Natural hook point: right after `actions_to_queue` is computed and found empty in `AfterStreamFinished` (`controller.rs:3814-3831`, `:3887-3891`), or on `BlocklistAIHistoryEvent::UpdatedConversationStatus` → `Success`.
- **Existing mechanisms to feed a follow-up prompt back automatically** **[verified]**:
  - `QueuedQueryModel` (`app/src/ai/blocklist/queued_query.rs`): queued prompts auto-dispatch when the conversation becomes dispatchable (`controller.rs:695-706`, `can_dispatch_queued_warp_agent_prompt` `:3269`), and can be "steered" into a follow-up (`steer_head_prompt_for_request`, `controller.rs:1851`).
  - `request_follow_up_after_actions` (`controller.rs:1627-1658`) and `send_custom_ai_input_query` / `send_ai_input_with_context` (`controller.rs:1515`, `:1661`) send arbitrary `AIAgentInput`s.
  - `resume_conversation` (`controller.rs:2563`) / `schedule_auto_resume_after_error` (`:2665`).
  - Orchestration events injection (`inject_pending_events_for_request`) and `AgentWake` (`send_agent_wake`, `controller.rs:1452-1483`).
  - Passive suggestion results (`send_passive_suggestion_result`, `controller.rs:1679`).
  A "stop hook returns follow_up prompt" feature could reuse `send_user_query_in_conversation_internal` with `EntrypointType::AgentInitiated` (pattern at `controller.rs:2068-2082`).
- Blockable: `AfterStreamFinished` handling is sync; a blocking stop hook would need to defer `queue_actions`/status finalization behind a spawn.
- Shared: yes. In Oz, `Success` is what triggers the idle-timeout exit (`driver.rs:3514-3524`).

### k. Assistant text and thinking streaming

- Streaming arrives as proto `AgentOutput` / `AgentReasoning` messages via `AddMessagesToTask` (first chunk) and `AppendToMessageContent` (subsequent chunks, field-mask append) → `Task::append_to_message_content` (`task.rs:750-789`) → `AIAgentExchange::upsert_output_for_message`, producing `AIAgentOutputMessageType::Text(AIAgentText)` / `Reasoning{text, finished_duration}` (`convert_from.rs:223-238`, `agent/mod.rs:1901-1908`). Every chunk emits `BlocklistAIHistoryEvent::UpdatedStreamingExchange{exchange_id, conversation_id, ...}` (`conversation.rs:3478-3483`).
- **Boundary events:** there is no dedicated "text block complete" event. Heuristics available: (1) for reasoning, `finished_duration` becomes `Some` when the server sends the final update (`convert_from.rs:228-230`); (2) a new message id appearing after a text message implies the previous one ended; (3) stream end (`Finished`). The Oz driver already writes per-exchange output at `AppendedExchange` / completion (`driver.rs:3597-3640`, `write_exchange_output` `:4035`) — i.e. it treats the *exchange* as the boundary.
- Modifiable: not meaningfully — text is written into the proto message list that is replayed to the server next turn (`api.rs:433`), so client-side rewriting would desync history. Observe-only.
- Shared: yes.

### l. Inline completion / editor hooks (Cursor Tab)

- Warp has a code editor ("Warp Code": `app/src/code/`, `local_code_editor.rs`, `global_buffer_model.rs`) with LSP support (`language_server_extension.rs`, `crates/lsp`) and git diff viewing, but **no AI inline completion in the editor was found** (searched `InlineCompletion|inline_completion|ghost_text` in `app/src/code` — only LSP install callbacks and vim tests match). Terminal-side AI input prediction exists (`app/src/ai/predict/next_command_model.rs`, `generate_ai_input_suggestions.rs`) but that is for the terminal/agent input box, not editor edits.
- Editor file reads/writes outside an agent session go through `GlobalBufferModel::save` (`global_buffer_model.rs:806`) and `FileModel` (`crates/warp_files`); these are user actions, not agent actions. If "editor-driven" hooks are wanted, `FileModel` is the common write funnel for GUI editor, TUI diff storage, and agent edits **[inferred from call sites: `global_buffer_model.rs:863-897`, `tui_diff_storage.rs:255`]**.
- Conclusion: no Tab-hook equivalent surface today; "not found".

### m. Workspace open

- Warp has no "open project folder" concept for the agent. The nearest signals:
  - Session cwd changes (`ActiveSession::current_working_directory`), surfaced to the agent per request as `AIAgentContext::Directory{pwd, home_dir}` and via `SessionContext`.
  - GUI `TerminalView::current_repo_path` / git status subscription (`view.rs:7806`, `update_git_status_subscription`), and `pwd_if_local` (`view.rs:24548`).
  - Codebase indexing registration: `CodebaseIndexManager::index_directory` (`view.rs:7751`) and `get_codebase_index_statuses` (`input_context.rs:101`) — the closest thing to "workspace registered".
  - Oz runs: `AgentDriverOptions.working_dir` + `WorkspaceConfiguration` / environment checkout (`driver.rs:646`, `:672`, `agent_sdk/driver/environment*.rs`) — an explicit "workspace prepared" moment exists only in the cloud runtime (`prepare_harness` / `run_internal`, `driver.rs:1915`, `:2847`).
  - Note: `app/src/workspace/` is the *window/tab* workspace, and `app/src/workspaces/` is *teams*; neither is a project root.
- A `workspaceOpen` hook would most defensibly fire on (a) new conversation start with the cwd at that time, and (b) Oz run environment-ready. **[inferred recommendation, not code]**

---

## Section 4 - Permission system

### 4.1 Settings model **[verified]**

- `AIExecutionProfile` (crate `cloud_object_models`, `crates/cloud_object_models/src/ai_execution_profile.rs`) holds per-profile: `apply_code_diffs`, `read_files`, `execute_commands`, `mcp_permissions` (each `ActionPermission::{AgentDecides, AlwaysAllow, AlwaysAsk, Unknown}`), `write_to_pty: WriteToPtyPermission::{AlwaysAllow, AskOnFirstWrite, ...}`, `command_allowlist`, `command_denylist`, `directory_allowlist`, `mcp_allowlist`, `mcp_denylist`, `computer_use`, `ask_user_question`, `run_agents`, `web_search_enabled`, models, `context_window_limit`.
- Active profile per terminal surface: `AIExecutionProfilesModel::active_profile(terminal_view_id)` (`app/src/ai/execution_profiles/profiles.rs`); `BlocklistAIPermissions::permissions_profile_for_id` layers **team/enterprise overrides** from `UserWorkspaces::ai_autonomy_settings(scope)` (`permissions.rs:176-243`); sandboxed mode swaps in an org denylist only (`:233-243`). Headless default is `create_default_cli_profile` (all AlwaysAllow; `ai_execution_profile.rs:463-503`).
- Per-conversation override: `AIConversation.autoexecute_override == RunToCompletion` ("auto-approve") short-circuits most checks (`permissions.rs:706-711`, `:785-790`, `:869-874`, `:941-970`).

### 4.2 Decision functions (the composition points)

| Action | Decision fn | Precedence (top wins) |
|---|---|---|
| Shell command | `can_autoexecute_command` (`permissions.rs:912-1031`) | 1) **denylist** (org denylist always; user denylist unless auto-approve bypass setting `auto_approve_bypasses_command_denylist`) → `Denied(ExplicitlyDenylisted)`; 2) auto-approve → `Allowed(RunToCompletion)`; 3) `AgentDecides`: `is_risky==Some(false)` (flag `AgentDecidesCommandExecution`) → allow; contains redirection → deny; allowlist match (all sub-commands) → allow; `is_read_only` → allow; else deny; 4) `AlwaysAllow` → allow; 5) `AlwaysAsk`: allowlist → allow else deny |
| PTY write | `can_write_to_pty` (`:493`), used at `shell_command.rs:247-283` | AlwaysAllow / AskOnFirstWrite (allowed once the agent already wrote) |
| Read files | `can_read_files_with_conversation` (`:698-768`) | auto-approve → allow; conversation temp grants (`add_temporary_file_read_permissions`, `:1208`) → allow; `AgentDecides` → **always allow**; `AlwaysAllow`; `AlwaysAsk` → directory allowlist |
| Write files | `can_write_files` (`:771-793`) | protected paths (MCP config files) → deny unless sandboxed; auto-approve → allow; `AgentDecides` → **deny (ask)**; `AlwaysAllow`; `AlwaysAsk` → deny |
| MCP | `can_call_mcp_tool` / `can_read_mcp_resource` → `can_use_mcp_server` (`:796-888`) | auto-approve → allow; `AgentDecides`/`AlwaysAsk` → allowlisted && !denylisted; `AlwaysAllow` → !denylisted |
| Ask user question | `can_ask_user_question` (`:1220-1234`) | Never / AskExceptInAutoApprove / AlwaysAsk |
| Run agents | `RunAgentsExecutor::should_autoexecute` (`run_agents.rs:458-500`) | autonomous or child conversation → allow; approved orchestration config; `run_agents.is_always_allow()` |
| Everything else | per-executor `should_autoexecute` (`execute.rs:975-1073`); several return `true` unconditionally (`InitProject`, `OpenCodeReview`, `InsertCodeReviewComments`) | — |

The result of each is a bool consumed at `execute.rs:618` (`can_auto_execute`). The composed rule is at `execute.rs:624-626`:
`needs_confirmation = !(is_user_initiated || can_auto_execute || (is_agent_autonomous && action.is_request_command_output()))`.

### 4.3 Confirmation presentation and resume **[verified]**

- Not executed → `TryExecuteResult::NotExecuted{NeedsConfirmation}` → action re-inserted in the queue; `BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(action_id)` and `ConversationStatus::Blocked{blocked_action}` (`action_model.rs:848-871`).
- GUI: `TerminalView::handle_ai_action_model_event` (`view.rs:7671-7681`) refocuses; `AIBlock` renders the card; desktop notification "needs your permission" (`view.rs:8064-8100`, `:16753-16767`). Accept/edit command → `handle_requested_command_accepted` (`action_model.rs:1312-1341`, rewrites `command` then `execute_action` with `is_user_initiated=true`); generic accept → `execute_action` / `execute_next_action_for_user` (`:791-826`); reject → `cancel_action_with_id` (`:1100-1129`) → `cancel_pending_action` → cancelled result through `handle_action_result`.
- TUI: `TuiPermissionPrompt` in `tui_shell_command_view.rs:247-272` (same two action-model calls), `tui_file_edits_view.rs:735`, `tui_generic_tool_call_view.rs:173`.
- Accepting also lets the user add to allowlist / toggle auto-approve (`add_command_to_autoexecution_allowlist`, `permissions.rs:1037`; `enable_autoexecute_override`, `block.rs:3303-3305`).
- Resume: `execute_action` → `start_pending_action_by_id(.., is_user_initiated=true)` bypasses `should_autoexecute` entirely (`execute.rs:624`).

### 4.4 Headless contexts

- **Oz cloud agent / `oz agent run`** (`ExecutionMode::Sdk`): default profile allows everything (4.1), so normally nothing blocks. Denylisted commands produce a `Denylisted` tool result instead of a prompt (`execute.rs:632-656`). If a non-command action does need confirmation (custom restrictive profile), the conversation goes `Blocked` and the driver exits with `ConversationBlocked` (`driver.rs:832-834`). There is **no interactive "ask" channel** in this mode (shared-session viewers can send prompts but cannot approve actions: `is_shared_session_viewer` guards at `execute.rs:607-612`).
- **TUI**: fully interactive; same semantics as GUI.
- **Local orchestration children**: hidden surfaces; `run_agents` auto-allows for child conversations because "a confirmation card would be invisible and hang the run" (`run_agents.rs:469-481`). Other actions in children still go through the inherited profile; a blocked child reports `Blocked` status to the parent via `start_agent_error_message_for_status` (`start_agent.rs:306-339`).

### 4.5 Where a hook decision (allow / deny / ask) must compose

- The clean composition point is `BlocklistAIActionExecutor::try_to_execute_action` between the permission check (`execute.rs:618`) and the `needs_confirmation` computation (`:624-626`). A hook returning *deny* should produce a synthetic failed/cancelled result the way the denylist path does (`:634-656`, using `action.action.cancelled_result()` or a tool-specific error variant) so the model gets feedback; *ask* should force `NeedsConfirmation`; *allow* should set `can_auto_execute = true`. Existing precedence to preserve: org denylist and protected write paths are evaluated *inside* the permission functions and would need to run before (or be unaffected by) a hook "allow".
- Because `try_to_execute_action` is sync, an async hook must run earlier (preprocess phase, `action_model.rs:982-998`) or the executor must gain a "pending hook" state similar to `NotExecutedReason::NotReady`.

---

## Section 5 - Messaging to the user and to the model

### 5.1 Non-blocking user messages from inside a turn **[verified]**

- Workspace toasts (GUI): `ToastStack::add_ephemeral_toast / add_persistent_toast` (`app/src/workspace/toast_stack.rs:23-41`), `DismissibleToast`.
- Agent input bar ephemeral messages (GUI agent view): `EphemeralMessage` / `DismissalStrategy` (`app/src/ai/blocklist/agent_view/ephemeral_message_model.rs:12-30`), `agent_message_bar.rs`.
- Desktop notification / banner: `TerminalView::send_agent_desktop_notification_or_show_banner` (`view.rs:16769`), driven by `get_ai_notification_summary` (`view.rs:8064`).
- TUI: `show_transient_hint` / `show_success_hint` (`crates/warp_tui/src/terminal_session_view.rs:3533-3539`).
- Inside the transcript: `AIAgentOutputMessageType::DebugOutput{text}` exists but is gated to debug channels (`convert_from.rs:483-494`); there is no general "system note" output message type for user-visible hook messages — one would need adding to `AIAgentOutputMessageType` (`agent/mod.rs:1901`) plus GUI/TUI renderers.
- Oz: stdout JSON/text output via `AgentDriver::write_output` (`driver.rs:4053`); `report_driver_error` (`:4763`).

### 5.2 Injecting text into the model context mid-turn **[verified]**

- Per-request context: `AIAgentContext` list attached to every input (`input_context_for_request`, `input_context.rs:47-98`); adding a variant (or reusing `File`/`SelectedText`) is the client-side way to append "additional context". The server decides how context is rendered into the prompt.
- Tool-result annotations: `AIAgentInput::ActionResult{result, context}` — the `context` slice is a natural carrier for hook-provided `agent_message` text alongside a tool result (`controller.rs:266-274`, `:862-869`).
- Agent-to-agent messages: `AIAgentInput::MessagesReceivedFromAgents` / `EventsFromAgents` (`agent/mod.rs:3096-3102`) are already "out-of-band text injected into the next turn" and are rendered as `MessagesReceivedFromAgents` output messages.
- Deny feedback: `RequestCommandOutputResult::Denylisted{command}` is rendered to the model as "Command ... was on denylist" (`agent/mod.rs:1313-1318`); the same pattern (an error-variant result with text) works for hook denials.
- No `<system-reminder>` mechanism exists client-side (searched `system-reminder|SystemReminder`: not found); system prompt is server-owned.
- Limitation: nothing can be inserted *during* a streaming response; injection happens at request boundaries.

### 5.3 Telemetry **[verified]**

- Macro `send_telemetry_from_ctx!` (`crates/warp_core/src/telemetry.rs:147`) with the app's `TelemetryEvent` enum (`app/src/server/telemetry/events.rs:1262`). Agent-specific events include `AutoexecutedAgentModeRequestedCommand{reason}` (`shell_command.rs:236-239`), `AIInputNotSent` (`controller.rs:3050`), `RequestFileEditsTelemetryEvent::{EditReceived, EditResolved}` (`request_file_edits.rs:196-209`, `:238-246`), orchestration events (`app/src/ai/blocklist/telemetry.rs`, `send_run_agents_completed_telemetry`), `CLISubagentActionExecuted`.
- Join keys: `AIIdentifiers` (`agent/telemetry.rs:46`).
- `AIAgentOutput.telemetry_events` is a per-output buffer flushed after completion (`agent/mod.rs:469-472`).
- Structured logging with `operation_id` for MCP (`call_mcp_tool.rs:147-165`); `tracing` span with `conversation_id/request_id/run_id` on the SSE stream (`warp_multi_agent_client/src/lib.rs:91-115`).

---

## Section 6 - Summary table

| Cursor hook | Warp location(s) | Data available | Blockable? | Modifiable? | Shared across front-ends? | Notes |
|---|---|---|---|---|---|---|
| sessionStart | `history_model.rs:1421` `start_new_conversation`; `StartedNewConversation` event; Oz `driver.rs:3441` `execute_run` | `AIConversationId`, surface id, cwd (via `ActiveSession`), autoexecute mode; server token only after first `StreamInit` | No (sync) | n/a | Yes | Resume = `restore_conversations` / `resume_conversation` |
| sessionEnd | `UpdatedConversationStatus` → terminal status; `clear_conversations_for_terminal_surface` (`history_model.rs:2268`); Oz `driver.rs:4450` `cleanup` | status, final exchange, usage | Oz: yes; GUI/TUI: no | n/a | Partly | No single "closed" event today |
| beforeSubmitPrompt | `controller.rs:759` `send_query` (sync, has ctx) or `api/impl.rs:16` `generate_multi_agent_output` (async, has `RequestParams`) | query, attachments, context list, conversation/task ids, model ids | impl.rs: yes; send_query: no | Yes (owned `String`/`Vec<AIAgentInput>`; redaction precedent `impl.rs:55`) | Yes | Slash commands bypass `send_query` body → use impl.rs for full coverage |
| preToolUse (generic) | `execute.rs:599` `try_to_execute_action` (`:618` perms, `:660` dispatch); async option `action_model.rs:968` preprocess phase | `AIAgentAction{id, task_id, action}`, conversation id, permission verdict | preprocess: yes; try_to_execute: no (sync) | Yes (owned action; `handle_requested_command_accepted` precedent) | Yes | Compose allow/deny/ask at `:624-626`; deny via synthetic result like `:634-656` |
| postToolUse (generic, incl. failure) | `action_model.rs:1343` `handle_action_result` (observe); `execute.rs:821-835` future/`on_complete` or `controller.rs:2105` drained results (modify) | `AIAgentActionResult`, `cancellation_reason`, `is_failed()` | Wrap `execute_future` at `execute.rs:807`: yes | Yes before `FinishedAction` emission | Yes | Sync executors need separate path |
| beforeShellExecution | `shell_command.rs:307` `execute` → event `:372`; perms `shell_command.rs:203` / `permissions.rs:912`; front-end run GUI `view.rs:7863`, TUI `terminal_session_view.rs:5143` | command, `is_read_only`, `is_risky`, `uses_pager`, `wait_until_completion`; cwd from session; **proto `risk_category` not plumbed** (`convert.rs:21-40`) | Only if made async (preprocess is a no-op today) | Command string yes | Executor yes; PTY bridge no | Interact mode = `wait_until_completion=false` → snapshot + server CLI subagent |
| afterShellExecution | `shell_command.rs:975` `action_result_for_requested_command`; generic post-tool | command, output, exit_code, block_id, start/finish ts; snapshot grid for LRC | Yes (future) | Output yes | Yes | PTY writes (`WriteToLongRunningShellCommand`) are separate actions |
| beforeMCPExecution | `call_mcp_tool.rs:79` `execute` (args owned, `:110-131`); perms `permissions.rs:796-888`; `read_mcp_resource.rs` | server installation uuid → template uuid, tool name, JSON args, `operation_id` | Yes (async) | Args yes | Yes (not wasm) | Tool catalog to model built at `api.rs:267-335` |
| afterMCPExecution | `call_mcp_tool.rs:161` `on_complete` → `handle_call_tool_result` | raw rmcp result, `CallMCPToolResult` | Yes | Yes | Yes | — |
| afterFileRead | `read_files.rs:228`, `:264` `on_complete`; `execute.rs:1145` `read_local_file_context`; remote via `read_files.rs:142` | `FileContext{file_name, content, line_range, last_modified}` | Yes | Yes (redact/replace before result) | Yes | Also `search_codebase`, `get_files`, `read_skill`, grep/glob, `@file` attachments (server-side read, inferred) |
| afterFileEdit (and pre-write) | preprocess `request_file_edits.rs:214`/`272` (diffs computed, no write); approval → `execute` `:143` → `diff_storage.rs:83` `accept_and_save` → `start_saving` (write) | path, old content (`DiffBase.content`), `DiffType`, final content & changed lines (`FileSnapshot`), `was_edited` | Yes (futures at both stages) | Pre-review `FileDiff`s yes; pre-write per surface | Executor + `diff_storage.rs` yes; storage impl per front-end | Write happens only at execute (after user approval or auto-approve) |
| subagentStart / stop | `run_agents.rs:402` `execute` / `:238` dispatch; `start_agent.rs:251`; results via `controller.rs:2447` `inject_pending_events_for_request` and `:2180` piggyback; `wait_for_events.rs` | child name/prompt/model/harness/mode; `RunAgentsResult::Launched{agent_id}`; later `MessagesReceivedFromAgents`/`EventsFromAgents` | Start: yes (async); stop: injection point is sync but owns the `Vec<AIAgentInput>` | Request yes; injected events yes | Executor yes; child creation per front-end; remote child runs elsewhere | No synchronous child-finished callback on parent |
| preCompact | `/compact` only: `slash_command.rs:58` → `AIAgentInput::SummarizeConversation`; automatic compaction is server-side (`conversation.rs:3501` `MoveMessagesToNewTask`) | full `AIConversation`, optional prompt | `/compact`: via prompt hook; auto: **no** | `/compact` prompt yes | Yes | Client cannot preempt server auto-summarization |
| stop | `controller.rs:3788-3933` `AfterStreamFinished` (actions empty → turn ends); `controller.rs:3988` reasons; `UpdatedConversationStatus(Success)` | `StreamFinished.reason`, final `AIAgentOutput.messages`, `ConversationStatus`, `CancellationReason` | No (sync) without restructuring | Follow-up prompt can be injected via `send_user_query_in_conversation_internal` / `QueuedQueryModel` | Yes | Oz exits on `Success` after idle window (`driver.rs:3514`) |
| assistant text / thinking | `conversation.rs:3435-3484` `AppendToMessageContent`; `convert_from.rs:223-238`; `UpdatedStreamingExchange` | `AIAgentText`, `Reasoning{finished_duration}` | No | No (history replayed to server) | Yes | No per-block boundary event; reasoning end = `finished_duration.is_some()` |
| Tab / editor hooks | **not found** (no AI inline completion in `app/src/code`); editor saves `global_buffer_model.rs:806` | — | — | — | GUI only | Terminal input prediction (`app/src/ai/predict`) is not an editor surface |
| workspaceOpen | **no equivalent**; nearest: cwd on new conversation, `CodebaseIndexManager::index_directory` (`view.rs:7751`), Oz `AgentDriverOptions.working_dir` + environment prep (`driver.rs:646`, `:1915`) | cwd, repo path (GUI), indexed codebases | Oz env prep: yes | n/a | No | Would need to be defined |

---

## Caveats / unknowns

1. Everything about *how the server* builds prompts, decides to summarize, runs subagents, and maps `risk_category` → `is_risky` lives in warp-server (not in this repo) — inferred only from request/response fields.
2. `@file` attachment content delivery (client vs server read) was not traced to the end; marked inferred.
3. The exact "current user" accessor (`AuthManager` API) was not pinned down; the `User` struct and its fields are verified.
4. Remote-child and `ExternalHarness` (Claude Code via ACP) conversations never execute tools on the local client; hooks for those must live in the child's own Oz runtime or the external harness.
5. I did not audit every executor (`ask_user_question`, `use_computer`, `start/stop_recording`, `upload_artifact`, `fetch_conversation`, `send_message`); they all pass through the same `try_to_execute_action` / `handle_action_result` funnel, so the generic hook points cover them.
6. Line numbers are for HEAD `393e97a3`; files like `controller.rs` (4k lines) and `conversation.rs` (5k lines) churn frequently.
