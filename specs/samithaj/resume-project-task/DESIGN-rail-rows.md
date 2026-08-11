# Design: rail rows resume in place (supersedes the "Recent tasks" pane)

**Source:** synthesized by a 3-reader study of the Orbit codebase
(`/Users/sam/Documents/dev/tools/orbit`) plus direct verification against this checkout.
**Supersedes:** the Recent-tasks-pane design in `plan.md` §Phase B and the first `rail-mock.html`.
**Direction it implements:** no separate pane; the existing per-project rows resume themselves; every
row shows a conversation name, never a repeated truncated path.

> **Verified corrections to earlier drafts** (checked in this checkout, not taken on trust):
> - `agent_session_title()`'s doc comment claims it prefers "a plugin-backed CLI agent's own title
>   (what `/rename` updates)". **It does not.** `cli_agent_title` resolves to `title_like_text()`
>   (`cli_agent_sessions/mod.rs:113-119`), which returns `session_context.summary` — a permission
>   blurb written only by `PermissionRequest`/`QuestionAsked` and wiped by
>   `clear_permission_scoped_state()` (`:177-181`). It is a status string, not a name.
> - **The v1 event protocol has no `title` field at all** (`event/v1.rs` — grep for `title` returns
>   nothing). Warp today has no channel carrying a Claude Code `/rename`.
> - `transcript_path` **does** arrive on the wire (`event/mod.rs:45`, parsed at `event/v1.rs:52`)
>   but `CLIAgentSessionContext` (`mod.rs:44-53`) has no field for it, so it is dropped.

---

# Revised design — the project rail as one list of rows

> Scope: everything below sits behind `project_layout_active` (`app/src/workspace/tab_settings.rs:815`) and `FeatureFlag::Projects`.

---

## 1. What changes

- **The "Recent tasks" pane is deleted from the design.** There is no second list, no second header, no second click target. The rail is one column: project rows, each followed by its own task rows.
- **A task row resumes itself.** The row a user is already looking at is the affordance. Clicking a dormant row opens a tab in that session's cwd and prefills its resume command — the same shape as the existing `continue_third_party_conversation_locally` → `set_pending_command` path (`app/src/workspace/view.rs:13630-13715`, `app/src/terminal/view.rs:9339`). No new surface.
- **`ProjectLayout` stops being a pure function of open tabs.** `ProjectLayout::compute(tabs, ctx)` (`app/src/workspace/project_layout.rs:59`) gains a second input: a durable handle store. This is the one real architectural edit — it is also what fixes "after a restart with no tabs the rail is entirely empty", because a project can now exist with zero open tabs.
- **`transcript_path` stops being thrown away.** It already arrives on the wire (`app/src/terminal/cli_agent_sessions/event/mod.rs:45`) and is already parsed (`event/v1.rs:52`), but `CLIAgentSessionContext` (`cli_agent_sessions/mod.rs:45-54`) has no field for it, so it is dropped on the floor. Persisting it next to `session_id` is the single change that makes transcript-derived naming possible for exited and post-restart rows. **Naming and resuming become one schema change, not two features.**
- **The permission `summary` is demoted out of the name cascade.** Today `cli_agent_title` = `title_like_text()` = `session_context.summary` (`tab_title.rs:237`, `cli_agent_sessions/mod.rs:113-119`), and `summary` is only ever written by `PermissionRequest`/`QuestionAsked` (`mod.rs:225`, `:233`) then wiped by `clear_permission_scoped_state()` (`mod.rs:177`) on the next prompt. It is a status string, not a name. That plus the truncated-cwd fallback is why rows read `..uments/dev/tools/orbit`.
- **Label resolution moves off the render path.** `rail_task_label` (`tab_title.rs:130`) runs synchronously inside element layout with `&AppContext`. It can never read a transcript. A resolver model computes labels off-thread and `notify()`s; the render path only reads a cache.

---

## 2. One row model

A project's children are a single `Vec<RailRow>`. `live: bool` is a field on the row, not a section boundary.

