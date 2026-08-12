---
name: factory-mcp
description: Use the Warp Factory MCP to hand work to a software factory and collaborate with it — bundle local work and send it to the cloud, resolve factory tasks from a PR / Slack thread / ticket / branch / description, and pull a task down to test, continue locally, or iterate and hand it back
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
2. **Find or continue** factory work — from a PR, a Slack thread, a ticket, a
   branch, or a description — and decide between read-only status/coordination
   and continuing it locally.
3. **Pull a task down** to test or continue locally, in one combined call.
4. **Pull a task down to iterate**, then **hand it back** to the factory.

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

`get_task` is gaining an optional `reference` argument — an alternative to
`factory_task_uid` that resolves a task directly from a PR/Slack/ticket/branch
pointer (see Workflow 2). Not every connected server exposes it yet, so check
the live `get_task` input schema for `reference` each session before relying on
it, and fall back to the `list_tasks`-based path in Workflow 2 when it's absent.

## The tools at a glance

- `list_factories` — list the factories visible to you (each with its agent
  roster). Start here: a factory's `uid` is the `factory_uid` every other tool needs.
- `create_factory` — create a new factory for a team, seeded with its runner
  and full agent roster (foreman, triage, spec, implement, review, verify).
- `list_tasks` — list a factory's tasks for discovery (each carries a
  `factory_task_uid`, ticket metadata, stage, linked outputs, `run_url`, and a
  `trigger_url` back to its origin).
- `get_task` — read one task's full status and history, by `factory_task_uid`
  or (when the live schema advertises it) an exact `reference` — a PR/Slack/
  ticket/branch pointer; with `start_working=true` it also returns the exact
  local git commands to work on it.
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

## Workflow 2 — Find or continue existing factory work

Use this whenever you have a pointer to existing work — a GitHub PR, a Slack
thread, a Linear or Jira ticket, a branch — or need to search from a plain
description. Settle two things before any tool call: what identifies the task,
and whether you need it locally or only need its status.

### Inspect the reference before you search

When you're handed something concrete — a PR URL, a Slack link, a ticket, a
branch name — read *that* first (the PR/ticket/thread metadata: title, URL,
repository) instead of calling `list_tasks` and scanning for a match. The exact
URL/ID/ref you extract from it is what you hand to `get_task` next.

### Prefer resolving by reference

