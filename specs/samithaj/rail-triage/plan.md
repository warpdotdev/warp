# Rail triage: priority ranking, waiting-on-you prominence, nag-until-unblocked

**Status:** v1 — decided with Sam 2026-08-05, all four open questions answered. Mock:
[`rail-mock.html`](./rail-mock.html).
**Grounding:** infra map of existing status/notification/ordering machinery (agent survey,
2026-08-05, file:line refs throughout); prior-art research (Claude Code hook ecosystem,
multi-agent managers, incident-tooling escalation).
**Related:** [`../resume-project-task/plan.md`](../resume-project-task/plan.md) (the rail),
[`../lazy-shell-startup/plan.md`](../lazy-shell-startup/plan.md).

---

## 1. Problem

The rail names every task but treats them identically: a rank-1 project's agent blocked on a
permission prompt renders exactly like a scratch project idling for a week. Sam wants:

1. **Priority**: an ordered rank over projects, user-managed, changing over time — and consumed
   later by per-project coding-agent **token budgets**, so rank must be a readable attribute, not
   just a sort.
2. **Prominence**: agents *waiting on the user* visible at a glance, in place — no duplicate
   "waiting" list; rank already puts important projects on top.
3. **Persistence**: sound + OS notifications that **repeat until the agent is actually
   unblocked**, scaled by priority.

## 2. What already exists (infra map highlights)

| Piece | Where | State |
|---|---|---|
| "Waiting on user" state | `CLIAgentSessionStatus::Blocked` (`cli_agent_sessions/mod.rs:28-38`), set from plugin OSC 777 `PermissionRequest` / `QuestionAsked` events | exists, first-party |
| Desktop notification on block | `NotificationsTrigger::NeedsAttention` (`terminal/view.rs:779-785`, fire path `:13270-13417`) | exists — single-shot, unfocused-only |
| Settings gates + sound flag | `NotificationsSettings` (`session_settings.rs:70-95`) | exists |
| Per-row status badge | `IconWithStatusVariant` ring badge (`ui_components/icon_with_status.rs:436-496`) | exists |
| Project-header badge | `render_status_element` + `project_status` (`workspace/view.rs:23151-23164`) | exists, **broken — see §3** |
| Project ordering | first-seen insertion order only (`project_layout.rs:85-123`) | no sort, no persistence |
| Per-directory persisted settings precedent | `DirectoryTabColors` settings map (`tab_settings.rs:170-230`) | pattern to copy |
| Recurring timer pattern | self-rescheduling `Timer::after` + `AbortHandle` (`heartbeat.rs:90-103`) | pattern to copy; no interval primitive exists |

## 3. Phase 0 — two status bugs that block everything (fix regardless of design)

1. **The project-header aggregate cannot see a CLI agent's `Blocked`.** `project_status` →
   `tab_conversation_status` (`workspace/view.rs:23130-23164`) returns `InProgress` for any
   long-running pane and otherwise reads only `BlocklistAIHistoryModel`; CLI-agent status reaches
   that model only for orchestrated child conversations. A user-typed `claude` blocked on a
   permission prompt never changes the header — today's crescent means only "something runs here".
   Fix: aggregate must consult `CLIAgentSessionsModel` per pane, the same source the row badge
   uses (`terminal_view_agent_icon_variant`).
2. **No `blocked_since` timestamp.** `Blocked { message }` carries no time; the orange→red
   escalation (§5) and the nag engine (§6) both need one. Add it where the status is set
   (`mod.rs:236-250`).
3. *(Smaller, same area)* command-detection-only sessions (`listener: None`) show **no badge at
   all** (`agent_icon.rs:149-150`) — not even "running". Give them a neutral running badge so a
   plugin-less agent is distinguishable from a plain shell.

## 4. Priority: ordered rank in its own UI

- **Model:** one ordered list of projects. Rank 1..N for ranked projects; everything else keeps
  first-seen order below a divider. No numeric levels, no scores.
- **UI:** "Project Priorities" panel — drag-to-reorder list of every project the rail knows.
  Entry points: command palette entry (per AGENTS.md rule for toggleables), right-click on the
  rail's "Projects" header, Settings. Right-click a project header → "Move to top of priorities"
  as the quick path.
- **Storage:** settings map in the `DirectoryTabColors` mold, but keyed by a canonical
  **`ProjectKey` string encoding** (`project_key.rs:20-30` — `StandardizedPath` already has manual
  Serialize), NOT raw cwd. `LocalGit` keys on the shared `.git` dir, so **all worktrees of a repo
  share one rank** — deliberately, because token budgets will want per-repo, not per-worktree.
  `ProjectId::Other` is unrankable (no stable identity) and stays in the unranked band.