### The structural consequence you cannot skip

`ProjectLayout` is index-parallel to `Workspace::tabs` end to end: `tab_project` / `tab_pane_group_ids` (`project_layout.rs:50-54`), `visible_tab_indices() -> Vec<usize>` (`:107-113`), and the rail loop does `for index in layout.visible_tab_indices(...)` then `self.tabs.get(index)` (`view.rs:22975-22978`). **A dormant row has no tab index.** So the projection must emit a row identity, not a `usize`:

- `RailRowKey::Live(EntityId)` — the pane-group id, already tracked at `project_layout.rs:73`.
- `RailRowKey::Dormant(SessionHandleId)` — a row from the durable store.

`visible_tab_indices` stays exactly as it is and keeps its current callers (tab-bar filtering, `cycle_next`/`cycle_prev` at `project_layout.rs:120-141`, `view.rs:11963`/`:11992`). It remains live-only. A new `rows_for_project()` is what the rail renders.

**Dedup rule:** a dormant handle whose `session_id` matches any live tab's `session_context.session_id` is suppressed; the live row wins. This is Orbit's `seen`-set idea (`src/feed/ingest.ts:131-137`) applied per-session instead of per-host.

### How the two differ visually

Spend the difference only on knobs the render already has (`view.rs:23001-23038`):

| | Live row | Dormant row |
|---|---|---|
| Icon | `terminal_view_agent_icon_variant` + `render_icon_with_status` with the live status ring (`view.rs:22989-23000`) | Same agent icon variant, **status argument omitted**, drawn at `theme.nonactive_ui_detail()` |
| Label colour | `text_color` when it is the active tab, else `muted_color` (`view.rs:23015-23019`) | Always `muted_color` |
| Background | `selected_bg` when active, `hover_bg` on hover (`view.rs:23034-23038`) | **Never `selected_bg`** — a dormant row can never be "active". `hover_bg` on hover only |
| Cursor | `Cursor::PointingHand` | `PointingHand` when resumable; default when not (see §4) |
| Trailing affordance | none | on hover only, a small `↻` glyph at `nonactive_ui_detail` — the only new mark in the whole design |

No divider, no section label, no indent difference. Dormant rows use the same `TASK_ROW_INDENT` (`view.rs:22888`).

### How the two differ behaviourally

This is the clean line, and it is worth stating as the invariant:

> **A live row is a tab. A dormant row is a handle. Live rows are navigable, cyclable and status-bearing; dormant rows are resumable and nothing else.**

Concretely:
- Dormant rows are absent from `visible_tab_indices`, so they never appear on the top tab bar, never participate in `cycle_next`/`cycle_prev`, and never affect the active∈selected invariant.
- **`project_status` (`view.rs:22859`) iterates open tabs only. A project whose rows are all dormant therefore shows no status indicator.** That is correct — a dormant session has no live status to report — but it must be stated rather than left implicit, because it means the project row can look "quiet" while showing five rows.
- Dormant rows are not drag-reorderable and are not drop targets.

---

## 3. Naming

### The blocking fact, stated plainly

**Warp today has no field that carries a Claude Code `/rename` title.** The v1 protocol (`cli_agent_sessions/event/v1.rs:63-88`) has no title field at all; the only title-like input is `summary`, which is a permission blurb (§1). `conversation_display_title` (`tab_title.rs:230`) is the *Oz* conversation title, not the CLI agent's.

So direction #3's literal ask requires a **protocol v2 field** (`title`, emitted on `session_start` and whenever the agent renames), plus a bump to `current_protocol_version()` (`event/mod.rs:79-81`) and a matching release of `warpdotdev/claude-code-warp` (pinned `>= 2.1.0` at `plugin_manager/claude.rs:24`). Everything else in the cascade below works with what Warp has today, and the cascade is designed so that adding tier 1 later is a one-line insertion, not a redesign.

### The cascade — first hit wins, ordered

Rank order and its rationale should be written as a comment block above the resolver, the way Orbit does (`src/feed/labels.ts:1-19`).

