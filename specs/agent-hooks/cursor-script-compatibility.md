# What a Cursor hook script sees under Warp

This is the one place that lists every way a script written for Cursor's `hooks.json` sees
something different when Warp runs it. `architecture.md` §4.5 records the same facts as design
decisions; this file is organized by what a script author has to know. It is kept up to date by
every pull request that changes a payload, an event, or an environment variable.

Status: as of the Phase 1 pull requests listed in `architecture.md` §14, plus the pull requests
listed at the end of this file.

## Identity fields

- `conversation_id` is a plain UUID. Cursor's cloud runs use ids that start with `bc-`; Warp has
  no prefix. To tell a cloud run apart, read the environment variable `WARP_CODE_REMOTE` (or its
  alias `CURSOR_CODE_REMOTE`), which is `"true"` for an Oz cloud run and absent otherwise. In
  Phase 1 every run that has hooks is an Oz run, so the check is always true.
- `generation_id` is Warp's turn id. Before a turn exists (on `sessionStart`) it is the
  conversation id, so the key is always present.
- `transcript_path` is always `null`, and `CURSOR_TRANSCRIPT_PATH` is never set. Warp keeps
  transcripts in its database. A script that used the transcript path to find files should use
  `workspace_roots` instead.
- `cursor_version` holds Warp's version string; `warp_version` says the same thing under its own
  name.
- `model` and `model_id` are present only when Warp knows the model for the event; both hold
  the same id. `model_params` is always `[]`.
- `user_email` is `null` when Warp does not know it.

## Tool names

Warp uses Cursor's names where a Warp tool maps onto a Cursor tool (`Shell`, `Read`, `Write`,
`Delete`, `Grep`, `Task`, `MCP:<name>`) and Warp-specific names for the rest (`Glob`,
`SearchCodebase`, `MCPResource:<name>`, `ShellWrite`, `ShellRead`, `ShellHandoff`, and the
PascalCase name of every other Warp tool). A `"*"` matcher sees all of them. The full table is in
`cursor-hooks-coverage.md` §4.2.

- A skill loaded from disk is reported as a `Read` of its `SKILL.md`, which is how a Cursor
  agent loads a skill and what Cursor scripts look for. Its `tool_input` is
  `{ "file_path": "<absolute path to SKILL.md>", "skill": true }`.
- A bundled skill (one Warp ships, with no file on disk) is reported as `ReadSkill` with
  `tool_input: { "bundled_skill_id": "<id>" }`. There is no `SKILL.md` to open for it.

## `tool_input` shapes

Cursor does not document `tool_input` field by field for `preToolUse` and `postToolUse`; Warp
follows the field names Cursor uses on `beforeReadFile` and `afterFileEdit` and in its Claude
compatibility layer.

- `Shell`: `{ command, cwd?, is_read_only?, is_risky?, wait_until_completion, uses_pager?,
  rationale? }`. The command is the model's command, before any pager or shell decoration Warp
  adds when it runs it.
- `Read` (a file read): `{ files: [ { path, lines: [[start, end], ...] } ] }`. Warp reads a
  batch of files in one call, so there is no single `file_path` yet; this changes in the pull
  request that fires one event per file.
- `Write` and `Delete`: `{ file_path?, title?, edits: [ ... ] }`. `file_path` is present when
  every edit in the batch is to one file. Each edit is one of
  `{ kind: "edit", file_path, old_string, new_string }`,
  `{ kind: "edit", file_path, move_to, hunks }` (a V4A patch),
  `{ kind: "create", file_path, content, allow_overwrite }`, or
  `{ kind: "delete", file_path }`.
- `MCP:<name>`: `tool_input` is the tool's own arguments, exactly as the model sent them, so a
  parameter such as a SQL string is read from `tool_input.<parameter>`. The server is named
  beside it as `mcp_server_id` (Warp's installation id; the server's name is not available to
  hooks). On `beforeMCPExecution` and `afterMCPExecution` the tool name has no `MCP:` prefix and
  `tool_input` is a JSON string, as Cursor documents for those events.
- `MCPResource:<name>`: `{ name, uri? }`, with `mcp_server_id` beside it.
- `Task` (`run_agents`): the batch's `summary`, `base_prompt`, `model_id`, `harness_type`,
  `execution_mode`, `plan_id`, `skills`, `metadata`, and one entry per child under `agents`.

## `tool_output`

A JSON string, as Cursor documents, but the object inside it is Warp's own result shape with a
`kind` field (for example `{ "kind": "completed", "command", "output", "exit_code", ... }` for a
shell command, or `{ "kind": "success", "files": [ { "path", "content", ... } ] }` for a read).
It is not Cursor's result shape. Binary file content is summarized as `is_binary` and
`size_bytes` rather than included.

## Events

- Events that fire: `sessionStart`, `beforeSubmitPrompt`, `preToolUse`, `beforeShellExecution`,
  `beforeMCPExecution`, `postToolUse`, `afterShellExecution`, `afterMCPExecution`,
  `postToolUseFailure`, `stop`, `afterAgentResponse`, `afterAgentThought`.
- Not yet fired (parsed, with a warning): `sessionEnd`, `subagentStart`, `subagentStop`,
  `beforeReadFile`, `afterFileEdit`, `preCompact`. Never fired: `beforeTabFileRead`,
  `afterTabFileEdit`, `workspaceOpen`.
- `beforeSubmitPrompt` does not fire for the first prompt of an Oz cloud run, because the server
  resolves that prompt. It fires for later prompts. `attachments` is currently always `[]`.
- `stop` fires only for a turn that completed (`status: "completed"`); a cancelled or failed turn
  does not fire it.
- `postToolUse.duration` is usually absent; only an interact-mode shell command's completion
  measures one. `afterShellExecution` computes `duration` from the command's timestamps.
- `postToolUseFailure.failure_type` is `permission_denied` or `error`; Warp tools do not time
  out, so `timeout` never appears. `cancellation_reason` is a Warp addition naming why a tool was
  cancelled.
- `afterAgentThought.text` is the model's full reasoning text as the server sends it (some
  providers send a summary). `afterAgentResponse` is decided on the client: a text message counts
  as finished when the next message starts or the stream ends.

## Environment variables

Cursor-format sources get `CURSOR_PROJECT_DIR`, `CURSOR_VERSION` (Warp's version),
`CURSOR_USER_EMAIL` (when known), `CURSOR_CODE_REMOTE` (`"true"` for an Oz run, else absent), and
`CLAUDE_PROJECT_DIR`, beside the `WARP_*` names (`WARP_PROJECT_DIR`, `WARP_VERSION`,
`WARP_HOOK_PROTOCOL_VERSION`, `WARP_RUN_ID`, `WARP_USER_EMAIL`, `WARP_CODE_REMOTE`).
`CURSOR_TRANSCRIPT_PATH` is never set. Warp's own credentials are removed from the hook's
environment; third-party credentials present in Warp's environment pass through. The full policy
is in `architecture.md` §9.1.

## Where hooks are configured

There is no team dashboard, no `~/.cursor/hooks.json`, and no scanning of repositories. A run's
hooks come only from the `hooks` lists in its factory's `factory.yaml` and `agent.md`, delivered
with the run. `architecture.md` §7.1 has the rules.

## Pull requests after Phase 1

- Cursor-faithful `tool_input` shapes: `ReadSkill` as `Read`, `file_path` and
  `old_string`/`new_string` on edits, MCP arguments as the whole input with `mcp_server_id`
  beside it.