- **API:** `ProjectPriorities::rank_of(&ProjectKey) -> Option<u32>`. The rail sorts by it; the
  future budget allocator reads the same number. Budget work itself is **out of scope** here.

## 5. Rail presentation: rank sorts, color signals, nothing moves on events

- Ranked band on top in rank order, thin divider, unranked band below in first-seen order.
  **Projects never reorder on agent events** — spatial memory is the point of a sidebar.
- **Row color states** (per task row, tinting the row label + the ring badge):
  - **Orange** — agent waiting on you (`Blocked`).
  - **Red** — still waiting after **5 minutes** (decided). One gradient, not two states: the
    oldest fire reads at a glance.
  - **Green** — agent finished (`Success`), results not yet seen. "Seen" = the pane gets focused;
    then the row returns to neutral.
  - Neutral — working / idle, current rendering.
- **Project header inherits the most urgent child**: red > orange > green > running-crescent.
  (This is Phase 0's fix made visible.)
- **Header chips** on the "Projects" title row (decided: keep, both kinds):
  - `⏳ N` — agents blocked anywhere (including unranked projects). Click jumps to the next
    blocked task, cycling. The escape hatch for "rank-8 project is on fire".
  - `✅ M` — agents done with unseen results / idle waiting for the next instruction. Click jumps
    likewise.
  These are navigation affordances, not lists — no duplicate rows anywhere (decided).

## 6. Notifications: two sounds, repeat until unblocked, priority-scaled

Prior art: the Claude Code hook ecosystem's converged pattern is urgent-sound-on-approval +
soft-chime-on-done; repeat-until-acknowledge is standard in incident tooling (OnPage/Opsgenie).
Warp catches the plugin event natively — no shell hooks.

| Event | Sound | OS notification | Repeat |
|---|---|---|---|
| Blocked, **ranked** project | urgent, immediate | immediate | every **3 min** until unblocked |
| Blocked, unranked | urgent, after **60 s** debounce | after debounce | every **15 min** |
| Done (`Success`) | soft chime, once | once | never |

Decided semantics:

- **Stop condition = the agent leaves `Blocked`** (the user actually answered). Nothing else
  stops the nag permanently.
- **Focusing the blocked pane silences the current cycle** (acknowledge); if the pane loses focus
  while the agent is *still* blocked, the nag re-arms after a grace period (~2 min).
- **Warp focused, other tab** (decided: yes): OS banners stay suppressed when Warp is frontmost
  (existing `is_navigated_away_from_window` gate), but the **sound still plays** when the blocked
  pane's tab is not the active tab — Sam can be deep in project A while B blocks.
- **Coalesce:** one notification for multiple waiters — "3 agents waiting (inbox-ai-flow, warp
  +1)" — never three banners.
- Respect existing gates: `is_needs_attention_enabled`, `play_notification_sound`, macOS
  Focus/DND (Time Sensitive interruption level is the ceiling; Critical needs an Apple
  entitlement — not pursued).
- Implementation: self-rescheduling `Timer::after` + `AbortHandle` per the `Heartbeat` pattern;
  new per-session state `blocked_since` (Phase 0) + `last_nagged_at` + per-session abort handle.
  The existing one-shot `NeedsAttention` path stays the entry point; the nag engine wraps it.

## 7. Phases

| Phase | Scope | Depends on |
|---|---|---|
| 0 | Header aggregate sees `Blocked`; `blocked_since`; badge for plugin-less sessions | — |
| 1 | Rank storage (ProjectKey-keyed settings), priorities panel, rail sort + divider | — |
| 2 | Row/header color escalation (orange→red@5min, green-unseen), header jump-chips | 0 |
| 3 | Nag engine: two sounds, debounce, repeat 3/15 min, coalesce, acknowledge/grace | 0, 1 |
| 4 | (later, separate spec) token budgets consuming `rank_of` | 1 |

## 8. Risks

| Risk | Note |
|---|---|
| Nag fatigue → feature gets turned off | Coalescing + debounce + per-task snooze are not polish, they are the feature's survival |
| `Blocked` fidelity varies by agent | Rich status only for plugin sessions (`supports_rich_status`); Codex OSC-9 fallback only emits opaque `Stop` — rows without rich status can't go orange, and must not pretend to |
| A `Blocked` that never clears (agent killed) | Handle-store `Forget`/session-removal must cancel the nag timer; a dead session must never nag forever |
| Sound-while-focused could double with the OS banner | The two gates are disjoint by construction (frontmost vs not); test both windowsful and windowless paths |