| # | Candidate | Source | Refresh |
|---|---|---|---|
| 0 | **User's explicit tab rename** | `pane_group.custom_title` (`tab_title.rs:49`); persisted in `tabs.custom_title` (`crates/persistence/src/schema.rs`, `tabs` table) | On edit. Live rows only. Never overridden. |
| 1 | **Agent-reported conversation title** (`/rename`) | *Does not exist yet* — protocol v2 field, would land on `CLIAgentSessionContext` next to `session_id` (`mod.rs:191-194`) | On every event carrying it |
| 2 | **Oz conversation display title** | `selected_conversation_display_title` (`tab_title.rs:230`) | On conversation update. Live rows only. |
| 3 | **Durable label** from the side table (see below) | resolver-computed, keyed on session UUID | Terminal once written; never goes stale |
| 4 | **Latest live user prompt** | `session_context.query` via `latest_user_prompt()` (`mod.rs:104-110`) | On `PromptSubmit`/`Stop` (`mod.rs:207`, `:216`). Live rows only. |
| 5 | **First real user prompt from the transcript** | bounded head read of `transcript_path` | Memoised on `(path, mtime)`; re-resolved after 30s TTL |
| 6 | **Transcript slug** | the immutable `slug` field near the transcript head | same cache |
| 7 | **Agent + short id** — `"Claude Code · 4f2a1c3d"` | `CLIAgent::display_name()` (`cli_agent.rs:~218`) + `session_id[..8]` | static |

**Explicit rejections, enforced as a filter, not left to luck:**
- The truncated cwd / `display_title()` is **never** a name. Today `rail_task_label` falls through to `tab_title()` → `pane_group.display_title()` (`tab_title.rs:132` → `:58`), which is the entire reported bug. It stays available as a *tooltip* and as the project header text (`project_key.rs:80-101`), never as a row name.
- `summary` is rejected (§1).
- Empty/whitespace is rejected — the guard at `tab_title.rs:160` already does this for live text; the durable store needs the same guard, mirroring Orbit's `AND label <> ''` (`src/feed/labels.ts:165-168`).

**Tier 6 must rank below tier 5.** Borrowed directly, including the reason: Orbit found empirically that ranking the weak slug above the first prompt let a weak label win, get strong-cached, and permanently block the better tier (`src/feed/labels.ts:174-178`).

### What names a row whose agent has EXITED

`remove_session` deletes the `HashMap<EntityId, CLIAgentSession>` entry the instant the agent exits (`cli_agent_sessions/mod.rs:321`, `:418-425`), taking the title and the `session_id` with it. Tiers 1, 2 and 4 all die with it.

The fix is Orbit's central insight, transferred verbatim: **the durable thing lives in a side table keyed by the agent's own session UUID, never on the volatile row** (Orbit `src/db/schema.ts:129-142`, rationale at `src/feed/labels.ts:13-15`).

A new `agent_session_handles` table in `crates/persistence` (alongside `terminal_panes` / `tabs` in `schema.rs`, with a migration under `crates/persistence/migrations/`):

```
(agent, session_id) PK · cwd · transcript_path · first_seen_at · last_seen_at · last_status
```

Written **eagerly** on first sight of a `session_id` — the same opportunistic capture point as `apply_event` (`mod.rs:191-194`) — and `last_seen_at` refreshed on every event. Never written only at exit: a `kill -9` or an app crash would lose it.

An exited row therefore names itself via tier 3 → 5 → 6 → 7. It survives because the handle was written at session *start*.

**Deliberately not persisted:** the project. Derived at read time by feeding `handle.cwd` to the existing `ProjectKey::for_path` (`project_key.rs:40-70`) — the same function live tabs use (`project_layout.rs:84-89`), not a fork. Orbit's rule (`src/feed/ingest.ts:27-42`) and its own lesson about making the resolver shared rather than duplicated.

