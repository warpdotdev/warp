# TECH: Per-state, per-tool status text for TUI tool call rows

## Context

The TUI transcript previously rendered every agent tool call as a static dim row reading "executed a tool call". This change gives each tool call a one-line row with a colored status glyph and per-state label (pending / awaiting approval / running / succeeded / failed / cancelled), modeled on the GUI's inline action text and status icons.

How the surrounding system works:

- `crates/warp_tui/src/agent_block.rs` — `TuiAIBlock::sections` extracts `AIAgentOutputMessageType::Action` messages into `TuiAIBlockSection::ToolCall(Box<AIAgentAction>)`. `RequestFileEdits` is special-cased: it gets a stateful `TuiFileEditsView` child view (`crates/warp_tui/src/tui_file_edits_view.rs`).
- `app/src/ai/blocklist/action_model.rs:606` — `BlocklistAIActionModel::get_action_status` is the source of truth for per-action status, returning `AIActionStatus` (`Preprocessing`, `Queued`, `Blocked`, `RunningAsync`, `Finished(Arc<AIAgentActionResult>)`). The model emits `BlocklistAIActionEvent` on every status transition (same file, lines 1431-1452).
- `crates/ai/src/agent/action_result/mod.rs` — per-tool result enums carry Success/Error/Cancelled plus payloads usable for counts (e.g. `SearchCodebaseResult::Success { files }`, `RunAgentsResult::Launched/Denied/Failure`).
- `app/src/tui_export.rs` — the only surface through which `warp_tui` consumes app types.

## Implementation

### Display state

`crates/warp_tui/src/tool_call_labels.rs` defines `ToolCallDisplayState` and collapses `Option<&AIActionStatus>`, the exchange's output-streaming flag, and (for shell-command rows) the backing terminal block's `CommandBlockState` into it. A resolved block state supersedes the status entirely — `Running` → `Running`; `Finished` → `Cancelled` (SIGINT) / `Succeeded` (`was_successful`) / `Failed` per exit code (see footnote 1). Otherwise:

- `None` while the exchange output is still streaming → `Constructing`: the tool call's arguments are still streaming in and may be empty/partial, so labels never interpolate them. Each tool has an arg-free loading label in `label_for_action`, indexed on the GUI's loading copy (`common.rs` `LOAD_OUTPUT_MESSAGE_*`, e.g. "Generating command…", "Grepping…", "Finding files…", "Reading files…", "Searching codebase…", "Preparing question…"). This mirrors the GUI's `get_action_status(id).is_none() && status.is_streaming()` gating (and fixes the blank-args window the GUI itself still has for Grep/FileGlob).
- `None` (stream finished) / `Preprocessing` / `Queued` → `Pending`
- `Blocked` → `AwaitingApproval` (renders the Pending text plus the suffix " (awaiting approval)")
- `RunningAsync` → `Running`
- `Finished(result)` → `Cancelled` / `Failed` / `Succeeded` via `is_cancelled()` / `is_failed()`, else success

Per-tool special results are handled inside the per-tool match: `RunAgentsResult::Denied` (a failed state with its own copy), `SuggestNewConversationResult::Rejected` (a success-state user decision, not a failure), denylisted commands, and long-running command snapshots (no-block fallback).

### Label function

`tool_call_label(action: &AIAgentAction, status: Option<&AIActionStatus>, output_streaming: bool, block_state: Option<CommandBlockState>) -> String` is a pure function matching exhaustively over all `AIAgentActionType` variants (no `_` arm); `output_streaming` comes from `AIBlockOutputStatus::is_streaming()` and `block_state` from `TuiAIBlock::command_block_state` at render time. Helpers:

