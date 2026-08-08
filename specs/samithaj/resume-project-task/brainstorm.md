# Resuming a specific task within a project after a restart

**Status:** brainstorm / design exploration
**Branch context:** `samithaj/project-tabs-layout`
**Date:** 2026-08-03

## The ask

> After a Mac restart, when I open Warp I need an easy way of resuming a *specific task* that
> belongs to a project.

Read plainly, this is a **pull**, not a push: start clean, then let me go find *one* task, scoped
by project, and drop back into it. That is a different mechanism from "restore all my tabs
automatically," and the two should not be collapsed — both are live asks upstream
([#9416](https://github.com/warpdotdev/warp/issues/9416),
[#7760](https://github.com/warpdotdev/warp/issues/7760)), and they fail in different ways.

This doc covers the pull mechanism as the primary design, and treats auto-restore as a second,
complementary mechanism in §6.

---

## 1. What already exists (verified in this checkout)

The good news is that most of the machinery is already built. Four independent layers:

### 1.1 A durable conversation store

| Thing | Where |
|---|---|
| `agent_conversations` table (`conversation_id`, `conversation_data`, `last_modified_at`, `summary`) | `crates/persistence/src/schema.rs:11` |
| `agent_tasks` table | `crates/persistence/src/schema.rs:20` |
| `terminal_panes.conversation_ids` / `active_conversation_id` | `crates/persistence/src/schema.rs:418-419` |
| `summary` column added | migration `2026-07-07-000000_add_summary_to_agent_conversations` |

Conversations survive a reboot **today**. Issue [#7760](https://github.com/warpdotdev/warp/issues/7760)
was closed as resolved by a maintainer, who confirmed restore works on current versions (with a
deliberate exception: logout/login does not restore, for security reasons).

### 1.2 A normalized, filterable entry model

`AgentConversationsModel::get_entries(&AgentManagementFilters, ctx)`
(`app/src/ai/agent_conversations_model.rs:1297`) returns `Vec<AgentConversationEntry>`, merging
local + cloud + ambient tasks, sorted by `last_updated` desc.

Each entry (`app/src/ai/agent_conversations_model/entry.rs:70`) already carries:

```rust
pub struct AgentConversationDisplayData {
    pub title: String,
    pub initial_query: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
    pub status: AgentRunDisplayStatus,
    pub working_directory: Option<String>,   // <-- the join key we need
    pub harness: Option<Harness>,
    // ...
}
```

`working_directory` is populated from
`ConversationNavData { initial_working_directory, latest_working_directory }`
(`app/src/ai/conversation_navigation/mod.rs:26-27`), preferring latest and falling back to initial
(`entry.rs:630-634`).

### 1.3 A conversation list UI

`app/src/workspace/view/conversation_list/` (~2,200 lines) — already shipped, already in the left
panel, already has fuzzy search over titles (`view_model.rs:126-148`) and filters out entries where
`!capabilities.can_open`.

Verified not feature-gated as a surface: it's reached by the `OpenConversationListView` action
(`app/src/workspace/view.rs:26532`, also exposed over local control at
`app/src/local_control/handlers/app_state.rs:175`). The only `FeatureFlag` inside it gates one
interaction detail (`ActiveConversationRequiresInteraction`, `view.rs:333`), not the list. So
Phase 1 below really is a filter on a live surface, not a new one.

The filter struct (`agent_conversations_model.rs:318`) has **eight** dimensions:

```rust
pub struct AgentManagementFilters {
    pub owners: OwnerFilter,          pub status: StatusFilter,
    pub source: SourceFilter,         pub created_on: CreatedOnFilter,
    pub creator: CreatorFilter,       pub artifact: ArtifactFilter,
    pub environment: EnvironmentFilter,  pub harness: HarnessFilter,
}
```

**None of them is project, repo, or directory.** That is the hole.

Note `EnvironmentFilter` already establishes the pattern we want:
`enum { All, NoEnvironment, Specific(String) }` (`agent_conversations_model.rs:277`).

### 1.4 A working resume path — including cwd

`WorkspaceView::restore_conversation_in_new_tab` (`app/src/workspace/view.rs:13527`), plus
`_in_active_pane` and `_in_split_pane` variants, dispatched by the `OpenConversationPreference`
setting (`view.rs:13327`).

Critically, the restore path **does** re-enter the original directory
(`app/src/pane_group/mod.rs:6363-6379`):

```rust
// Get the initial working directory from the restored conversation.
let startup_directory = restoration
    .initial_working_directory()
    .map(PathBuf::from)
    .filter(|path| path.is_dir());
// ... PaneGroup::create_session(startup_directory, ...)
```

This matters a lot: it means a resumed task's tab lands in the right cwd, so
`ProjectLayout::project_of_tab_data` will bucket it into the correct project **automatically**,
with no extra work. The projection and the resume path already agree.

### 1.5 The new project layer (this branch)

- `ProjectKey` (`app/src/workspace/project_key.rs`) — `LocalGit(common_git_dir)` deliberately
  collapses every worktree of a repo into one project, so parallel-agent worktrees appear as
  *tasks under one project*. `LocalDir` and `Remote` variants too.
- `ProjectLayout::compute(tabs, ctx)` (`app/src/workspace/project_layout.rs:59`) — **a pure
  projection over `Workspace::tabs`.**

---

## 2. The precise gap

Three statements, in order of importance:

### (a) Projects have no existence independent of open tabs

`ProjectLayout` is a pure function of the currently-open tabs. Close the tab and the project — and
every task in it — vanishes from the rail. The conversation is still sitting in
`agent_conversations`, fully resumable, but the rail is structurally incapable of showing it.

This is a correct design for the *live* projection, and it should stay that way. But it means the
rail can only ever answer "what am I doing right now," never "what was I doing on this project."

### (b) The durable list has no project dimension

`AgentConversationsModel` is the one component that *can* enumerate past tasks — and it has no way
to scope them to a project. `working_directory` is a raw `Option<String>` that is displayed
(`conversation_details_panel.rs:476`) but never resolved into a `ProjectKey`.

So today the two halves miss each other: the rail knows about projects but only sees open tabs; the
conversation list sees everything but knows nothing about projects.

### (c) Auto-restore is the wrong mechanism for this ask

Even where auto-restore works, it is all-or-nothing *and* conditional on how you quit. From
[r/warpdotdev](https://reddit.com/r/warpdotdev/comments/1td8ac5/is_warp_sessionwindowpane_restore_broken/),
a Warp team member answering a user whose restore "does not work no matter what I do":

> "Ah, yes we view this as *closing a window* and assume that you don't want it back. If you select
> **Quit** from the App Bar Icon or **Quit Warp** from the Menu Bar, then sessions should be
> restored when you restart the app."

and when the user asked for the close-button case:

> "We unfortunately don't have a workaround at this point."

A Mac restart, a forced reboot, a crash, or a power loss may not deliver a clean Quit at all. Any
design that makes "get my task back" depend on the quit path will fail exactly in the scenario the
user described. **The pull mechanism must read the durable store, not the window snapshot.**

### (d) Edge case worth fixing while we're here

`.filter(|path| path.is_dir())` silently drops the cwd when the directory no longer exists — the
resumed tab then starts in the default directory and gets bucketed into the **wrong** project by
`ProjectLayout`. This is not hypothetical: `ProjectKey::LocalGit` exists precisely to unify
parallel-agent worktrees, and short-lived worktrees are the most likely directories to have been
deleted since the task ran.

---

## 3. Prior art

### 3.1 Claude Code's session picker — the closest match to the ask

This is the single best model to borrow from, because it solves *exactly* this problem
([docs](https://code.claude.com/docs/en/sessions)):

| Entry point | Behavior |
|---|---|
| `claude --continue` | Resume the most recent session **in the current directory** — no picker |
| `claude --resume` | Open the interactive picker |
| `claude --resume <name>` | Resume by name directly |
| `/resume` | Switch sessions from inside a live session |

Picker design details worth stealing wholesale:

- **Scoped by default, widenable on demand.** Default = current worktree. `Ctrl+W` widens to all
  worktrees of the repo; `Ctrl+A` widens to every project on the machine. This is the same
  worktree-collapsing decision `ProjectKey::LocalGit` already made.
- **Discriminating metadata per row**: name or generated title, time since last activity, **git
  branch**, size. Widening to all projects adds the project path to each row.
- `Ctrl+B` filters to the current git branch. `Space` previews content. `Ctrl+R` renames in place.
- Sessions get an **auto-generated title** from the first prompt when unnamed — so rows are
  readable without user effort.
- Cross-project selection degrades gracefully: picking a session from an unrelated project copies a
  `cd && resume` command to the clipboard instead of silently resuming in the wrong place.

### 3.2 tmux-resurrect + tmux-continuum — the canonical reboot-survival prior art

`resurrect` saves/restores the environment; `continuum` automates it (save every 15 min,
`@continuum-restore 'on'`). The lesson: **periodic snapshotting decoupled from the exit path.**
A crash or power loss loses at most one interval — it does not lose everything, the way a
quit-triggered snapshot does. This is the direct answer to §2(c).

### 3.3 AgentsRoom — a competitor shipping this feature today

[agentsroom.dev/features/restore-session](https://agentsroom.dev/features/restore-session):
project-scoped snapshot stored in project metadata on disk, restoring agents (with provider, role,
project context, conversation state), terminal cwds, and long-running dev servers. On quit it asks
via three checkboxes what to restore; on launch it offers one-click restoration. Their pitch —
"across reboots, across power outages, across macOS updates" — is aimed squarely at this gap.

### 3.4 The community cottage industry (strong demand signal)

An unusual number of people have independently built this tool, which is the clearest evidence that
the built-in affordance is missing across the whole category:

| Tool | Signal |
|---|---|
| [`ctx`](https://news.ycombinator.com/item?id=47836740) — "a /resume that works across Claude Code and Codex" | 72 pts, 28 comments |
| [Claudebin](https://news.ycombinator.com/item?id=47073488) — share/resume sessions by link | 30 pts |
| [Jeeves](https://github.com/robinovitch61/jeeves) — TUI for browsing/resuming agent sessions | Show HN |
| [`agf`](https://github.com/subinium/agf), [`fast-resume`](https://github.com/angristan/fast-resume), [`cc-sessions`](https://github.com/chronologos/cc-sessions), [`ccs`](https://github.com/agentic-utils/ccs), `showagent` | all "find and resume my agent sessions" |
| [CCC — Command Center for Claude](https://reddit.com/r/ClaudeCode/comments/1sze0kt/created_a_system_to_manage_multiple_claude/) | local Kanban for parallel sessions |

Three design lessons from those threads:

1. **Read the durable on-disk state, not your own in-memory registry.** CCC's headline feature:
   *"Sees every Claude session on your Mac, not just ones launched through the dashboard. Other
   tools only see what you started through them. Open a terminal and type `claude`? Invisible."*
   For us: derive the list from `AgentConversationsModel`, never from open tabs.

2. **Titles alone don't discriminate.** From the `ctx` thread: *"I have like 15 concurrent sessions
   I leave up for weeks"* and *"it's not always obvious which one is the right one to pick back
   up."* Rows need branch + last-activity + status + first prompt, not just a name.

3. **Derived indexes corrupt; the store must be the source of truth.** A user who upgraded macOS and
   rebooted found their [resume picker showing only 1–2 sessions](https://reddit.com/r/ClaudeCode/comments/1razdg0/claude_code_session_history_vanished_after_macos/)
   while the underlying transcripts were intact — spawning `claude-rescue` and similar re-indexers.
   Any cache we add must be rebuildable from `agent_conversations` on demand.

Also relevant: users want a **one-keystroke "last session"** shortcut alongside the picker — the
`--continue` case is the common one and shouldn't cost an extra selection.

### 3.5 Adjacent Warp issues

- [#9382](https://github.com/warpdotdev/warp/issues/9382) — workspace-style project tabs with
  multiple internal sessions. Currently labeled `needs-mocks`; the maintainer note explicitly calls
  for defining "close/rename/**reopen** behavior" — i.e. this doc's territory.
- [#9416](https://github.com/warpdotdev/warp/issues/9416) — keep sessions *alive* across relaunch.
  Different mechanism (don't die vs. come back); the PTY is torn down on update by design
  (`app/src/terminal/view.rs:11094`). Out of scope here, but see §6.

---

## 4. Candidate designs

### Design A — Project filter on the existing conversation list

Add `ProjectFilter { All, Specific(ProjectKey) }` to `AgentManagementFilters`, mirroring
`EnvironmentFilter`. Resolve each entry's `display.working_directory` through
`ProjectKey::for_path` at read time inside `matches_filters`. Wire the rail's `selected_project`
into the conversation list's filter.

- **Cost:** low. One enum, one match arm, one call site. No migration.
- **Gets you:** "show me every past task in this project," searchable, in UI that already exists.
- **Doesn't get you:** past tasks visible *in the rail* itself.
- **Do not call `ProjectKey::for_path` inside the filter predicate.** `get_entries` runs
  `matches_filters` per entry per refresh, and `for_path` does a `DetectedRepositories` lookup on
  every call — with a few hundred conversations that's a repo-detection lookup per conversation, on
  a path that already re-runs on `ConversationsLoaded` / `NewTasksReceived` / `TasksUpdated`
  (`view_model.rs:37-41`). Resolve into a memoized `HashMap<String /* working_directory */,
  ProjectId>` once per refresh and look up from that. This is the difference between Phase 1 being
  trivially correct and being a visible-jank bug.
- **Risk:** `ProjectKey::for_path` consults `DetectedRepositories`, which may not have finished at
  cold start (noted in `project_key.rs:58-60`) — a task could resolve to `LocalDir` for the first
  frames and then upgrade to `LocalGit`, i.e. briefly land in a duplicate project bucket.

### Design B — Persist the project↔task join

Store a resolved project key (plus git branch) alongside the conversation, so project identity does
not have to be re-derived from a path at startup.

- **Cost:** medium. One migration + write path + backfill (backfill is cheap: derive from
  `working_directory`).
- **Gets you:** correct bucketing at cold start regardless of `DetectedRepositories` timing;
  survives the directory being deleted (§2d) — you still know which project the task belonged to
  even when the worktree is gone; enables branch as a picker column and a `Ctrl+B`-style filter.
- **Note:** `ProjectKey::LocalGit(StandardizedPath)` is already a path, so it serializes to a
  string trivially.

### Design C — Recent tasks in the project rail

Extend the rail so each project shows its open tabs (as today) **plus** recent closed tasks, visually
dimmed/separated. Clicking a closed task calls the existing `restore_conversation_*` path.

Keep `ProjectLayout` a pure projection over tabs — do **not** contaminate it. Instead introduce a
second, separately-computed list (recent tasks per project, from `AgentConversationsModel`) and
merge the two only at render time in the rail view.

- **Cost:** medium. New view-model + rail rendering; the union/dedup rule (a task that is both open
  and in history must appear once, as open) needs care.
- **Gets you:** the actual ask, with zero navigation — reopen Warp, the rail already shows the
  project and its tasks, click one.
- **This is the design that most directly matches "an easy way."**

### Design D — Project-scoped resume-on-launch

On launch (or on selecting a project), offer "Resume last session in `<project>`" — the
`--continue` analogue. One keystroke, no picker.

- **Cost:** low once A or C exists (it's "take the top entry of the project-filtered list").
- **Gets you:** the common case with zero selection cost, which the `ctx` thread explicitly asked
  for.

### Design E — Snapshot decoupled from the quit path

Periodically persist the window/tab snapshot (tmux-continuum style) rather than only on clean quit,
so the close-button and crash cases stop losing state.

- **Cost:** medium-high; touches the app-state write path and has real I/O-frequency questions.
- **Gets you:** fixes the auto-restore complaint in §2(c) and the r/warpdotdev report directly.
- **Caveat:** this is the *push* mechanism. It complements but does not replace A–D — and on its
  own it still can't answer "resume this one specific task."

---

## 5. Recommendation

> **Read §7 first if the tasks you care about are CLI agents you launch by typing `claude`.** Those
> live in a different model (`CLIAgentSessionsModel`) that never reaches `AgentConversationsModel`,
> so the ordering below does not apply to them — §7.4/§7.5 replaces Phase 1 for that population.

Ship **A → C → D** in that order, with **B** folded in as soon as the cold-start bucketing or the
deleted-worktree case actually bites. Each phase is independently shippable and useful.

**Phase 1 (A).** `ProjectFilter` on `AgentManagementFilters`, resolved from `working_directory` via
the existing `ProjectKey::for_path`. Wire the rail's selected project into the conversation list.
No migration, no new UI surface. After this, "find a past task in this project" is possible.

**Phase 2 (C).** Rail shows each project's recent closed tasks under its open tabs, dimmed, with
status + last-activity. Click resumes via `restore_conversation_in_new_tab`, which already restores
the cwd, which means `ProjectLayout` re-buckets the new tab into the right project on its own. This
is the "easy way" the ask is really about. Worth doing mocks first: the maintainer on
[#9382](https://github.com/warpdotdev/warp/issues/9382) explicitly asked for product/design mocks
before code on precisely this rail UI (and labeled it `needs-mocks`), including "close/rename/reopen
behavior."

**Phase 3 (D).** "Resume last task in this project" — command palette entry + keybinding.

**Phase 4 (B), when needed.** Persist project key + branch on the conversation. Do this the moment
you see wrong-bucket flicker at cold start, or want branch as a column/filter.

Borrow from Claude Code's picker throughout: default-scoped to the selected project, widenable to
all projects; rows carrying branch + last activity + status; search over title *and* first prompt.

**Guardrail (from §3.4 lesson 3):** whatever per-project index this adds must be a *cache*, derived
from `agent_conversations` and rebuildable on demand — never the authority. The store is the
authority.

### Stated scope boundary: agent-backed tasks only

Designs A–D enumerate from `AgentConversationsModel`, so they cover exactly the tasks that have a
row in `agent_conversations`. **A plain terminal tab with no agent conversation has no such row and
is therefore not resumable this way** — not ambiguously scoped, simply absent.

This is a mismatch with the rail as built: `rail_task_label` (`app/src/workspace/tab_title.rs:130`)
deliberately falls back through repo name and last command so that "a row is never blank," i.e. the
rail today lists non-agent tabs as tasks too. After a restart those rows would not come back under
A–C.

That is an acceptable boundary for v1 — the ask is about resuming *work in progress*, which in this
workflow means an agent session, and the branch history already leans that way
(`475469aa4 name horizontal tabs after the agent session running in them`). But it should be stated
in the UI rather than left implicit, and plain shell tabs stay dependent on the window snapshot
(§6) until Design E lands. If that split turns out to be confusing in practice, the fix is to give
the rail a durable per-project record of *tabs*, not just conversations — which is Design B widened.

---

## 6. Relationship to auto-restore (the other mechanism)

Design E and issue [#9416](https://github.com/warpdotdev/warp/issues/9416) are the push side.
They're worth doing, but note they are strictly weaker for this ask:

- Auto-restore is conditional on a clean Quit (§2c) — exactly what a Mac restart may not provide.
- #9416 (keeping PTYs alive across relaunch) is an architectural change; the maintainer analysis on
  that issue notes the terminal-server helper is deliberately parent-app-bound and cleans up on
  exit, so it is *not* a persistence layer today.

The pull mechanism (A–D) works regardless of how Warp exited, because it reads the conversation
store rather than the window snapshot. That is why it should go first.

---

## 7. Harness-native resume — `claude --resume <id>` per task

The designs above resume a task **into Warp's own agent surface**. A different and in some ways
better framing: for a CLI coding agent, the durable resume handle is the *harness's own session id*,
and the rail can simply offer the harness's own resume command per task.

### 7.1 There are two populations, and only one of them is durable

This is the single most important fact for this design, and it is easy to get wrong:

| | **Warp Agent Mode** (Oz / `agent_sdk` drivers) | **CLI agent you launch by typing `claude`** |
|---|---|---|
| Model | `AgentConversationsModel` | `CLIAgentSessionsModel` |
| Keyed by | conversation id | `EntityId` — an in-memory view id (`cli_agent_sessions/mod.rs:321`) |
| Durable across restart? | **yes** (`agent_conversations` sqlite) | **no** |
| Appears in the conversation list? | yes | **no** |
| Consumers | conversation list, agent management panel | `tab_title.rs:216`, `agent_icon.rs:38`, `vertical_tabs.rs:1066` — live UI only |

Verified: `agent_conversations_model.rs` and `agent_conversations_model/entry.rs` contain **zero**
references to `CLIAgent`/`cli_agent`. Plugin-detected CLI agent sessions never enter the
conversations model at all.

**So Designs A–D (§4–5) serve population 1 only.** A task where you typed `claude` in a tab is
population 2: tracked live for the tab title and status pill, then gone.

### 7.2 What already exists (more than you'd expect)

- **Claude: Warp mints the session uuid itself.** `None => (Uuid::new_v4(), None)`
  (`claude_code.rs:285`), pinned with `--session-id <uuid>` (`claude_code.rs:210-211`). Warp knows
  the id at spawn — it doesn't have to scrape it.
- **Warp already builds resume command strings**:
  `ClaudeLocalContinuation { command: format!("claude --resume {session_id}") }`
  (`claude_transcript.rs:256`, `:337`).
- **Warp already repairs Claude's own index**: `write_session_index_entry` upserts
  `~/.claude/sessions-index.json` so `--resume <uuid>` resolves (`claude_transcript.rs:345+`) —
  the very index whose corruption caused the "lost months of history" thread in §3.4.
- **The plugin path already captures ids**: `CLIAgentSessionContext { cwd, project, session_id, … }`
  (`cli_agent_sessions/mod.rs:44-52`), fed by hook events (`session_start`, `prompt_submit`,
  `stop`, …). Plugin managers exist for claude / codex / gemini / opencode, with `can_auto_install`
  and `install`.

### 7.3 Per-harness resume commands (verified from Warp's own source)

| Harness | Resume by id | Id assignable at start? | In Warp today | Source |
|---|---|---|---|---|
| Claude Code | `claude --resume <uuid>` | **yes** — `--session-id <uuid>` | driver + plugin | `claude_code.rs:210`, `claude_transcript.rs:256` |
| Codex | `codex resume <id>` | **no** — learned from the rollout JSONL | driver + plugin | `codex.rs:189-192`, `:221` |
| Cursor CLI | `agent --resume <id>` (`agent ls` lists) | unknown | **detected, but no session handler** | [Cursor CLI docs](https://cursor.com/docs/cli/using); `cli_agent.rs:151` |
| OpenCode | `opencode --session <id>` / `-s <id>` | unknown | plugin + handler | [OpenCode CLI docs](https://opencode.ai/docs/cli/) |
| Gemini | not supported yet | — | driver + plugin | `gemini.rs:81-82` ("Gemini does not support conversation resume yet") |

**Two enums, and the CLI one is what matters here.** `AIAgentHarness`
(`app/src/ai/agent/conversation.rs:4437`) is only `{ Oz, ClaudeCode, Gemini, Codex, Unknown }` — the
*driver* harnesses. The detection path keys on `CLIAgent` (`app/src/terminal/cli_agent.rs:140`),
which has **15** variants including `CursorCli`, `Amp`, `Droid`, `Auggie`, `Pi`, `Copilot`, `Goose`,
`Hermes`, `Antigravity`, plus `command_prefix()` and `to_serialized_name()` helpers. Any resume
command table should key off `CLIAgent`.

**Cursor specifically:** `CLIAgent::CursorCli` exists and its `command_prefix()` is `"agent"` —
matching Cursor's own docs. But `is_agent_supported()` excludes it and `create_handler` returns
`None` (`cli_agent_sessions/listener/mod.rs:39`, `:70-78`), so Warp gets **no `session_start` event
and no session id** for Cursor. It is detected (tab title, icon) but not instrumented. Session
handlers exist today for: Claude, OpenCode, Codex, Gemini, Auggie, Droid, Pi, OhMyPi.

Instrumenting it does **not** require Cursor to expose hooks, though: Cursor keeps chats on disk at
`~/.cursor/chats/<md5(cwd)>/<chat-uuid>/meta.json` (`{schemaVersion, createdAtMs, hasConversation,
title, updatedAtMs, cwd}`) plus a `store.db` — verified locally, and the md5-of-cwd directory key is
direct evidence that Cursor sessions are **directory-scoped**, like Claude's. A filesystem watcher
gets `(id, cwd, title)` the same way Codex's rollout-file walk does. Undocumented internals, so gate
it and fail soft. See `plan.md` for the full write-up.

**Codex breaks the "save the id when we start" framing.** Its own comment says it outright:
*"Unlike claude, codex does not support assigning a session_id to a new conversation"*
(`codex.rs:187-188`). Warp learns it afterwards via `session_id: OnceLock<Uuid>` populated from the
first rollout-file write. So the stored record must be **nullable and updatable** — written when the
id becomes known, not once at row creation.

### 7.4 Phase 0 — the version that needs no storage at all

`claude --continue` resumes the most recent session **in the current directory**. The cwd is already
persisted (`working_directory`, §1.2). So:

> rail row for a project → "Resume last agent session here" → open a tab at the project cwd →
> run `claude --continue`

That is the common case, with **no schema change, no id capture, nothing new persisted**. Worth
shipping first; storing ids is what buys you resuming a *specific, older* task rather than the last
one.

### 7.5 Phase 1 — the record you described

Persist, per agent session:

```
harness            (claude | codex | gemini | opencode)
harness_session_id (nullable, updatable — Codex fills it in late)
cwd                (load-bearing, see 7.6)
project_key        (so the rail can group without re-deriving)
last_seen, title   (so rows are discriminating — see §3.4 lesson 2)
```

keyed to the terminal pane / conversation. Write it on the plugin's `session_start` event, and at
spawn for the Warp-driven Claude path where the uuid is already in hand. Treat it as a cache
rebuildable from the harness's own on-disk state, per §3.4 lesson 3.

Then the rail row renders "Resume" → open tab at `cwd` → run the harness's resume command. Offer
"copy command" as an escape hatch, mirroring how Claude Code's picker copies a `cd && resume` when
the target is an unrelated project.

### 7.6 Four footguns

1. **Do not reuse `claude_command()` to build the user-facing resume command.** It hardcodes
   `--dangerously-skip-permissions` and a stdin prompt-file redirect (`claude_code.rs:211`). A
   resume button must emit plain `claude --resume <id>`, or it silently escalates permissions on an
   interactive session.
2. **The cwd is load-bearing, not decoration.** Per Claude Code's docs, "session ID lookup is scoped
   to the current project directory and its git worktrees" — run it from the wrong directory and you
   get `No conversation found with session ID`. This dovetails well with `ProjectKey::LocalGit`,
   which already collapses worktrees of a repo into one project; Claude resolves resume-by-id across
   the repository and its worktrees too.
3. **Ids go stale.** Claude's default `cleanupPeriodDays` is 30 days; `/clear` starts a new session;
   the worktree may be deleted. Validate cheaply (does
   `~/.claude/projects/<encoded(cwd)>/<uuid>.jsonl` exist — a path Warp already computes) and render
   stale rows dimmed rather than failing on click.
4. **The capture path depends on the plugin.** No CLI-agent plugin installed for that harness → no
   `session_start` event → no id → no resumable row. The plugin managers already expose
   `can_auto_install`; the rail should prompt to install rather than silently showing nothing.

## 8. Open questions

1. **Granularity of "task."** Given the §5 boundary (agent-backed only), one sub-question remains:
   a tab can hold several conversations (`terminal_panes.conversation_ids` is plural). Is the
   restored unit the conversation, or the reconstructed tab with all of them?
2. **How many closed tasks per project in the rail?** Last N, last 7 days, or "until dismissed"?
   Needs a rule that doesn't make the rail unbounded.
3. **Cloud/ambient tasks.** `get_entries` merges local + cloud. A cloud task has no local
   `working_directory` in the same sense — does it get a project bucket, or land in `Other`?
4. **Deleted worktrees.** When `is_dir()` fails, do we resume in the repo root (the common git dir's
   parent, which `ProjectKey::LocalGit` already knows), prompt, or resume with no cwd? Resuming at
   the repo root seems clearly better than silently landing in the default directory.
5. **Cold-start ordering.** Confirm empirically whether `DetectedRepositories` lag actually produces
   visible wrong-bucketing on the restore path, or whether it settles before first paint. This
   determines whether B is Phase 4 or Phase 1.5.
6. **One list or two?** §7.1 establishes two populations (Warp agent conversations vs. CLI-agent
   sessions). Does the rail merge them into one "tasks" list per project — which means unifying two
   models with different identity, durability, and resume mechanics — or show them as separate
   groups? Merging is nicer UX and materially more work.
7. **OpenCode's resume syntax** is unverified (§7.3). Confirm from OpenCode itself before filling in
   that cell.

---

## Sources

- [Warp #9416 — keep sessions alive across relaunches](https://github.com/warpdotdev/warp/issues/9416)
- [Warp #7760 — agent chats not restored](https://github.com/warpdotdev/warp/issues/7760)
- [Warp #9382 — workspace-style project tabs](https://github.com/warpdotdev/warp/issues/9382)
- [Warp docs — session restoration](https://docs.warp.dev/terminal/sessions/session-restoration/)
- [r/warpdotdev — Is Warp session/window/pane restore broken?](https://reddit.com/r/warpdotdev/comments/1td8ac5/is_warp_sessionwindowpane_restore_broken/)
- [Claude Code — Manage sessions](https://code.claude.com/docs/en/sessions)
- [tmux-resurrect](https://github.com/tmux-plugins/tmux-resurrect) · [restoring tmux sessions after reboot](https://blog.stephansama.info/articles/restoring-tmux-sessions-after-a-system-reboot/)
- [AgentsRoom — restore session](https://agentsroom.dev/features/restore-session)
- [HN — Show HN: Ctx, a /resume across Claude Code and Codex](https://news.ycombinator.com/item?id=47836740)
- [r/ClaudeCode — CCC, Command Center for Claude](https://reddit.com/r/ClaudeCode/comments/1sze0kt/created_a_system_to_manage_multiple_claude/)
- [r/ClaudeCode — session history vanished after reboot](https://reddit.com/r/ClaudeCode/comments/1razdg0/claude_code_session_history_vanished_after_macos/)
- [r/ClaudeCode — managing agents across multiple projects](https://reddit.com/r/ClaudeCode/comments/1v94h7w/how_is_everyone_managing_agents_when_working_on/)
