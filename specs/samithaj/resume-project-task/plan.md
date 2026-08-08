# Plan v4: resume a coding-agent task from the project rail

**Status:** Phases A + B + C implemented behind `FeatureFlag::ResumeProjectTasks` (2026-08-04) and
**validated end to end in a real build**: an agent session's conversation name survives the agent
exiting and a full app quit/relaunch, its project is derived from the session's own cwd, and
clicking the row resumes it. Phase D (Agent Mode project filter) not started; Cursor support and
the preview-before-resume step (D11) still open.

Three defects only a real build exposed, each now fixed and regression-tested:
1. `ResumeProjectTasks` sat in `DOGFOOD_FLAGS`, which OSS builds deliberately skip — nothing was
   ever written. Added to the local opt-in in `app/src/bin/oss.rs` alongside `Projects`.
2. Matching a tab to its session by directory let two sessions in one repo borrow each other's
   names, and duplicated a restored tab with its own handle. Now matched on `terminal_panes.uuid`.
3. Pane uuid **alone** was still wrong: a shell that has `cd`-ed away — or a restored pane reopening
   at its own startup directory — kept claiming the session, so the task was filed under the wrong
   project and resume ran in the wrong directory (`No conversation found with session ID`). A tab
   now hosts a session only when both its pane uuid *and* its current directory match.
**Supersedes:** plan v3 in full. History: [`REVISION-01`](./REVISION-01.md) (v1→v2),
[`REVISION-02`](./REVISION-02.md) (v2→v3). Research: [`brainstorm.md`](./brainstorm.md).
Row design detail: [`DESIGN-rail-rows.md`](./DESIGN-rail-rows.md). Mock: `rail-mock.html`.
**Gate:** `project_layout_active(ctx)` — `FeatureFlag::Projects` + `appearance.project_layout.enabled`
(`app/src/workspace/tab_settings.rs:815`), plus a new flag of its own.

---

## 1. The problem, in one paragraph

`CLIAgentSessionsModel` is `HashMap<EntityId, CLIAgentSession>`
(`app/src/terminal/cli_agent_sessions/mod.rs:321`), and `remove_session` (`:418`) deletes the entry
when the agent exits. **The conversation title and the `session_id` live in that same live-only
object**, so both vanish on agent exit and again on app restart. The rail then falls through
`agent_session_title() → None → tab_title() → display_title()` to a truncated cwd — which is why six
rows under `poa-agent` all read `..uellig/repos/poa-agent`. Separately,
`ProjectLayout::compute(tabs)` (`app/src/workspace/project_layout.rs:59`) derives **both** the
project list and the rows by iterating open tabs, so after a restart the rail is entirely empty.

**Naming and resuming are therefore the same problem: the row's identity is ephemeral.** Give the
row a durable record and it keeps its name *and* gains a resume handle.

## 2. Goal

Select a project in the rail → see its tasks by name → preview one to confirm it's the right work →
resume it, in its own directory, with the agent's context. Works after a Mac restart and does not
depend on how Warp was quit.

## 3. Non-goals

