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
- `model` and `model_id` are present on every event and hold the same id: on `sessionStart` and
  `beforeSubmitPrompt` it is the model the request about to be sent uses, on every later event
  the model of the request the event belongs to. `model_params` is always `[]`.
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
- `Read` (a file read): `{ file_path, lines: [[start, end], ...] }`, one event per file (see
  "Batched reads and edits" below).
- `Write` and `Delete`: `{ file_path?, title?, edits: [ ... ] }`. `file_path` is present when
  every edit in the event is to one file, which is always the case once a batch over several
  files has been split into one event per file. Each edit is one of
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

## Batched reads and edits

Warp's agent reads several files in one tool call and can edit several files in one call.
Cursor's `Read`, `Write`, and `Delete` work on one file at a time, so on `preToolUse`,
`postToolUse`, and `postToolUseFailure` Warp presents such a call as one event per file: each
event has that file's `file_path` (and, for edits, only that file's `edits`, named `Delete` when
they are all deletes), that file's slice of the result in `tool_output`, and the call's one
`tool_use_id`. Things to know:

- Your hook runs once per file. The matcher is tested per file, so a `^Delete$` matcher sees the
  delete in a batch that also writes.
- A deny (or an `ask`, which a cloud run cannot answer) from any file's run denies the whole
  call, and the reason the model reads starts with that file's path.
- The events of one call share a `tool_use_id` and report the batch's `duration`, so a script
  that counts spans by event counts one per file.
- A read of one file is one event in the same shape with no special marking; an edit batch to
  one file is one event.

## Timeouts

A hook's `timeout` is a budget for its own run: the clock starts when the hook starts, not when
it was queued. At most four hooks run at once; a hook waiting for a slot keeps its whole timeout
and waits until a slot frees or the firing is cancelled. A large batch (many files times many
hooks) therefore takes longer to finish, but no hook times out because of the queue.

## `tool_output`

A JSON string, as Cursor documents, but the object inside it is Warp's own result shape with a
`kind` field (for example `{ "kind": "completed", "command", "output", "exit_code", ... }` for a
shell command, or `{ "kind": "success", "files": [ { "path", "content", ... } ] }` for a read).
It is not Cursor's result shape. Binary file content is summarized as `is_binary` and
`size_bytes` rather than included.

## Events

- Events that fire: `sessionStart`, `beforeSubmitPrompt`, `preToolUse`, `beforeShellExecution`,
  `beforeMCPExecution`, `postToolUse`, `afterShellExecution`, `afterMCPExecution`,
  `postToolUseFailure`, `subagentStart`, `stop`, `afterAgentResponse`, `afterAgentThought`.
- Not yet fired (parsed, with a warning): `sessionEnd`, `subagentStop`, `beforeReadFile`,
  `afterFileEdit`, `preCompact`. Never fired: `beforeTabFileRead`, `afterTabFileEdit`,
  `workspaceOpen`.
- `subagentStart` fires once per child of a `run_agents` call (Cursor's `Task` tool), for every
  child at once, before any child launches. `subagent_type` is always `generalPurpose`, so a
  matcher of `""`, `"*"`, or `generalPurpose` sees every child and an `explore` or `shell`
  matcher never does. `subagent_id` is an id Warp makes up for the child before it exists (the
  child's own id only exists once it runs). `task` is the child's full prompt, `tool_call_id` is
  the `run_agents` call, `subagent_model` is the child's model when one was named, and
  `is_parallel_worker` is `true` when the call launches more than one child. `git_branch` is
  never present. Warp adds `subagent_name`, `harness` (`oz`, `claude-code`, `codex`, ...), and
  `execution_mode` (`local` or `remote`). A `deny` (or an `ask`, which this event treats as
  deny) keeps that one child back: it shows up in the model's result as a failed child with
  your `user_message`, and the other children launch. Server-side subagents (Warp's own
  research, advice, and shell helpers) never fire it.
- `beforeSubmitPrompt` fires for the first prompt of an Oz cloud run as well as for later
  prompts. The run's first prompt is stored on the server, so when a `beforeSubmitPrompt` hook is
  configured Warp fetches its text before starting the run and your hook sees it as `prompt`;
  the task's attachment files are listed as `file` attachments. A `continue: false` (or exit 2)
  on that prompt ends the run as failed with your `user_message`, since the run has nothing else
  to do. If the text cannot be fetched the run goes on without the hook and says so in a warning,
  unless the hook's source is `strict`, in which case the run ends as failed.
- The first prompt's text is the run's first user message exactly as the conversation stores
  it: the task's prompt (with the agent's base prompt first when the run has one, and a
  `/<skill>` prefix when the run was started with a skill). Attachments are not listed in the
  text; they are in `attachments`. When the run continues an earlier execution's inputs (a
  handoff), or several prompts were queued before the run started, the hook sees the first
  message of the run, the one that starts it. A server older than warp-server #22256 returns
  the prompt in the shape it builds for third-party harnesses instead, which can add an
  `Attached files:` section, inline a skill's text, or append a `<system-reminder>` block.
- `beforeSubmitPrompt.attachments` lists the files the person attached
  (`{ "type": "file", "file_path" }`, in path order) and then the rule files whose content went
  with the prompt (`{ "type": "rule", "file_path" }`; Warp's `WARP.md` and `AGENTS.md` files for
  the working directory and its ancestors, plus global rules). A rule file for another part of
  the repository, which the agent can read later but whose content was not sent, is not listed.
  Images and other attachment kinds with no file on disk are not listed.
- `stop` fires once for every turn: `status: "completed"` when the turn finished, `"aborted"` when
  it was cancelled, and `"error"` when it failed (a request that failed for good, a shell the
  agent exited, a hook that ended the run). A `followup_message` is sent only after a
  `completed` stop; after `aborted` or `error` it is ignored with a warning. In a cloud run that
  ends on a cancelled or failed turn, Warp waits for that turn's `stop` hooks before the run
  exits (for at most the hook's `timeout` plus 5 seconds), so a hook that records the end of
  every turn sees the last one.
- `postToolUse.duration` and `postToolUseFailure.duration` are the time the tool ran, in
  milliseconds, from when Warp started it (after every check and confirmation, so approval wait
  is not counted) to its result. A result known at once (a hook denial, a tool with no work to
  wait for) reports `0`; a pending action cancelled before it started reports no `duration`.
  `afterShellExecution` takes its `duration` from the command's own timestamps when it has them.
- `postToolUseFailure.failure_type` is `permission_denied` or `error`; Warp tools do not time
  out, so `timeout` never appears. `cancellation_reason` is a Warp addition naming why a tool was
  cancelled.
- `afterAgentThought.text` is the model's full reasoning text as the server sends it (some
  providers send a summary). `afterAgentResponse` is decided on the client: a text message counts
  as finished when the next message starts or the stream ends. A cloud run's final answer is one
  such message and arrives whole: the server's own finish call that carries it is not counted
  as the next message.

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
- One event per file for batched reads and edits, and per-command timeouts that start when the
  hook starts.
- `duration` on `postToolUse` and `postToolUseFailure` for every tool.
- `attachments` on `beforeSubmitPrompt`, and `model`/`model_id` on `sessionStart` and the first
  `beforeSubmitPrompt` of a conversation.
- `subagentStart` for the children of a `run_agents` call.
- `beforeSubmitPrompt` for the first prompt of an Oz cloud run.
- `stop` with `status: "aborted"` and `"error"` for cancelled and failed turns.
