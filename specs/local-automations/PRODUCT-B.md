# Local Automations — Slice B (In-app cron scheduler)

## Summary
Slice B makes local automation `schedule` fields fire while the Warp app is running. Enabled automations run on cron into the same local agent/shell tabs as Run now, with honest catch-up and missed-run visibility when Warp was closed or the machine slept.

## Goals / Non-goals
**Goals**
- Evaluate cron schedules in-process while Warp is open.
- Catch up once on launch/wake when a run was recently due; otherwise mark missed.
- Show last/next/missed status on the Settings → Automations list.
- Respect `enabled` for schedules (Run now still works when disabled).
- Enforce `timeout_seconds` when set (best-effort).
- Clear copy that Warp must be running and the machine awake.

**Non-goals**
- Background daemon / launchd / login item.
- Event triggers, auto cloud fallback, promote wizard (Slice C).
- Full run history route or OS notifications (optional later).
- Multi-machine sync of local schedule state.

## Figma
Figma: none provided

## Behavior

### When schedules fire
1. While Warp is running and `FeatureFlag::LocalAutomations` is on, the app evaluates each valid, **enabled** automation's `schedule` on a short interval (about every 30 seconds) and when the automations list reloads.
2. When a schedule is due, Warp starts a run using the same path as **Run now** (resolve cwd/worktree, open agent or shell tab, unattended profile for Warp agent).
3. Disabled automations (`enabled = false`) never start from the scheduler. Run now still works with the existing warning.
4. Invalid cron / unparseable `schedule` strings never crash the app. That automation does not auto-fire; the list shows an invalid-schedule status and Open config remains available.
5. Schedules do **not** run when Warp is quit, crashed, or not running. Copy in the list header and skills states that the app must be open and the machine awake.

### Cron and presets
6. Supported schedules:
   - Standard 5-field cron: `minute hour day-of-month month day-of-week` (same family as common Unix crons; local timezone).
   - Presets: `@hourly`, `@daily`, `@weekly`, `@monthly`, `@yearly` / `@annually`.
7. Other strings fail closed as invalid schedule (no silent ignore that looks like success).
8. Evaluation uses the machine's local timezone.

### Catch-up and missed runs
9. On each evaluation (including app launch and return from sleep), for each enabled automation with a valid schedule Warp computes whether any scheduled fire time is due since the last successful scheduled fire (or since first seen, if never fired).
10. **Catch-up window:** if the most recent due fire time is within the last **6 hours** and has not yet been executed as a scheduled run, Warp starts **exactly one** catch-up run (multiple missed ticks inside the window coalesce into one run).
11. **Missed (no auto-run):** if the most recent due fire time is **older than 6 hours** and was never executed, Warp increments/stores a missed indicator, does **not** auto-start a run, and shows missed status on the list. The next future cron tick can fire normally once it becomes due while Warp is open.
12. Catch-up and live fires both count as scheduled runs for "last scheduled fire" bookkeeping so the same past tick never re-fires after restart.
13. Manual **Run now** does not clear missed status by itself, and does not count as satisfying a scheduled tick (so a due schedule still fires on its own). Optional future polish may link them; Slice B keeps them separate for honesty.

### Overlap / single-flight
14. If a **scheduled** run for the same automation (same source file) is already starting or marked in-flight, the scheduler skips additional scheduled starts for that automation until the in-flight flag clears.
15. In-flight clears when the run tab has been opened (spawn attempted) or after a short safety timeout if spawn fails, so a failed spawn cannot block forever.
16. Concurrent **Run now** while a scheduled run is in flight remains allowed (user-initiated). Two overlapping scheduled starts for the same file must not happen.
17. Only one workspace/window consumes each scheduled start request (no double tabs from multi-window dogfood).

### Timeout
18. When `timeout_seconds` is set and the runner is **shell**, Warp runs the command under a time limit on supported platforms (e.g. wraps with `timeout` on macOS/Linux when available, or equivalent). Exceeding the limit fails the shell run.
19. When `timeout_seconds` is set and the runner is **warp_agent**, Slice B records the limit for display and logs it; hard-killing an agent conversation is **not** required in B if platform support is incomplete. Prefer documenting the limitation over fake enforcement.
20. When `timeout_seconds` is omitted, behavior matches Slice A (no extra limit).

### List visibility
21. Settings → Automations header copy no longer says schedules don't fire. It states that schedules run while Warp is open and the machine is awake, and that quitting Warp skips fires (with catch-up/missed rules).
22. Each valid row shows, in addition to Slice A fields, a compact status line when known:
    - Next scheduled time (local), or "invalid schedule", or "disabled" (already shown).
    - Last scheduled run time when one exists.
    - Missed indicator when the automation has an outstanding missed catch-up (e.g. "Missed while Warp was closed").
23. Empty and error-row behavior from Slice A still holds.
24. Status updates when the scheduler writes state and when configs reload, without requiring leaving Settings.

### Skills and create prompts
25. Bundled skills and the copy-prompt text no longer claim that schedules never fire. They state schedules fire only while Warp is running, describe catch-up briefly, and still mention Run now for immediate execution.
26. Create/edit skill may still write the same TOML schema; no new required fields in B.

### What must not happen
27. No double-fire storms on wake (at most one catch-up per automation per gap).
28. No schedule execution for disabled or invalid-schedule automations.
29. No daemon outside the Warp process.
30. Bad TOML or bad cron must not crash the app.
31. Scheduler must not overwrite the user's automation TOML files (state lives in a separate app-owned file).

### Discoverability
32. Existing entry points (Settings → Automations, palette, toolbar) remain. No separate "scheduler" surface.