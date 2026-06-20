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
### TUI active session
Add a TUI-native active session model rather than reusing or faking the GUI `ActiveSession`.
```rust
pub(crate) struct TuiActiveSession {
    active: Arc<Session>,
    current_working_directory: Option<String>,
}
```
For v0 this owns a single local `Session`, created from process cwd/shell environment and backed by the existing local command executor. This gives TUI a real session concept that can later grow to multiple concurrent sessions/processes.
### Session snapshot for tools
Make reusable executors independent of `ModelHandle<ActiveSession>` by passing an explicit snapshot through action preprocessing/execution.
```rust
pub(crate) struct AgentToolSessionSnapshot {
    owner_id: AgentSessionOwnerId,
    session_context: SessionContext,
    current_working_directory: Option<String>,
    shell_launch_data: Option<ShellLaunchData>,
    shell_type: Option<ShellType>,
    session: Arc<Session>,
}
```
GUI builds this from `ActiveSession` at the action boundary. TUI builds it from `TuiActiveSession`. Sub-executors that only need session data take the snapshot and stop storing `ModelHandle<ActiveSession>`.
### Shared action backing model
Do not make TUI depend directly on a model named `BlocklistAIActionModel`. Extract a shared backing model for queue/result state and let GUI/TUI wrap it.
```rust
pub(crate) struct AgentToolActionModel {
    // shared queue/result state
}

pub(crate) struct BlocklistAIActionModel {
    inner: ModelHandle<AgentToolActionModel>,
    // GUI/blocklist conveniences and compatibility API
}

pub(crate) struct TuiToolActionModel {
    inner: ModelHandle<AgentToolActionModel>,
    // TUI-specific convenience API if needed
}
```
The shared model owns action queueing, preprocessing, running/finished state, action ordering, cancellation, and finished result draining. GUI-specific inline views, shared-session viewer UI, terminal block events, `TerminalModel`, `AIBlock`, `RequestedCommandView`, and `CodeDiffView` stay out of the shared model.
For v0, TUI tools auto-accept. The shared model should avoid a user-confirmation blocked state for TUI and immediately execute tools when permission checks allow or when v0 auto-accept rules apply.
### Shared-first tool executor
Tool execution should be centralized in a shared executor used by both GUI and TUI. Shared tools are handled directly by the shared executor. Inherently surface-specific tools are delegated to a required surface executor implemented by both surfaces.
```rust
pub(crate) struct AgentToolExecutor<S> {
    surface: S,
    shared_context: AgentToolExecutionContext,
}

pub(crate) trait SurfaceSpecificToolExecutor {
    fn preprocess_shell(
        &mut self,
        input: PreprocessActionInput<'_>,
        ctx: &mut AppContext,
    ) -> BoxFuture<'static, ()>;

    fn execute_shell(
        &mut self,
        input: ExecuteActionInput<'_>,
        ctx: &mut AppContext,
    ) -> AnyActionExecution;

    fn should_autoexecute_shell(
        &mut self,
        input: ExecuteActionInput<'_>,
        ctx: &mut AppContext,
    ) -> bool;

    fn preprocess_file_edits(
        &mut self,
        input: PreprocessActionInput<'_>,
        ctx: &mut AppContext,
    ) -> BoxFuture<'static, ()>;

    fn execute_file_edits(
        &mut self,
        input: ExecuteActionInput<'_>,
        ctx: &mut AppContext,
    ) -> AnyActionExecution;

    fn should_autoexecute_file_edits(
        &mut self,
        input: ExecuteActionInput<'_>,
        ctx: &mut AppContext,
    ) -> bool;
}
```
`AgentToolExecutor` owns the only top-level `AIAgentActionType` dispatch. For shared tools like grep, file glob, and read files, it runs shared default logic. For non-shared tools like shell commands and file edits, it delegates to `SurfaceSpecificToolExecutor`.
This avoids optional override registries. Since GUI and TUI are the only surfaces, if a tool is inherently non-shared, both surfaces must implement the same required method and make an explicit decision.
The GUI implementation delegates to existing GUI machinery:
* shell commands use `ShellCommandExecutor`, `TerminalModel`, and terminal blocks;
* file edits use `RequestFileEditsExecutor` and `CodeDiffView`;
* GUI-only events remain in the GUI action executor unless they become true shared agent tools.
The TUI implementation supplies only TUI-specific behavior:
* shell commands use the TUI local `Session` and TUI-owned process state;
* file edits use shared diff application plus v0 auto-save;
* simple cards summarize surface-specific execution for the transcript.
`app/src/ai/blocklist/action_model/basic_tool_executor.rs` must not remain as a TUI-only top-level dispatch tree. Its reusable pieces should move into shared executor helpers or TUI surface-specific methods, and both GUI and TUI must call the same `AgentToolExecutor<S>` before this PR is complete.
### Tool-result follow-up
Move action-result follow-up out of `BlocklistAIController` and into shared conversation machinery. The tool-result turn is part of the shared loop:
```text
stream response -> execute tools -> send tool results -> continue streaming
```
Preferred shape:
```rust
impl AgentConversationEngine {
    pub(crate) fn connect_action_results(...);
}
```
If this is large enough to deserve a separate type, use `AgentToolFollowUpCoordinator` as an implementation detail owned by `AgentConversationEngine`, not as another surface-specific controller.
The shared follow-up logic should wait until a conversation has no unfinished actions, preserve relevant passive-diff and long-running-command completion behavior, drain finished action results in original tool-call order, build `RequestInput` for action results from the active session snapshot, and send the follow-up through `AgentConversationEngine`.
### TUI tool pipeline
`CoreTuiModel` should own or reference:
* the single `AgentSessionOwnerId`;
* `TuiActiveSession`;
* `TuiToolActionModel` or the extracted shared action model;
* the `AgentConversationEngine` tool-result follow-up hookup.
After this is wired, remove `supported_tools_override: Some(vec![])` from normal TUI prompt sends.
### Command running
Command execution is surface-specific and should be required by `SurfaceSpecificToolExecutor`.
The GUI implementation keeps existing terminal-block behavior through `ShellCommandExecutor`.
The TUI implementation should not pretend it has terminal blocks. It should track commands by a TUI-owned command id and convert to the legacy `BlockId` field only at the action-result/API boundary, because existing result types store command ids as `BlockId` ([`crates/ai/src/agent/action_result/mod.rs:183 @ f2592f0`](https://github.com/warpdotdev/warp/blob/f2592f04a9c6544780d830058d6571a2f091df80/crates/ai/src/agent/action_result/mod.rs#L183)).
`TuiCommandModel` stores command text, start/finish timestamps, exit code, captured stdout/stderr buffer, stdin handle, and cancellation handle. `RequestCommandOutput` returns `Completed` when finished and `LongRunningCommandSnapshot` with bounded captured output when still running at the wait boundary. `ReadShellCommandOutput` and `WriteToLongRunningShellCommand` look up the TUI command mapping and return the existing result variants.
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
* Generalize `ActiveSession` if the parallel `TuiActiveSession` and GUI `ActiveSession` converge.
* Remove compatibility wrappers once `BlocklistAIActionModel` is thin enough to rename or delete.