Check whether the connected `get_task`'s live input schema advertises a
`reference` argument (see [Prerequisites](#prerequisites)).

**If `reference` is available**, call `get_task` once with the exact identifier
from above:
- a GitHub PR URL, e.g. `https://github.com/acme/app/pull/42`
- a Slack message permalink, e.g. `https://acme.slack.com/archives/C0123/p1234567890123456`
  (a reply permalink's `thread_ts` is canonicalized to the thread root)
- a ticket ref or URL — Linear or Jira, e.g. `linear:APP-1234`, `jira:PROJ-123`,
  a Linear/Jira issue URL, or the generic `ticket:<source>:<id>` form
- an Oz run URL, a bare run or task UUID, or an explicit `run:<uuid>` /
  `task:<uuid>` reference — **there is no separate task-URL form**
- a bare branch name plus `repository` (`owner/repo`), or a self-contained
  `branch:<owner>/<repo>:<branch>` reference; `get_task` cannot resolve a bare
  branch without repository scope

Pass `factory_uid` and/or `repository` too whenever you already know them —
they narrow the match instead of searching across every factory you can see,
and can resolve an otherwise-ambiguous reference.

```text
get_task(reference = "https://github.com/acme/app/pull/42")
```

- A resolved call returns the same task result you'd get by `factory_task_uid`.
- `resolution_status: "ambiguous"` means several tasks matched: you get a
  bounded `candidates` list and **no side effect has fired** — no `next_actions`
  are computed and no foreman notification is sent. Narrow with `factory_uid` /
  `repository`, or ask which one is right, then re-call using the
  `factory_task_uid` from the chosen candidate. Don't guess a winner.
- A reference that matches nothing returns the same not-found result whether no
  such task exists or you simply can't see it — don't conclude either way from
  this alone.

**If `reference` is not available yet**, fall back to the list-and-match path
below.

### Compatibility fallback: list_tasks and match client-side

1. `list_factories` → choose the factory → take its `uid`.
2. `list_tasks(factory_uid = ...)`. Narrow the set:
   - `created_by_me = true` for the caller's own tasks (never guess an email) —
     try this first when the task is likely yours. This requires a
     **user-issued** API key; an agent/automation key has no user identity and
     this errors, so use `created_by` (or just scan the list) instead.
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

Once you've resolved a task by either path, decide what you actually need: for
plain status or coordination, stay with a read-only `get_task` (as above) or
`message_foreman`; to work on it locally, continue to Workflow 3.

## Workflow 3 — Pull a task down to test or continue locally

Use this when you need a factory task's code on your machine — to test it or
keep iterating on it. If you only need status or want to leave a note, use a
read-only `get_task` or `message_foreman` (Workflow 2) instead — don't pull a
task down just to check on it.

Make **one combined call** — don't resolve the task and then work it up with a
separate call:

```text
get_task(
  reference        = "<PR/Slack/ticket/branch reference, from Workflow 2>",
  # or factory_task_uid = "<uid>" if that's what the fallback path gave you
  start_working    = true,
  notify_foreman   = true,               # optional: one-shot pickup heads-up
  workspace_dir    = "/abs/path/to/local/clone",
  branch           = "<optional: a specific PR branch>"
)
```

This resolves the reference to a single task and, only when resolution is
unambiguous, hydrates it, returns `next_actions`, and sends the pickup
notification — all in the same round trip. An ambiguous reference instead
returns `candidates` and fires none of those side effects (see Workflow 2);
narrow it and re-call before setting `start_working`.

Pass `workspace_dir` (the absolute path of your local clone of the task's repo)
so the result renders exact worktree/checkout commands, and optionally `branch`
to select which PR's branch to work from (it defaults to the newest
pull-request branch in the task's outputs). The result adds an **active-run
report** (`active_runs`) and the exact local git `next_actions`. When the task
has no branch in its outputs yet, `next_actions` instead starts a fresh
worktree from `origin/HEAD`. In a multi-repo factory, make sure `workspace_dir`
is a clone of the repo this task targets — `get_task` returns the factory's
`factory_repositories` and warns when the target can't be inferred.

Run those `next_actions` yourself in an **isolated worktree** of your own —
**the server never touches your disk**, and a dedicated worktree keeps this
task's checkout separate from any unrelated local changes.

## Workflow 4 — Pull a task down to iterate, then send it back

Use this to make local changes to a factory task and return it to the factory.

1. Pull the task down exactly as in Workflow 3 — one combined `get_task` call
   with `start_working = true`, and run the `next_actions`. Optionally include
   `notify_foreman = true` with a short `note` in that same call to give the
   task's foreman a one-shot pickup heads-up that you are taking it locally —
   it also tells the foreman which branch you are starting from
   (`notify_foreman` requires `start_working = true`).
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

Reach for `get_conversation` only when you have a named context gap that the
task's fields, its `note`, or your own context can't fill — it is not a default
step in any workflow above. When you do call it, start small: pass `limit`
around 10–15 turns (e.g. `limit = 12`) instead of the default 50-turn window,
and page further back only if that's still not enough.

To analyze or drive a task, read its foreman transcript with `get_conversation`
(by `factory_task_uid`). It returns a bounded, most-recent-first window of turns
(each with the decoded message, type, id, and timestamp). When older turns
remain, `truncated` / `has_more_before` are set and you page toward the start by
passing `next_before_index` back as `before_index`; a `download_url` links the
complete raw transcript when the window is not enough.

## Tips

- **Resolve the factory first.** Every task-level tool needs a `factory_uid` (or
  a `factory_task_uid` that came from `list_tasks`); start with `list_factories`.
- **A concrete reference beats listing.** When you're given a PR, Slack thread,
  ticket, or branch, resolve it directly with `get_task(reference = ...)` when
  your session's schema advertises it, instead of listing tasks and matching
  client-side; pass `factory_uid` / `repository` to narrow or disambiguate it.
- **One combined call to work locally.** Resolve and pick up a task in a single
  `get_task(..., start_working = true, workspace_dir = ...)` call rather than
  resolving first and calling again to start working.
- **Ambiguous never notifies.** A `reference` matching more than one task
  returns bounded `candidates` and skips `next_actions` / foreman notification
  until you narrow it to one — don't guess a winner.
- **`send_task` is the only write path for handing off work.** New work needs
  `factory_uid` + `title`; a hand-back needs `factory_task_uid`. Never open a
  second task for the same unit of work — hand it back to the same
  `factory_task_uid`.
- **Always push before you send.** Neither new intake (aside from the WIP
  snapshot token) nor a hand-back can see unpushed work.
- **Prefer named links over IDs** (`run_url`, `trigger_url`, `pr_url`) when
  reporting a task to a human.
