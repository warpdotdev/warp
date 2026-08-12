# Factory MCP tool reference

Deeper reference for the `warp-factory` MCP tools used by the `factory-mcp`
skill. The live tool descriptions and input schemas in your session are
authoritative — read them first. These summaries reflect the current surface and
are a convenience, not a replacement; when they disagree with the live schema,
follow the live schema.

Every task-level tool is keyed off IDs that come from the discovery tools: a
`factory_uid` from `list_factories`, and a `factory_task_uid` from `list_tasks`.

## list_factories

List the software factories visible to you, including each factory's roster of
named agents (foreman, triage, spec, implement, review, verify). Start here: the
returned factory `uid` is the `factory_uid` parameter the other tools require.

Inputs (all optional):
- `team_uid` — filter factories to a single team.
- `cursor` — pagination cursor returned by a previous call.

## create_factory

Create a new software factory for a team, seeded with its runner and the full
roster of named agents (foreman, triage, spec, implement, review, verify).
Omit `default_environment` to have one auto-created from the name and
repositories, and omit `default_model` to capture the caller's current
default model. Returns the same factory shape as `list_factories`, so the new
`uid` is immediately usable as the `factory_uid` parameter of the other
factory tools. Set the factory's avatar with the REST endpoint
`POST /api/v1/factory/avatar`; avatars are not settable over MCP.

Inputs:
- `team_uid` (required) — UID of the team that will own the factory.
- `name` (required) — display name for the factory.
- `code_forge` (required) — source-control provider hosting the repositories:
  `GITHUB` or `GITLAB`.
- `repositories` (required) — repositories the factory works in; each entry is
  `owner/repo` (a GitLab entry may carry a full namespace such as
  `group/subgroup/repo`).
- `alias` — optional non-unique display handle for the factory.
- `description` — optional description of the factory.
- `default_environment` — optional UID of an existing team environment to use
  as the factory default; omit to auto-create one from the name and
  repositories.
- `default_model` — optional default model ID for the factory's agents; omit
  to capture the caller's current default model.

## list_tasks

