# Survey: who reaches a `TerminalManager` through a pane

**Purpose:** step 1 of [`plan.md`](./plan.md) §11 — the gate on scoping lazy shell startup.
**Method:** every reach-through, classified as *fine-without* / *must-start* / *requires-live*.
**Date:** 2026-08-04

---

## 1. Headline: the surface is small

The manager is reached through exactly **one** seam and **7** call sites. It is not sprayed across
the codebase, and **no code outside `pane_group/` reaches it directly**.

```
TerminalPane::terminal_manager(ctx)  →  self.view.as_ref(ctx).child_data(ctx).clone()
                                        └── the only child_data reach in the app
```

`PaneGroup::terminal_manager(pane_index, app)` (`mod.rs:7578`) is the public wrapper and already
returns `Option`, because a pane may not be a terminal pane at all. **That is the shape deferral
needs**, and callers already handle `None`.

> This materially lowers the risk in `plan.md` §10 ("liveness assumptions … needs enumeration").
> The enumeration is done, and it is 7 sites, not hundreds.

## 2. The one asymmetry to fix

`TerminalPane::terminal_manager` returns a **non-`Option`** `ModelHandle`, because `child_data` is
infallible today. Deferral makes "no manager yet" representable, so this accessor becomes
`Option<...>` (or `&mut`-taking "start-then-return"). Its blast radius is the 7 sites below —
`PaneGroup::terminal_manager` already returns `Option` and absorbs the change for external callers.

## 3. The call sites

| # | Site | Purpose | Class | Note |
|---|---|---|---|---|
| 1 | `terminal_pane.rs:591` (`focus`) | focus the pane | **must-start** | Focus is the natural startup trigger; this is the "activation" path in practice |
| 2 | `mod.rs:2526` `send_prompt_change_bindkey_to_all_sessions` | push a prompt-mode bindkey to **all** sessions | **fine-without** | Broadcast-shaped. A deferred pane has no shell to configure and will pick up the setting when it starts. Must **skip**, not start — otherwise one setting change starts every tab |
| 3 | `mod.rs:3708` `load_data_into_transcript_viewer` | load a conversation into a viewer | **fine-without** | Viewer panes have no local shell today and already pass `None` for startup directory |
| 4 | `mod.rs:4458` `close_pane_with_confirmation` | ask the manager whether a long-running command is active | **fine-without** | Directly answers §8.5: a deferred pane has no running command, so "needs confirmation" is trivially `false`. **Closing must not start it** |
| 5 | `mod.rs:6935` `active_session_terminal_model` | the *active* session's model | **must-start** (already) | Only the active session; by definition it has been activated. 7 downstream callers, all via this one accessor |
| 6 | `mod.rs:7010` attach-execution-session | attach a shared/execution session | **requires-live** | Already logs and bails when there is no manager — the failure path exists |
| 7 | `mod.rs:7584` `PaneGroup::terminal_manager` | the public wrapper | n/a | The `Option` boundary itself |

**Classification totals:** 2 must-start · 3 fine-without · 1 requires-live (already handles absence)
· 1 boundary. Plus one path that bypasses the accessor entirely — synchronized input, §4b.

## 4. What this resolves in `plan.md` §8

| Contract | Status after survey |
|---|---|
| §8.1 exactly-once start of the active pane | Site 5 is reached only for the *active* session, and site 1 (`focus`) is the trigger. Startup hangs off focus, and idempotency is a check on the optional manager slot |
| §8.2 broadcast input | **Site 2 is the concrete case** and the answer is *skip*: it is a settings push, not user input, and starting 50 shells to deliver a bindkey defeats the feature entirely. Genuine broadcast *typed* input still needs its own answer — see §5 |
| §8.5 close before start | **Resolved.** Site 4 is the only close-path reach, and a deferred pane trivially has no long-running command. Block cleanup goes through `delete_blocks`, which uses `self.uuid` and never touches the manager |
| §8.6 partial-start failure | Site 6 shows the established pattern: log and bail rather than panic |

## 4b. Synchronized (typed) input — resolved

It exists, and it does **not** go through the manager accessor. `PaneGroup::send_sync_event_to_session`
(`mod.rs:1195`) resolves a `TerminalView` and calls
`TerminalView::receive_sync_input_event` (`terminal/view.rs:3026`), which reaches the PTY only via
`self.model` — `write_to_pty_for_syncing_long_running_commands` (`:3050`).

So typed broadcast bypasses the 7 sites entirely and is its **own** contract:

- It is user input, so *"skip silently"* is the wrong default here (unlike site 2's settings push) —
  a user typing into synchronized panes expects every pane to receive it.
- **Recommendation:** synchronized input **starts** a deferred pane. It is explicitly opt-in
  (the user turned syncing on for those panes), so it is rare and intentional, and the cost is
  bounded by how many panes the user chose to sync.
- Sharp edge: enabling *"Synchronize All Panes in All Tabs"* (`sync_inputs.rs:32`) then typing would
  start every deferred pane at once — the very stampede this feature avoids. Acceptable because it
  is an explicit user action, but it must be a stated consequence, not a surprise.

## 5. Still open after this survey

1. **The lifecycle split itself** (`plan.md` §5) is untouched by this survey. `create_session` still
   returns surface and manager together; the survey says *who* would tolerate `None`, not *how* to
   build a surface without a PTY.
3. **Lossless re-snapshot** (`plan.md` §6) remains the blocker. Nothing here changes it — it is a
   `snapshot()` problem, not a manager-reach problem.

## 6. Scope verdict

**Contained, not a month** — with one caveat.

- The reach-through is 7 sites with a single seam, and the public wrapper is already `Option`.
- Two sites need a start trigger, three tolerate absence, one already handles it.
- The real work is therefore **not** the audit (done, small) but §5's lifecycle split and §6's
  snapshot contract. Those are two focused problems in `create_session` and `snapshot`, not a broad
  refactor.

Recommended next step: the **§6 snapshot contract**. It is the one that loses user data if wrong,
and it can be designed, implemented and tested *before* any deferral is enabled — so it carries no
risk on its own. The lifecycle split (§5) follows.
