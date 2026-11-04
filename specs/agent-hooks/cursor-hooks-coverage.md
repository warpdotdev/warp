# Cursor agent hooks: what Warp can cover

Status: this is the coverage half of the agent-hooks scoping. Recommendations on architecture, permissions, config sources, and trust live in [`architecture.md`](architecture.md); this report only summarizes them where a coverage verdict depends on them. Where the Phase 1 implementation (`architecture.md` §14) settled or changed a fact this report states, the report says what was built.

Sources: the research reports, cited as "report 01" through "report 05": [report 01](research/01-cursor-hooks-catalog.md) is the Cursor hooks catalog, [report 02](research/02-claude-code-and-codex-hooks.md) covers Claude Code and Codex hooks, [report 03](research/03-warp-agent-loop-map.md) maps Warp's agent loop, [report 04](research/04-warp-config-process-infra.md) covers Warp's config and process infrastructure, and [report 05](research/05-cursor-cloud-ask-behavior.md) records what the vendors document about hook `ask` with no user present. Plus read-only checks against `warp` at `393e97a30a5ed85cf062b75e88326f83746dbecb` (master). Cursor documentation was read on 2026-10-19. Where a Cursor behavior is undocumented, this report says "docs do not say" and does not guess. Where a Warp detail comes from report 03 or 04 without being re-read here, it is marked "inferred" when the report marked it that way.

Repo paths are relative to the `warp` checkout at `393e97a30a5ed85cf062b75e88326f83746dbecb` (master). Line numbers are from that commit and will drift.

## 1. Summary

Cursor hooks are user-supplied scripts that Cursor runs at fixed points in its agent loop: before and after tool calls, when a prompt is submitted, when a turn ends, and at a few session and editor lifecycle moments. Each hook gets one JSON object on stdin describing the moment, and may print one JSON object on stdout to allow, deny, annotate, or (in a few cases) rewrite what the agent is about to do. Hooks are configured in `hooks.json` files at enterprise, team, project, and user scope. A second hook type, `prompt`, asks a model to evaluate a natural-language condition instead of running a script. Cursor documents 21 hook events.

Headline coverage. Of the 21 hooks:

- 9 are **Full**: Warp can honor every documented input and output with client-only work.
- 9 are **Partial**: `sessionStart`, `sessionEnd`, `preToolUse`, `postToolUse`, `postToolUseFailure`, `subagentStart`, `subagentStop`, `beforeShellExecution`, `workspaceOpen`. Some field, output, or firing moment cannot be matched with client-only work. Five of these are Partial only because one output needs a proto or server change: `additional_context` for `sessionStart`, `postToolUse`, and `postToolUseFailure` (new `hook_context` fields plus server rendering), and the model-visible deny reason (`agent_message`) for `preToolUse` and `beforeShellExecution` on shell commands, which Phase 1 delivered through the new `hook_denied` slot; for `preToolUse` the reason is still missing on the few tools whose result has no text slot (section 3.0).
- 1 is **Blocked on server work**: `preCompact`. Automatic compaction is triggered inside warp-server and the client cannot see it before it happens.
- 2 are **Not applicable**: `beforeTabFileRead`, `afterTabFileEdit`. Warp has no AI inline-completion surface in its editor.

Counting only the 18 hooks Cursor lists under "Agent hooks" (excluding the two Tab hooks and `workspaceOpen`): 9 Full, 8 Partial, 1 Blocked on server work. All 18 can fire with client-only work, and every blocking hook among them can block with client-only work.

warp-server work is needed for: `preCompact` (blocked); `additional_context` on the allow path for `sessionStart`, `postToolUse`, and `postToolUseFailure` (new `hook_context` fields on the request and tool-result protos, rendered by the server); a reason field on the shell `PermissionDenied` result, which Phase 1 shipped (`architecture.md` §9.3: `hook_denied { reason }`, rendered by the server as `An agent hook denied running <command>: <reason>`), and on the empty-result tools, which Phase 1 did not: a hook denial of `InitProject`, `OpenCodeReview`, `WaitForEvents`, `SuggestPrompt`, or one of the three long-running-shell actions reaches the model without a reason (the action finishes with its cancelled result, or for `InitProject` and `OpenCodeReview` the completed marker, and the reason reaches only the user's notice); `subagentStart` and `subagentStop` for Warp's server-side subagents; and prompt-based hooks for every event (a new "evaluate prompt" endpoint). Blocking itself is client work for every blocking hook; only the model-visible reason depends on the wire, and now only for the no-text-slot tools.

Verdict definitions used in this report:

- **Full**: all documented inputs can be populated (possibly with plumbing) and all documented outputs can be honored, with client-only work. "Available with work" fields do not downgrade a Full verdict. Outputs that Warp defers to a later phase but can honor client-only (for example `updated_input`) do not downgrade it either.
- **Partial**: at least one documented input is not available, or one documented output cannot be honored or needs warp-server work, or the moment Cursor fires at has no exact Warp equivalent. The matrix in section 5 names the gap.
- **Blocked on server work**: the hook cannot fire at the right moment at all without a change to warp-server.
- **Not applicable**: Warp has no surface where the hook's purpose makes sense.

The most important findings:

1. **Warp's loop already has the seams.** Every tool call enters execution through `BlocklistAIActionExecutor::try_to_execute_action` (`app/src/ai/blocklist/action_model/execute.rs`) and every terminal outcome leaves through `BlocklistAIActionModel::handle_action_result` (`app/src/ai/blocklist/action_model.rs`). Prompt submission goes through `BlocklistAIController::send_query` and turn end through the `AfterStreamFinished` handler in `app/src/ai/blocklist/controller.rs`. These are shared by the GUI, the TUI, and Oz cloud runs. The main structural change is that the permission-and-dispatch step is synchronous today; a blocking pre-tool hook needs a new "waiting on hook" state in the action pipeline.
2. **The surfaces mostly differ in config roots, not in hook execution.** Reports 03 and 04 confirm this for GUI, TUI, and Oz. Three real exceptions: remote orchestration children run no loop on the parent's machine (their hooks must run in the child's own Oz runtime); headless runs have no channel for an `ask` decision (it degrades to deny with a reason the model reads in the tool result; report 05 shows Cursor does not document this case, Claude Code documents deny-with-reason for `-p` runs, and Codex treats `ask` as a failed hook); and the user-message channel differs per surface (toast, TUI hint, stdout). An Oz run's initial prompt is also resolved server-side, so `beforeSubmitPrompt` cannot see it. The first phase enables only Oz cloud runs and `warp agent run`; the GUI and TUI follow in Phase 1.5 (`architecture.md` §11).
3. **Three things are out of the client's reach.** Automatic context compaction is decided and performed by warp-server. Warp's server-side subagents (CLI, research, advice, computer-use) start and stop without a client-side pre-execution point. And there is no per-result context channel on the wire: `convert_input` (`app/src/ai/agent/api/convert_to.rs`) sends one request-level context and converts each `ActionResult` to the bare result, and the pinned proto `Request.Input.ToolCallResult` has only `tool_call_id` and the result oneof. A deny reason can reach the model inside the denied tool result only where that tool's result has a free-text error or denied variant: MCP, `read_files`, file edits, `run_agents`, and most other tools do; shell does since Phase 1 added `PermissionDenied.hook_denied` (`CancelledBeforeExecution` is still dropped from the wire, and a hook denial does not use it); the empty-result tools still do not. `additional_context` on the allow path needs new proto fields and server rendering everywhere. Architecture §9.3 has the same finding, the shell slot as built, and the open recommendation for the remaining families.
4. **Prompt hooks need one new server endpoint, and Warp could then offer them in cloud runs.** Warp never calls a model directly; every side call is a warp-server endpoint. Cursor cannot run prompt hooks in its cloud agents because of auth wiring; Warp's client is always authenticated, so once the endpoint exists it works everywhere.
5. **Permissions, config sources, and trust are settled in `architecture.md`; this report summarizes them.** One component, `HookPermissionPolicy`, composes hook verdicts with Warp's permission result. Hooks only tighten in the first phase (`deny` and `ask` honored, `allow` ignored). A later phase lets hook `allow` satisfy only the confirmation Warp would show under `ActionPermission::AgentDecides`, never under `AlwaysAsk` and never past denylists or protected paths, and only for sources granted a separate "may approve" capability. Phase 1 reads the project `.cursor/hooks.json` at each detected git root, explicit sources delivered with the run, and factory agent definitions; the user file `~/.cursor/hooks.json` follows in Phase 1.5 behind a setting; no Warp-owned file is planned. Project hooks activate automatically only when the run's execution profile already auto-approves command execution (the Oz default), and otherwise need an explicit flag or config entry; the active set is pinned at run start and hook files are protected write paths.

## 2. How Cursor hooks work

This section is a condensed reading of report 01. See that report for field-by-field detail and source links.