List a factory's authoritative tasks for discovery. Each task includes the
`factory_task_uid` required by `get_task`, plus its canonical ticket metadata,
stage, and linked outputs (pull requests and external references). Tasks also
carry `run_url` (the task's run in Oz) and, when triggered by an integration,
`trigger_source` with a `trigger_url` back to its origin (e.g. the Slack thread
or Linear issue). Prefer these links over bare IDs when reporting tasks to a
human. Use `get_task` for a task's full run history.

Inputs:
- `factory_uid` (required) — the factory to list tasks for (from `list_factories`).
- `created_by_me` — set true to list only tasks started by the authenticated
  caller. Never guess the caller's email. Requires a **user-issued** API key; an
  agent/automation key has no user identity and this errors, so use `created_by`
  (an email) or scan the list instead.
- `created_by` — filter by the email of the user who started the task; use
  `created_by_me` for the caller's own tasks.
- `cursor` — pagination cursor from a previous call; omit for the first page.

Paging: pass the returned `next_cursor` to page through additional tasks; keep
paging while `has_next_page` is true, since a page carries however many whole
tasks fit the tool response size limit.

## get_task

Get one factory task by its authoritative `factory_task_uid`.

Default (read-only): status and context (runs, stage, outputs) — safe to call
anytime. The task and every run in its history carry a `run_url` opening them in
Oz; runs an integration triggered also carry `trigger_source` and a `trigger_url`
back to that origin (runs the factory dispatched itself have no trigger). Prefer
these links over bare IDs when reporting a task to a human.

Working locally: set `start_working = true` and the result additionally returns
an active-run report (`active_runs`), the factory's `factory_repositories`, and
exact local git `next_actions`. **The server never touches the caller's disk** —
you run the returned commands yourself. When the task has no branch in its
outputs yet, `next_actions` starts a fresh worktree from `origin/HEAD`; in a
multi-repo factory it warns to point `workspace_dir` at the repo the task
targets. Once work is underway, use `message_foreman` rather than re-calling
this tool to say something.

Inputs:
- `factory_task_uid` (required) — authoritative task UID from `list_tasks`.
- `start_working` — set when the caller wants to work on this task locally; adds
  the active-run report and local git `next_actions`.
- `workspace_dir` — absolute path of a local clone of the task's repo, used only
  to render exact worktree commands (the server never touches the disk).
- `branch` — branch for the local worktree `next_actions`; defaults to the newest
  pull-request branch in outputs. Pass another output branch to work from a
  different PR.
- `notify_foreman` — requires `start_working = true`; sends the task's latest
  foreman run a coordination heads-up.
- `note` — optional note included in the foreman heads-up.

## send_task

The ONE write path for handing the factory work, new or in-flight. New tickets
start a foreman intake run; hand-backs resume the ticket's existing foreman
conversation as a follow-up, so one unit of work keeps one conversation and one
factory task record. With no `ticket_ref`, a synthetic adhoc ref is generated; a
`title` is required only when the ticket is new to the factory. Push your branch
before sending work back. The result's `run_url` opens the receiving run in Oz.
New intake returns `run_url`, `state`, and a minted `ticket_ref` (an `adhoc:` ref
when you omit one) but **not** a `factory_task_uid` — discover that with
`list_tasks` once the task record appears. A hand-back returns `mode: handback`,
resumes the existing foreman run, and reports how many eligible artifacts it
transferred (idempotent).

Inputs:
- `note` (required) — what the factory should act on: the task description for
  new work, or the hand-back note for an existing task.
- `factory_uid` — destination factory UID; required for new intake and ignored
  when `factory_task_uid` is provided.
- `factory_task_uid` — authoritative task UID; required when handing work back to
  an existing task.
- `title` — short task title; required for new intake.
- `ticket_ref` — ticket for new intake in `<source>:<id>` form; omit to mint an
  adhoc ref.
- `ticket_url` — optional URL of the task's ticket.
- `branch` — pushed branch containing the work being handed over.
- `pr_url` — pull request containing the work being handed over.
- `stage_hint` — suggested stage for the foreman's next dispatch; the foreman may
  override it.
- `initial_snapshot_token` — WIP snapshot token for new intake; not supported for
  existing-task hand-backs.
- `source_conversation_id` — a conversation containing plan files or screenshots
  to transfer to an existing factory task.
- `artifact_uids` — optional subset of artifact UIDs from
  `source_conversation_id`; omit to transfer all eligible artifacts.

New intake vs. hand-back:
- **New intake** — provide `factory_uid` + `title` (+ `note`, and `ticket_ref` /
  `ticket_url` when the work has a ticket).
- **Hand-back** — provide `factory_task_uid` (+ `note`, `branch` / `pr_url`, and
  optionally `source_conversation_id` / `artifact_uids` to transfer artifacts to
  the existing task). `factory_uid` is ignored, and `initial_snapshot_token` is
  not supported.

## message_foreman

Send a message to a factory task's foreman — the agent orchestrating the
task — by its authoritative `factory_task_uid`. This is the coordination
channel while you work: a status update, a question, a blocker, or what you
just pushed. The message is delivered to the task's latest foreman run and
the result reports which run received it; read the reply with
`get_conversation`. It does not hand the work back or change the task's
stage — use `send_task` for that. For the one-shot heads-up when you first
pick a task up, `get_task`'s `start_working = true` + `notify_foreman = true`
is the better call, since it also tells the foreman which branch you are
starting from.

Inputs:
- `factory_task_uid` (required) — authoritative task UID from `list_tasks`.
- `message` (required) — what to tell the task's foreman: a status update, a
  question, a blocker, or what you just pushed.

## complete_task

Mark a factory task complete (its terminal `COMPLETE` stage). The completing
agent calls this once when the work is done. Idempotent and terminal:
completing an already-complete task is a success no-op, and this only ever sets
`COMPLETE` — never any other stage. See the skill's
[Completing a task](../SKILL.md#completing-a-task) section for the rules on
*when* to call it — a merged PR is the primary trigger, but never call it for
abandoned, rejected, or merely handed-off work.

Inputs (provide one):
- `run_id` — any run id in the task's run tree (e.g. the calling agent's own run
  id). Required unless `factory_task_uid` is provided.
- `factory_task_uid` — explicit alternative to `run_id`: the authoritative task
  UID from `list_tasks`.

## get_conversation

Get the raw foreman conversation for a factory task by its authoritative
`factory_task_uid` — the transcript of the foreman agent that orchestrates the
task, for analyzing or driving it. Resolves the task's latest foreman run and
returns its conversation as an ordered list of turns; each turn carries the full
decoded message (raw MAA transcript JSON) plus its type, id, and timestamp.
Read-only and always authorization-checked against the caller.

Foreman transcripts can be large and a tool response is size-limited, so the
result is a bounded window: it returns the most recent turns by default, sets
`truncated` and `has_more_before` when older turns remain, and returns
`next_before_index` to page toward the start. A `download_url` is a short-lived
link to the complete raw transcript when the window is not enough.

Inputs:
- `factory_task_uid` (required) — authoritative task UID from `list_tasks`.
- `limit` — max turns to return in the recent-first window; defaults to 50,
  capped at 200.
- `before_index` — page toward the start: return the turns immediately before
  this 0-based turn index (pass `next_before_index` from a prior call); omit to
  read the most recent turns.