**Recommended: also not persisted — the label itself.** Persist the handle (ids, cwd, path, timestamps); re-derive tiers 5/6 into the in-memory cache on demand. This is exactly Orbit's split (`src/feed/labels.ts:13-15`) and it keeps user prompt text out of Warp's DB, where it would otherwise sit outside the transcript the user already controls. Tier 3 stays in the design as the slot a future durable label would occupy.

### What names a row after an APP RESTART

Identical to the exited case — after a restart nothing is live, so tiers 3/5/6/7. Session restoration (`app/src/workspace/view.rs:23422`, `GeneralSettings::restore_session`) restores tabs and their cwds but starts a fresh shell with no CLI agent, so restored tabs are live rows with **no** agent session; they name themselves from tier 0 or fall through to tier 7 until an agent starts.

The rail is no longer empty after a restart because projects come from handle cwds as well as tabs.

**Fallback if `transcript_path` was never captured** (old plugin, or the field only arrives on some events): derive the directory with the existing `encode_cwd` (`app/src/ai/agent_sdk/driver/harness/claude_transcript.rs:80-82`) and look for `<config>/projects/<encode_cwd(cwd)>/<session_id>.jsonl`, with `claude_config_dir()` (`:89-96`). **Go cwd → directory only, never directory → cwd** — the slug is lossy (`/` and `.` both become `-`), the same hazard Orbit flags (`src/session-search.ts` ~528).

### Where Orbit transfers and where it does not — naming specifically

**Transfers:** the tiered first-hit-wins cascade with ordering rationale as comments (`labels.ts:146-180`); the durable-side-table-keyed-on-UUID split (`schema.ts:129-142`); one bounded head read serving several consumers (`claude.ts:115-154` serves title, slug and ticket keys from a single 256 KB read — Warp gets tiers 5 and 6 plus cwd verification from one read); per-source cache semantics — weak tiers expire at 30 s so a better name can upgrade mid-conversation, strong tiers never invalidate (`labels.ts:66-89`); explicit junk-name rejection rather than "any non-empty string wins" (`labels.ts:125-133`); and a terminal UI fallback so a row can never render blank (`public/index.html:954`).