- Keeping PTYs alive across relaunch ([#9416](https://github.com/warpdotdev/warp/issues/9416)).
- Replacing window-snapshot session restoration; this is a complementary *pull* mechanism.
- Plain (non-agent) terminal tabs — snapshot-only.
- **Warp Agent Mode runs.** They already have a durable conversation record and a working restore
  path; they are Phase D and never enter the handles table.

---

## 4. Model

### 4.1 A row is the task

The rail is the index of a project's tasks; the horizontal tab bar is what's currently loaded.
Whether Warp holds a tab for a task right now is an implementation detail, not a user-facing
category.

**One list per project. `live` is a property of a row, not a section.** No "Recent" header, no
divider, no second pane.

| | Live row | Dormant row |
|---|---|---|
| Backed by | an open tab (`EntityId`) | a stored handle (`SessionHandleId`) |
| Icon | agent icon **with** status ring (`view.rs:22989-23000`) | same icon, status omitted, `nonactive_ui_detail` |
| Text | `text_color` when active, else `muted_color` | always `muted_color` |
| Background | `selected_bg` when active | never `selected_bg`; `hover_bg` only |
| Trailing | — | hover-only `↻` |
| On the tab bar | yes | **never** |
| In `cycle_next`/`prev` | yes | **never** |
| Click | activate the tab | **preview** (no side effect) |

### 4.2 Click is asymmetric on purpose

Activating an existing tab is free and reversible. *Creating* one is a real side effect. Same
gesture, different consequences — so the dormant path gets a deliberate second step:

- **Click a dormant row → preview only.** Nothing is created.
- Preview shows: name, agent, relative time, branch, message count, full cwd, **the last exchange**
  (your last message + the agent's last reply), and the exact resume command.
- **Resume** commits. Also: *Copy command*, *Forget*.

The last exchange is the identifying signal — a title alone doesn't tell you where the work stopped.

### 4.3 Resuming selects the project

The tab bar is project-scoped. Resuming task *T* of project *P* must select *P* and switch the bar,
or the new tab lands in a bar the user cannot see.

### 4.4 Placement follows the existing preference

The button says **Resume**, not "Resume in new tab" — placement is a preference, not a promise.
Honour `open_conversation_layout_preference`
(`app/src/util/file/external_editor/settings.rs:123`) and the existing
`RestoreConversationLayout { ActivePane, SplitPane, NewTab }` (`app/src/workspace/action.rs:71`).

*Note:* right after a restart — the case this feature exists for — the target project has no panes,
so placement resolves to `NewTab` regardless of the preference. That is correct, not a bug; say so in
any debugging notes.

⚠️ **Never restore into a pane belonging to a different project.** A pane's project is derived from
its cwd (`project_of_tab_data`, `project_layout.rs:84`), so reusing a foreign pane would rewrite that
pane's cwd and silently re-bucket an existing tab. `ActivePane`/`SplitPane` are legal only inside the
target project; otherwise fall back to `NewTab`.

### 4.5 Ordering and overflow

Live rows first in `Workspace::tabs` order (so the rail agrees with the tab bar — never sort live
rows by recency), then dormant rows by `last_seen_at DESC`. No separator. **Cap dormant rows at 5**
with an in-place `N more…`; dormant rows must never bury live ones.

*Consequence to accept knowingly:* `project_status` (`view.rs:22859`) iterates open tabs, so a
project whose rows are all dormant shows no status dot. Correct, but it means a project can look
quiet while listing rows.

---

## 5. Phase A — durable handles

### Schema

`crates/persistence/migrations/<date>_add_agent_session_handles/up.sql`

```sql
-- Rebuildable index. The agent's own transcript directory is the source of truth;
-- this table may be dropped and re-derived by scanning ~/.claude/projects et al.
CREATE TABLE agent_session_handles (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    agent         TEXT    NOT NULL,  -- CLIAgent::to_serialized_name()
    session_id    TEXT,              -- NULL until the agent reveals it (Codex, Cursor)
    cwd           TEXT    NOT NULL,  -- CANONICAL. Project is derived from this, never stored.
    pane_uuid     BLOB    NOT NULL,  -- provenance + in-flight slot key. NOT task identity.
    title         TEXT,              -- CACHE of the last resolved label (see §6.2 tier 0)
    created_at    TIMESTAMP NOT NULL,
    last_seen_at  TIMESTAMP NOT NULL
);

-- Task identity: one row per upstream session, whichever pane resumed it.
CREATE UNIQUE INDEX idx_ash_task
    ON agent_session_handles (agent, session_id) WHERE session_id IS NOT NULL;

-- In-flight: at most one un-identified launch per pane per agent.
CREATE UNIQUE INDEX idx_ash_inflight
    ON agent_session_handles (pane_uuid, agent) WHERE session_id IS NULL;

CREATE INDEX idx_ash_recent ON agent_session_handles (last_seen_at);
```

Both partial indexes are verified against SQLite: a second NULL row for the same `(pane, agent)` is
rejected; the same `session_id` from a different pane is rejected.

### Identity

`(agent, session_id)` is the task. `pane_uuid` is provenance. Every id in scope is a UUID (Warp mints
Claude's with `Uuid::new_v4()`; Codex's rollout id is a `Uuid`; Cursor's chat dirs are UUIDs), so no
cwd scoping is needed — **check before adding an agent with only locally-unique ids.**

**Lifecycle:** insert at `session_start` with `session_id = NULL` keyed by `pane_uuid`; `UPDATE` on id
arrival. If that id already exists as a task row (a resume of a known session), merge — refresh
`last_seen_at`/`pane_uuid` on the existing row and delete the in-flight row. NULL-id rows render as
"starting…" and are never resumable.

`pane_uuid` = `terminal_panes.uuid`, already `Binary`, persisted (`schema.rs:407`) and written back on
save (`sqlite.rs:1253`) — **not** `EntityId`, which is in-memory.

### Write path — one, not two

| Trigger | Where |
|---|---|
| plugin `session_start` → insert in-flight row | `cli_agent_sessions/mod.rs:191-194`, `:389-390` |
| `session_context.session_id` becomes `Some` → identify/merge | same |
| subsequent events → throttled `last_seen_at` touch | same |

Only agents where `is_agent_supported()` is true (`listener/mod.rs:39` — Claude, OpenCode, Codex,
Gemini, Auggie, Droid, Pi, OhMyPi) produce events at all. **Agents without a handler produce no rows
whatsoever** — not NULL rows, no rows. Today that excludes Cursor, Amp, Copilot, Goose, Hermes, Vibe,
Antigravity.

**Warp Agent Mode drivers do not write here.** `claude_code.rs`/`codex.rs` are untouched by Phase A.

> ⚠️ **Invariant — `remove_session` must not delete the handle.** The live→dormant transition
> happens exactly at `cli_agent_sessions/mod.rs:418`, and symmetry will tempt an implementer to add
> handle cleanup there. **That would delete the feature.** `remove_session` clears in-memory state
> only; the handle survives by design — it *is* the dormant row. It should stamp `last_seen_at` on
> the way out, since dormant ordering depends on that field. Cover this with a test named for the
> invariant, not for the function.

### GC

Cap ≈20 handles per project and age out at 30 days (aligned with Claude's `cleanupPeriodDays`).
Borrow Orbit's `null` vs `[]` distinction (`orbit/src/feed/claude.ts:5-7`): a transient read error
must **not** prune. A handle whose transcript is missing is marked orphaned and hidden immediately,
deleted at the age window.

---

## 6. Phase B — the naming resolver

### 6.1 It cannot live on the render path

`rail_task_label` (`tab_title.rs:130`) runs synchronously inside element layout holding
`&AppContext`. **It can never read a transcript.** A `RailLabels` model resolves off-thread, caches,
and `ctx.notify()`s; the render path only ever reads cache-or-fallback. Getting this wrong
beach-balls the rail on a slow disk — the hazard `AGENTS.md` already warns about for `model.lock()`.

Orbit resolves its cascade inline in an HTTP handler (`orbit/src/feed/labels.ts`); the *idea*
transfers, the *mechanism* inverts.

### 6.2 Cascade — first hit wins

| # | Candidate | Source | Refresh |
|---|---|---|---|
| 0 | **Cached label** from `agent_session_handles.title` | the store | written whenever a higher tier resolves; lets a dormant row paint its name with **no disk read at all**, then upgrade in place when the resolver lands |
| 1 | Explicit tab rename | `pane_group.custom_title` (`tab_title.rs:49`) | on edit; live only; never overridden |
| 2 | **`ai-title` from the transcript** ⭐ | `{"type":"ai-title","aiTitle":…}` records in `<uuid>.jsonl` | **last** occurrence wins; re-read on mtime change |
| 3 | Session-memory summary | `~/.claude/projects/<enc-cwd>/<uuid>/session-memory/summary.md` | on mtime change |
| 4 | Latest user prompt | `session_context.query` (`mod.rs:104-110`) | on `PromptSubmit`/`Stop`; live only |
| 5 | First user prompt in transcript | bounded head read | memoised on `(path, mtime)`, 30 s TTL |
| 6 | `Claude · 998add3f` | `CLIAgent` + `session_id[..8]` | static |

### ⭐ Tier 2 is the workhorse, and it already exists on disk

Claude Code writes AI-generated session titles **into the transcript** as `ai-title` records — and
`/rename` values land in the same field (kebab-case entries such as `per-branch-whatsapp-routing`
appear alongside sentence-case generated ones). Measured over a random 200-transcript sample on this
machine:

| Session size | Has `ai-title` |
|---|---|
| tiny (< 6 messages) | **0 %** |
| real (≥ 6 messages) | **87 %** |
| substantial (≥ 20 messages) | **100 %** |

Real values: *"Add Jira search and fix orbit dashboard startup"*, *"Debug cargo install compilation
error for imessage-exporter"*.

**Two consequences that change the plan's shape:**

1. **The protocol-v2 `title` field is no longer on the critical path.** The `/rename`-quality name is
   already readable from disk with the same bounded read the cascade performs anyway. A v2 event
   field remains a nice-to-have for *live* rows (avoids a disk read while an agent is running), not a
   prerequisite for the feature. This removes the coordinated `claude-code-warp` release from the
   dependency chain.
2. **`ai-title` presence is a quality signal.** 192 of 200 sampled transcripts have fewer than six
   messages — abandoned or trivial. See §6.5.

*Verify before relying on it:* `ai-title` is an undocumented internal record type. Treat a missing
field as a normal cascade fall-through, never an error, and keep tiers 5–6 as the guaranteed floor.

**Rejected as names, enforced as a filter:**
- **Truncated cwd / `display_title()`** — today's behaviour and the entire reported bug. Stays as a
  *tooltip*, never a row name.
- **Permission `summary`** — `cli_agent_title` currently resolves to `title_like_text()` →
  `session_context.summary` (`mod.rs:113-119`), written only by `PermissionRequest`/`QuestionAsked`
  and wiped by `clear_permission_scoped_state()` (`:177`). It is a status blurb.
  ⚠️ `agent_session_title()`'s doc comment (`tab_title.rs:22-23`) claims it prefers the agent's own
  `/rename` title. **It does not.** Fix the comment.
- **Claude's auto name `<dir>-<2hex>`** (e.g. `poa-agent-0f`) — Claude's docs confirm it is not a
  resume handle. Orbit rejects it too (`labels.ts:7`).
- Empty/whitespace, and unfilled `summary.md` templates (the one on this machine is a placeholder:
  `_A short and distinctive 5-10 word descriptive title_`).


### 6.5 Only surface substantive sessions

There are **9,400** transcripts on this machine; ~96 % of a random sample were under six messages.
Listing them all would be unusable regardless of naming.

**Filter: a handle is only shown once its session has ≥ 6 messages** (equivalently: once it has an
`ai-title`, which is nearly the same set). This is a display filter, not a write filter — the handle
row is still written at `session_start` so the in-flight state is tracked. It also gives GC a cheap
first pass.

Every tier above works with what Warp has today — no protocol bump is on the critical path. A
future protocol-v2 `title` event (`current_protocol_version()`, `event/mod.rs:79-81`, plus a
coordinated `claude-code-warp` release — the installed plugin pins
`PLUGIN_CURRENT_PROTOCOL_VERSION=1`) would only optimise *live* rows by avoiding a disk read; it
slots between tiers 1 and 2 as a one-line insertion later, not a redesign.

### 6.3 Transcript path is derived, not awaited

`transcript_path` is emitted **only** by `on-stop.sh` / `on-stop-failure.sh`; `session_start` sends
only `plugin_version`. Warp does not need it: `encode_cwd` (`claude_transcript.rs:80-82`) plus
`session_id` and `cwd` — which arrive on *every* event — give

```
~/.claude/projects/<encode_cwd(cwd)>/<session_id>.jsonl
```

Verified on disk: `/Users/sam/Documents/dev/zuellig/repos/poa-agent` →
`-Users-sam-Documents-dev-zuellig-repos-poa-agent`, which is the real directory, with
`61f785ca-….jsonl` inside. Treat a hook-supplied `transcript_path` as confirmation.
*Caveat:* `encode_cwd` replaces `/` and `.`; confirm against paths with other punctuation.

### 6.4 Two reads, two budgets

- **Head, 256 KB** — naming (tiers 5/6). Memoised on `(path, mtime)`. Off the render path.
- **Tail, 64 KB** — preview's last exchange. **On selection only, one row at a time.** Never per row
  during render.

Both sizes are Orbit's (`labels.ts:16-17`).

---

## 7. Phase C — rail integration

`ProjectLayout::compute` gains a second input (the handle store) and must emit a row *identity*, not
a `usize`, because a dormant row has no tab index:

```
RailRowKey::Live(EntityId)          // pane-group id, already tracked at project_layout.rs:73
RailRowKey::Dormant(SessionHandleId)
```

`visible_tab_indices` (`:107-113`) stays exactly as it is and stays **live-only** — it keeps its
current callers (tab-bar filtering, `cycle_next`/`cycle_prev` at `:120-141`). A new `rows_for_project()`
is what the rail renders.

**Dedup:** a dormant handle whose `session_id` matches a live tab's is suppressed; the live row wins.

**Preview surface:** reuse the existing hover-details sidecar machinery — `show_details_on_hover`
(`tab_settings.rs:773`) with the safe-triangle overlay (`vertical_tabs.rs:492-520`,
`VERTICAL_TABS_DETAIL_SIDECAR_POSITION_ID`). Hover gives a light preview; click pins it.

**Mocks first** — [#9382](https://github.com/warpdotdev/warp/issues/9382) is `needs-mocks` for this
exact UI. `rail-mock.html` is the current proposal.

---

## 8. Phase D — Warp Agent Mode population

Add `ProjectFilter { All, Specific(ProjectKey) }` to `AgentManagementFilters`
(`agent_conversations_model.rs:318`), mirroring `EnvironmentFilter`. Resolve
`display.working_directory` → `ProjectId` via **the same shared memoized helper as Phase A** — never
`ProjectKey::for_path` inside the per-entry predicate. Resume uses the existing
`restore_conversation_in_new_tab`, which already restores cwd.

---

## 9. Command safety

Session ids are **externally sourced and unvalidated today** — plugin hook JSON, Codex rollout files,
Cursor's `meta.json`. No `Uuid::parse_str`, no sanitisation anywhere in `cli_agent_sessions`.

1. **Validate on ingest:** accept only `[A-Za-z0-9_-]{1,64}`. Reject → the row is never resumable.
2. **Reject control characters, newlines above all.** Prefill is *not* self-securing: a newline
   inside an id would submit the line the moment it lands in the input.
3. **Quote at the boundary.** Never `format!`-splice a raw id; defence in depth for *Copy command*,
   where the string leaves Warp.
4. Same rules for `cwd` (used as a launch dir and possibly `cd <cwd>`).
5. **Never emit `--dangerously-skip-permissions`.** `claude_command()` (`claude_code.rs:210-218`)
   hardcodes it for Warp's headless driver; `resume_command.rs` is separate and plain.
6. **Adversarial tests are a ship blocker**, not a follow-up.

---

## 10. Testing

| Layer | What |
|---|---|
| Unit | `resume_command_tests.rs` — exact string for all **16** `CLIAgent` variants incl. `Unknown` → `None`; assert `--dangerously-skip-permissions` never appears; Cursor always emits an id (bare `--resume` opens its own picker) |
| Unit | **Injection suite** — `;` `` ` `` `$( )` `&&` quotes spaces **newlines** non-ASCII, each rejected at ingest; explicit newline-in-prefill test |
| Unit | `agent_session_handles_tests.rs` — in-flight insert then identify; second NULL row for same `(pane, agent)` rejected; same `session_id` from another pane **merges, not duplicates**; two distinct sessions on one pane → two rows |
| Unit | Naming cascade — each tier in isolation; rejection of truncated cwd, permission `summary`, `<dir>-<2hex>`, and unfilled `summary.md` templates |
| Unit | cwd→project: a handle written before repo detection completes buckets identically to the live tab once detection lands (`LocalDir`→`LocalGit` upgrade) |
| Unit | `encode_cwd` round-trip against a real `~/.claude/projects` directory name |
| Unit | **Preview degradation** — transcript deleted between render and click, truncated JSONL, corrupt/partial lines, zero-byte file: preview shows “no preview available” and Resume stays enabled; never errors, never blocks |
| Integration | GUI: click dormant row → preview appears, **no tab created**; Resume → tab opens at stored cwd, command prefilled **and not executed**, project selected, tab bar switched (`crates/integration`) |
| Manual | Reboot cycle per agent; **quit via the window close button, not Quit** — the case Warp's own restore does not cover |

---

## 11. Risks

| Risk | Mitigation |
|---|---|
| Ids are attacker-influenced and reach a shell | §9 in full; adversarial tests block ship |
| Transcript read on the render path freezes the UI | Resolver model off-thread; render reads cache only |
| Handle table drifts from the agent's on-disk state | Rebuildable cache; validate on render; `null`≠`[]` on read errors |
| Restoring into a foreign project's pane re-buckets it | §4.4 guard; fall back to `NewTab` |
| Cursor's `~/.cursor/chats` layout is undocumented | Check `schemaVersion`; fail soft; ship last |
| Rail grows unbounded | Cap 5 dormant + `N more…`; GC at 20/30 days |
| Chatty writes per plugin event | Insert/identify on transitions only; throttle `last_seen_at` |

---

## 12. Decisions

| # | Decision | Status |
|---|---|---|
| D2 | ~~Two groups in the rail~~ → **one list, `live` as a row property** | **superseded in v4** |
| D3 | ≈20 handles/project, 30 days | settled |
| D4 | v1 agents: Claude, then Codex + OpenCode | settled |
| D5 | New table, not an extension | settled |
| D6 | Full history; identity `(agent, session_id)` | settled |
| D7 | Handles cover CLI-launched only; Agent Mode is Phase D | settled |
| D8 | Prefill, don't execute | settled |
| D9 | Zero-storage "continue at project root" phase dropped | settled |
| D10 | Project derived from `cwd` at read time, never persisted | settled |
| **D11** | **Click a dormant row = preview only; Resume is a separate commit** | **new in v4** |
| **D12** | **Verb is "Resume"; placement follows `open_conversation_layout_preference`** | **new in v4** |
| **D13** | **Never restore into a pane of a different project** | **new in v4** |

---

## 13. Sequencing

**A** (handles + `resume_command` + safety tests) → **B** (naming resolver) → **C** (rail, after
mocks sign-off) → **D** (Agent Mode filter) → Cursor, behind an `~/.cursor/chats` fs-watch spike.

A and B are independently testable without any UI — but **land A and B together behind the flag**,
with a minimal read surface (a debug command that dumps resolved handles per project). A table
nothing reads for two phases can be silently wrong for a long time; a dump command makes Phase A
verifiable the day it lands. C is the first user-visible change and the one gated on mocks.
