---
name: factory-mcp
description: Use the Warp Factory MCP to hand work to a software factory and collaborate with it — find factory tasks from a Slack thread / Linear ticket / description, pull a task down to continue, iterate on, or test it locally, hand it back to the factory after iterating, or bundle local work and send it to the cloud. When a user asks to continue/pick up/resume work locally, pull the task down — don't default to sending it to the factory.
---

# factory-mcp

Use the **Warp Factory MCP** (the `warp-factory` MCP server) to work *with* a
software factory. A factory is a team of cloud agents — a **foreman** that
orchestrates the pipeline (triage → spec → implement → review → verify) plus the
named agents that run each stage. You drive the factory through its MCP tools:
hand it new work, discover and inspect its tasks, and move a task between the
cloud and your local machine.

This skill covers four everyday workflows:
1. Start work locally, then **send it to the factory** (bundling it well first).
2. **List and find** factory work — from a Slack thread, a Linear ticket, or a description.
3. **Pull a task down** from the factory to continue, iterate on, or test it locally.
4. **Pull a task down to iterate**, then **hand it back** to the factory.

Match what the user actually asked for to one of these before calling any
write tool — see "Choosing a workflow" below. "Continue"/"pick up"/"keep
working on" a task almost always means #3, not #1 — unless the user names a
factory agent as the one who should continue (e.g. "have Wilson continue"),
which means #1/#4's `send_task` instead.

## Prerequisites

These tools come from the connected `warp-factory` MCP server, so it must be
available in your session. If the factory tools below are not present, the server
is not connected — stop and tell the user to connect the Factory MCP rather than
guessing.

The tools are your source of truth. Read each tool's live description and input
schema in your session before relying on the summaries here — arguments and
returned fields can evolve. The per-tool reference in
[references/factory-mcp-tools.md](./references/factory-mcp-tools.md) captures the
current surface in more depth; treat it as a starting point, and follow the live
tool schema when the two differ.

## The tools at a glance

- `list_factories` — list the factories visible to you (each with its agent
  roster). Start here: a factory's `uid` is the `factory_uid` every other tool needs.
- `create_factory` — create a new factory for a team, seeded with its runner
  and full agent roster (foreman, triage, spec, implement, review, verify).
- `list_tasks` — list a factory's tasks for discovery (each carries a
  `factory_task_uid`, ticket metadata, stage, linked outputs, `run_url`, and a
  `trigger_url` back to its origin).
- `get_task` — read one task's full status and history; with `start_working=true`
  it also returns the exact local git commands to work on it.
- `send_task` — the **one write path for handing off work**: hand the factory
  new work, or hand an existing task back after working on it locally.
- `message_foreman` — send an ongoing coordination message (status, question,
  blocker, what you just pushed) to a task's foreman; read replies with
  `get_conversation`.
- `complete_task` — mark a task complete (terminal `COMPLETE` stage).
- `get_conversation` — read the raw foreman transcript for a task.

Always start by resolving the factory:

```text
list_factories  ->  pick the factory  ->  use its uid as factory_uid
```

## Choosing a workflow

Match the user's words to a workflow before calling any write tool. A task
already living in the factory does not by itself mean the user wants the
factory to act on it next.

- **Local continuity language** — "continue/pick up/keep working on/resume
  `<task>`", "let's get back to X", or any request to work on an existing
  task in the current local session, **without naming a factory agent as the
  actor** — means the user wants the task on their machine. Find it first if
  needed (Workflow 2), then pull it down with `get_task(start_working =
  true)` (Workflow 3). Do **not** call `send_task` for this — nothing needs
  to be handed anywhere until the user has iterated locally, pushed, and
  explicitly asks to hand the work back (Workflow 4).
