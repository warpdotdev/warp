# TUI tool execution for Agent Mode
## Context
The `warp-tui` prototype can submit prompts and fold streamed text into shared conversation history, but it disables client tools by setting `supported_tools_override: Some(vec![])` in [`app/src/tui.rs:758 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/tui.rs#L758). This spec enables v0 TUI tool execution with automatic tool acceptance and minimal status cards.
The GUI Agent Mode tool path already has the right high-level shape:
* Streamed server `ClientActions` become `AIAgentAction`s and are folded into history by `AgentConversationEngine` ([`app/src/ai/blocklist/agent_conversation_engine.rs:38 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/agent_conversation_engine.rs#L38)).
* GUI queues completed-stream actions through the engine delegate ([`app/src/ai/blocklist/controller.rs:2744 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/controller.rs#L2744)).
* `BlocklistAIActionModel` owns action queueing, preprocessing, running/finished state, and ordered finished results ([`app/src/ai/blocklist/action_model.rs:247 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/action_model.rs#L247), [`app/src/ai/blocklist/action_model.rs:914 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/action_model.rs#L914)).
* `BlocklistAIController` drains finished results into `RequestInput::for_actions_results` and sends the tool-result follow-up ([`app/src/ai/blocklist/controller.rs:213 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/controller.rs#L213), [`app/src/ai/blocklist/controller.rs:1522 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/controller.rs#L1522)).
The reusable session layer already exists. `Session` owns shell metadata, launch data, session type, path conversion, and `CommandExecutor` access ([`app/src/terminal/model/session.rs:901 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/terminal/model/session.rs#L901), [`app/src/terminal/model/session.rs:951 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/terminal/model/session.rs#L951), [`app/src/terminal/model/session.rs:1472 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/terminal/model/session.rs#L1472)). `ActiveSession` is a GUI terminal-pane model that points at the currently active bootstrapped `Session` and tracks cwd from terminal metadata ([`app/src/terminal/model/session/active_session.rs:16 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/terminal/model/session/active_session.rs#L16), [`app/src/terminal/model/session/active_session.rs:69 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/terminal/model/session/active_session.rs#L69)).
The main coupling to remove is not `Session`; it is the direct dependency from reusable tools onto GUI terminal/editor concepts:
* Many read-only tools only need cwd/shell/session data, but currently store `ModelHandle<ActiveSession>` ([`read_files.rs:15 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/action_model/execute/read_files.rs#L15), [`grep.rs:178 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/action_model/execute/grep.rs#L178), [`file_glob.rs:31 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/action_model/execute/file_glob.rs#L31)).
* `ShellCommandExecutor` is terminal-block-backed: it emits terminal events and waits for `TerminalModel` block output ([`shell_command.rs:38 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/action_model/execute/shell_command.rs#L38), [`shell_command.rs:235 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/action_model/execute/shell_command.rs#L235), [`shell_command.rs:487 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/action_model/execute/shell_command.rs#L487)).
* `RequestFileEditsExecutor` preprocesses candidate diffs but execution waits on a registered GUI `CodeDiffView` to save/reject ([`request_file_edits.rs:45 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/action_model/execute/request_file_edits.rs#L45), [`request_file_edits.rs:120 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/action_model/execute/request_file_edits.rs#L120), [`code_diff_view.rs:1022 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/inline_action/code_diff_view.rs#L1022)).
## Proposed changes
### TUI local session
The TUI owns a single local `Session` built by `tui_local_session()` from the process cwd/shell environment and backed by `LocalCommandExecutor`. It is held directly on `TuiToolActionModel` as `session: Arc<Session>`; there is no separate `TuiActiveSession` model. This can later grow to multiple concurrent sessions/processes.
### Execution context for shared tools
Surface-neutral tools never store `ModelHandle<ActiveSession>`. Each surface instead produces an `AgentToolExecutionContext` on demand through `SurfaceSpecificToolExecutor::tool_execution_context`.
```rust
pub(crate) struct AgentToolExecutionContext {
    current_working_directory: Option<String>,
    shell_launch_data: Option<ShellLaunchData>,
    session: Option<Arc<Session>>,
    terminal_view_id: Option<EntityId>,
}
```
GUI builds it from `ActiveSession`; TUI builds it from its local `Session` (with no `terminal_view_id`). Shared file-edit diff application takes a `SessionContext` derived the same way.
### Shared action state and scheduling
`AgentToolActionModel` is plain embedded state (not an `Entity`): preprocessing queues, pending actions, running actions, finished results, action ordering, and past results. Both surfaces hold it as a `tools` field rather than wrapping a `ModelHandle`.
```rust
pub(crate) struct AgentToolActionModel {
    // shared queue/result state
}
```
The scheduling loop — preprocessing fan-out, the pending queue, serial/parallel phase admission, ordered result draining, and follow-up readiness — lives in a shared `AgentToolScheduler` parameterized over an `AgentToolScheduleHost` trait. The host is implemented by the model that owns the state (`BlocklistAIActionModel` for GUI, `TuiToolActionModel` for TUI), so scheduler callbacks (`ctx.spawn`, event emission, status updates) land on the owning model.
```rust
pub(crate) trait AgentToolScheduleHost: Sized {
    type Context<'a>;
    fn tools(&mut self) -> &mut AgentToolActionModel;
    // execution: preprocess / try_execute / can_autoexecute / action_phase
    // side effects: on_action_enqueued / on_action_started / on_action_not_executed
    //               / on_action_finished / on_phase_drained / should_enqueue
}

pub(crate) struct AgentToolScheduler; // generic static methods over the host
```
`AgentToolScheduler` owns `queue_actions`, `try_to_execute_available_actions`, `start_pending_action_by_id`, and `finish_action`. Surface-specific execution (shell, file edits) and side effects (history status, event emission, TUI cards) are delegated to the host. GUI-specific inline views, shared-session viewer UI, terminal block events, `TerminalModel`, `AIBlock`, `RequestedCommandView`, and `CodeDiffView` stay in the GUI adapter.
Generic state queries also live on `AgentToolActionModel`, not on the surface wrappers. `get_pending_actions`, `get_pending_actions_for_conversation`, `get_pending_action_by_id`, `has_unfinished_actions_for_conversation`, `get_finished_action_results`, `get_action_result`, `restore_action_results_from_exchanges`, `blocked_action_for_conversation`, and `get_action_status` are implemented on the shared model. `get_action_status` takes `is_view_only: bool` as a parameter so the GUI can pass its view-only flag and the TUI can pass `false`; the `Blocked` vs `Queued` distinction is the only status rule that depends on surface state. The surface wrappers keep thin delegating methods so existing GUI view code (`block.rs`, `output.rs`, etc.) continues to call `action_model.get_action_status(id)` unchanged. Running-action recording is consolidated into `AgentToolActionModel::record_running_action` (with the phase-consistency `debug_assert`), so the GUI no longer carries a duplicate `add_running_action`.
Queries that genuinely depend on surface state stay on the wrappers: `get_pending_action(app)`, `get_pending_or_running_action_id(app)`, `has_unfinished_actions(app)`, and `get_async_running_action(app)` all resolve the active conversation through GUI-specific `terminal_view_id` + `BlocklistAIHistoryModel`, and `get_async_running_action` additionally consults the GUI executor's in-flight action map. `mark_action_as_remotely_executing` stays GUI-specific because it is gated on `is_view_only` and emits a GUI event.
The two surfaces keep different async-completion mechanisms behind `try_execute`: GUI execution completes via its executor's `FinishedAction` event subscription, TUI execution via its own `ctx.spawn` callback; both call back into `AgentToolScheduler::finish_action`. For v0, TUI auto-accepts (no user-confirmation blocked state) and inherits the shared phased loop, so read-only tools fan out in one parallel phase while shell and file-edit tools run as serial barriers.
### Shared-first tool executor
Tool execution is centralized in a shared `AgentToolExecutor` used by both GUI and TUI. Shared tools are handled directly; inherently surface-specific tools are delegated to a required `SurfaceSpecificToolExecutor` implemented by each surface. `AgentToolExecutor` is a unit type with static methods generic over the surface — it holds no state.
```rust
pub(crate) struct AgentToolExecutor; // static methods generic over the surface

pub(crate) trait SurfaceSpecificToolExecutor {
    type Context<'a>;

    fn tool_execution_context(&self, ctx: &Self::Context<'_>) -> AgentToolExecutionContext;
    fn tool_execution_context_from_app(&self, ctx: &AppContext) -> AgentToolExecutionContext;
    fn app_context<'a, 'b>(ctx: &'a Self::Context<'b>) -> &'a AppContext;

    // Required, surface-specific tool families.
    fn preprocess_shell(&mut self, input: PreprocessActionInput<'_>, ctx: &mut Self::Context<'_>) -> BoxFuture<'static, ()>;
    fn execute_shell(&mut self, input: ExecuteActionInput<'_>, ctx: &mut Self::Context<'_>) -> AnyActionExecution;
    fn should_autoexecute_shell(&mut self, input: ExecuteActionInput<'_>, ctx: &mut Self::Context<'_>) -> bool;
    fn preprocess_file_edits(&mut self, input: PreprocessActionInput<'_>, ctx: &mut Self::Context<'_>) -> BoxFuture<'static, ()>;
    fn execute_file_edits(&mut self, input: ExecuteActionInput<'_>, ctx: &mut Self::Context<'_>) -> AnyActionExecution;
    fn should_autoexecute_file_edits(&mut self, input: ExecuteActionInput<'_>, ctx: &mut Self::Context<'_>) -> bool;

    // Defaulted fallbacks for GUI-only tools (no-op / cancelled / serial).
    fn preprocess_other(&mut self, input: PreprocessActionInput<'_>, ctx: &mut Self::Context<'_>) -> BoxFuture<'static, ()> { /* no-op */ }
    fn execute_other(&mut self, input: ExecuteActionInput<'_>, ctx: &mut Self::Context<'_>) -> AnyActionExecution { /* cancelled */ }
    fn should_autoexecute_other(&mut self, input: ExecuteActionInput<'_>, ctx: &mut Self::Context<'_>) -> bool { false }
    fn action_phase_other(&self, action: &AIAgentAction, ctx: &AppContext) -> RunningActionPhase { /* Serial */ }
}
```
`AgentToolExecutor`'s static `preprocess_action` / `execute_action` / `should_autoexecute` / `action_phase` own the only top-level `AIAgentActionType` dispatch. Read files, grep, and file glob run shared default logic (using the surface's `AgentToolExecutionContext`); shell commands and file edits delegate to the required `SurfaceSpecificToolExecutor` methods; every remaining tool falls through to the defaulted `*_other` hooks.
The GUI `BlocklistAIActionExecutor` implements the trait by delegating to existing GUI machinery:
* shell commands use `ShellCommandExecutor`, `TerminalModel`, and terminal blocks;
* file edits use `RequestFileEditsExecutor` and `CodeDiffView`;
* GUI-only tools (MCP, computer use, start/run agents, documents, etc.) are handled in `preprocess_other`/`execute_other`.
The TUI `TuiToolExecutor` supplies only TUI-specific behavior:
* shell commands run on the TUI local `Session`;
* file edits use shared diff application plus v0 auto-save;
* unsupported tools fall through to the cancelled `*_other` defaults.
Both surfaces route through the same `AgentToolExecutor`; there is no separate TUI-only dispatch tree.
### Tool-result follow-up
The tool-result turn closes the loop:
```text
stream response -> execute tools -> send tool results -> continue streaming
```
This stayed surface-specific rather than moving into `AgentConversationEngine`. The GUI keeps `BlocklistAIController`'s follow-up path. The TUI drives it from `CoreTuiModel::send_action_results`, triggered by `TuiToolActionEvent::ActionsFinished` — which the scheduler raises from its `on_phase_drained` hook once a conversation has no pending or running actions. `send_action_results` drains finished results in original tool-call order, builds `RequestInput` from the local session, and sends the follow-up through `AgentConversationEngine`.
### TUI tool pipeline
`CoreTuiModel` owns or references:
* the single `AgentSessionOwnerId`;
* `TuiToolActionModel`, which owns the local `Session` and the shared `AgentToolActionModel` state;
* the follow-up hookup: it subscribes to `TuiToolActionModel` and calls `send_action_results` on `ActionsFinished`.
`supported_tools_override: Some(vec![])` has been removed from normal TUI prompt sends.
### Command running
Command execution is surface-specific, required by `SurfaceSpecificToolExecutor`. The GUI keeps terminal-block behavior through `ShellCommandExecutor`.
For v0 the TUI runs each `RequestCommandOutput` synchronously on its local `Session`, captures combined stdout/stderr, and returns `Completed` with a freshly generated `BlockId` at the result boundary (existing result types still key commands by `BlockId` — [`crates/ai/src/agent/action_result/mod.rs:183 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/crates/ai/src/agent/action_result/mod.rs#L183)). It has no persistent command registry yet, so `ReadShellCommandOutput`, `WriteToLongRunningShellCommand`, and `TransferShellCommandControlToUser` return `BlockNotFound`. A `TuiCommandModel` with snapshots, stdin, and cancellation is a follow-up.
### File edits
Keep `diff_application::apply_edits` as the shared diff parser/matcher; it already accepts `SessionContext` and a file-read closure ([`diff_application.rs:161 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/action_model/execute/request_file_edits/diff_application.rs#L161)).
Change `ApplyDiffModel::apply_diffs` to take the session snapshot/session context as input instead of storing `ModelHandle<ActiveSession>` ([`apply_diff_model.rs:25 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/action_model/execute/request_file_edits/apply_diff_model.rs#L25)).
File edit execution is surface-specific and should be required by `SurfaceSpecificToolExecutor`, but diff application should be shared. GUI can keep `CodeDiffView` for approval and saving. TUI uses the same diff application logic, then auto-saves for v0.
For v0, TUI auto-accepts file edits after preprocessing succeeds. It writes local files, computes unified diff/line stats/deleted files, and returns `RequestFileEditsResult::Success`. Diff application failures return `RequestFileEditsResult::DiffApplicationFailed` so the agent can recover ([`request_file_edits.rs:135 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/app/src/ai/blocklist/action_model/execute/request_file_edits.rs#L135), [`convert.rs:197 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/crates/ai/src/agent/action_result/convert.rs#L197)).
### Minimal TUI tool UI
Add simple TUI tool cards next to the current transcript/input views. No rich approval/edit UI in this PR.
Example command card:
```text
┌ Tool: run_shell_command ──────────────────────┐
│ cargo check -p warp --features tui            │
│ exit 0 · 213 lines captured                    │
└───────────────────────────────────────────────┘
```
Example file-edit card:
```text
┌ Tool: apply_file_diffs ───────────────────────┐
│ app/src/tui.rs                                │
│ +24 -8 · applied automatically                │
└───────────────────────────────────────────────┘
```
## Testing and validation
Automated tests:
* TUI model test: tool call is queued, auto-executed, result is sent as a follow-up, final conversation stays ordered.
* Command backend tests: completed command, denylisted command, long-running snapshot, read-after-snapshot, write-to-running-command, cancellation, pager decoration.
* File-edit tests: candidate diff generation, auto-accept/save success, malformed/unmatched diff failure, protected-path denial, returned updated file context.
* TUI presenter tests: minimal command/file-edit/tool summary cards.
Targeted commands:
```sh
cargo test -p warp --features tui <tui_tool_test_name>
cargo test -p warp --features tui <command_backend_test_name>
cargo test -p warp --features tui <file_edit_test_name>
cargo check -p warp --features tui
```
Manual validation is required. Build and run `warp-tui`, submit prompts inside the TUI that trigger command and file-edit tools, observe minimal tool cards and follow-up behavior, fix issues, and repeat until the interactive behavior works as intended. This is required because the feature is an interactive terminal UI, not only a model path.
## Parallelization
Do not use parallel implementation agents for the first pass. The core refactor crosses shared action state, session plumbing, shell execution, file edits, and the TUI transcript, and those pieces need to be changed in a tight sequence to keep the code compiling. Parallelization becomes useful after the shared backing model and session snapshot boundaries land; at that point UI polish and additional tool executor conversions can be split off safely.
## Follow-ups
* Rich TUI approval/edit UI for commands and file edits.
* Better TUI command interleaving for user-authored shell commands.
* Generalize `ActiveSession` if the GUI `ActiveSession` and the TUI local `Session` converge.
* Remove compatibility wrappers once `BlocklistAIActionModel` is thin enough to rename or delete.