**Config files and scopes.** A `hooks.json` file has a numeric `version` (Cursor's examples use `1`) and a `hooks` object mapping event names to arrays of hook definitions. Each definition has a `command` (shell string or path), optional `type` (`command`, the default, or `prompt`), `timeout` (seconds), `matcher` (regex string), `failClosed` (boolean, default false), `loop_limit` (for `stop` and `subagentStop`, default 5), and for prompt hooks a `prompt` and optional `model`. Files live at four native scopes: enterprise (OS-managed path such as `/Library/Application Support/Cursor/hooks.json`), team (distributed from the Cursor dashboard), project (`<project-root>/.cursor/hooks.json`), and user (`~/.cursor/hooks.json`). Cursor plugins may also ship `hooks/hooks.json`. Cursor reloads these files on save. Project hooks only run in a trusted workspace. (Report 01 §1.1 to §1.3.)

**Precedence and merge.** All matching hooks from every scope run. Priority order is Enterprise, Team, Project, User. Permission results merge by severity: `deny` beats `ask` beats `allow`, regardless of scope. `user_message` and `agent_message` strings concatenate. For every other field the docs say "last response wins", and the main Hooks page says lower-priority scopes can therefore override higher-priority ones; the third-party compatibility page says the opposite. Report 01 flags this contradiction and it is not resolved here. Cursor's CLI runs matching hooks in parallel; the desktop app's scheduling is not documented. (Report 01 §1.2, §2.3, §7.)

**The stdin/stdout contract.** A command hook is a child process. It receives one JSON object on stdin and may write one JSON object on stdout. Every event's input includes a common envelope (`conversation_id`, `generation_id`, `model`, optional `model_id` and `model_params`, `hook_event_name`, `cursor_version`, `workspace_roots`, `user_email`, `transcript_path`) plus event-specific fields. `workspaceOpen` uses a reduced envelope. Output fields are event-specific; for example `permission`, `user_message`, `agent_message`, `updated_input`, `additional_context`, `continue`, `followup_message`, `env`, `pluginPaths`. Cursor does not publish a formal JSON schema. (Report 01 §2.1, §2.4.)

**Exit codes and failure policy.** Exit 0 means success and the JSON output is used. Exit 2 means block, the same as `permission: "deny"`. Any other non-zero exit, a crash, or a timeout is a failure, and the action proceeds (fail-open) unless `failClosed: true`. Invalid JSON or a schema mismatch blocks the action for the six hooks Cursor calls permission hooks (`beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile`, `beforeTabFileRead`, `subagentStart`, `preToolUse`) even when `failClosed` is false. Behavior for empty stdout is ambiguous in the docs. (Report 01 §2.2.)

**Matchers.** A `matcher` is a regex string; `""` and `"*"` match everything. The value matched depends on the event: tool name for `preToolUse`/`postToolUse`/`postToolUseFailure` (`Shell`, `Read`, `Write`, `Grep`, `Delete`, `Task`, `MCP:<tool>`), the full command string for shell hooks, subagent type for subagent hooks, and fixed literals for most others. Regex flavor, anchoring, and case sensitivity are not documented. (Report 01 §1.4.)

**Timeouts.** `timeout` is in seconds. The default is described only as "platform default"; no number is published. (Report 01 §1.1, report 02 §D.)

**Prompt hooks.** `type: "prompt"` replaces the script with a model evaluation of a natural-language `prompt`. `$ARGUMENTS` in the prompt is replaced by the hook input JSON (or the JSON is appended). The model returns `{ "ok": boolean, "reason": string }`. The docs do not say which events accept prompt hooks (the example is `beforeShellExecution`), do not name the default "fast model", and do not say how `ok` maps onto each event's native output. Prompt hooks do not run in Cursor's cloud agents. (Report 01 §4.)

**Claude Code compatibility layer.** Cursor also loads Claude Code hooks from `.claude/settings.local.json`, `.claude/settings.json`, and `~/.claude/settings.json` when third-party imports are enabled (on by default). Eight Claude events map to Cursor events (`PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `Stop`, `SubagentStop`, `SessionStart`, `SessionEnd`, `PreCompact`); `Notification` and `PermissionRequest` are explicitly unsupported. Tool names are mapped (`Bash` to `Shell`, `Edit` to `Write`, and so on). Cursor accepts Claude's nested `hookSpecificOutput` response shape and translates `permissionDecision` to `permission`, `permissionDecisionReason` to `user_message`, and `updatedInput` to `updated_input`. For `Stop`, `decision: "block"` plus `reason` becomes a `followup_message`. These Claude files sit after native User hooks in the combined priority order. (Report 01 §6.) Cursor does not say whether a native `hooks.json` script may answer in that shape. Warp's Cursor adapter accepts it exactly on the three events where Cursor documents the translation, `preToolUse` (`permissionDecision`, `permissionDecisionReason`, `updatedInput`), `stop`, and `subagentStop` (`decision: "block"` with a `reason`, flat or nested), as a fallback when the flat field is absent; everywhere else `hookSpecificOutput` is an undocumented key, so a `beforeShellExecution` hook that answers only in Claude's shape has no decision and denies. A benign Claude-shaped field copied onto another event (`continue: true`, `permission: "allow"`) is logged, not refused. This is a conservative choice awaiting David's decision (`architecture.md` §4.5).

**Cloud agents.** Cursor-managed cloud agents run command hooks only, from project, team, and enterprise sources (no `~/.cursor`). They skip `sessionStart`, `sessionEnd`, the MCP hooks, the Tab hooks, and `workspaceOpen`. (Report 01 §5.)

## 3. Per-hook coverage

### 3.0 Conventions and shared mechanics

**Input status labels.** *Available now*: Warp already has the value at the interception point. *Available with work*: the value exists elsewhere or needs plumbing; the work is named. *Not available*: Warp does not have it, with the reason.

**Output status labels.** *Supportable now*: an existing Warp mechanism can honor it with little more than wiring. *Supportable with work*: a named mechanism needs to be built or extended, client-only. *Supportable with server work*: needs a warp-server change (a new proto field or server-side rendering) in addition to client wiring; this makes the hook's verdict Partial. *Not supportable*: no mechanism, and the reason.

**Common envelope.** Every hook except `workspaceOpen` receives these fields (report 01 §2.1). Their status is the same at every interception point, so they are listed once:

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `conversation_id` | Available now | `AIConversationId` (`crates/ai_types/src/agent.rs`). Exists before the server assigns `ServerConversationToken`. |
| `generation_id` | Available with work | Cursor's changes per user message. Warp maps it to a user-turn id allocated by the prompt-submit hook (section 3.14) and carried across every exchange of that turn; `AIConversation::turn_exchange_ids` (`app/src/ai/agent/conversation.rs`) already groups exchanges per turn. `AIAgentExchangeId`, which changes per request including tool-result follow-ups, is exposed separately as a request id. |
| `model` | Available now | `RequestInput.model_id` (`LLMId`, `app/src/ai/blocklist/controller.rs`). The model actually used arrives later in `Message::ModelUsed`. |
| `model_id` | Available now | Same `LLMId`. |
| `model_params` | Available with work | Warp has no structured parameter list on the client. Send an empty array, or derive effort from the model id. |
| `hook_event_name` | Available now | Set by the hook runner. |
| `cursor_version` | Available now | `app_version()` in `crates/warp_core/src/channel/state.rs`. See section 4.4 on naming. |
| `workspace_roots` | Available with work | Warp has no first-class project root. Use the detected git root for the session cwd (`DetectedRepositories::get_root_for_path`, `crates/repo_metadata/src/repositories.rs`), falling back to the cwd. Oz runs: `AgentDriverOptions.working_dir` and cloned repos. See section 4.1 for the $HOME caveat. |
| `user_email` | Available now | `AuthState::user_email()` via `AuthStateProvider` (`app/src/auth/auth_manager.rs`). Null for anonymous users, which Cursor's schema allows. |
| `transcript_path` | Available with work | Warp keeps the transcript in SQLite (`AIConversation.task_store`), not a file. Null is schema-valid. A JSONL export per conversation would be needed to give a real path. |

**Pre-tool hook: where a blocking hook can sit.** Report 03 §3c identifies two candidate points, and the choice matters for every "before" hook on a tool:

- Point A, the preprocess phase. `BlocklistAIActionModel::queue_actions` already awaits a future per action before anything is enqueued (`action_model.rs:968-999`). A hook awaited here blocks for free. But it runs before Warp's permission check and before the user sees a pending card, so a user who later edits a command in the confirmation card invalidates what the hook saw, and it runs for the whole batch before earlier commands have changed the cwd.
- Point B, the execute step. `try_to_execute_action` (`execute.rs:599`) computes the permission verdict at `:618` and dispatches at `:660`. This is where the final, possibly user-edited action exists. It is synchronous. Supporting an async hook here means a "waiting on hook" state (a distinct `NotExecutedReason::WaitingOnHooks` arm, so it can be told apart from `NotReady`) and re-entering the action model when the hook resolves (`architecture.md` §6.1).

This report assumes Point B for permission-style hooks, because the hook must see the action that will actually run, because a hook `ask` has to be known before Warp makes its own confirmation decision there, and because `deny` after user approval should still deny (policy beats consent). `architecture.md` §6.1 gives the full reasoning. Where the report says "blockable with work (pending-hook state)", this is the work meant. Point A remains a cheap option for observe-only hooks.

**Vendor-faithful defaults and the strict override.** By default Warp follows the adapter's documented semantics. For Cursor that means: `ask` is honored on `beforeShellExecution` and `beforeMCPExecution`, ignored on `preToolUse` with a log line (Cursor says it is accepted by the schema but not enforced), read as a denial on `subagentStart` (as Cursor documents), and a schema mismatch, which denies, on `beforeReadFile` and `beforeTabFileRead`, whose schemas do not list it; the six permission hooks deny on malformed output, where a JSON object with no `permission` or with an unknown value counts as malformed; every other hook fails open unless `failClosed` is set; empty stdout is a no-output failure governed by `failClosed` where the hook decides something. The "strict hook policy" (a per-run flag in Phase 1, `--hooks-strict` or `hooks_strict`) changes two things: `ask` is honored on `preToolUse` too, and every evaluation failure on an event that can block, including `beforeSubmitPrompt`, blocks. It never weakens `subagentStart`'s `ask`-means-deny. Strict mode is a Warp choice, not Cursor parity, and the per-hook tables below describe the default. The `preToolUse` default and the strict reading for `beforeSubmitPrompt` are conservative choices awaiting David's decision (`architecture.md` §4.5, §6.4).

**Rewrites.** `updated_input` is not supported in the first phase. A rewrite returned on a pre-execution boundary that cannot honor it denies the action with a diagnostic, rather than silently running the original. A later phase adds rewrites under a two-pass contract: apply the rewrite, recompute preprocessing and the permission verdict for the new input, re-run the matching hooks on the final input, let a deny from either pass survive, and fail the action if the second pass rewrites again. Scripts may therefore run twice for one tool call. Any rewrite clears the model's risk flags (`is_risky`, `is_read_only`, and `risk_category` once plumbed) because they described the original input.

**Post-tool point.** Every async executor's future is spawned at one place (`execute.rs:807-836`). Wrapping that future lets a hook run before `FinishedAction` is emitted, with the owned result still mutable in `on_complete`. Synchronous results (`InitProject`, `OpenCodeReview`, `InsertCodeReviewComments`, and the denylisted-command short circuit at `:634-656`) need a parallel path. `handle_action_result` (`action_model.rs:1343`) sees every terminal outcome but holds the result behind an `Arc`, so it is observe-only.

**Message channels.** `user_message` has no transcript-level home today: there is no "system note" output message type (report 03 §5.1). Near-term channels: GUI toast (`ToastStack`) or agent-input ephemeral message (`EphemeralMessage`), TUI `show_transient_hint`, Oz stdout via `AgentDriver::write_output`.

Model-visible text is narrower than report 03 §5.2 suggested. `AIAgentInput::ActionResult { context }` is not transmitted per result: `convert_input` (`app/src/ai/agent/api/convert_to.rs`) picks one request-level context (the last input that has one) and `convert_input_to_user_input` converts an `ActionResult` to the bare result; the pinned proto `Request.Input.ToolCallResult` has only `tool_call_id` and the result oneof. Consequences:

- A deny reason can reach the model only inside the denied tool result itself, and only where that result type has a free-text slot on the wire. Verified in `crates/ai/src/agent/action_result/convert.rs`: `CallMCPToolResult::Error` and `ReadMCPResourceResult::Error` map to `Error { message }`; `ReadFilesResult::Error` and per-file `failed_reads[{path, message}]` carry text; `RequestFileEditsResult::DiffApplicationFailed { error }` maps to `apply_file_diffs_result::Error { message }`; `RunAgentsResult::Denied { reason }` maps to a purpose-built `run_agents_result::Denied { reason }`; and the `Error(message)` variants of `Grep`, `FileGlob`, `SearchCodebase`, `ReadSkill`, the document tools, computer use, recordings, `UploadArtifact`, `AskUserQuestion`, `FetchConversation`, and `SendMessageToAgent` all carry text. For those families the reason is client-only work, the way Claude Code delivers it: the model reads the reason in the tool result.
- Shell is covered by a Phase 1 proto change. `PermissionDenied.reason` has a `hook_denied { reason }` member next to the empty `denylisted_command` (warp-proto-apis #404); the client finishes a hook-denied command with `RequestCommandOutputResult::HookDenied { command, reason }`, which converts to that member, so a hook denial is never relabeled as a denylist match; and the server renders it to the model (warp-server #20448). `CancelledBeforeExecution` is still mapped to `ConvertToAPITypeError::Ignore` and never reaches the wire; a hook denial does not use it. The families with no text slot have no slot in Phase 1, and what the model sees differs by family (`app/src/ai/blocklist/hooks/denied_result.rs`): the PTY-write, read, and handoff actions (`ShellCommandError` has only `CommandNotFound`), `SuggestPrompt`, and `WaitForEvents` finish with their cancelled result, which sends no follow-up request when it is the batch's last action, so the conversation is marked cancelled, the turn ends, and the model never learns the action was denied; `InitProject` and `OpenCodeReview` have a single result, which reaches the wire as the completed marker, so the model believes the action happened. `SuggestNewConversation` finishes `Rejected`, the answer the model expects for "no". The reason reaches the user through the `Denied` notice in every case. This is a conservative choice awaiting David's decision, with the recommendation in `architecture.md` §9.3 to add a slot for at least the three shell families and `WaitForEvents`. Blocking itself is client-only everywhere.
- `additional_context` and `agent_message` on non-deny paths need new `hook_context` fields on the request and tool-result protos plus server-side rendering. That is warp-server work.
- The request-level context slot, `AIAgentContext::SelectedText`, and appending text to a tool's output are not compatible channels. The slot is lossy when several results are batched in one request, and the other two change what the tool appears to have said.

**Tool names.** Cursor's `tool_name` vocabulary is `Shell`, `Read`, `Write`, `Delete`, `Grep`, `Task`, and `MCP:<tool>`. Warp has about 35 `AIAgentActionType` variants (`crates/ai/src/agent/action/mod.rs`). Section 4.2 gives the proposed mapping. Warp-only tools get Warp-specific names; a `"*"` matcher will match them.

**Normalized event sketch.** To keep the Cursor format and the execution layer apart, the runner should consume one internal event shape and let a per-format adapter serialize it. The sketch below is illustrative; `architecture.md` §4 defines the canonical model.

```rust path=null start=null
enum HookMoment { PreTool, PostTool, PostToolFailure, PromptSubmit, Stop, /* ... */ }
struct HookEvent { moment: HookMoment, conversation: AIConversationId, exchange: AIAgentExchangeId,
                   payload: serde_json::Value /* event-specific, format-neutral */ }
enum HookDecision { Allow, Ask, Deny { user_message: Option<String>, agent_message: Option<String> } }
struct HookOutcome { decision: Option<HookDecision>, updated_input: Option<serde_json::Value>,
                     additional_context: Vec<String>, followup_message: Option<String> }
trait HookSource { fn scope(&self) -> Scope; fn hooks_for(&self, moment: HookMoment) -> Vec<HookDef>; }
trait HookFormat { fn parse(&self, bytes: &[u8]) -> Result<Vec<HookDef>>;
                   fn encode_input(&self, ev: &HookEvent) -> serde_json::Value;
                   fn decode_output(&self, moment: HookMoment, out: &[u8]) -> Result<HookOutcome>; }
```

---

### Agent hooks

### 3.1 `sessionStart`

**Purpose and timing.** Fires when a new composer conversation is created. Fire-and-forget; Cursor does not wait for it. (Report 01 §3.1.)

**Warp moment.** The barrier at the start of `prepare_turn` inside `BlocklistAIController::send_query` (`app/src/ai/blocklist/hooks/session.rs`), before the conversation's first request, once per conversation id for a conversation this client drives that has no exchanges yet. Not `BlocklistAIHistoryModel::start_new_conversation`: at creation the run is not yet pinned, so project hooks are not visible, and the driver is not yet known, so a conversation an external harness later drives would fire it. Oz runs: `AgentDriver::execute_run` records the run but does not fire the event; the first request's barrier does. Resume paths (`restore_conversations`, `resume_conversation`) produce conversations with exchanges and never fire it; Cursor does not say whether resume fires `sessionStart`.

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `session_id` | Available now | Same as `conversation_id`. |
| `is_background_agent` | Available now | `AppExecutionMode::is_autonomous()` (`crates/warp_core/src/execution_mode.rs`), or the conversation is an orchestration child. |
| `composer_mode` | Available with work | Warp has `UserQueryMode::{Normal, Plan, Orchestrate}` (`app/src/ai/agent/mod.rs`) per query, not per session. Send `agent` at creation; Cursor's `ask`/`edit` have no Warp equivalent. |
| `repo_urls`, `repos[]` | Available with work | Self-hosted-only in Cursor. Oz runs know the cloned repos from environment prep (`register_cloned_repo`, `app/src/ai/agent_sdk/driver/environment.rs`). Local surfaces: `AIAgentContext::Repository`/`Git` carry owner, name, branch. |

**Output fields.**

| Field | Status | Mechanism |
| --- | --- | --- |
| `env` | Supportable now | Hook-runner state: a per-conversation env map merged into later hook spawns. Cursor does not say whether `env` reaches tool processes; this report assumes hooks only. |
| `additional_context` | Supportable with server work | Needs a `hook_context` field on the request proto and server-side rendering (section 3.0). The request-level `AIAgentContext` slot is not a compatible channel. |
| `continue`, `user_message` | Supportable now | Accepted and ignored, as in Cursor. |

**Blocking and modification.** Creation cannot be denied, as in Cursor. The first request of the conversation waits for `sessionStart` to finish, up to the longest matching binding's timeout (a second request that arrives during the wait shares the firing and waits at most 5 s more), so that `env` is in place for later hooks; a hook that fails or times out adds no environment and the request goes out. One owner fires the event, the barrier; `AgentDriver::execute_run` does not.

**Surface notes.** Oz: the barrier runs after environment prep and after the run is pinned, so project hooks are visible; Cursor's cloud agents skip this hook, but Warp has no read-only exploratory phase and can fire it. Local orchestration children create a second conversation in the same process; they fire it too, with `is_background_agent` from `AppExecutionMode::is_autonomous()`.

**Verdict: Partial.** Inputs and `env` are client-only work; `additional_context` needs warp-server work.

### 3.2 `sessionEnd`

**Purpose and timing.** Fires when a composer conversation ends. Fire-and-forget, for logging and cleanup. (Report 01 §3.2.)

**Warp moment.** None single. Candidates (report 03 §3a): `UpdatedConversationStatus` reaching a terminal status, `clear_conversations_for_terminal_surface` / `delete_conversation` (`history_model.rs`), and for Oz `AgentDriver::cleanup`. Warp conversations persist across restarts and can be resumed, so "end" must be defined.

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `session_id` | Available now | `AIConversationId`. |
| `reason` | Available with work | Map: Oz run exit to `completed`/`aborted`/`error`; GUI/TUI conversation delete or surface close to `user_close`/`window_close`. A conversation that simply goes idle has no end event. |
| `duration_ms` | Available with work | Conversation creation time to now; needs the creation timestamp read from the conversation. |
| `is_background_agent` | Available now | As in 3.1. |
| `final_status` | Available now | `ConversationStatus` (`conversation.rs`). |
| `error_message` | Available with work | From the `Error` status or the Oz driver error. |

**Output fields.** None documented. Nothing to honor.

**Blocking and modification.** Not applicable. Oz `cleanup` is async and could wait; GUI/TUI close paths are synchronous and should not wait.

**Surface notes.** Oz has a clean end (run exit). GUI/TUI do not; firing on app quit for every open conversation is possible but noisy. Recommend firing on explicit conversation deletion, surface close, and Oz run exit, and documenting that idle conversations never "end".

**Verdict: Partial.** The data exists, but Warp has no session-end moment that matches Cursor's for the interactive surfaces; the mapping is a definition, not a port.

### 3.3 `preToolUse`

**Purpose and timing.** Fires before any tool executes. The generic pre-tool hook; a matcher narrows by tool name. (Report 01 §3.3.)

**Warp moment.** `BlocklistAIActionExecutor::try_to_execute_action` (`execute.rs:599`), Point B in section 3.0. Point A (`queue_actions` preprocess) is the async-friendly alternative.

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `tool_name` | Available with work | Map `AIAgentActionType` to Cursor names (section 4.2). |
| `tool_input` | Available with work | A JSON serialization of each action variant's arguments. Shell: `{command, cwd, ...}`. Not all action types derive `Serialize`; a dedicated encoder in `crates/ai` is needed. |
| `tool_use_id` | Available now | `AIAgentActionId` (the server `tool_call_id`). |
| `cwd` | Available now | `ActiveSession::current_working_directory()` for the surface. |
| `agent_message` | Available with work | The assistant `Text` output message preceding the tool call in the same exchange (`AIAgentExchange`). |

**Output fields.**

| Field | Status | Mechanism |
| --- | --- | --- |
| `permission: allow/deny` | Supportable with work | Compose with the permission verdict between `execute.rs:618` and `:624-626`. `deny` emits a synthetic terminal result the way the denylist short circuit does (`:634-656`): the tool's error or denied variant where one exists, otherwise `cancelled_result()`. Blocking is client-only for every tool. See section 4.7 for precedence. |
| `permission: ask` | Supportable with work | Default: no opinion with a log line, as Cursor documents for this hook ("accepted by the schema but not enforced"). Under the strict hook policy (section 3.0) it is honored as `ask` and forces `NeedsConfirmation` so the GUI/TUI card appears. Headless: see surface notes. |
| `user_message` | Supportable with work | Message channel (section 3.0). On this hook Cursor documents it as shown only on a deny, so on an `allow` it is dropped with a log line. |
| `agent_message` | Supported for `Shell` and every family with a text slot; needs server work for the no-text-slot tools | Delivered on deny only, per Cursor (on an `allow` it is logged and dropped). Carried as text inside the synthetic denied result, `Denied by agent hook: <reason>`, for every family whose result has a free-text error or denied variant, and for `Shell` through `PermissionDenied.hook_denied` (section 3.0). For `ShellWrite`, `ShellRead`, `ShellHandoff`, `SuggestPrompt`, and `WaitForEvents` the wire still has no slot and the action finishes with its cancelled result; `InitProject` and `OpenCodeReview` reach the wire as the completed marker; `SuggestNewConversation` finishes `Rejected`. |
| `updated_input` | Supportable with work (later phase) | Not supported in the first phase; a rewrite returned here denies with a diagnostic. Later, under the two-pass contract in section 3.0: the action is owned at `:601` and `handle_requested_command_accepted` (`action_model.rs:1312-1341`) already rewrites a command in place. Needs the inverse of the `tool_input` encoder. |

**Blocking and modification.** Blockable with work (pending-hook state at Point B). Modifiable: the action is owned; rewrites are deferred to a later phase.

**Surface notes.** Shared executor, so GUI, TUI, Oz, and local children all pass through. Oz and local children: `ask` has no approver; `try_to_execute_action` already treats autonomous-mode confirmation as a denylisted result for shell commands and `Blocked` for anything else (`execute.rs:627-657`, `driver.rs` `ConversationBlocked`). Settled and built: in headless mode `ask` degrades to `deny` with the reason `Approval unavailable: an agent hook asked for confirmation (<hook reason>) and this run has no one to ask.`, recorded in the notice and inside the tool result where the family has a slot, which is what Claude Code documents for a `-p` run; Cursor's docs do not define the case and Codex treats `ask` as a failed hook (report 05, section 4.7). For shell the slot is `PermissionDenied.hook_denied`. Remote children: the hook runs in the child's runtime. Web search and web fetch run on warp-server and never reach this point; Cursor's `WebSearch`/`WebFetch` tool names cannot be observed.

**Verdict: Partial.** Blocking, `user_message`, and `updated_input` (deferred) are client-only, and `agent_message` reaches the model for `Shell` and every family with a text slot. For the no-text-slot tools (`ShellWrite`, `ShellRead`, `ShellHandoff`, `SuggestPrompt`, `WaitForEvents`, `InitProject`, `OpenCodeReview`) the model-visible deny reason still needs a denied variant, which is server work.

### 3.4 `postToolUse`

**Purpose and timing.** Fires after a tool succeeds. Audit, analytics, context injection. (Report 01 §3.4.)

**Warp moment.** Wrapper around `execute_future` at `execute.rs:807-836` (modifiable), with `handle_action_result` (`action_model.rs:1343`) as the observe-only backstop.

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `tool_name`, `tool_input`, `tool_use_id`, `cwd` | As in 3.3 | The action is retained in `AsyncExecutingAction` until completion. |
| `tool_output` | Available with work | JSON string of `AIAgentActionResultType`. The result types convert to proto for the server (`crates/ai/src/agent/action_result/convert.rs`) but do not all derive `Serialize`; a JSON encoder is needed. |
| `duration` | Available with work | Shell results carry `start_ts`/`completed_ts`. Other tools need a timer between `ExecutingAction` and `FinishedAction`. |

**Output fields.**

| Field | Status | Mechanism |
| --- | --- | --- |
| `updated_mcp_tool_output` | Supportable now | `CallMCPToolExecutor` `on_complete` owns the result before it becomes the action result (`execute/call_mcp_tool.rs`). |
| `additional_context` | Supportable with server work | Needs a `hook_context` field on `ToolCallResult` and server-side rendering (section 3.0). Appending to the tool output is not a compatible substitute. |

**Blocking and modification.** Blockable: yes, by awaiting inside the wrapped future. Modifiable: MCP output yes; other outputs also possible in `on_complete` (a superset of Cursor, which documents MCP only).

**Surface notes.** Shared. Sync executors need the parallel path noted in section 3.0.

**Verdict: Partial.** Inputs and `updated_mcp_tool_output` are client-only work; `additional_context` needs warp-server work.

### 3.5 `postToolUseFailure`

**Purpose and timing.** Fires when a tool fails, times out, or is denied. (Report 01 §3.5.)

**Warp moment.** Same as 3.4; failure detection via `AIAgentActionResultType::is_failed()` / `is_cancelled()` and `cancellation_reason` in `handle_action_result`.

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `tool_name`, `tool_input`, `tool_use_id`, `cwd`, `duration` | As in 3.4 | |
| `error_message` | Available now | `Display` on the result variant, for example `RequestCommandOutputResult::Denylisted`. |
| `failure_type` | Available with work | `error` from `is_failed()`; `permission_denied` from user rejection, denylist, or hook deny; `timeout` has no Warp source (Warp tools do not time out, except the `wait_for_events` idle timer). |
| `is_interrupt` | Available now | `CancellationReason` user-initiated variants (`app/src/ai/agent/mod.rs`). |

**Output fields.**

| Field | Status | Mechanism |
| --- | --- | --- |
| `additional_context` | Supportable with server work | As in 3.4. |

**Blocking and modification.** Same as 3.4. Cannot undo the failure, as in Cursor.

**Surface notes.** A hook `deny` from 3.3 should produce this event, matching Cursor's "denied" trigger. Denylisted commands in autonomous mode take the synchronous short circuit at `execute.rs:634-656` and need the sync path.

**Verdict: Partial.** The event and all inputs are client-only work; its only documented output needs warp-server work.

### 3.6 `subagentStart`

**Purpose and timing.** Fires before spawning a subagent (Cursor's Task tool); can deny creation. (Report 01 §3.6.)

**Warp moment.** Warp has two kinds of subagent. Client-launched children via `run_agents`: `RunAgentsExecutor::execute` then per-child `StartAgentExecutor::dispatch` (`execute/run_agents.rs`, `execute/start_agent.rs`); the executor is async and the request is an owned clone. Server-side subagents (`SubagentType::{Cli, Research, Advice, ComputerUse, Summarization}`, report 03 §1.4) start inside warp-server; the client only sees `Message::Subagent` output after the fact.

**Input fields** (for `run_agents` children).

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `subagent_id` | Not available before launch | The child's server id arrives in `RunAgentsResult::Launched { agent_id }` after dispatch. A client-side placeholder could be generated earlier (small work), but it would not be the id Cursor's later `subagentStop` would use. |
| `subagent_type` | Available with work | Cursor uses `generalPurpose`/`explore`/`shell`. Warp has `harness_type` plus local/remote `execution_mode` (`RunAgentsRequest`). Map to Warp-specific values. |
| `task` | Available now | Composed child prompt (`compose_run_agents_child_prompt`). |
| `parent_conversation_id` | Available now | The parent's `AIConversationId`. |
| `tool_call_id` | Available now | `AIAgentActionId` of the `run_agents` call. |
| `subagent_model` | Available now | `model_id` on the batch or the per-child override. |
| `is_parallel_worker` | Available with work | Definitional. `true` when the batch has more than one child is one option. |
| `git_branch` | Not available | Warp does not create a branch for a child at launch; a child may create a worktree later on its own. |

**Output fields.**

| Field | Status | Mechanism |
| --- | --- | --- |
| `permission: allow/deny` | Supportable now | The executor is async; a deny resolves the action with `RunAgentsResult::Denied { reason }`, which maps to `run_agents_result::Denied { reason }` on the wire (`action_result/convert.rs`), so the reason reaches the model client-only. |
| `user_message` | Supportable with work | Message channel. |

**Blocking and modification.** Blockable: yes for `run_agents` (async executor). Modifiable: the request is owned, so prompt or model could be rewritten in a later phase (superset; Cursor documents no rewrite). Server-side subagents: not blockable and not observable before start.

**Surface notes.** Child creation differs per front-end (hidden pane in GUI, background session in TUI, cloud run for remote), but the dispatch point is shared. Remote children need the hook in the child's runtime for their own tools, but the parent still fires `subagentStart` for the launch.

**Verdict: Partial.** Full for client-launched children except `subagent_id` and `git_branch`. Server-side subagents cannot be gated without a server-emitted pre-start event.

### 3.7 `subagentStop`

**Purpose and timing.** Fires when a subagent completes, errors, or is aborted; may queue a follow-up. (Report 01 §3.7.)

**Warp moment.** No synchronous child-finished callback in the parent executor (report 03 §3h). Signals: local children emit `UpdatedConversationStatus` for the child conversation in the same process; remote children arrive as `OrchestrationEventStreamerEvent::ChildStatusChanged` and as `EventsFromAgents`/`MessagesReceivedFromAgents` injected by `inject_pending_events_for_request` (`controller.rs`). Server-side subagents: completion shows up in the stream as output messages only.

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `subagent_type` | Available with work | As in 3.6. |
| `status` | Available now | Child `ConversationStatus` or `ChildStatusChanged`. |
| `task`, `description` | Available now | From the retained `run_agents` request. |
| `summary` | Available now | The child's completion message in the injected agent events. |
| `duration_ms` | Available with work | Launch time to status change; both are known to the parent. |
| `message_count`, `tool_call_count`, `modified_files` | Available with work for local children; Not available for remote children | Local: the child's `AIConversation` is in the same `BlocklistAIHistoryModel`, so counts and edited paths can be derived. Remote: the child ran on another machine; only what it reports arrives. |
| `loop_count` | Available now | Hook-runner state. |
| `agent_transcript_path` | Not available | No transcript files (see envelope). |

**Output fields.**

| Field | Status | Mechanism |
| --- | --- | --- |
| `followup_message` | Supportable with work | Inject as a user-style input alongside the child's event at `inject_pending_events_for_request`, or via `QueuedQueryModel`. Honored only when `status` is `completed`, as Cursor documents. Same loop protection as `stop` (section 3.16). |

**Blocking and modification.** The injection point is synchronous but owns the `Vec<AIAgentInput>`; a short deferral is possible. Nothing to undo.

**Surface notes.** The parent is often parked in `wait_for_events`; the follow-up should be delivered with the wake, not as a separate turn.

**Verdict: Partial.** Remote children cannot supply counts or modified files; server-side subagents are not covered at all without server events.

### 3.8 `beforeShellExecution`

**Purpose and timing.** Fires before a shell command runs. If a `preToolUse` matcher also matches `Shell`, both run; relative order undocumented. (Report 01 §3.8.)

**Warp moment.** `ShellCommandExecutor::execute` (`execute/shell_command.rs`) builds the decorated command and emits `ShellCommandExecutorEvent::ExecuteCommand` before returning its future; the front-end bridge actually runs it in the PTY (GUI `app/src/terminal/view.rs`, TUI `crates/warp_tui/src/terminal_session_view.rs`). The hook must run before the event is emitted, which is one reason the pending-hook state sits at `try_to_execute_action` rather than inside this executor (`architecture.md` §6.1).

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `command` | Available now | `RequestCommandOutput { command }`. Recommend sending the model's command before pager decoration and PowerShell wrapping (judgment call; see section 7). |
| `cwd` | Available now | Session cwd at execute time (not carried in the event; read from `ActiveSession`). |
| `sandbox` | Available with work | `ShellLaunchData::DockerSandbox` on the session (`app/src/terminal/model/session/command_executor.rs`) or `AppExecutionMode::is_sandboxed()` for cloud runs. |
| `tool_use_id` | Available now | `AIAgentActionId`. |

Also available but not in Cursor's schema: `is_read_only`, `is_risky`, `uses_pager`, `wait_until_completion`. The proto's finer `risk_category` (READ_ONLY through RISKY) is dropped by `From<RunShellCommand>` in `crates/ai/src/agent/action/convert.rs:21-40`; verified, see section 6.

**Output fields.**

| Field | Status | Mechanism |
| --- | --- | --- |
| `permission: allow/deny/ask` | Supported | Same composition as 3.3. `deny` resolves the action with `RequestCommandOutputResult::HookDenied { command, reason }`, a result of its own; it does not borrow `Denylisted`, which would tell the model the command was on a denylist. `ask` is a documented value on this hook (unlike `preToolUse`), so the default honors it by forcing the confirmation card; Cursor does not specify its UI (report 01 §3.8). A response with no `permission`, or an unknown value, is malformed and denies even when the definition fails open (a conservative choice awaiting David's decision). |
| `user_message` | Supportable with work | Message channel; the GUI command card could also carry it. |
| `agent_message` | Supported | Deny path only. `HookDenied` converts to `PermissionDenied.hook_denied { reason }` (warp-proto-apis #404), and the server renders it to the model as `An agent hook denied running <command>: <reason>` (warp-server #20448). `TryFrom<RequestCommandOutputResult>` (`crates/ai/src/agent/action_result/convert.rs`) still drops `CancelledBeforeExecution` from the wire; a hook denial does not use that variant. |

**Blocking and modification.** Blockable with work (async point as above). Modifiable: the command string is owned (the card already rewrites it), but rewrites are a later phase (section 3.0). Cursor documents no rewrite field on this hook; `preToolUse.updated_input` is the rewrite path.

**Surface notes.** Interact mode (`wait_until_completion == false`) returns a `LongRunningCommandSnapshot`; the server's CLI subagent then issues `WriteToLongRunningShellCommand` PTY writes. Those are separate actions with no Cursor equivalent, exposed through `preToolUse` as `ShellWrite`, `ShellRead`, and `ShellHandoff` rather than firing `beforeShellExecution` per keystroke. Headless: `ask` degrades to deny with the approval-unavailable reason, which the model reads in `hook_denied` (section 4.7; report 05 for the vendor precedents).

**Verdict: Partial as scoped, Full as built.** Blocking and `user_message` are client-only, and the model-visible deny reason ships with the Phase 1 proto and server change. Inputs are complete.

### 3.9 `afterShellExecution`

**Purpose and timing.** Fires after a shell command finishes. (Report 01 §3.9.)

**Warp moment.** `action_result_for_requested_command` (`execute/shell_command.rs`) builds `RequestCommandOutputResult::Completed`; generic post-tool point otherwise.

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `command` | Available now | `Completed { command }` (the executed command). |
| `output` | Available now | `Completed { output }` from the terminal block. Note: this is the unredacted block output; Warp's secret redaction runs later at request send (`redact_inputs`, `app/src/ai/agent/api/impl.rs:55-57`). |
| `duration` | Available now | `start_ts`/`completed_ts` on the result; these are command timestamps, so approval wait is excluded as in Cursor. |
| `sandbox` | Available with work | As in 3.8. |
| `tool_use_id` | Available now | |

**Output fields.** None documented.

**Blocking and modification.** Blockable: yes (post-tool wrapper). Output modifiable in `on_complete` (superset).

**Surface notes.** The event fires whenever the command ran and returned, at any exit code (including 130) and for a shell that ended under the command (`ShellRecovered`); a hook that wants failures only reads `exit_code`, which Warp adds to the payload. It does not fire for a command that never ran (a denial, a cancellation, a busy terminal), which goes to `postToolUseFailure`. Interact mode: the first result is a `LongRunningCommandSnapshot` while the command is still running. `afterShellExecution` does not fire on that snapshot; the completion funnel remembers the snapshot by block id and, when a later `ReadShellCommandOutput`, `WriteToLongRunningShellCommand`, or `TransferShellCommandControlToUser` result reports `CommandFinished` or `ShellRecovered` for that block, fires it once on the command's own action with the command text, output, exit code, and duration from the timestamps. It does not fire for a user-started command the agent only polls, since the agent did not run it. Three exits are not observable this way and fire nothing: a command the model stops polling, a poll cancelled before the exit, and a command handed to the user whose exit no later poll reports. Firing only on an exit the client saw, and never synthesizing one, is a conservative choice awaiting David's decision. `postToolUse` fires for each step as usual.

**Verdict: Full.**

### 3.10 `beforeMCPExecution`

**Purpose and timing.** Fires before an MCP tool executes. (Report 01 §3.10.)

**Warp moment.** `CallMCPToolExecutor::execute` (`execute/call_mcp_tool.rs`): arguments are an owned `serde_json::Map` (already mutated by `coerce_integer_args`), then `call_tool` is awaited. `ReadMCPResource` has its own executor and no Cursor counterpart.

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `tool_name` | Available now | `CallMCPTool { name }`. |
| `tool_input` | Available now | JSON-stringify the `serde_json::Value`. |
| `mcp_server_name` | Available now | Server name resolved via `TemplatableMCPServerManager` (`app/src/ai/mcp/`). |
| `mcp_server_url`, `url` | Available with work | `TransportType::ServerSentEvents { url }` on the server template (`crates/cloud_object_models/src/mcp.rs`); needs a lookup from installation id to template. |
| `command` | Available with work | `TransportType::CLIServer { command, args }`, joined with spaces. |
| `tool_use_id` | Available now | |

**Output fields.**

| Field | Status | Mechanism |
| --- | --- | --- |
| `permission: allow/deny/ask` | Supportable with work | Same composition as 3.3; `deny` resolves with `CallMCPToolResult::Error(message)`. |
| `user_message` | Supportable with work | Message channel. |
| `agent_message` | Supportable with work | Deny path only. `CallMCPToolResult::Error(message)` maps to `call_mcp_tool_result::Error { message }` on the wire (`action_result/convert.rs`), so the reason reaches the model client-only. `ReadMCPResourceResult::Error` has the same shape. |

**Blocking and modification.** Blockable: yes (async executor). Modifiable: arguments yes (precedent exists).

**Surface notes.** Not available on wasm (MCP runtime is native-only). Cursor's cloud agents skip this hook; Warp's Oz runs can fire it. Warp fires it for `CallMCPTool` only; `ReadMCPResource` is reachable through `preToolUse` as `MCPResource:<name>`, because firing the MCP events for it would need a made-up `tool_name`. The matcher is tested against the MCP tool's own name without the `MCP:` prefix. In Phase 1 the payload carries `mcp_server_id` and omits `mcp_server_name`, `url`, and `command`, which the envelope does not yet carry; `tool_input` is sent as a JSON string, as Cursor documents for the MCP events.

**Verdict: Full.**

### 3.11 `afterMCPExecution`

**Purpose and timing.** Fires after an MCP tool executes. (Report 01 §3.11.)

**Warp moment.** `on_complete` in `CallMCPToolExecutor::execute`, before `handle_call_tool_result`.

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `tool_name`, `tool_input`, `mcp_server_name`, `mcp_server_url`, `tool_use_id` | As in 3.10 | |
| `result_json` | Available now | The raw rmcp result, serialized. |
| `duration` | Available with work | Timer around the awaited call. |

**Output fields.** None documented. (`postToolUse.updated_mcp_tool_output` is the rewrite path.)

**Blocking and modification.** Blockable: yes. Modifiable: yes (superset).

**Surface notes.** Same as 3.10. The event fires for a returned result and for an MCP error result alike (`result_json` is `{"error": "<message>"}` for a failed call); a cancelled call goes to `postToolUseFailure`.

**Verdict: Full.**

### 3.12 `beforeReadFile`

**Purpose and timing.** Fires before the agent reads a file; the payload includes the file contents. Cursor positions it as access control for sensitive files. (Report 01 §3.12.)

**Warp moment.** The hook fires after Warp has read content locally and before that content is placed in a tool result for the model. This matches Cursor's contract: the read happens, then the hook decides whether the model sees it. Warp's scope follows Cursor's "before the agent reads a file" semantics as closely as its tool set allows, which means the agent-invoked tools that read specific files:

- `ReadFiles` (`ReadFilesExecutor::execute`, `execute/read_files.rs`): the `ReadFilesResult` is owned in `on_complete` before it becomes the action result. One hook call per file.
- `ReadSkill` (`execute/read_skill.rs`): an agent-invoked read of a specific `SKILL.md`. One call per skill file. Cursor's docs do not say whether loading a skill fires `beforeReadFile` in Cursor; the hook's matcher target is the `Read` tool name, and nothing in the hooks or skills pages mentions skills. Warp fires it deliberately, since there is no skill-specific hook.
- `GetFiles` (`execute/get_files.rs`): the agent asks for relevant files and Warp reads them through `read_local_file_context`. One call per file returned.
- `SearchCodebase` (`execute/search_codebase.rs`): the search runs first, then one call per surfaced file with the content that would be shown. A denied file is removed from the results before they reach the model.

Out of scope by design: project rules content (`AIAgentContext::ProjectRules`) is not gated; as in Cursor, rule files appear in the `attachments` array with `type: "rule"`. Passive-suggestion `relevant_files` are not part of the agent loop. `Grep` match lines are not gated, which matches Cursor, whose `Grep` tool is not covered by `beforeReadFile` (report 01 §1.4).

A correction to report 03: user-attached files are sent as path references (`AIAgentAttachment::FilePathReference`, `app/src/ai/agent/mod.rs`; built in `add_pending_file_attachments`, `controller.rs`), not as content. The type's doc comment says the server presents the path to the model as an inline reference, so the content reaches the model only through a later `read_files` call, which is hookable. The server side of this was not verified.

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `file_path` | Available now | `FileContext.file_name`, resolved to an absolute path by `host_native_absolute_path`. |
| `content` | Available now | `AnyFileContent::StringContent`. Binary/image reads (`BinaryContent`) have no string form; recommend omitting content and marking the type (judgment call). Reads are capped at 1 MB per file. |
| `attachments` | Available with work | Cursor's prompt attachments (`file` or `rule`). Warp: `FilePathReference` paths and `ProjectRules` paths from the current request's inputs. |

**Output fields.**

| Field | Status | Mechanism |
| --- | --- | --- |
| `permission: allow/deny` | Supportable now | On deny, move the file to `failed_files` with a message; `ReadFilesResult` carries `failed_reads[{path, message}]` and an `Error { message }` variant on the wire (`action_result/convert.rs`), so the model learns why client-only. |
| `user_message` | Supportable with work | Message channel. |

**Blocking and modification.** Blockable: yes (async executor). Modifiable: yes, content can be replaced or redacted before the result is returned. Cursor documents no redaction output (report 01 §7 item 3); Warp could offer it as an extension.

**Surface notes.** Shared across GUI, TUI, and Oz. Remote (SSH) sessions read through `RemoteServerManager`; under the section 4.1 proposal, hooks do not run for SSH or WSL sessions at first, so those reads are not gated.

**Verdict: Full.** Every input and output is client-only. The work is four executor call sites rather than one, and `SearchCodebase` needs the search-then-gate ordering above.

### 3.13 `afterFileEdit`

**Purpose and timing.** Fires after the agent edits a file. (Report 01 §3.13.)

**Warp moment.** Post-write: `on_complete` of `RequestFileEditsExecutor::execute` (`execute/request_file_edits.rs`) after `accept_and_save` has written through `DiffStorageHelper` (`app/src/ai/blocklist/diff_storage.rs`). Warp also has two earlier points Cursor does not expose: pre-review (`on_diffs_applied`, where `FileDiff`s are owned and nothing is on disk) and pre-write (inside `accept_and_save` before `start_saving`).

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `file_path` | Available now | From `FileSnapshot.updated.path` / `FileDiff.base.file_path`. |
| `edits[].old_string`, `edits[].new_string` | Available with work | `StrReplaceEdit { search, replace }` maps directly; `V4AEdit` hunks have `old`/`new`; `Create` is `""` to content; `Delete` is content to `""`. If the user edited the diff in the GUI `CodeDiffView` before accepting (`UpdatedFileState.was_edited`), per-edit strings no longer align; send whole-file old/new in that case. Cursor's Jan 8 note that `old_string` carries "the file's previous content" is ambiguous between per-edit and whole-file; carried forward unresolved. |

**Output fields.** None documented.

**Blocking and modification.** Blockable: yes (async futures at both stages). Modifiable: the edit is already on disk at Cursor's moment; Warp's pre-review and pre-write points allow filtering or rewriting as an extension.

**Surface notes.** GUI writes through editor buffers after optional user review; TUI writes straight through `FileModel` (`crates/warp_tui/src/tui_diff_storage.rs`); Oz auto-approves. The hook sees the final state in all three. After the hooks return, Warp must refresh its file snapshots (`FileSnapshot`, GUI editor buffers) because a formatter hook may have changed the file on disk; otherwise the next edit's `DiffBase.content` is stale and the diff fails to apply.

**Verdict: Full.**

### 3.14 `beforeSubmitPrompt`

**Purpose and timing.** Fires after the user hits Send and before the backend request; can block submission. (Report 01 §3.14.)

**Warp moment.** `prepare_turn` inside `BlocklistAIController::send_query` (`controller.rs`, `app/src/ai/blocklist/hooks/prompt.rs`), after the request has been built and before it goes out; `send_query` is split at that point and either sends on the same call or spawns the decision and finishes in `conclude_prepared_query`. This is the right semantic match: it covers user-typed prompts and hook follow-ups only, not custom inputs, slash commands, or tool-result follow-ups, and every `send_query` starts a turn there, so the turn id exists before the hook runs. The alternative, `generate_multi_agent_output` (`app/src/ai/agent/api/impl.rs`, where `redact_inputs` already mutates inputs inside an async fn), covers every request kind including tool-result follow-ups and slash commands, which Cursor does not fire this hook for, and it runs after the exchange has been appended and the status set to `InProgress`, so blocking there means unwinding UI state.

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `prompt` | Available now | `query: String`. |
| `attachments` | Available with work | `referenced_attachments` holds file references, blocks, drive objects, diffs, plan documents; images are `AIAgentContext::Image`. Map `FilePathReference` to `file` and `ProjectRules` paths to `rule`; other kinds are Warp extensions or omitted. |

**Output fields.**

| Field | Status | Mechanism |
| --- | --- | --- |
| `continue: false` | Supported | The split in `send_query` awaits the hook before anything is appended, so blocking is clean: the prompt is not sent, a `Denied` notice carries the hook's reason, the conversation's status is unchanged, and the dirty orchestration events the query took are put back. A `deny` verdict and an `ask` halt the same way, since no confirmation step exists for a prompt (conservative choices awaiting David's decision). Exit 2 halts too. A newer prompt for the same conversation replaces one still waiting on its hooks. |
| `user_message` | Supportable now (GUI) / with work | GUI agent view already has `EphemeralMessage` in the input bar; TUI `show_transient_hint`; Oz stdout. |

**Blocking and modification.** Blockable: yes (the split in `send_query`). Modifiable: the query is an owned `String`; Cursor's changelog claims prompt modification but its schema has no field (report 01 §3.14). Warp could offer a rewrite extension; the rewritten text would be visible in the transcript unless kept in display fields.

**Surface notes.** Queued prompts (`QueuedQueryModel`) and shared-session prompts also pass through `send_query`. Oz runs: the first prompt is sent as `AIAgentInput::StartFromAmbientRunPrompt`, which carries only the run id; the server resolves the prompt text from the stored task (`app/src/ai/agent/mod.rs`, `api/convert_to.rs`). The client never sees that text, so that turn starts (turn id, session barrier) but `beforeSubmitPrompt` does not fire for an Oz run's initial prompt without a server change. Later prompts into the run (shared-session prompts) do pass through `send_query`. Hook-originated follow-ups (a `stop` or `subagentStop` `followup_message`) also pass this hook, with `HookFollowUp` provenance on the envelope so a hook can see and veto them; Cursor's schema has no provenance field, so exposing it in the Cursor payload would be a Warp extension field.

**Verdict: Full.** The Oz initial prompt is a surface gap, not a field or output gap; it is listed in the matrix.

### 3.15 `preCompact`

**Purpose and timing.** Fires before context compaction. Observational; cannot block or change compaction. (Report 01 §3.15.)

**Warp moment.** Automatic compaction is decided and performed by warp-server; the client learns of it only when `Message::Summarization` and the `MoveMessagesToNewTask` client action arrive (`conversation.rs`, report 03 §3i). User-triggered `/compact` becomes `AIAgentInput::SummarizeConversation` (`controller/slash_command.rs`) and is observable before send.

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `trigger` | `manual`: Available now. `auto`: Not available before the fact | The server does not signal "about to summarize". |
| `context_usage_percent`, `context_tokens`, `context_window_size` | Available with work | `context_window_limit` is sent per request (`api.rs`); token usage arrives with each finished stream. Values are as of the last response, not the moment the server decides. |
| `message_count` | Available now | From the conversation's tasks. |
| `messages_to_compact` | Not available | Server decision. |
| `is_first_compaction` | Available now | Usage metadata `was_summarized`. |

**Output fields.**

| Field | Status | Mechanism |
| --- | --- | --- |
| `user_message` | Supportable with work | Message channel. |

**Blocking and modification.** Not applicable (observational in Cursor too).

**Surface notes.** Shared. Oz runs compact the same way.

**Verdict: Blocked on server work.** `/compact` can be covered today. Automatic compaction needs warp-server to emit a pre-summarization event carrying the metrics; because the hook is observational, that event does not need a round trip.

### 3.16 `stop`

**Purpose and timing.** Fires when the agent loop ends; may auto-submit a follow-up. (Report 01 §3.16.)

**Warp moment.** `hold_or_finalize_turn` in `controller.rs`, which replaces the two places that set `ConversationStatus::Success` at the end of a turn (the `Done` arm of the stream-finished handler, where `actions_to_queue` is found empty, and the `FinishedAction` subscription when the last action finishes with no follow-up); the dispatch starts once the turn's own stream has been cleaned up (`app/src/ai/blocklist/hooks/turn.rs`). Only a completed turn fires it in Phase 1; a cancelled or failed turn does not (report 03 §3j for the signals).

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `status` | Available with work | `completed` = stream `Done` with no actions, or the last action finished with no follow-up. `aborted` and `error` are defined in the payload but never sent in Phase 1, since a cancelled or failed turn is not a stop event. `TransientError` with a scheduled auto-resume is not terminal and does not fire. `WaitingForEvents` is not a stop. |
| `loop_count` | Available now | Hook-runner state. |

**Output fields.**

| Field | Status | Mechanism |
| --- | --- | --- |
| `followup_message` | Supported | Appended to `QueuedQueryModel` by conversation id with `HookFollowUp` provenance and a `QueuedQueryOrigin::HookFollowUp` origin, sent through the normal prompt path (`dispatch_queued_warp_agent_prompt`, then `send_query`), so it passes the prompt-submit hook (section 3.14), gets its own turn id, is sent as written (no slash command or mode prefix is read from it), and queues behind any real user input. Honored only when `status` is `completed`; this is Warp's choice, since Cursor's `stop` docs state no status condition while its `subagentStop` docs do. Loop protection: per-binding counts within one continuation chain (`loop_limit`, default 5; `null` or more than 8 becomes 8) plus a host cap of 8 for the whole chain across all bindings, the same cap Claude Code documents; a refused follow-up ends the turn with a warning diagnostic naming the limit. Verdicts and `halt_turn` on this event are ignored with a diagnostic. |

**Blocking and modification.** Status finalization is held until the stop hooks answer: the conversation stays `InProgress` while they run, and `Success` is set only when they answered with no follow-up, when the follow-up could not be sent, or when the hold timed out (the longest matching binding's timeout, default 60 s; a late answer is discarded). A person's prompt during the hold takes the status and the follow-up queues behind it; the follow-up is still queued rather than dropped (a conservative choice awaiting David's decision). Cancelling the conversation drops the hold.

**Surface notes.** Oz: `Success` starts the idle-timeout exit (`driver.rs`); because the status is held, the idle exit cannot race a follow-up, and a queued follow-up keeps the conversation `InProgress` through its own request. A follow-up that cannot be sent yet stays queued with a warning and the turn is finalized; on an Oz run the queued row keeps the idle exit from firing. Local children: a stop hook on a child fires inside the parent process; make scope explicit in config.

**Verdict: Full.**

### 3.17 `afterAgentResponse`

**Purpose and timing.** Fires after the agent completes an assistant message. No documented output. (Report 01 §3.17.)

**Warp moment.** Streaming text arrives via `AppendToMessageContent` into `AIAgentOutputMessageType::Text` (`convert_from.rs`, `conversation.rs`). There is no per-message completion event; the boundaries available are a new message id after a text message, or stream end (report 03 §3k). The Oz driver already treats the exchange as the boundary (`write_exchange_output`).

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `text` | Available with work | Final text of each `Text` output message, emitted when the exchange's stream finishes (or when the next message starts). |

**Output fields.** None documented.

**Blocking and modification.** Observe-only. Text is replayed to the server as history next turn, so client-side rewriting would desync (report 03 §3k).

**Surface notes.** Shared.

**Verdict: Full** (observe-only; the boundary is a heuristic, which Cursor also does not specify).

### 3.18 `afterAgentThought`

**Purpose and timing.** Fires after a thinking block completes. No supported output. (Report 01 §3.18.)

**Warp moment.** `AIAgentOutputMessageType::Reasoning { text, finished_duration }`; `finished_duration` becomes `Some` on the final update (`convert_from.rs`).

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `text` | Available now | Aggregated reasoning text, as the server sends it (may be provider-summarized). |
| `duration_ms` | Available now | `finished_duration`. |

**Output fields.** None supported.

**Blocking and modification.** Observe-only.

**Surface notes.** Shared.

**Verdict: Full.**

### Tab hooks

### 3.19 `beforeTabFileRead`

**Purpose and timing.** Fires before Cursor Tab inline completions read a file. IDE-only. (Report 01 §3.19.)

**Warp moment.** None. Warp Code (`app/src/code/`) has LSP and git diff support but no AI inline completion; report 03 §3l searched for it and found nothing. Terminal input prediction (`app/src/ai/predict/`) is a different surface.

**Input and output fields.** `file_path`, `content`; output `permission`. Not applicable.

**Verdict: Not applicable.**

### 3.20 `afterTabFileEdit`

**Purpose and timing.** Fires after Tab inline completions edit a file. IDE-only. (Report 01 §3.20.)

**Warp moment.** None, as above. If editor-driven hooks are ever wanted, `FileModel` (`crates/warp_files`) is the common write funnel for the GUI editor, TUI diff storage, and agent edits (report 03, inferred from call sites).

**Verdict: Not applicable.**

### App lifecycle

### 3.21 `workspaceOpen`

**Purpose and timing.** Fires once when Cursor opens a workspace and on every workspace-folder change, independent of any agent session. Reduced envelope (`hook_event_name`, `cursor_version`, `workspace_roots`, `user_email`). The only output, `pluginPaths`, adds Cursor plugin directories. (Report 01 §3.21.)

**Warp moment.** Warp has no project-open concept. The nearest signal is a new repo root being detected for a session (`DetectedRepositoriesEvent::DetectedGitRepo`, triggered on cwd change in GUI and TUI and on environment prep in Oz; report 04 §1.1). Codebase index registration and Oz environment-ready are alternatives (report 03 §3m).

**Input fields.**

| Field | Status | Warp source or work needed |
| --- | --- | --- |
| `hook_event_name`, `cursor_version`, `user_email` | Available now | |
| `workspace_roots` | Available with work | The detected repo root(s). |

**Output fields.**

| Field | Status | Mechanism |
| --- | --- | --- |
| `pluginPaths` | Not supportable | Warp has no Cursor-plugin loader. |

**Blocking and modification.** Not applicable.

**Surface notes.** Firing on repo detection would make this hook run every time a user `cd`s into a new repo in any tab, which is much more often than Cursor's workspace open. Oz environment-ready is a better single moment for cloud runs.

**Verdict: Partial.** The inputs can be supplied by defining a Warp moment, but the only output has no meaning in Warp. Recommend deferring this hook.

## 4. Cross-cutting coverage

### 4.1 Config discovery and locations

**What Warp has today.** Nothing reads `.cursor/hooks.json`, `.claude/settings.json` `hooks`, or Codex hook config (report 04 §1.6, verified by grep). But the surrounding machinery exists:

- Project root. `DetectedRepositories::detect_possible_local_git_repo` (`crates/repo_metadata/src/repositories.rs`) walks up from the session cwd to a `.git` directory or gitfile and **stops at `$HOME`**. It runs on every cwd change in the GUI and TUI and on environment prep in Oz. This is the only project-root notion Warp has. Consequence: a cwd under `$HOME` that is not inside a git repo has no project root, so `.cursor/hooks.json` in such a directory would never be found. Cursor's "workspace folder" has no such rule.
- Per-provider config files at home and repo root. `MCPProvider::{home_config_path, project_config_path}` plus `FileMCPWatcher` and `FileBasedMCPManager` (`app/src/ai/mcp/`) watch `.warp/.mcp.json`, `~/.claude.json` / `.mcp.json`, `~/.codex/config.toml`, and `.agents/.mcp.json`, parse with diagnostics, re-parse on change, and gate the first agent turn on the initial scan. This is the closest template for a hooks loader (report 04 §1.3, §1.8). `MCPProvider` does not include Cursor today.
- Skill providers already know `.cursor/skills`, `.claude/skills`, `.codex/skills`, and `.agents/skills` at home and project scope (`SKILL_PROVIDER_DEFINITIONS`, `crates/ai/src/skills/skill_provider.rs`).
- Home directories. GUI config lives under `~/.warp*`; TUI under `~/.warp_cli*` (`crates/warp_core/src/paths.rs`). A hooks file in a Warp-owned home location would need to pick one or be shared.
- Cloud agents. Oz runs register cloned repos through `register_cloned_repo` and widen skill scope with `SkillManager::set_cloud_environment(true)`; extra inputs arrive through `WARP_SKILL_DIRS` and `AgentConfigSnapshotFile`, which has no hooks key (report 04 §1.7). The run's `$HOME` is the container's, so a `~/.cursor/hooks.json` is absent unless the environment setup writes one.
- SSH and WSL sessions. Repo config is read through `RepoMetadataModel` snapshots; `FileMCPWatcher` is local-only; nothing runs commands on the remote host for config purposes (report 04 §1.7). Proposal: hooks do not run on the local host against remote or WSL paths, and these sessions are unsupported at first (see section 7).

**Options for which Cursor locations Warp reads.**

Option 1: Cursor's own files only (`<repo>/.cursor/hooks.json`, `~/.cursor/hooks.json`).
- For: zero-config for teams already using Cursor hooks; the partner integrations Cursor lists (Semgrep, Snyk, 1Password, Endor Labs, MintMCP) work immediately.
- Against: scripts written for Cursor assume Cursor env vars, Cursor tool names, and Cursor-only events; a user who runs both tools on one repo gets double execution; Warp cannot extend the schema (Warp-only tools and events) without colliding with Cursor's file; running project scripts a user configured for a different product is a trust surprise.

Option 2: a Warp-owned location using the Cursor schema (`<repo>/.warp/hooks.json`, `~/.warp/hooks.json`).
- For: explicit opt-in; room for Warp extensions; matches `.warp/.mcp.json` and `.warp/skills`.
- Against: no instant compatibility; users copy files by hand.

Option 3: both, with Warp-owned files higher in priority and a setting that disables third-party import.
- For: compatibility and a clean extension point; mirrors Cursor's own third-party-import toggle and Warp's MCP provider pattern; later Claude and Codex sources slot in as more providers.
- Against: two files can disagree; the merge rules must be explained.

**Settled recommendation (see `architecture.md` §7).** Phase 1 is cloud and CLI only and reads three kinds of source: the project `.cursor/hooks.json` at each detected git root (the git root for the session cwd, found by `DetectedRepositories`, which stops at `$HOME`; the set for a tool call is the one whose root is the deepest known root containing the action's cwd, so a nested repository is its own project even without a hook file, and a cwd outside any repo has no project source), activated under the trust policy in section 4.9; explicit sources delivered with the run, a `hooks` entry in the `warp agent run` config file, the `WARP_HOOK_DIRS` input modeled on `WARP_SKILL_DIRS`, and the repeatable `--hook-dir` flag (a config-file entry whose file is inside a git repository follows the project rule rather than being approved by construction); and factory agent definitions (`hooks/hooks.json` at the factory root and `agents/<name>/hooks/hooks.json` per agent, in Cursor format, delivered by warp-server as explicit sources). The factory delivery is designed but not built: cloud workers pass no `--config`, so the sources would arrive through `ExecutionConfiguration`, and the full proposal with its open sub-decisions is in `factory-hooks-proposal.md` in the implementation run's directory on David's machine (`/Users/david/.agent-scratch/agent-hooks-impl/`, not in any repository), awaiting David's approval. For the same reason a remote orchestration child receives none of its parent's explicit sources in Phase 1 (`SpawnAgentRequest` has no field for them): a strict run with explicit sources refuses the launch and any other run warns the parent that the child runs only its own repository's hooks (`architecture.md` §8). Phase 1.5 adds the user file `~/.cursor/hooks.json` behind an explicit setting, with a first-run discovery prompt on the GUI and TUI when a project or user file is found and not yet active. Third-party import is **not** on by default for the user file; this is a deliberate difference from Warp's MCP and skill imports, because hooks run arbitrary code. No Warp-owned hooks file is planned; it is a possible future addition if repository-scoped Warp-only policy is requested. Relative `command` paths in project files resolve against the project root; in user files, against the config file's directory; in factory definitions and other run-delivered sources, against the declaring directory. Because of that last rule, pointing `WARP_HOOK_DIRS` or `--hook-dir` at a repository's own `.cursor` directory runs the file once, as the run-delivered copy, from `.cursor` rather than the project root, so root-relative commands are not found and fail open. That is the one spelling available today for turning a repository's own hooks on under a profile that asks, since Phase 1 ships no dedicated flag; the open question for David is whether to keep project semantics for an explicitly activated root `.cursor/hooks.json` or to add the flag and stop recommending the spelling (`architecture.md` §7.3). Until then a delivered directory should hold its own `hooks.json` with commands relative to that directory. Hooks are deduplicated by resolved source identity (the canonical path), never by command text. When a hook was loaded from a Cursor-format source, the `CURSOR_*` environment aliases are also set (section 4.4).

Enterprise and team scopes: Cursor's MDM file and dashboard distribution have no Warp equivalent. Warp has team-level AI autonomy settings (`UserWorkspaces::ai_autonomy_settings`), so a team-distributed hooks object is plausible later but needs warp-server. Out of scope for a first version.

### 4.2 Matcher semantics

Cursor documents a regex string with `""` and `"*"` meaning match-all, and no flavor, anchoring, or case rule (report 01 §1.4). Claude Code and Codex both treat a restricted character set as exact names and anything else as an unanchored regex (report 02). Recommendation: the Rust `regex` crate, unanchored search, case-sensitive, with `""` and `"*"` special-cased (a bare `*` is not a valid regex). This is a judgment call; it is compatible with every documented Cursor example.

Match targets per event follow report 01 §1.4. For the three tool events the target is the tool name, which requires a mapping from `AIAgentActionType`:

| Warp action | Cursor `tool_name` |
| --- | --- |
| `RequestCommandOutput` | `Shell` |
| `ReadFiles` | `Read` |
| `RequestFileEdits` | `Write` (or `Delete` when every edit is a delete; judgment call) |
| `Grep` | `Grep` |
| `RunAgents` | `Task` |
| `CallMCPTool { name }` | `MCP:<name>` |

| Warp action | Proposed Warp-only `tool_name` |
| --- | --- |
| `FileGlob`, `FileGlobV2` | `Glob` (Cursor has no Glob; its Claude layer marks it unsupported) |
| `SearchCodebase` | `SearchCodebase` |
| `ReadMCPResource` | `MCPResource:<name>` |
| `WriteToLongRunningShellCommand`, `ReadShellCommandOutput`, `TransferShellCommandControlToUser` | `ShellWrite`, `ShellRead`, `ShellHandoff` |
| `ReadSkill`, `UseComputer`, `RequestComputerUse`, `StartRecording`, `StopRecording`, `ReadDocuments`, `EditDocuments`, `CreateDocuments`, `AskUserQuestion`, `SendMessageToAgent`, `FetchConversation`, `WaitForEvents`, `UploadArtifact`, `SuggestNewConversation`, `SuggestPrompt`, `InitProject`, `OpenCodeReview`, `InsertCodeReviewComments` | PascalCase variant name |

A `"*"` matcher on `preToolUse` will fire for the Warp-only names. Hook authors who wrote for Cursor will see unfamiliar tool names and must allow by default. Cursor's `WebSearch`/`WebFetch` never appear because those tools run on warp-server.

For `subagentStart`/`subagentStop`, Cursor matches on `generalPurpose`/`explore`/`shell`. Warp would match on its own type string (section 3.6).

### 4.3 Timeouts and process lifecycle

Cursor publishes no default timeout. Claude Code defaults to 600 s for command hooks; Codex to 600 s with 1 s for `SessionEnd` and `Interrupt` (report 02). Defaults, all configurable per hook via `timeout` up to a ceiling of 600 s (a definition above it is skipped with an error): 30 s for pre-execution events (`preToolUse`, `beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile`, `subagentStart`, `beforeSubmitPrompt`), 60 s for post-tool and all other events, 5 s for `sessionEnd`. Judgment call; see section 7.

Warp has no shared subprocess timeout helper (report 04 §3.1). The pattern to copy is the LSP transport (`crates/lsp/src/transport.rs`): `select!` against a `Timer`, then SIGTERM, grace, SIGKILL. Spawn through `command::r#async::Command` (`crates/command/src/async.rs`), the only sanctioned API (clippy bans raw `std::process::Command`), using `new_with_process_group` so a timeout kills the hook's children too. Pipe stdin, stdout, and stderr; write the JSON, close stdin, read stdout to EOF, drain stderr to the hook log. The closest existing "JSON over stdio" code is the LSP transport, but it is framed and long-lived, so a small one-shot helper is new work.

Matching hooks run concurrently (Cursor's CLI does; Claude and Codex do), at most four at a time across the process, and merge in a deterministic order: source priority, then the adapter's declared order (Cursor's event order, then array index). Failure policy follows report 01 §2.2: exit 2 is deny; other non-zero, crash, or timeout is fail-open unless `failClosed`; invalid JSON or schema mismatch is deny on the six permission hooks regardless of `failClosed`. Whatever is left of a hook's process tree is ended on every path, including after a normal exit (stop request, two seconds of grace, kill), so a hook gets no background work; stdout is capped at 1 MiB and a cut-off response is malformed output. `architecture.md` §9.1 has the full runner contract and the environment policy.

Windows: `new_with_process_group` is unix-only (report 04 §3.1), so a process group alone cannot clean up a timed-out hook's descendants there. The first phase adds per-child kill-on-close Job Objects to the `command` crate (the child is created without `CREATE_BREAKAWAY_FROM_JOB` and assigned to a nested job; one that cannot be assigned does not run), so Windows is supported in Phase 1 with the same cleanup guarantee as Unix. The command string runs under `cmd.exe /S /C`; the quoting, Unicode stdin, and no-orphan tests exist behind `cfg(windows)` and run first in CI.

wasm: the hook runner must be `cfg`-gated or stubbed, as `command::r#async`, `watcher`, and `local_fs` code are (report 04 §4.3).

### 4.4 Environment variables

Cursor sets `CURSOR_PROJECT_DIR`, `CURSOR_VERSION`, `CURSOR_USER_EMAIL` (if logged in), `CURSOR_TRANSCRIPT_PATH` (if transcripts enabled), `CURSOR_CODE_REMOTE` (`"true"` for remote workspaces), and `CLAUDE_PROJECT_DIR` as a compatibility alias (report 01 §1.5). Warp can provide:

| Cursor variable | Warp source | Status |
| --- | --- | --- |
| `CURSOR_PROJECT_DIR` | Detected git root or cwd (section 4.1) | Available with work |
| `CURSOR_VERSION` | `app_version()` | Available now |
| `CURSOR_USER_EMAIL` | `AuthState::user_email()` | Available now |
| `CURSOR_TRANSCRIPT_PATH` | None until a transcript export exists | Not available (omit, which Cursor allows) |
| `CURSOR_CODE_REMOTE` | Oz cloud run (`is_autonomous()`), or an SSH-backed session (`SessionContext::is_remote`) | Available now |
| `CLAUDE_PROJECT_DIR` | Same as project dir | Available with work |

As built: the `WARP_*` names (`WARP_PROJECT_DIR`, `WARP_VERSION`, `WARP_HOOK_PROTOCOL_VERSION`, `WARP_RUN_ID`, `WARP_USER_EMAIL`, `WARP_CODE_REMOTE`) are always set, and the `CURSOR_*` and `CLAUDE_PROJECT_DIR` aliases are set when the hook came from a Cursor-format source, the same way Cursor sets `CLAUDE_PROJECT_DIR` for Claude compatibility; `WARP_CODE_REMOTE` and `CURSOR_CODE_REMOTE` are `"true"` for an Oz run only. Precedent for advertising version to child processes: `WARP_CLIENT_VERSION` and `WARP_CLI_AGENT_PROTOCOL_VERSION` in `crates/warp_core/src/cli_agent_protocol.rs`. The envelope field `cursor_version` carries Warp's version string (the schema has no other slot), and `warp_version` sits beside it.

Inherited environment: Cursor does not document what else a hook inherits. Warp's only login-shell capture is PATH (`LocalShellState`, `app/src/terminal/local_shell/mod.rs`); MCP servers inherit Warp's process env plus PATH (report 04 §3.2, §3.3). Hooks are spawned with Warp's process env under a scrub list (every Warp-owned credential Warp reads from its own environment, the `WARP_METADATA_*` names, and the session identity and shell-bootstrap switches an enclosing Warp terminal exports), the captured PATH, `HOME`, any `sessionStart.env` values, and then the variables above, which the session environment cannot replace. Third-party credentials a worker put in Warp's process are passed through on purpose, since the partner hooks Cursor advertises need them. The exact list is in `architecture.md` §9.1 and is a conservative choice awaiting David's decision. Running each hook through `execute_local_command_in_login_shell` would give the full shell env at the cost of sourcing rc files per call; not done.

### 4.5 Working directory rules

Cursor runs project hooks in the project root, user hooks in `~/.cursor/`, and enterprise hooks in the enterprise config directory; relative `command` paths resolve against that directory (report 01 §1.2). Warp can match this per source: project hooks in the detected git root, user hooks in the directory that holds the config file (`~/.cursor/`), and run-delivered and factory sources in their declaring directory. The `$HOME` stop rule in `find_git_repo` is the one behavioral difference and should be documented.

### 4.6 Source-priority merging

Cursor: all matching hooks run; `deny` over `ask` over `allow`; `user_message`/`agent_message` concatenate; for other fields the two Cursor pages disagree on whether higher- or lower-priority sources win (report 01 §1.2, §7 item 4). Recommendation: implement severity merge and concatenation as documented, and for scalar fields let the **higher-priority** source win. This is the safer reading (enterprise and team policy should not be overridden by a user file) and matches the third-party page. It deviates from the main Hooks page's "last response wins" sentence; flag this in user docs.

For `preToolUse` and `beforeShellExecution` both matching one shell command, run both and merge the same way; Cursor does not order them. Duplicate detection across sources uses the resolved source identity (file, scope, position in the array), never the command text; two files that happen to name the same script are two hooks.

### 4.7 Interaction with Warp's permission settings

Today's decision is a single bool from the per-tool `should_autoexecute` (`execute.rs:975-1073`), composed at `execute.rs:624-626` with `is_user_initiated` and autonomous mode. Shell commands go through `BlocklistAIPermissions::can_autoexecute_command` (`app/src/ai/blocklist/permissions.rs`), whose precedence is: org denylist, then user denylist, then per-conversation auto-approve, then the profile's `AgentDecides`/`AlwaysAllow`/`AlwaysAsk` rule with allowlist matches winning inside `AlwaysAsk` (report 03 §4.2). File writes have protected paths (MCP config files) that always require approval. MCP tools use allowlists and denylists per server. `architecture.md` §6.5 names one component, `HookPermissionPolicy` in `crates/ai/src/hooks`, as the only place a hook verdict is combined with that result; the options below are the policies it could encode.

Options for where a hook decision sits:

Option A: hooks can only tighten. `deny` and `ask` are honored; `allow` is ignored and Warp's own check still applies.
- For: simplest and safest; no way for a hook to widen access.
- Against: a hook that says `allow` for a known-safe command still produces a confirmation card under `AlwaysAsk`, which defeats a common use (auto-approving vetted commands).

Option B: hooks decide first; `allow` short-circuits to execute, `deny` blocks, `ask` forces the card, no decision falls through to Warp's check.
- For: matches how Cursor users think about hooks as policy.
- Against: a project hook could bypass a user's `AlwaysAsk` setting and the user denylist; dangerous without a trust decision in front of it.

Option C: map hook decisions onto Warp's existing tiers.
- Org denylist and protected write paths remain absolute and are evaluated first.
- Hook `deny` behaves like a user-tier denylist hit: blocked, with the user told why (`user_message`) and the model told why inside the denied result where the result type has a text slot (not shell; section 3.0). This runs before per-conversation auto-approve.
- Hook `ask` forces `NeedsConfirmation`, even under auto-approve.
- Hook `allow` behaves like a dynamic allowlist entry: it permits auto-execution the way an allowlist match does today; it does not override a denylist hit.
- No decision (hook absent, or output without `permission`) leaves Warp's check untouched.
- For: reuses denylist, allowlist, and confirmation; composes at one point between `execute.rs:618` and `:626`.
- Against: an allowlist match beats `AlwaysAsk` today, so a hook `allow` would too; a project file could then silence a user's explicit "always ask".

**Settled recommendation (see `architecture.md` §6 for the reasoning).** Two phases, both implemented inside `HookPermissionPolicy` so the rule changes in one place.

- First phase: tightening only (Option A). `deny` and `ask` are honored; `allow` is accepted and ignored; Warp's own permission check always runs.
- Later phase: bounded loosening. A hook `allow` may satisfy only the confirmation Warp would otherwise show under `ActionPermission::AgentDecides`. It never satisfies `AlwaysAsk`, never passes an org or user denylist hit, never passes a protected write path, and is honored only for sources that were granted a separate "may approve" capability, distinct from trust-to-run. This matches Claude Code's documented behavior, where an `allow` from a hook "skips the permission prompt" while "deny and ask rules are still evaluated".
- Headless (Oz runs, `warp agent run`, local hidden children, decided per conversation): `ask` degrades to `deny` with the reason `Approval unavailable: an agent hook asked for confirmation (<hook reason>) and this run has no one to ask.`, recorded in the notice and delivered inside the tool result where the family has a slot. Report 05 records the precedents: Cursor's docs do not define hook `ask` with no user present (managed cloud, self-hosted workers, or `agent -p`) and its cloud agents auto-run commands without per-step approval; Claude Code documents that in a `-p` run the model "reads the reason in the tool result"; Codex treats `PreToolUse` `ask` as a failed hook and proceeds. For shell the slot is `PermissionDenied.hook_denied`; for the no-text-slot tools there is none yet (section 3.0). The run flag `--hooks-on-ask fail-run` selects fail-the-run instead: the action is still denied, and the driver fails the conversation at once with the action and the hook's reason in the message, so the task ends FAILED through the normal error path.
- A user who has already approved an action (`is_user_initiated == true`) on an interactive surface is not asked again by a hook `ask` for that exact revision, but a hook `deny` still blocks, and the approval does not carry over to a check restarted because the command, working directory, or hook configuration changed. On a headless surface an approval mark can only come from an automated re-entry, so a hook `ask` denies there regardless (a conservative choice awaiting David's decision).

One qualification from the architecture cross-critique: Warp's permission floors are not one uniform layer today, so "floors stay absolute" is new plumbing rather than a free property. `is_user_initiated` bypasses `should_autoexecute` entirely in `try_to_execute_action` (`execute.rs:624`); the `auto_approve_bypasses_command_denylist` setting lets per-conversation auto-approve skip the user denylist (`permissions.rs`, report 03 §4.2); and protected write paths are enforced only outside sandbox mode (`can_write_files`, report 03 §4.2). A hook layer that promises never to pass those floors has to make them explicit first.

### 4.8 Prompt-based hooks

Warp's client never calls a model directly; every auxiliary call is a warp-server endpoint with server-side model choice (report 04 §6). Supporting `type: "prompt"` therefore needs:

- Server: a new endpoint (for example `/ai/evaluate_hook_prompt`) that takes the prompt text, the hook input JSON, and an optional model override, substitutes `$ARGUMENTS` (or appends the JSON), and returns `{ ok, reason }`. Model default should be a fast model chosen server-side, as Cursor describes. Quota and billing treatment is an open question.
- Client: a `PromptHookExecutor` beside the `CommandHookExecutor`, both behind one executor trait so the runner does not care which ran. The client applies the timeout and maps `ok` onto the event's native output. Cursor does not document that mapping; this report assumes `ok: true` means allow or continue, and `ok: false` means deny or `continue: false` with `reason` used as both `user_message` and `agent_message`. Which events accept prompt hooks is also undocumented; the safe assumption is "any event with a permission or continue output".

Because the client is always authenticated, prompt hooks can run in Oz cloud runs and local children as well as the GUI and TUI, which Cursor cannot offer. The architecture should keep prompt hooks as a second executor from day one even if the endpoint ships later.

### 4.9 Workspace trust

Cursor runs project hooks only in a trusted workspace; Claude Code and Codex gate project hooks on folder trust too (report 02). Warp has no workspace or folder trust concept for its own agent: grep for trust across `app/src`, `crates/ai`, and `crates/repo_metadata` finds only the code that writes trust settings into third-party harness configs in cloud environments (`driver/harness/codex.rs`, `driver/harness/gemini.rs`). Project rules, skills, and file-based MCP servers are all picked up from a repo without a prompt, and no sandboxing or risk classification applies to processes Warp spawns for itself (report 04 §3.5).

Hooks are different in kind: a hooks file causes arbitrary scripts from the repo to run with the user's privileges on every tool call. The settled design (see `architecture.md` §8):

- Location never grants permission to execute code. A `TrustPolicy` trait decides activation; its Phase 1 implementation is a static table keyed by surface, source kind, and the execution profile's command-execution posture.
- The posture rule: project hooks activate automatically when the run's execution profile already auto-approves command execution, which the Oz default profile does (`create_default_cli_profile`, all `AlwaysAllow`). The posture is the profile's execute-commands setting read through the team policy of every open window, strictest answer wins: `AlwaysAllow` auto-approves; `AlwaysAsk` and `AgentDecides` require approval; the per-conversation auto-approve toggle does not count. Otherwise project hooks need an explicit flag or configuration entry for the run; Phase 1 ships no dedicated flag, and the available spelling (`WARP_HOOK_DIRS` or `--hook-dir` pointing at the repository's `.cursor` directory) runs the file from `.cursor` rather than the root, so how a run under a profile that asks should turn a repository's own hooks on is an open question for David (section 4.1, `architecture.md` §7.3). Explicit sources delivered with the run are approved by construction, except a config-file entry whose file is inside a git repository, which follows the project rule. The reasoning: in an all-`AlwaysAllow` run the agent can already run any repo script on its own, so the marginal risks are guaranteed execution with full payload visibility (accepted and made visible), agent-authored hooks (closed by pinning and protected paths), and restrictive-profile runs (closed by the posture rule).
- The active hook set, and the set of project roots, is pinned when the run's first request is prepared (on Oz and CLI runs, at `execute_run`); project files are inert until then, because a command-line run applies `--profile` after the terminal has navigated into its repository. After pinning the set neither grows nor shrinks: changed, newly written, or removed files, and newly detected roots, do not take effect mid-run and are reported as diagnostics. Those diagnostic-only changes move the snapshot's `generation` but not its `revision`, so they do not disturb in-flight pre-tool verdicts.
- Hook config files and the scripts they reference become protected write paths in `can_write_files`, so the agent cannot edit its own hooks through `RequestFileEdits`; this covers every loaded source, active or inert, and applies in sandboxed runs. Protected files are matched by resolved absolute path: the file-edit executor resolves the model's paths against the session's working directory before the check, since a relative `.cursor/hooks.json` could otherwise slip past it. Two callers outside the hook stack (the ACP harness policy and a relative path before the session reports a working directory) still reach the check unresolved; the recommended follow-up is a question for David (`architecture.md` §8). Before this, protected paths covered only MCP config files (report 03 §4.2).
- Interactive surfaces (Phase 1.5): a per-repo-root review card listing events, matchers, and commands, keyed by repo root, execution host, resolved source identity, and config revision; re-approval on change; approvals stored locally. The "may approve" grant is a separate decision from trust-to-run.
- A malformed reload keeps the last valid snapshot active and marks it stale in diagnostics, rather than silently dropping hooks or running a half-parsed file.

### 4.10 Observability

Cursor exposes a Hooks tab and an output channel listing configured and executed hooks and errors; it does not say where stderr goes (report 01 §7). For Warp, Phase 1 keeps one `log::` line per hook invocation from the runner (event, source, binding, exit code, duration, escaped stderr tail; never the payload) and one summary line per dispatch; in Oz and CLI runs notices and diagnostics are also written to the run's stdout as `hook_notice` and `hook_diagnostic` records. Telemetry, a diagnostics ring, and any panel move to Phase 2 (`architecture.md` §3). The full picture:

- Logs: `log::` with `conversation_id`, `action_id`, source path, exit code, and duration, plus a per-hook stderr capture, in the existing state-dir log files. MCP servers already forward stderr line by line to a logger (`crates/mcp/src/runtime.rs`); copy that.
- Config errors: a diagnostic type modeled on `FileMCPConfigDiagnostic` surfaced as a toast (GUI), a hint (TUI), and a stderr line (Oz).
- Hook denials and failures during a turn: the same message channel as `user_message` (section 3.0), with the hook's source path included so the user knows which file acted.
- Telemetry: a feature-local event (`HookExecuted { event, source_scope, format, outcome, duration_ms, timed_out }`) registered with `register_telemetry_event!`, joined on `AIIdentifiers` (report 04 §5, report 03 §5.3).
- A Hooks settings page listing discovered files, trust state, and recent executions is desirable but not required for correctness.

### 4.11 Surfaces: confirm or challenge

The user's hypothesis is that GUI, TUI, Oz cloud runs, and orchestration children differ mainly in which configs load from where. Reports 03 §1.6 and 04 §1.7 **confirm this for parsing and execution**: all four run the same `BlocklistAIController`, `BlocklistAIActionModel`, executors, and permission model, and the same `initialize_app` registers the same discovery singletons. The hook runtime is written once in `crates/ai/src/hooks`, with the interception points in `app/src/ai/blocklist`, and reaches every surface. Phase 1 enables only Oz cloud runs and `warp agent run` (which includes remote orchestration children); the GUI and TUI follow in Phase 1.5.

Three places where the hypothesis needs qualification:

1. **Remote orchestration children.** The parent's process holds only a `ConversationDriver::RemoteChild` placeholder; the child's tool calls happen on another machine. Hooks for that child must run in the child's own Oz runtime, which loads config from the cloned repo. The parent can still fire `subagentStart`/`subagentStop` for the launch and the result. Third-party harness children (`ExternalHarness`) execute no native tools at all, so native hooks do not apply to them.
2. **Headless `ask`.** Oz runs, `warp agent run`, and local hidden children have no approver. Today an unapprovable action produces a `Denylisted` result (shell) or a `Blocked` status that fails the run. Hook `ask` degrades to `deny` there, with the reason recorded in the notice and delivered to the model in the tool result (section 4.7, section 3.0; the shell slot is the Phase 1 proto change). Report 05 shows no vendor defines this case better: Cursor's docs are silent, Claude Code documents deny-with-reason for `-p`, and Codex treats `ask` as a failed hook. This is a behavior difference, not a config difference. Project hooks in headless runs activate under the posture rule in section 4.9, and an Oz run's initial prompt is not visible to `beforeSubmitPrompt` (section 3.14).
3. **Message delivery and the Tui/Gui config split.** `user_message` goes to a toast, a TUI hint, or stdout depending on surface. Home config directories differ between GUI (`~/.warp*`) and TUI (`~/.warp_cli*`), and `FileMCPWatcher` ignores `CloudEnvironmentPrep` in TUI mode (report 04 §1.7); a hooks watcher must decide the same matrix explicitly.

## 5. Summary matrix

### 5.1 Verdicts

| Hook | Cursor category | Warp verdict | Main gap |
| --- | --- | --- | --- |
| `sessionStart` | Agent | Partial | `additional_context` needs a `hook_context` field on the request proto plus server rendering; `env` and inputs are client-only |
| `sessionEnd` | Agent | Partial | No session-end moment in GUI/TUI; must be defined |
| `preToolUse` | Agent | Partial | Deny reason reaches the model for `Shell` (via `hook_denied`, built in Phase 1) and every family with a text slot; the no-text-slot tools (`ShellWrite`, `ShellRead`, `ShellHandoff`, `SuggestPrompt`, `WaitForEvents`, `InitProject`, `OpenCodeReview`) still need a denied variant (server) |
| `postToolUse` | Agent | Partial | `additional_context` needs a `hook_context` field on `ToolCallResult` plus server rendering; encoder and timer are client work |
| `postToolUseFailure` | Agent | Partial | Its only output, `additional_context`, needs the same server work; event and inputs are client-only |
| `subagentStart` | Agent | Partial | No `subagent_id` before launch; server-side subagents invisible |
| `subagentStop` | Agent | Partial | No counts for remote children; server-side subagents invisible |
| `beforeShellExecution` | Agent | Full as built (Partial as scoped) | The deny reason's text slot, `PermissionDenied.hook_denied`, shipped in Phase 1 with server rendering |
| `afterShellExecution` | Agent | Full | None significant |
| `beforeMCPExecution` | Agent | Full | Transport lookup for `url`/`command` |
| `afterMCPExecution` | Agent | Full | None significant |
| `beforeReadFile` | Agent | Full | Hook calls on four executors (`ReadFiles`, `ReadSkill`, `GetFiles`, `SearchCodebase` after the search runs); client work |
| `afterFileEdit` | Agent | Full | Edit-string derivation when the user edited the diff |
| `beforeSubmitPrompt` | Agent | Full | An Oz run's initial prompt is resolved server-side and cannot be seen without a server change; a hook `ask` halts the prompt because no confirmation step exists for one |
| `preCompact` | Agent | Blocked on server | Server must signal automatic compaction |
| `stop` | Agent | Full | Finalization is held while the hooks run, bounded by their timeout; only a completed turn fires it |
| `afterAgentResponse` | Agent | Full | Message-boundary heuristic |
| `afterAgentThought` | Agent | Full | None |
| `beforeTabFileRead` | Tab | Not applicable | No inline completion |
| `afterTabFileEdit` | Tab | Not applicable | No inline completion |
| `workspaceOpen` | App lifecycle | Partial | `pluginPaths` has no meaning; moment must be defined |

### 5.2 Blocking, modification, and seams

| Hook | Blockable in Warp? | Modifiable in Warp? | Primary Warp seam |
| --- | --- | --- | --- |
| `sessionStart` | No (not needed) | Context, env | The barrier in `prepare_turn` before the conversation's first request |
| `sessionEnd` | Oz only | No | `AgentDriver::cleanup`; conversation delete/close |
| `preToolUse` | With work | Owned action; rewrites in a later phase | `BlocklistAIActionExecutor::try_to_execute_action` |
| `postToolUse` | Yes | Yes (owned result); context needs server | `execute_future` wrapper in `execute.rs` |
| `postToolUseFailure` | Yes | Context only (needs server) | Same |
| `subagentStart` | Yes | Owned request; rewrites in a later phase | `RunAgentsExecutor` / `StartAgentExecutor::dispatch` |
| `subagentStop` | Short deferral | Follow-up only | `inject_pending_events_for_request` |
| `beforeShellExecution` | With work | Command string; rewrites in a later phase | `ShellCommandExecutor::execute` (pre-emit) |
| `afterShellExecution` | Yes (on real exit) | Output | The completion funnel; for interact mode, the later poll that reports `CommandFinished` or `ShellRecovered` |
| `beforeMCPExecution` | Yes | Arguments; rewrites in a later phase | `CallMCPToolExecutor::execute` |
| `afterMCPExecution` | Yes | Result | `CallMCPToolExecutor` `on_complete` |
| `beforeReadFile` | Yes | Content (redact) | `ReadFilesExecutor`, `ReadSkillExecutor`, `GetFilesExecutor` `on_complete`; `SearchCodebase` results after the search |
| `afterFileEdit` | Yes | Pre-write only | `RequestFileEditsExecutor::execute` / `accept_and_save` |
| `beforeSubmitPrompt` | Yes | Query string | `prepare_turn` inside `BlocklistAIController::send_query` |
| `preCompact` | No | No | `/compact` only: `slash_command.rs` |
| `stop` | Yes (held finalization) | Follow-up | `hold_or_finalize_turn` in `controller.rs` |
| `afterAgentResponse` | No | No | `UpdatedStreamingExchange` / stream end |
| `afterAgentThought` | No | No | `Reasoning.finished_duration` |
| `beforeTabFileRead` | n/a | n/a | none |
| `afterTabFileEdit` | n/a | n/a | none |
| `workspaceOpen` | No | No | `DetectedRepositoriesEvent::DetectedGitRepo` (if defined) |

## 6. Gaps and sizing

Sizes: small is days, medium is one to three weeks, large is more. "Client" means the `warp` repo only.

| Gap | Size | Where | Unblocks |
| --- | --- | --- | --- |
| G1. Hook config model and Cursor parser: event names, per-hook options, matcher compile, source and format traits (`crates/ai/src/hooks/`) | Medium | Client | All hooks |
| G2. Hook runner: one-shot JSON-over-stdio spawn with timeout, process-group kill, parallel execution, merge rules, exit-code policy, `failClosed` | Medium | Client | All command hooks |
| G3. Discovery and watching for project `.cursor/hooks.json` at each detected git root, run-delivered sources (`AgentConfigSnapshotFile` `hooks`, `WARP_HOOK_DIRS`), and factory agent definitions, modeled on `FileMCPWatcher`, in `crates/ai/src/hooks`; startup scan barrier; `~/.cursor/hooks.json` in Phase 1.5 | Medium | Client (+ small server for factory delivery) | All hooks |
| G4. Pending-hook state in the action pipeline (a `NotExecutedReason::WaitingOnHooks` arm and a verdict cache keyed by conversation, action, arguments, cwd, and registry revision) and `HookPermissionPolicy` with tightening-only composition per section 4.7 and the floors named; the later bounded-loosening phase and the two-pass rewrite contract are separate follow-ons | Medium | Client | `preToolUse`, `beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile`, `subagentStart` |
| G5. Tool name mapping plus JSON encoders/decoders for every `AIAgentActionType` and `AIAgentActionResultType` | Medium | Client (`crates/ai`) | `preToolUse`, `postToolUse`, `postToolUseFailure` |
| G6. Post-tool wrapper around `execute_future`, sync-result path, per-action duration timer | Small | Client | `postToolUse`, `postToolUseFailure`, `afterShellExecution`, `afterMCPExecution`, `afterFileEdit` |
| G7a. Deny reasons inside the denied tool result for families with a free-text error or denied variant on the wire: MCP (`Error { message }`), `read_files` (`failed_reads`, `Error`), file edits (`apply_file_diffs_result::Error`), `run_agents` (`Denied { reason }`), and the other `Error(message)` results. Includes a dedicated client-side denied variant where today's name is a misnomer (for example `DiffApplicationFailed`) | Small | Client | Deny paths of `preToolUse` (non-shell tools), `beforeMCPExecution`, `beforeReadFile`, `subagentStart` |
| G7b. `hook_context` fields on the request proto and on `ToolCallResult`, plus server-side rendering, for `additional_context` and any non-deny `agent_message`. The existing request-level context slot is lossy when results are batched and is not a substitute | Small (client) + server | Server + client | `sessionStart`, `postToolUse`, `postToolUseFailure` to Full |
| G7c. A `hook_denied { string reason }` member in `PermissionDenied.reason` on `RunShellCommandResult` with server rendering: built in Phase 1 (warp-proto-apis #404, warp-server #20448). Still open: a text-bearing denied variant for the no-text-slot tools (`InitProject`, `OpenCodeReview`, `WaitForEvents`, `SuggestPrompt`, and the three long-running-shell actions), recommended in architecture §9.3 and awaiting a decision. The `CancelledBeforeExecution` drop is unchanged; a hook denial does not use it | Done (shell); small (client) + small server (the rest) | Server + client | `beforeShellExecution` to Full (done); `preToolUse` on the no-text-slot tools |
| G8. `HookMessenger` trait with the Oz structured-stdout implementation in Phase 1; GUI (toast, ephemeral message, cards) and TUI (hint) implementations in Phase 1.5; optional transcript system note type later | Small (Oz) / Small (GUI, TUI) / Medium (transcript note) | Client | All hooks with `user_message` |
| G9. `send_query` sync-to-async split for the blocking prompt-submit hook | Small to medium | Client | `beforeSubmitPrompt` |
| G10. Stop-hook deferral; follow-up routed through the prompt-submit hook with hook provenance and queued behind user input; per-definition `loop_limit` plus the host cap of 8 | Small to medium | Client | `stop`, `subagentStop` |
| G11. Plumb `risk_category` from the proto through `From<RunShellCommand>` (`crates/ai/src/agent/action/convert.rs:21-40`). Confirmed dropped at HEAD; the proto marks `is_read_only`/`is_risky` deprecated in its favor | Small | Client | Richer `tool_input` for shell hooks; not a Cursor-parity blocker |
| G12. Define and emit a session-end event for GUI/TUI | Small to medium | Client | `sessionEnd` |
| G13. Project-root and `workspace_roots` derivation, hook env vars, per-source cwd | Small | Client | All hooks |
| G14. Trust: a `TrustPolicy` trait whose Phase 1 implementation is a static table keyed by surface, source kind, and the profile's command-execution posture (project hooks auto-activate when the run already auto-approves command execution; otherwise an explicit flag or config entry; run-delivered sources approved by construction); pinning of the active set at run start; hook files and scripts registered as protected write paths in `can_write_files`; stale-snapshot handling on malformed reload. Phase 1.5: the per-root review card keyed on root, host, source identity, and config revision, with re-approval on change | Small (Phase 1) / Medium (Phase 1.5) | Client | All project-scoped hooks |
| G15. Transcript export to a file for `transcript_path` | Medium | Client | Envelope completeness; optional |
| G16. Server-emitted pre-compaction event with usage metrics; client fires the hook on receipt | Small (client) + server | Server | `preCompact` |
| G17. Server-emitted lifecycle events for server-side subagents (start with type/task/model, stop with status/summary) | Server + small client | Server | `subagentStart`, `subagentStop` to Full |
| G18. Hook calls on the other agent-invoked file-content executors: `ReadSkill`, `GetFiles`, and `SearchCodebase` (after the search runs, once per surfaced file). Project rules, passive `relevant_files`, and `Grep` match lines are out of scope by design (section 3.12) | Small to medium | Client | `beforeReadFile` beyond `ReadFiles` |
| G19. Prompt-hook evaluation endpoint and client `PromptHookExecutor` | Medium (server) + small (client) | Server + client | Prompt hooks on every event |
| G20. Run-delivered sources: a `hooks` key in `AgentConfigSnapshotFile`, a `WARP_HOOK_DIRS` input, and factory agent hook definitions delivered by warp-server (naming decided with the factory file schema); local-child scope rules in Phase 1.5 | Small (client) + small server | Client + server | Oz, CLI, and child surfaces |
| G21. Observability: Phase 1 is one `log::` line per invocation (event, source, exit code, duration, stderr tail); telemetry event, diagnostics ring, and optional settings page are Phase 2 | Small (Phase 1) / Small to medium (Phase 2) | Client | All |
| G22. Feature flag in Phase 1; settings toggles (hooks on/off, user-scope Cursor import, strict hook policy) and command palette pairs in Phase 1.5 | Small | Client | Rollout |
| G23. Windows descendant cleanup via Job Objects in the `command` crate (kill-on-close job, child assigned, no `CREATE_BREAKAWAY_FROM_JOB` for hook children), in Phase 1 | Small to medium | Client | Hooks on Windows |
| G24. Post-`afterFileEdit` snapshot refresh so formatter changes are picked up before the next edit | Small | Client | `afterFileEdit` correctness |

## 7. Open questions for the Warp team

Answered in review and recorded in `architecture.md` §12: shell hooks see the model's command, not the decorated one; hook payloads are not redacted; `hook_context` and the `afterAgentResponse` finality heuristic are acceptable; no Warp-owned hooks format for now; Windows ships in Phase 1 via Job Objects; SSH and WSL sessions are unsupported at first, with hooks never run on the local host against remote paths.

1. Factory hooks: the exact file names and schema for factory-wide and per-agent hook definitions, to be settled with the owners of the factory file schema that warp-server publishes. A full proposal (directory convention, a `FactoryHookDirs` field on the task snapshot and `ExecutionConfiguration`, and eight sub-decisions) is in `/Users/david/.agent-scratch/agent-hooks-impl/factory-hooks-proposal.md`; the same server route would let a parent run forward hook sources to its remote children, which Phase 1 cannot do.
2. Timeouts: the proposed defaults are 30 s for pre-execution events, 60 s for post-tool and other events, and 5 s for `sessionEnd`; Claude Code and Codex use 600 s for most hooks. Are these the right numbers?
3. For scalar output fields, should higher-priority sources win (this report's choice) or should Warp copy the main Cursor page's "last response wins"?
4. Trust UI (Phase 1.5): the per-root trust-to-run decision and the later "may approve" capability are separate grants. How should they be presented in the GUI and TUI, and how is the posture rule's explicit override set for Oz environments and teams?
5. Should `preToolUse` fire for Warp-only tools under a `"*"` matcher, given that Cursor-authored hooks will not recognize the names?
6. `sessionEnd`: is it acceptable that idle conversations never end, or should app quit fire it for every open conversation?
7. `workspaceOpen`: defer, or define it as "new repo root detected" and ship it observe-only?
8. `beforeSubmitPrompt`: fire for queued prompts and shared-session prompts (proposed yes)? Hook-originated follow-ups pass this hook with provenance; should the Cursor-format adapter expose that provenance as an extra payload field, or hide it?
9. `stop` and `subagentStop` follow-ups: should they count against the user's AI quota the same as a typed prompt?
10. Prompt hooks: which events accept them, and how is the model call billed?
11. Is a per-exchange `transcript_path` export worth building, or should the field stay null?
12. Should Warp add a transcript-visible "system note" output type for `user_message`, or are toasts and hints enough?
13. `hook_context` protos: should the server render hook context as a system-style note, as part of the tool result, or as a separate message type? This decides what `additional_context` looks like to the model.
14. Does Cursor fire `beforeReadFile` when it loads a skill? The docs do not say. Worth checking empirically with an audit hook, so Warp knows whether firing on `ReadSkill` is parity or a superset.