- **Explicit cloud/factory language** — "send this to the factory", "kick
  this up to `<agent name>`", "let the factory take it from here", "hand it
  back to the factory", or **naming a factory agent as the one who should
  continue** ("have/ask `<agent name>` to continue", e.g. "have Wilson
  continue") — means the user wants the cloud to act. The tell is whether a
  factory agent is named as the actor: that always means `send_task`, even
  though the sentence itself may say "continue". Use `send_task`: new work
  with `factory_uid` + `title` (Workflow 1), or an existing task's follow-up
  with `factory_task_uid` (Workflow 4, step 4).
- **Genuinely ambiguous phrasing** — ask once whether the user wants to work
  on it locally or hand it to the factory, rather than guessing. Never
  default to `send_task` when unsure: a pull-down is read-only and cheap to
  correct, a hand-off moves the work into the cloud.

**If the user cancels or denies any factory write call — `send_task`,
`message_foreman`, `complete_task`, or `create_factory` — that is a
rejection of the requested action, not a transient failure.** Do not
immediately retry the identical call. Stop, re-read what the user actually
asked for, and either pick the appropriate workflow or ask what they want
instead. The common case is a denied `send_task`: re-route to the local
pull-down workflow (Workflow 3) rather than retrying the hand-off.

## Workflow 1 — Start work locally, then send it to the factory

Use this when you (or the user) have started a change locally and want the
factory to carry it forward. `send_task` with a `factory_uid` (and no
`factory_task_uid`) starts a **new** foreman intake run.

**Bundle the work well before sending it to the cloud.** The receiving foreman
starts from what you hand it, so a thin hand-off means the cloud run starts cold.
Before calling `send_task`:

1. **Push your branch.** The cloud run cannot see your uncommitted local state or
   an unpushed branch, so commit your work and push the branch, then pass its
   name as `branch`. Uncommitted work is no exception — commit and push it first.
2. **Write a decision-complete `note`.** This is the task description. Include:
   the goal / desired outcome, what you have already done and why (approach and
   key decisions), the current state (files touched, what works, what is
   half-done), what remains, how to validate it, and any constraints or things to
   avoid. Assume the foreman has none of your local context.
3. **Pass the refs you have.** `branch` (pushed) and `pr_url` if a PR already
   exists, so the factory continues on your work instead of re-deriving it.
4. **Identify the ticket and title (new intake).** `title` is required for new
   work. Pass `ticket_ref` in `<source>:<id>` form (e.g. `linear:APP-1234`) and
   `ticket_url` when the work already has a ticket; omit `ticket_ref` to mint an
   adhoc ref.
5. **Suggest a stage.** Set `stage_hint` to where the factory should pick up
   (e.g. implementation when the plan is settled). The foreman may override it.

To carry supporting artifacts (plan files, screenshots) that live in your current
conversation, transfer them when handing an **existing** task back (see Workflow
4) — `source_conversation_id` / `artifact_uids` target an existing task, not new
intake.

Example (new work):

```text
send_task(
  factory_uid = "<from list_factories>",
  title       = "Fix flaky login redirect",
  ticket_ref  = "linear:APP-1234",
  ticket_url  = "https://linear.app/acme/issue/APP-1234",
  branch      = "user/login-redirect-fix",   # pushed first
  pr_url      = "https://github.com/acme/app/pull/42",  # if one exists
  stage_hint  = "implement",
  note        = "Goal: ... / Done so far: ... / Current state: ... / Remaining: ... / How to validate: ..."
)
```

The result includes a `run_url` (share it when you report back to a human) and a
minted `ticket_ref` (an `adhoc:<id>` ref when you omit `ticket_ref`). It does
**not** return a `factory_task_uid`: new intake starts a foreman run and the
authoritative task record appears a little later. To act on the task afterward
(Workflows 3–4), discover its `factory_task_uid` with `list_tasks` once it shows
up (see Workflow 2) — retry briefly if it isn't there yet. The context you pass
(`note`, `branch`, `pr_url`) is delivered into the foreman's intake conversation,
not surfaced as structured task fields.

## Workflow 2 — List and find factory work

Use this to locate a task from a Slack thread, a Linear ticket, or a plain
description.

1. `list_factories` → choose the factory → take its `uid`.
2. `list_tasks(factory_uid = ...)`. Narrow the set:
   - `created_by_me = true` for the caller's own tasks (never guess an email).
     This requires a **user-issued** API key; an agent/automation key has no user
     identity and this errors, so use `created_by` (or just scan the list)
     instead.
   - `created_by = "<teammate email>"` for someone else's.
   - Page through results with the returned cursor while `has_next_page` is true;
     one page holds however many whole tasks fit the response size limit.
3. **Match the task** using the fields on each entry:
   - **From a Slack thread or Linear issue** — match `trigger_source` /
     `trigger_url`, which link back to the task's origin (the Slack thread, the
     Linear issue, etc.). Runs the factory dispatched itself have no trigger.
   - **From a Linear ticket** — match the task's canonical ticket metadata.
   - **From a description** — match against the ticket title / metadata and
     linked outputs (pull requests, external references).
4. `get_task(factory_task_uid = ...)` for the selected task's full run history
   and outputs.

When reporting a task to a human, prefer its `trigger_url` and `run_url` (named
links) over bare IDs.

## Workflow 3 — Pull a task down to continue, iterate, or test locally

Use this whenever the user wants an existing factory task's work on their own
machine — to continue it, iterate on it, or just test it. This is also the
first step of Workflow 4 when the plan is to hand the work back afterward.
A request to "continue" or "pick up" a task routes here, not to `send_task`
— unless the user names a factory agent as the one who should continue
(see "Choosing a workflow"), which is a `send_task` hand-off instead.