- `single_line`: first line only, capped at 80 chars, `…` appended when trimmed (mirrors the GUI's `format_command_text`).
- `display_path`: `"."` → "the current directory" (mirrors `app/src/ai/blocklist/block/view_impl/output.rs:2425-2433`).
- `files_summary`: comma-joined base names for ≤3 files, else "{n} files".
- `count_label`: pluralization ("1 file" / "2 files").

### Text table

Placeholders: `{cmd}`=command, `{q}`=query, `{qs}`/`{pats}`=comma-joined queries/patterns, `{files}`=file summary, `{n}`=count. Backticks are literal characters in the rendered row (the TUI has no inline-code styling).

| Tool | Pending | Running | Success | Failed | Cancelled |
|---|---|---|---|---|---|
| RequestCommandOutput | Run `{cmd}` | Running `{cmd}` | Ran `{cmd}` [1] | `{cmd}` exited with code {code} [2] | Cancelled `{cmd}` |
| ReadFiles | Read {files} | Reading {files} | Read {files} | Failed to read {files} | Cancelled reading {files} |
| SearchCodebase | Search for "{q}" in {repo} | Searching for "{q}" in {repo} | Searched for "{q}" in {repo}, {n} results [3] | Search for "{q}" in {repo} failed [4] | Search for "{q}" in {repo} cancelled |
| Grep | Grep for {qs} in {path} | Grepping for {qs} in {path} | Grepped for {qs} in {path}, {n} matching files | Grep for {qs} failed | Grep for {qs} cancelled |
| FileGlob / FileGlobV2 | Find files matching {pats} in {path} | Finding files matching {pats} in {path} | Found {n} files matching {pats} [5] | File search for {pats} failed | File search for {pats} cancelled |
| CallMCPTool | Call MCP tool {name} | Calling MCP tool {name} | Called MCP tool {name} | MCP tool {name} failed | MCP tool {name} cancelled |
| ReadMCPResource | Read MCP resource {uri} | Reading MCP resource {uri} | Read MCP resource {uri} | MCP resource {uri} failed | MCP resource {uri} cancelled |
| ReadSkill | Read skill {skill} | Reading skill {skill} | Read skill {skill} | Failed to read skill {skill} | Cancelled reading skill {skill} |
| RequestFileEdits [6] | Preparing edits… | Preparing edits… | Edited {n} file(s) (+a −r) | — | — |
| CreateDocuments | Create plan | Generating plan… | Created plan [7] | Failed to create plan | Create plan cancelled |
| EditDocuments | Update plan | Updating plan… | Updated plan ({n} edits) | Failed to update plan | Update plan cancelled |
| ReadDocuments | Read {n} document(s) | Reading {n} document(s) | Read {n} document(s) | Failed to read documents | Cancelled reading documents |
| UploadArtifact | Upload {file} | Uploading {file} | Uploaded {file} | Upload of {file} failed | Upload of {file} cancelled |
| UseComputer / RequestComputerUse | {summary} | {summary} | {summary} | {summary} — failed | {summary} — cancelled |
| StartAgent | Start {remote }agent {name} | Starting {remote }agent {name}… | Started agent {name} | Failed to start agent {name} | Start agent {name} cancelled |
| SendMessageToAgent | Send message: {subject} | Sending message to {n} agent(s): {subject} | Sent message: {subject} | Failed to send message: {subject} | Send message cancelled |
| RunAgents | Configuring agents… | Spawning {n} agent(s)… | Spawned {n} agent(s) [8] | Failed to start orchestration [9] | Spawn agents cancelled |
| AskUserQuestion | Asking {n} question(s) | Asking {n} question(s) | Answered all {n} questions [10] | Questions failed | Questions cancelled |
| SuggestNewConversation | Suggested starting a new conversation | Suggested starting a new conversation | New conversation started [11] | Suggested starting a new conversation | New conversation suggestion cancelled |
| FetchConversation | Fetch conversation | Fetching conversation… | Fetched conversation | Fetch conversation failed | Fetch conversation cancelled |
| ReadShellCommandOutput | Read command output | Reading command output… | Read command output | Failed to read command output | Read command output cancelled |
| WriteToLongRunningShellCommand | Write input to running command | Writing input to running command… | Wrote input to running command | Failed to write to running command | Write to running command cancelled |
| TransferShellCommandControlToUser | Handing control to you: {reason} | Handing control to you: {reason} | You are in control | Control transfer failed | Control transfer cancelled |
| StartRecording | Start recording | Starting recording… | Started screen recording | Recording failed to start | Start recording cancelled |
| StopRecording | Stop recording | Stopping recording… | Saved screen recording | Failed to save recording | Stop recording cancelled |
| InsertCodeReviewComments | Insert {n} review comment(s) | Inserting {n} review comment(s)… | Inserted {n} review comment(s) | Failed to insert review comments | Insert review comments cancelled |
| WaitForEvents | Waiting for agent events… | Waiting for agent events… | Done waiting for agent events | Waiting for agent events failed | Wait for events cancelled |
| Fallback [12] | {name} | {name}… | {name} — done | {name} — failed | {name} — cancelled |

Footnotes:

1. For `RequestCommandOutput` rows, the terminal `Block` backing the command supersedes the action status/result for execution states whenever it exists (GUI parity: `RequestedCommandView` derives icon and expandability from the block, requested_command.rs:1148-1154, 1275-1307). Block running → "Running `{cmd}`"; finished → "Ran `{cmd}`" / "`{cmd}` exited with code {n}" / "Cancelled `{cmd}`" (SIGINT), per the block's exit code. This matters especially for agent-monitored commands, whose stored result stays a `LongRunningCommandSnapshot` forever; with no local block (viewers, restored sessions) that snapshot result falls back to "`{cmd}` is still running".
2. `{code}` from the completed exit code; `Denylisted` → "`{cmd}` denied (denylisted)". Exit code 130 is classified as cancelled by `AIAgentActionResultType::is_cancelled`.
3. 0 results → "…, no results". `{repo}` = file name of the request's `codebase_path`; the " in {repo}" segment is omitted when absent.
4. `CodebaseNotIndexed` appends " because the codebase isn't indexed".
5. Legacy `FileGlob` success carries no count: "Found files matching {pats}". Missing path defaults to "the current directory".
6. Rendered by the existing `TuiFileEditsView` child view (unchanged); the shown copy comes from that view. The label fn intentionally has no copy for tools with custom rendering — its `RequestFileEdits` arm returns an empty string and logs a warning if ever reached.
7. More than one document → "Created {n} documents".
8. Partial → "Spawned {launched} of {total} agents"; none launched → "Failed to spawn {n} agent(s)" (still a Succeeded display state, since `RunAgentsResult::Launched` is a successful result).
9. `Failure` appends ": {error}" when present; `Denied` → "Orchestration disabled — agents not launched".
10. One question → "Answered question"; partial → "Answered {answered} of {total} questions"; all skipped or `SkippedByAutoApprove` → "Questions skipped".
11. `Rejected` → "Continuing current conversation" (user decision, success display state).
12. Fallback covers `SuggestPrompt`, `InitProject`, `OpenCodeReview`, and future variants; `{name}` = `AIAgentActionType::user_friendly_name()` (`crates/ai/src/agent/action/mod.rs:425`).

### Rendering and styling

`render_tool_call_section(action, status, output_streaming, block_state, app)` in `crates/warp_tui/src/agent_block_sections.rs` renders a `TuiFlex::row()` of a two-cell status-glyph gutter followed by the label, so a wrapping label keeps a hanging indent under its own first column. The glyph mirrors the GUI's inline action icons (`action_icon` in `app/src/ai/blocklist/block/view_impl/output.rs:3501`, `inline_action_icons.rs`), per display state (`tool_call_glyph` in `crates/warp_tui/src/tool_call_labels.rs`):

- Constructing / Pending → `○` in `TuiUiBuilder::dim_text_style()` (GUI: grey circle)
- AwaitingApproval → `■` in `TuiUiBuilder::attention_glyph_style()` (`terminal_colors().normal.yellow`; GUI: yellow stop)
- Running → `●` in `attention_glyph_style()` (GUI: yellow running circle)
- Succeeded → `✓` in `TuiUiBuilder::success_glyph_style()` (`terminal_colors().normal.green`; GUI: green check)
- Failed → `✗` in `TuiUiBuilder::error_text_style()` (`terminal_colors().normal.red`; GUI: red x)
- Cancelled → `■` in `TuiUiBuilder::muted_text_style()` (`terminal_colors().bright.black`; GUI: grey cancelled block)

State lives in the glyph, so the label keeps `TuiUiBuilder::primary_text_style()` (normal foreground) for every state except Constructing / Pending, which use `dim_text_style()` until execution starts (GUI parity: labels are not tinted by outcome; only queued/streaming-not-first rows dim).

`TuiText` (`crates/warpui_core/src/elements/tui/text.rs`) additionally supports multiple styled runs via `TuiText::from_spans`, flowing as one wrapped paragraph with per-run styles patching over the base style. The tool-call row itself uses two single-style texts in the flex row; `from_spans` exists for upcoming rows that need mid-line colored fragments (e.g. the file-edits `+a −r` counts).

### Status plumbing and re-render

- `TuiAIBlock` stores the `ModelHandle<BlocklistAIActionModel>` and looks up `get_action_status(&action.id)` at render time for each tool-call section (`crates/warp_tui/src/agent_block.rs`). It also holds the surface's `Arc<FairMutex<TerminalModel>>`: for every shell-command action it resolves the backing block's ground truth via `command_block_state` (lookup by `block_for_ai_action_id`, with the snapshot result's `block_id` as fallback).
- `TuiTranscriptView` subscribes to `BlocklistAIActionEvent`; on any transition it finds the agent block whose output contains that action id (`TuiAIBlock::renders_action`), calls `mark_rich_content_dirty(view_id)`, and notifies (`crates/warp_tui/src/transcript_view.rs`). Dirty-marking is required because label text changes can change wrapped height, and block heights are cached in the block list. Each block maintains a `HashSet` of its action ids (populated by `sync_action_views` as output streams in, mirroring the GUI `AIBlock`'s `requested_action_ids`), so the per-event check is an O(1) set lookup per block rather than an output-message scan.
- `app/src/tui_export.rs` additionally exports `AIActionStatus`, `BlocklistAIActionEvent`, and the request/result types the label logic and its tests consume; `app/src/ai/blocklist/mod.rs` re-exports `AIActionStatus` and `BlocklistAIActionEvent` publicly.

## Testing and validation

- `crates/warp_tui/src/tool_call_labels_tests.rs` — a single lifecycle test asserting the label text changes as one action moves through constructing → pending → awaiting approval → running → cancelled/failed, plus the block-state overrides for a snapshot result (no block / running / exit 0 / exit 1 / exit 130). Per-tool string variants are intentionally not unit-tested; the table above is the source of truth for copy.
- `crates/warp_tui/src/agent_block_tests.rs` — transcript rendering asserts label text, glyph prefixes, per-state glyph/label colors (`tool_call_row_glyph_and_colors_reflect_state`), and message ordering through the real `BlocklistAIActionModel` fixture.
- `crates/warpui_core/src/elements/tui/text_tests.rs` — span construction: per-span styles patching the base style, wrapping across span boundaries, hard newlines inside spans, and empty spans occupying no rows.
- `cargo nextest run -p warp_tui`, `cargo nextest run -p warpui_core --features tui`, `./script/format --check`, and presubmit-style clippy on `warp` + `warp_tui` all pass.

No parallelization was used: the change is a single-crate feature plus a small export surface, implemented sequentially.

## Gaps and follow-ups

- No approval/confirmation UI in the TUI for `Blocked` actions — only the " (awaiting approval)" text suffix. An interactive accept/reject flow is future work.
- `RequestFileEdits` keeps its existing `TuiFileEditsView` (diff summary); it does not yet reflect failed/cancelled outcomes distinctly.
- `WebSearch` / `WebFetch` / `Subagent` output messages are separate `AIAgentOutputMessageType` variants (not tool-call actions) and remain unrendered by the TUI.
- Rich result bodies (file lists, collapsible output) are out of scope; rows are single-line labels only.
