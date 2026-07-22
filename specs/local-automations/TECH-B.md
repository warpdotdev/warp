# Local Automations — Slice B TECH

## Context
Slice B implements [`PRODUCT-B.md`](./PRODUCT-B.md) on top of Slice A ([`PRODUCT.md`](./PRODUCT.md), [`TECH.md`](./TECH.md)).

Relevant landed pieces @ `dc0154e9537f7d40852b9ff798b42a1bdfd52a2c`:
- Schema/load: [`app/src/local_automations/local_automation.rs`](https://github.com/warpdotdev/warp/blob/dc0154e9537f7d40852b9ff798b42a1bdfd52a2c/app/src/local_automations/local_automation.rs), `WarpConfig` + watcher in [`app/src/user_config/native.rs`](https://github.com/warpdotdev/warp/blob/dc0154e9537f7d40852b9ff798b42a1bdfd52a2c/app/src/user_config/native.rs)
- Run now: `WorkspaceView::run_local_automation` / `open_local_automation_tab` in [`app/src/workspace/view.rs`](https://github.com/warpdotdev/warp/blob/dc0154e9537f7d40852b9ff798b42a1bdfd52a2c/app/src/workspace/view.rs)
- List UI: [`app/src/local_automations/list_view.rs`](https://github.com/warpdotdev/warp/blob/dc0154e9537f7d40852b9ff798b42a1bdfd52a2c/app/src/local_automations/list_view.rs)
- Timers: `warpui::r#async::Timer::after` (used e.g. by update manager)
- Singletons: `ctx.add_singleton_model(...)` in `app/src/lib.rs` (pattern next to `ScheduledAgentManager`)
- No existing `cron` crate in the workspace; cloud schedules store opaque cron strings server-side.

## Proposed changes

### 1. Dependency
Add `cron = "0.15"` (or latest compatible 0.x with chrono) to `app/Cargo.toml`. Use 5-field Unix cron via `cron::Schedule` + chrono local times. Map presets before parse:
- `@hourly` → `0 * * * *`
- `@daily` → `0 0 * * *`
- `@weekly` → `0 0 * * 0`
- `@monthly` → `0 0 1 * *`
- `@yearly` / `@annually` → `0 0 1 1 *`

### 2. Pure schedule logic (`schedule.rs`)
- `normalize_schedule(raw: &str) -> Result<String, ScheduleError>`
- `parse_schedule(raw: &str) -> Result<cron::Schedule, ScheduleError>`
- `next_fire_after(schedule, after: DateTime<Local>) -> Option<DateTime<Local>>`
- `decision_for_automation(...)` implementing PRODUCT catch-up rules:
  - Inputs: now, last_scheduled_fire_at, enabled, parsed schedule, in_flight
  - Outputs: `Fire { due_at }`, `Missed { due_at }`, `Wait { next_at }`, `SkipDisabled`, `SkipInFlight`, `Invalid`

Catch-up constants (named):
- `CATCH_UP_WINDOW = Duration::hours(6)`
- Tick interval ~30s

Unit-test with fixed `DateTime`s (no sleep).

### 3. Persistent run state (`run_state.rs`)
Path: `data_dir().join("local_automations_state.json")` (app-owned; not user TOML).

```json
{
  "version": 1,
  "by_path": {
    "/abs/path/foo.toml": {
      "last_scheduled_fire_at": "2026-07-22T09:00:00-07:00",
      "last_missed_at": null,
      "missed_count": 0,
      "in_flight_since": null
    }
  }
}
```

- Load on scheduler start; atomic write (temp file + rename).
- Key = automation `source_path` display string (absolute).
- Prune entries whose path no longer exists in the loaded automation set (optional on reload).

### 4. Scheduler model (`scheduler.rs`)
`LocalAutomationsScheduler` singleton (`Entity` + `SingletonEntity`):
- On `new`: load state; subscribe to `WarpConfigUpdateEvent::LocalAutomations`; start tick loop via `ctx.spawn` + `Timer::after(30s)` loop (or reschedule each tick).
- Each tick / config update: read automations from `WarpConfig`, compute decisions, for `Fire`:
  1. Set `last_scheduled_fire_at = due_at`, clear missed fields as appropriate, set `in_flight_since = now`, persist
  2. Push automation onto internal `pending: VecDeque<(LocalAutomation, ScheduledRunReason)>`
  3. `ctx.emit(LocalAutomationsSchedulerEvent::PendingUpdated)` and `StatusUpdated` for list refresh
- For `Missed`: update missed_count/last_missed_at once per due_at (idempotent), persist, emit StatusUpdated
- API for workspace:
  - `pop_pending(&mut self) -> Option<(LocalAutomation, ScheduledRunReason)>` (single consumer)
  - `clear_in_flight(&mut self, path: &Path)` after tab open attempt
  - `status_for(&self, path: &Path) -> AutomationScheduleStatus` for list UI

Gated entirely on `FeatureFlag::LocalAutomations` + `local_fs`.

### 5. Workspace wiring
In `WorkspaceView::new` (or equivalent subscribe block):
- Subscribe to `LocalAutomationsScheduler`
- On `PendingUpdated`, loop `pop_pending` and call existing `run_local_automation`
- After spawn path begins / finishes open attempt, `clear_in_flight`
- Do **not** show the disabled warning toast for scheduled runs of enabled automations; optional quiet toast is unnecessary (tab appearing is enough). Skip disabled-warning branch when reason is scheduled (pass a `RunTrigger` into run path or clone automation with enabled true only for messaging).

Register singleton in `app/src/lib.rs` next to other AI/config models when the flag is enabled.

### 6. Shell timeout
In `open_local_automation_tab` for shell: if `timeout_seconds` is `Some(secs)` and `secs > 0`, set command to platform-appropriate timed invocation when possible:
- macOS/Linux: prefer `timeout {secs}s sh -c {shell_escape(command)}` if we detect `timeout` or `gtimeout`; else run bare command and log that timeout could not be enforced.
Keep agent path unchanged aside from logging timeout_seconds.

### 7. List UI + copy
- Subscribe list view to scheduler `StatusUpdated` as well as WarpConfig.
- Subtitle: append next fire / missed / invalid schedule.
- Header: schedules run while Warp is open; catch-up within 6h; otherwise marked missed.
- Update `mod.rs` prompts, bundled skills, PLAN.md index.

### 8. Module layout
```
app/src/local_automations/
  mod.rs
  local_automation.rs
  list_view.rs
  schedule.rs          # pure cron + decisions
  run_state.rs         # JSON persistence
  scheduler.rs         # singleton model
  *_tests.rs
```

## Testing and validation
| PRODUCT-B | Verification |
|-----------|----------------|
| 1–5 fire while open | Unit decisions + manual: minute cron while app open |
| 6–8 cron/presets | Unit parse presets + invalid |
| 9–13 catch-up/missed | Unit: due 1h ago → Fire once; due 8h ago → Missed; restart no re-fire |
| 14–17 single-flight | Unit in_flight skip; manual multi-window one tab |
| 18–20 timeout | Unit/command build; manual shell with timeout 1 |
| 21–24 list | Manual status strings |
| 25–26 skills | Manual / copy review |
| 27–31 must-not | Unit no double fire; invalid cron no panic |

Automated focus: `schedule` decision tests + run_state load/save + parse presets.

## Parallelization
Single engineer sequential: schedule pure logic → state → model → workspace → UI/skills → tests. Parallel agents not worth the merge cost for this slice.

## Risks and mitigations
- **Clock jump / sleep:** evaluate from `last_scheduled_fire_at` and now each tick; coalesce; never loop every missed minute as separate runs.
- **Multi-window double run:** pending queue + pop.
- **cron crate semantics** (seconds field vs 5-field): pin API and test `@daily` and `0 9 * * 1-5`.
- **State file corruption:** ignore/recreate on parse failure; log warning.

## Follow-ups
Slice C promote wizard; optional OS notify on fail; agent hard timeout; telemetry.