1. Find the task (Workflow 2) and note its `factory_task_uid`.
2. Call `get_task` with `start_working = true`. Pass `workspace_dir` (the
   absolute path of your local clone of the task's repo) so the result renders
   exact worktree/checkout commands, and optionally `branch` to select which
   PR's branch to work from (it defaults to the newest pull-request branch in the
   task's outputs). The result adds an **active-run report** (`active_runs`) and
   the exact local git `next_actions`. When the task has no branch in its outputs
   yet, `next_actions` instead starts a fresh worktree from `origin/HEAD`. In a
   multi-repo factory, make sure `workspace_dir` is a clone of the repo this task
   targets — `get_task` returns the factory's `factory_repositories` and warns
   when the target can't be inferred.
3. Run those `next_actions` yourself — **the server never touches your disk.**
   They set up the worktree / check out the branch so you can build and test.

```text
get_task(
  factory_task_uid = "<uid>",
  start_working    = true,
  workspace_dir    = "/abs/path/to/local/clone",
  branch           = "<optional: a specific PR branch>"
)
```

## Workflow 4 — Pull a task down to iterate, then send it back

Use this to make local changes to a factory task and return it to the factory.

1. Pull the task down exactly as in Workflow 3 (`get_task` with
   `start_working = true` and run the `next_actions`). Optionally set
   `notify_foreman = true` with a short `note` to give the task's foreman a
   one-shot pickup heads-up that you are taking it locally — it also tells the
   foreman which branch you are starting from (`notify_foreman` requires
   `start_working = true`).
2. Iterate locally: make your changes, commit them, and **push the branch.** You
   must push before handing the work back — the factory acts on the pushed
   branch, not your local state.
3. **Coordinate as you go with `message_foreman`.** Once you're underway, use it
   as the ongoing channel to the task's foreman — a status update, a question, a
   blocker, or what you just pushed — and read the reply with `get_conversation`.
   It does not hand the work back or move the task's stage; only `send_task`
   does that.
4. Hand it back with `send_task`, this time passing the **`factory_task_uid`** of
   the existing task (not a `factory_uid`). A hand-back resumes the ticket's
   existing foreman conversation as a follow-up, so one unit of work keeps one
   conversation and one factory task record — do not start a new task for the
   same work. Include a `note` describing what changed and what you want next,
   plus `branch` and/or `pr_url`, and a `stage_hint` if relevant.
5. **Transfer supporting artifacts (optional).** Because a hand-back targets an
   existing task, you can bring along plan files or screenshots from your current
   conversation: pass `source_conversation_id` (and optionally a subset via
   `artifact_uids`) so they travel with the task. Only *eligible* artifacts are
   transferred (the result reports how many), and repeated sends are idempotent.
   This artifact transfer is scoped to existing tasks, so it is not available on
   new intake.

```text
send_task(
  factory_task_uid = "<existing task uid>",
  branch           = "<pushed branch>",
  pr_url           = "<pr url if any>",
  stage_hint       = "review",   # foreman may override
  note             = "What I changed locally, why, and what the factory should do next."
)
```

The result's `run_url` opens the resumed run in Oz.

## Creating a factory

Use `create_factory` to set up a brand-new factory for a team — this is a
setup operation, not one of the four collaboration workflows above. One call
seeds the factory's runner and its full roster of named agents (foreman,
triage, spec, implement, review, verify). Pass `team_uid`, `name`,
`code_forge` (`GITHUB` or `GITLAB`), and at least one repository as
`owner/repo` (for GitLab, a full namespace such as `group/subgroup/repo`
is also allowed). Omit `default_environment` to have one auto-created from
the name and repositories, and omit `default_model` to capture your current
default model. The result has the same shape as a `list_factories` entry, so
the new `uid` is immediately usable as `factory_uid` in the other tools.
Avatars are not settable over MCP — set one afterward with the REST endpoint
`POST /api/v1/factory/avatar`.

## Completing a task

When the work is truly done, mark the task complete with `complete_task`. Pass a
`run_id` from anywhere in the task's run tree (for a factory agent, its own run
id works) or the `factory_task_uid` explicitly. It only ever sets the terminal
`COMPLETE` stage and is idempotent — completing an already-complete task is a
successful no-op. Do not use it to move a task to any other stage.

## Inspecting the foreman conversation

To analyze or drive a task, read its foreman transcript with `get_conversation`
(by `factory_task_uid`). It returns a bounded, most-recent-first window of turns
(each with the decoded message, type, id, and timestamp). When older turns
remain, `truncated` / `has_more_before` are set and you page toward the start by
passing `next_before_index` back as `before_index`; a `download_url` links the
complete raw transcript when the window is not enough.

## Tips

- **Resolve the factory first.** Every task-level tool needs a `factory_uid` (or
  a `factory_task_uid` that came from `list_tasks`); start with `list_factories`.
- **`send_task` hands work to the factory — it is not how you continue work
  locally.** Only call it when the user actually wants the cloud to act: new
  work needs `factory_uid` + `title`; a hand-back needs `factory_task_uid`.
  Never open a second task for the same unit of work — hand it back to the
  same `factory_task_uid`. Naming a factory agent as the one who should
  continue ("have Wilson continue") is a `send_task` request, not a local
  pull-down.
- **A cancelled or denied factory write is a rejection, not a glitch.**
  Whether it's `send_task`, `message_foreman`, `complete_task`, or
  `create_factory`, don't retry the identical call — re-route to the right
  workflow or ask (see "Choosing a workflow").
- **Always push before you send.** Neither new intake (aside from the WIP
  snapshot token) nor a hand-back can see unpushed work.
- **Prefer named links over IDs** (`run_url`, `trigger_url`, `pr_url`) when
  reporting a task to a human.