**Does not transfer:** Orbit's **T2, the descriptive agent name from `claude agents --json`** (`labels.ts:121-133`, `claude.ts:21-35`) — Warp never enumerates machine-wide sessions and, per the already-decided scope, covers CLI-launched sessions only. Orbit's **LLM label tier** (`labels.ts:382-415`) — cost, the privacy cost of shipping transcript digests off-machine from a terminal app, and no queue/cooldown infrastructure; Warp also has an on-device signal Orbit lacks (the plugin's live `query`), so the marginal value is lower.

---

## 4. Click behaviour per row state

| Row state | Plain click |
|---|---|
| **Live, not active** | Existing `WorkspaceAction::ActivateTaskByPaneGroupId` (`app/src/workspace/action.rs:155`), which also selects the project. Unchanged. |
| **Live, active** | Focus the pane. No navigation. |
| **Dormant, resumable** | Open a new tab in `handle.cwd`, then `set_pending_command(resume_command)` (`app/src/terminal/view.rs:9339`) — **prefilled, not run**. Mirrors `continue_third_party_conversation_locally` (`view.rs:13630-13715`), which already does create-pane-then-prefill for exactly this purpose. The row does not flip to live until the agent actually starts and reports a session. |
| **Dormant, cwd missing on disk** | Non-interactive: default cursor, label at `nonactive_ui_detail`, tooltip naming the missing path. Right-click → **Forget this session**. |
| **Dormant, no known resume verb** (`CLIAgent::Unknown` and the agents without a verified `--resume`) | Open a new tab in `handle.cwd` with no prefill. Hover text says so. |
| **Any dormant row, right-click** | Forget this session · Copy resume command · Reveal transcript in Finder. |

### Resume-command construction gets exactly one owner

`format!("claude --resume {session_id}")` is already hand-rolled in two places (`claude_transcript.rs:256`, `:337`). A third copy is not acceptable. One `resume_command(agent, session_id) -> Option<String>` becomes the single source, and both existing call sites move onto it. This matches this branch's own recent direction (`8063210e6 refactor(workspace): one owner for agent tab-title resolution`).

### Session-id validation is a requirement, not a nicety

The session id comes off disk or off a plugin payload and lands **in the user's shell input buffer**, one Enter away from execution. Ids are unvalidated anywhere in Warp today. `resume_command` must return `None` unless the id passes a strict per-agent charset check:

- **Claude** — canonical UUID shape (`8-4-4-4-12` hex).
- **Codex / Cursor** — `^[A-Za-z0-9_-]{1,64}$`.
- Anything else → the handle is marked non-resumable and the row falls into the "no known resume verb" behaviour above. No shell quoting heroics, no escaping — reject and degrade.

---

## 5. Ordering and overflow within a project

**Ordering — one list, two segments:**

1. **Live rows first, in `Workspace::tabs` order** — precisely what `visible_tab_indices` already returns (`project_layout.rs:107-113`). Do *not* sort live rows by recency: the rail must agree with the top tab bar, and tab order is user-controlled via drag.
2. **Dormant rows after, `last_seen_at` DESC** from the durable store. Dormant rows have no tab order, so the store must carry the sort key.

No separator between the segments. The transition reads as a colour/weight change, nothing more.

**Per-project cap on dormant rows: 5**, plus a `"N more…"` row that expands in place (in-memory per-project flag). Dormant rows must never be able to bury live ones — this is the layout expression of Orbit's rule that a weak signal must not outrank a strong one.

**Store GC.** Cap handles per project key (≈50) and age out at 30 days on startup. Warp cannot prove a session is gone any more than Orbit can prove a silent host is gone, so age out rather than hard-delete on a first miss — Orbit's soft-staleness choice for un-provable absence (`src/feed/ingest.ts:140-166`), and its harder lesson that a hard delete on a transient read destroys tool-owned state (`ingest.ts:131-137`). A handle whose transcript is missing at resolve time is marked orphaned and hidden immediately, deleted at the age window.

**Overflow is largely already solved.** The rail scrolls (`ClippedScrollable::vertical`, `view.rs:23052`) and resizes with min/max bounds (`Resizable`, `view.rs:23075-23083`). Two adjustments for a longer single list:

- Task labels currently use `Text::new`, which soft-wraps deliberately (`view.rs:23011-23014`). With more rows, clamp to **2 lines then ellipsis** so one long dormant name cannot dominate a narrow rail.
- Project rows keep single-line ellipsis as they are (`view.rs:22934`).

**The two existing settings must be resolved, not ignored:**

- **`RailTaskInfo`** (`tab_settings.rs:513-520`) has five variants, two of which — `WorkingDirectory` and `Branch` — reproduce exactly the failure direction #3 forbids. Resolution: `AgentSession` (the default, `:514-515`) is relabelled *"Session name (auto)"* and routes to the cascade in §3 instead of `agent_session_title`. The other four remain explicit user overrides, apply to **live rows only** (a dormant row has no terminal to read a command or branch from), and fall back to the cascade rather than to `tab_title()` — which is the current fallback at `tab_title.rs:132` and the direct cause of the repeated paths.
- **`rail_show_tasks`** (`tab_settings.rs:696-706`, default `true`) no longer gates a decoration; it gates whether the rail has rows at all. Keep it for one release with an updated description, then consider removal: with rows *as* the rail, turning them off leaves a list that only answers "which project", which the top tab bar already implies.

---

## 6. Better ideas from Orbit — and what does not transfer

### Missing from Warp's plan, worth adopting

**a. The resolver must be a model, not a function call on the render path — this is the primary adaptation, not a footnote.** Orbit resolves its cascade inside an HTTP handler and can afford a bounded inline disk read (`src/feed/claude.ts:115-154`). Warp's `rail_task_label` (`tab_title.rs:130`) runs synchronously inside element layout holding `&AppContext`. **You cannot read a transcript there.** So Orbit's on-demand-with-TTL model transfers as an idea but *inverts in mechanism*: a `RailLabels` model resolves off-thread, caches, and `ctx.notify()`s; the render path only ever reads cache-or-fallback. Get this wrong and the rail beach-balls on a slow disk — precisely the class of hazard `AGENTS.md` already warns about for `model.lock()`.

**b. Per-source cache semantics, not one flat TTL** (`labels.ts:66-89`). Weak tiers expire at 30 s so a name can *upgrade* mid-conversation as a better source appears; strong tiers are terminal. A single flat TTL either thrashes or freezes a bad name. Warp has no cache at all today and would land in one of those two failure modes by default.

**c. Explicit junk-name rejection** (`labels.ts:125-133`). Orbit rejects `<cwd-basename>-<2hex>` and bare hex prefixes so the cascade falls through to something human. Warp's analogue is rejecting the truncated cwd *and* the permission `summary` — the two strings producing today's identical rows.

**d. One bounded, mtime-memoised head read serving several consumers** (`claude.ts:108-154`). Warp gets tier 5, tier 6, and cwd verification from a single 256 KB read.

**e. Make "unknown" visible rather than idle** (`src/feed/ingest.ts:44-64`; Orbit's `return 2` with a comment recording that a bare `return 0` silently dropped unclassifiable sessions). Warp's `tab_conversation_status` (`view.rs:22838-22850`) returns `None` for anything it cannot classify, so an unrecognised agent state silently loses its indicator entirely. Adopt the *rule* — surface unknown as a neutral dot rather than nothing — without importing Orbit's 0..3 ladder.

**f. Declare the rebuildability contract in the file header** (`src/db/schema.ts:1-3`). One comment on the handles table saying *"rebuildable index — the agent's own transcript directory is the source of truth"* tells every future reader that the table may be dropped and re-derived, which it can be by scanning `~/.claude/projects`.

**g. Placeholder rows must not masquerade as labels** (`labels.ts:165-168`). Any durable-label read needs the non-empty guard that `tab_title.rs:160` already applies to live text.

**h. Rate-limit diagnostics emitted from a hot path** (`ingest.ts:66-74`). The resolver will run on notify storms; a per-`(agent, session_id)` warn-once set is the equivalent of Orbit's `warnCollision`.

### Does not transfer, with reasons

- **Poll-and-replace discovery via `claude agents --json`** (`src/feed/claude.ts:21-35`, `src/feed/poller.ts:10-19`). Warp is event-driven and pane-scoped — `HashMap<EntityId, CLIAgentSession>` (`cli_agent_sessions/mod.rs:321`) keyed on a Warp entity, not a machine-wide session registry. Enumerating every session on the box directly contradicts the already-decided "CLI-launched sessions only" scope: it would surface sessions Warp never launched, in terminals Warp does not own. Warp also supports 15 agents (`cli_agent.rs:140-158`), most of which have no enumeration CLI at all.
- **The `null`-vs-`[]` transient-read sentinel** (`claude.ts:29`, `:33`). Excellent discipline, but it guards a *prune*, and Warp has no snapshot and no prune. Its spirit survives as the GC rule in §5: never delete a handle because a read failed; only age it out.
- **The LLM label tier, its queue, cooldown and `session_label` job** (`labels.ts:382-415`, `schema.ts:129-142`). Cost, privacy, no job infrastructure, and a weaker payoff because Warp already has the live `query`.
- **Multi-host staleness sweeping and host-as-prune-bucket** (`ingest.ts:140-166`, `src/app.ts:443-454`). Single-machine feature. Remote SSH sessions are modelled as `remote_host` on the live session (`cli_agent_sessions/mod.rs:~137`), and v1 should simply not persist handles for them — the transcript is on the far side.
- **Cross-host id-collision guard** (`ingest.ts:80-88`). Not applicable; Warp's key is `(agent, session_id)`, local.
- **The `reviewed` bit and the 0..3 attention ladder as stored columns** (`schema.ts:119`, `src/attention.ts:12`). Warp's status is already derived per-render from `ConversationStatus` (`view.rs:22838-22850`) and the rail has no triage workflow. Import the "unknown is visible" rule (e); do not import the ladder.
- **Orbit's never-expiring `repoCache`** (`ingest.ts:12`) is called out in the findings as a bug, and Warp should explicitly not copy it: `DetectedRepositories` (`crates/repo_metadata/src/repositories.rs:40`) already owns repo detection with real invalidation, and `ProjectKey::for_path` already consults it (`project_key.rs:48-56`).

---

## 7. Open questions

1. **Does the Warp Claude plugin emit `transcript_path` on `session_start`, or only on `stop`?** `event/v1.rs:73` accepts it on any event, but the hook set lives in `warpdotdev/claude-code-warp` (pinned `>= 2.1.0`, `plugin_manager/claude.rs:24`) and cannot be checked from this repo. If it only arrives on `stop`, a long-running session has no path until it ends, and the `encode_cwd` fallback (`claude_transcript.rs:80-82`) carries tiers 5/6 in the meantime. Same question for the Codex, Gemini and OpenCode plugins.
2. **Is a protocol v2 `title` field in scope?** Direction #3's literal ask needs one (§3). Bumping `current_protocol_version()` (`event/mod.rs:79-81`) means a coordinated plugin release. And does an equivalent of `/rename` even exist for Codex, Gemini and Cursor, or is tier 1 Claude-only?
3. **Which agents get durable handles in v1?** Resume verbs are known for Claude, Codex and Cursor. The other twelve (`cli_agent.rs:140-158`) have no verified verb. Ship resume for three and dormant-naming-only for the rest, or gate handles to the three entirely?
4. **Remote sessions:** hide dormant rows for `remote_host` sessions, or show them as non-resumable? The transcript is unreachable, so every tier below 4 fails and they would all render as tier 7.
5. **Store location:** a diesel table in `crates/persistence` (migration + existing connection, consistent with `tabs` / `terminal_panes`) or a standalone JSON file (no migration for what is, by Orbit's framing, a rebuildable index)? Recommendation is the table, for the migration story alone — but it should be an explicit call.
6. **Does the store sync to cloud?** `TabSettings` sync via `SyncToCloud` (`tab_settings.rs:526`), but handles carry machine-local absolute paths. Recommendation: no.
7. **Persist the label, or re-derive it every time?** §3 recommends re-derive (matching `labels.ts:13-15`) so prompt text stays out of Warp's DB. If tier 3 is ever populated durably, this needs a privacy decision first.
8. **Does a resumed session keep its id?** The dedup key is `session_id`. `claude --resume <uuid>` keeps the uuid; Codex's `resume` may mint a new rollout id. If it does, the dormant row will not flip to live and the user sees a duplicate. Needs per-agent verification, and possibly a `resumed_from` link column.
9. **Does the `"N more…"` expansion state persist across restarts,** or reset per launch? In-memory is simpler and probably right.
10. **`rail_show_tasks` deprecation:** keep for one release with a reworded description, or remove now that rows *are* the rail?
---

## 8. Resolved: `transcript_path` emission, and what to reuse from Orbit

*Added after reading the installed plugin (`~/.claude/plugins/cache/claude-code-warp/warp/2.2.0`)
and Orbit's `src/feed/labels.ts` / `src/feed/claude.ts`. Both were open questions in §7.*

### 8.1 Open question #1 — answered: `stop` only, and it does not matter

`transcript_path` is passed as an extra `--arg` by exactly two scripts — `on-stop.sh` and
`on-stop-failure.sh`. `on-session-start.sh` sends only `plugin_version`. `build_payload`
(`scripts/build-payload.sh`) puts `session_id`, `cwd` and `project` on **every** event; everything
else is per-event.

The plugin also pins `PLUGIN_CURRENT_PROTOCOL_VERSION=1` and negotiates
`min(plugin, WARP_CLI_AGENT_PROTOCOL_VERSION)` — so a protocol v2 `title` field (naming tier 1)
requires a coordinated plugin release, confirming §3.

**But Warp does not need the hook for the path.** `encode_cwd` already exists
(`claude_transcript.rs:80-82`) and implements Claude's convention. Since `cwd` and `session_id`
arrive on every event, Warp can construct the path itself:

```
~/.claude/projects/<encode_cwd(cwd)>/<session_id>.jsonl
```

Verified against real state on this machine: `/Users/sam/Documents/dev/zuellig/repos/poa-agent`
encodes to `-Users-sam-Documents-dev-zuellig-repos-poa-agent`, which **is** the on-disk directory
name, and `61f785ca-….jsonl` sits inside it. There is even a standing
`TODO(REMOTE-1209): Use the transcript path reported by our hook` at `claude_transcript.rs:88`.

**Decision:** derive the path from `(cwd, session_id)`; treat the hook-supplied `transcript_path` as
a confirmation when it arrives. This removes the tier-5/6 dependency on the `stop` hook entirely, so
naming works from the first event of a session rather than the first completed turn.

*Caveat:* `encode_cwd` replaces `/` **and** `.`; confirm that matches Claude for paths containing
other punctuation before relying on it as the only source.

### 8.2 What to reuse from Orbit

Orbit is TypeScript/Bun and Warp is Rust, so nothing ports as code. The *designs* port directly, and
`labels.ts:1-19` is effectively a written spec for this problem.

**Adopt:**

| Orbit idea | Where | Why it matters for Warp |
|---|---|---|
| **A side table keyed by the Claude UUID**, because "the feed clobbers `session.name` every 4s and DELETEs rows on exit, so nothing on the session row is durable" (`labels.ts:12-15`) | `labels.ts` | Orbit hit **precisely** Warp's `remove_session` bug and reached the same answer. Independent validation of the handles table. |
| **Tier 1 — `session-memory/summary.md`** at `~/.claude/projects/<encoded-cwd>/<uuid>/session-memory/summary.md` | `labels.ts:6` | A real, human-written title Warp's cascade did not know about. **Verified present on this machine.** Cheap read, no LLM. ⚠️ The one found here is an *unfilled template* ("_A short and distinctive 5-10 word…_") — guard against placeholder text. |
| **Junk-name rejection of `<cwd-basename>-<2hex>`** | `labels.ts:7` | This is Claude's own auto-generated display name (`my-app-3f`), which its docs confirm is *not* a resume handle. Warp must reject it exactly as it rejects the truncated cwd. |
| **Bounded reads: 256 KB head, 64 KB tail** | `labels.ts:16-17` | Concrete numbers, already load-tested by Orbit. Serves tier 5, tier 6 and cwd verification from one read. |
| **`null` vs `[]` distinction** — a transient read error returns `null` so the caller knows **not** to prune; `[]` means genuinely no sessions | `claude.ts:5-7` | Directly applicable to handle GC. Prevents a hiccup from wiping the rail. |
| **Remote rows touch no disk** | `labels.ts:18` | Answers §7 Q4: remote sessions get tier 7 naming and no filesystem access. |
| **Kill switches** (`ORBIT_LABELS_DISABLE`, `ORBIT_LABEL_LLM=off`) | `labels.ts:19` | A feature reading user transcripts needs an off switch independent of the feature flag. |

**Do not adopt:**

- **The LLM labelling tier (T3).** Orbit fires an async model call over a <3 KB digest. For Warp this
  adds cost, latency and a privacy question to what must be a rail label. The free tiers plus
  `session-memory/summary.md` should carry it.
- **`claude agents --json` as the session source** (`claude.ts:22`). It enumerates *running* agents,
  which Warp already knows from its own event stream; it says nothing about historical sessions,
  which is the actual gap. Useful only as a cross-check.
- **Orbit's 0..3 status ladder.** Warp has its own status model; adopt only the *rule* that unknown
  states surface as a neutral marker rather than disappearing (§6e).
