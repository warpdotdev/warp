---
name: factory-mcp
description: Use the Warp Factory MCP to hand work to a software factory and collaborate with it — bundle local work and send it to the cloud, find factory tasks from a Slack thread / Linear ticket / description, and pull a task down to test or iterate locally and hand it back
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
3. **Pull a task down** from the factory to test locally.
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

## The tools at a glance

- `list_factories` — list the factories visible to you (each with its agent
  roster). Start here: a factory's `uid` is the `factory_uid` every other tool needs.
- `list_tasks` — list a factory's tasks for discovery (each carries a
  `factory_task_uid`, ticket metadata, stage, linked outputs, `run_url`, and a
  `trigger_url` back to its origin).
- `get_task` — read one task's full status and history; with `start_working=true`
  it also returns the exact local git commands to work on it.
- `send_task` — the **one write path**: hand the factory new work, or hand an
  existing task back after working on it locally.
- `complete_task` — mark a task complete (terminal `COMPLETE` stage).
- `get_conversation` — read the raw foreman transcript for a task.

Always start by resolving the factory. When the user has set a **default
factory**, use it and skip the discovery round-trip; otherwise fall back to
`list_factories`:

```text
default set & valid?  ->  yes: use its default_factory_uid as factory_uid (skip list_factories)
                      ->  no:  list_factories  ->  pick the factory  ->  use its uid as factory_uid
```

See [The default factory](#the-default-factory) below for exactly when to honor
the default, how to handle a missing/broken/stale default, and how to set or
clear it.

## The default factory

A user who almost always uses one factory can save it as a **default**, so a
workflow that needs a factory skips the `list_factories` pick. The default lives
in a small JSON file on disk (`{{factory_config_file_path}}` — channel-aware, the
same file across the native app and the TUI). **Do not read or write that file
directly.** Go through the `{{warp_cli_binary_name}} factory default` commands so
reads, writes, malformed handling, and unknown-key preservation all behave
identically everywhere:

- `{{warp_cli_binary_name}} factory default get` — prints the saved default as
  JSON, or `{}` when none is set (or the file is unreadable).
- `{{warp_cli_binary_name}} factory default set <uid> [--name <name>]` — saves a
  default, preserving every other key already in the file.
- `{{warp_cli_binary_name}} factory default clear` — removes the default,
  preserving every other key.

The JSON `get` prints has `default_factory_uid` (**authoritative** — use it
directly as `factory_uid`) and an optional `default_factory_name` (**advisory
only** — a label so you can say "using your default factory X"; it may be stale
after a rename, so **never** use it to look up or match a factory).

### Reading the default (before `list_factories`)

When a workflow needs a `factory_uid`, before calling `list_factories`, branch:

1. **The request already names a specific factory.** Use that factory. The
   explicit choice always wins over any stored default; do not consult the
   default at all.
2. **This is a management / discovery workflow** — listing all factories, or
   setting / switching / clearing the default (see below). Always call
   `list_factories` and operate on the real result; never short-circuit on the
   stored default.
3. Otherwise run `{{warp_cli_binary_name}} factory default get` and read its
   JSON:
   - **`{}` (no default set).** Fall back to `list_factories → pick → use uid`
     exactly as before, silently — no new message, no error.
   - **A non-empty `default_factory_uid`.** Use it directly as `factory_uid` and
     **skip `list_factories`**. Briefly tell the user you're using their default
     factory (use `default_factory_name` for the label when present).
   - **A warning on stderr that the config is unreadable** (the command still
     prints `{}`). Tell the user once that their default-factory config is
     unreadable, then fall back to `list_factories`. The command never rewrites
     or deletes the file, so a hand-edited file with a typo is preserved.
4. **Stale default** — `get` returned a uid but it no longer resolves to a
   factory visible to the current account (confirmed by a `list_factories` that
   does not contain that uid). Tell the user **once** that their saved default is
   no longer available, fall back to `list_factories` for this workflow, and
   offer to update the default (`... set <new_uid>`) or clear it
   (`... clear`). Do not silently re-run discovery every time.

### Writing the default (confirm-first — never silent)

Only ever run `{{warp_cli_binary_name}} factory default set` / `clear` when the
user asks for it:

- **Explicit set / remember.** The user asks to set, remember, or change their
  default factory (e.g. "make Acme my default factory") → run
  `{{warp_cli_binary_name}} factory default set <uid> --name "<name>"`.
- **Post-pick prompt (optional).** After the user picks a factory via
  `list_factories`, you may ask once whether to remember it as their default.
  Only run `set` on an affirmative answer.
- **Never auto-pin.** Do not save a default just because the user used a factory
  once. A default is written only with explicit consent.
- **Clearing.** Run `{{warp_cli_binary_name}} factory default clear` to remove
  the default; behavior then reverts to "no default set".

The `set`/`clear` commands read-modify-write the file, so any keys you don't own
are preserved, the write is atomic, re-setting the same value is a no-op, and a
malformed file is refused rather than clobbered. The native app and the TUI use
the same commands against the same file — the TUI is not read-only.

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

1. `list_factories` → choose the factory → take its `uid`. This is a discovery
   flow, so always use the real `list_factories` result; do not short-circuit on
   a stored default (see [The default factory](#the-default-factory)).
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

## Workflow 3 — Pull a task down to test locally

Use this to check out a factory task's work on your machine and test it.

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
   coordination heads-up that you are taking it locally (`notify_foreman`
   requires `start_working = true`).
2. Iterate locally: make your changes, commit them, and **push the branch.** You
   must push before handing the work back — the factory acts on the pushed
   branch, not your local state.
3. Hand it back with `send_task`, this time passing the **`factory_task_uid`** of
   the existing task (not a `factory_uid`). A hand-back resumes the ticket's
   existing foreman conversation as a follow-up, so one unit of work keeps one
   conversation and one factory task record — do not start a new task for the
   same work. Include a `note` describing what changed and what you want next,
   plus `branch` and/or `pr_url`, and a `stage_hint` if relevant.
4. **Transfer supporting artifacts (optional).** Because a hand-back targets an
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
  a `factory_task_uid` that came from `list_tasks`). Honor the user's default
  factory when one is set and valid (skipping `list_factories`); otherwise start
  with `list_factories`. Management/discovery flows and an explicitly named
  factory always bypass the default (see [The default factory](#the-default-factory)).
- **`send_task` is the only write path.** New work needs `factory_uid` + `title`;
  a hand-back needs `factory_task_uid`. Never open a second task for the same
  unit of work — hand it back to the same `factory_task_uid`.
- **Always push before you send.** Neither new intake (aside from the WIP
  snapshot token) nor a hand-back can see unpushed work.
- **Prefer named links over IDs** (`run_url`, `trigger_url`, `pr_url`) when
  reporting a task to a human.
