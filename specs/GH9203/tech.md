# Tech Spec: Launch configs that open at app startup

**Issue:** [warpdotdev/warp#9203](https://github.com/warpdotdev/warp/issues/9203)

## Context

The Launch Configs feature is fully implemented today. `LaunchConfig` (in [`app/src/launch_configs/launch_config.rs`](https://github.com/warpdotdev/warp/blob/master/app/src/launch_configs/launch_config.rs)) already serializes a multi-window, multi-tab, multi-pane layout with per-pane working directories, start commands, and pane modes. Configs are saved via the existing save modal and triggered via the command palette. The data model was designed for this exact use case; only the *time of application* is missing.

This spec adds:
1. A per-config `auto_launch_at_startup: bool` flag on `LaunchConfig`.
2. A startup hook in the existing app-init path that enumerates configs with the flag set and applies them.
3. A toggle on the existing Launch Configs list UI.

No new data model, no new module structure, no new persistence layer — the feature reuses the launch-config persistence and the launch-config-application path that the command-palette trigger already exercises.

### Relevant code

| Path | Role |
|---|---|
| `app/src/launch_configs/launch_config.rs` | The `LaunchConfig` struct gets a new `auto_launch_at_startup` field with `#[serde(default)]` so existing on-disk configs deserialize unchanged. |
| `app/src/launch_configs/mod.rs` | Module entry. The new startup hook lives in a new `startup.rs` submodule alongside `launch_config.rs` and `save_modal.rs`. |
| `app/src/launch_configs/save_modal.rs` | Existing save UI. The new toggle slots in here (or in the configs-list view, depending on which existing surface is the closest fit — verify at implementation time). |
| `app/src/lib.rs` / `app/src/app_state.rs` | The app-level startup path that loads previous-session tabs and creates the default window. The new hook fires after that completes. |
| `app/src/search/command_palette/launch_config/` | Existing launch-config-application code path (the same code the user triggers via `/launch-config <name>`). The new startup hook calls into the same entry point. |

### Related closed PRs and issues

- The existing **Restore previous tabs on startup** preference is the closest analog. It runs at app startup, opens windows/tabs, and is opt-in. The new auto-launch hook fires alongside it, not instead of it.

## Crate boundaries

All new code lives in `app/`. No new crate, no cross-crate boundary changes. The `LaunchConfig` struct remains in `app/src/launch_configs/launch_config.rs` (already used only within `app/`).

## Proposed changes

### 1. Add the flag

**File:** `app/src/launch_configs/launch_config.rs`.

```rust
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct LaunchConfig {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub active_window_index: Option<usize>,
    pub windows: Vec<WindowTemplate>,
    /// V1 addition: when true, this config opens automatically on app launch.
    /// Existing on-disk configs deserialize as `false` via `#[serde(default)]`.
    /// `from_snapshot` defaults to `false` — saving a snapshot doesn't auto-flip
    /// the flag; the user must toggle it explicitly.
    #[serde(default)]
    pub auto_launch_at_startup: bool,
}
```

`from_snapshot` is unchanged in behavior — newly-saved configs default to `false` for the flag. The user opts in via the toggle.

### 2. Startup hook

**File:** new `app/src/launch_configs/startup.rs`.

```rust
/// Enumerate saved launch configs with `auto_launch_at_startup = true`,
/// in their list order, and apply each. Called once during app startup,
/// after the default window and restore-previous-tabs path complete.
pub fn apply_auto_launch_configs(ctx: &mut AppContext) {
    let configs = load_all_launch_configs(); // existing loader
    let auto_launch: Vec<&LaunchConfig> = configs
        .iter()
        .filter(|c| c.auto_launch_at_startup)
        .collect();

    if auto_launch.is_empty() {
        return;
    }

    let mut failures = Vec::new();
    for config in auto_launch {
        match apply_launch_config(ctx, config) {  // existing application path
            Ok(report) => {
                if report.fallback_pane_count > 0 {
                    failures.push(StartupFailure::PartialFallback {
                        name: config.name.clone(),
                        fallback_count: report.fallback_pane_count,
                    });
                }
            }
            Err(err) => {
                tracing::warn!(
                    config_name = %config.name,
                    error = %err,
                    "auto-launch config failed to apply",
                );
                failures.push(StartupFailure::Skipped {
                    name: config.name.clone(),
                    reason: err.to_string(),
                });
            }
        }
    }

    if !failures.is_empty() {
        surface_startup_notifications(ctx, &failures);
    }
}
```

`apply_launch_config` is the existing function that the command-palette `/launch-config <name>` trigger calls. Its return type may need to grow a small `LaunchConfigApplicationReport` struct (or equivalent) to surface fallback-pane counts; if today's API silently substitutes fallback CWDs without reporting, this PR adds that signal as a backwards-compatible additive change.

### 3. Wire the hook into app startup

**File:** `app/src/lib.rs` (or the app's main init function — find via `grep -rn "App::new\|app_state.init\|fn init.*AppContext"`).

The startup sequence today, conceptually:

```
load settings
  → create default window with new tab
  → if `restore_previous_tabs`: restore from snapshot
  → window becomes interactive
```

Becomes:

```
load settings
  → create default window with new tab
  → if `restore_previous_tabs`: restore from snapshot
  → apply_auto_launch_configs()              ← new hook, last step before windows go interactive
  → windows become interactive
```

Placing the hook *after* restore-previous-tabs ensures the existing preference's behavior is byte-equivalent to today (invariant 1, 9). The auto-launch configs add windows alongside; they do not interact with the restore path.

### 4. UI toggle

**File:** the existing Launch Configs list view (locate via `grep -rn "LaunchConfig" app/src/search/command_palette/launch_config/`). Each row gains a toggle component.

Pseudocode for the row:

```
[Config name]  [Open at startup ☐]  [Edit] [Delete]
```

Click handler:

```rust
fn on_auto_launch_toggle_clicked(&mut self, config_id: &str, ctx: &mut ViewContext) {
    let mut configs = load_all_launch_configs();
    if let Some(c) = configs.iter_mut().find(|c| c.name == config_id) {
        c.auto_launch_at_startup = !c.auto_launch_at_startup;
        save_all_launch_configs(&configs);
        ctx.notify();
    }
}
```

The "Startup" pill rendered next to a config's name reads `auto_launch_at_startup` directly — no separate persistence.

### 5. Notification rendering

**File:** new `app/src/launch_configs/startup.rs` (helper alongside the hook).

Two notification types, both routed through the existing `ToastStack` infrastructure:

- `StartupFailure::PartialFallback { name, fallback_count }` → *"Auto-launch config `<name>`: `<n>` pane(s) opened with fallback working directory."*
- `StartupFailure::Skipped { name, reason }` → *"Auto-launch config `<name>` could not be opened: `<reason>`."*

Both are non-blocking. If multiple auto-launch configs each produced failures, render one notification per failure (capped at 3 — beyond that, render a single rolled-up notification *"`<n>` auto-launch configs had startup issues. See logs."*).

## Testing and validation

Each invariant from `product.md` maps to a test at this layer:

| Invariant | Test layer | File |
|---|---|---|
| 1 (no auto-launch → byte-equivalent) | unit | `app/src/launch_configs/startup_tests.rs` (new) — call `apply_auto_launch_configs` with no flagged configs, assert it returns without modifying the test app state. |
| 2 (one config applies on startup) | unit | startup_tests — flag one config, run the hook, assert the expected windows/tabs were applied. Mocks `apply_launch_config` to record calls. |
| 3 (two configs apply in list order) | unit | startup_tests — flag two configs in a known order, run the hook, assert `apply_launch_config` was called in that order. |
| 4 (toggle persists across restart) | unit | `app/src/launch_configs/launch_config_tests.rs` — round-trip a `LaunchConfig` with `auto_launch_at_startup = true` through serde JSON, assert the flag survives. |
| 5 (toggle off prevents auto-launch) | unit | startup_tests — set the flag false on a previously-flagged config, run the hook, assert no application happens. |
| 6 (corrupt config skipped) | unit | startup_tests — supply a config record that fails to deserialize / fails `apply_launch_config`, assert subsequent configs still apply and a notification is surfaced. |
| 7 (missing CWD → fallback + notification) | unit | startup_tests — feed a config whose `apply_launch_config` returns a `fallback_pane_count > 0`, assert the partial-fallback notification is surfaced. |
| 8 (disable all → today's behavior) | unit | startup_tests — flag all configs false, run the hook, assert no-op (same as invariant 1). |
| 9 (composes with restore-previous) | integration | `crates/integration/tests/` — set restore-previous to true, flag one launch config, launch, assert both restored tabs *and* the auto-launch config's windows are present. |
| 10 (hook fires once) | unit | startup_tests — call the hook twice in the same test app context, assert the second call is a no-op (or that the auto-launch tracker prevents duplicate application). |

### Cross-platform constraints

- The `auto_launch_at_startup` boolean serializes identically across all platforms.
- Working-directory fallback semantics are inherited from the existing manual `apply_launch_config` path; this PR doesn't change them.
- No new file-system, network, or shell interactions.

## End-to-end flow

```
User toggles "Open at startup" on a Launch Config row
  └─> [launch_config_view::on_auto_launch_toggle_clicked]   (new handler)
        ├─> load_all_launch_configs()
        ├─> mutate target config's auto_launch_at_startup
        └─> save_all_launch_configs()
              └─> existing persistence path (unchanged)

Next time Warp starts
  └─> [app_state::init]                                     (existing)
        ├─> load settings
        ├─> create default window with new tab
        ├─> if restore_previous_tabs: restore from snapshot  (existing)
        └─> [launch_configs::startup::apply_auto_launch_configs]    (new hook)
              ├─> load_all_launch_configs()
              ├─> filter where auto_launch_at_startup
              ├─> for each (in list order):
              │     └─> apply_launch_config(config)         (existing entry point)
              │           ├─> Ok(report)
              │           │     ├─> if report.fallback_pane_count > 0
              │           │     │     → push StartupFailure::PartialFallback
              │           │     └─> else: silent success
              │           └─> Err(_) → push StartupFailure::Skipped
              └─> if failures.is_empty() → done
                  else → surface_startup_notifications()
```

## Risks

- **Invocation idempotency.** If the hook runs twice (e.g. in a code path where app state can be re-initialized within a single process — closing all windows then reopening), the user would see duplicated windows. **Mitigation:** an `AUTO_LAUNCH_APPLIED: AtomicBool` guards the hook; subsequent calls are no-ops within the same process. Invariant 10 covers this.
- **Auto-launch storms.** A user with 10+ auto-launch configs would get 10+ windows on startup, possibly slow and overwhelming. **Mitigation:** V1 doesn't impose a cap, but the failure notification path (capped at 3 individual notifications + 1 rolled-up) prevents notification spam. If auto-launch storms become a real complaint, V2 can add a "max auto-launch configs" setting or a warning at startup.
- **Restore-previous + auto-launch double-counting.** A user with the same workflow saved as both an auto-launch config and as the previous session sees duplicate panes. **Mitigation:** documented in the user-experience section. Auto-detecting overlap is a follow-up.
- **Schema evolution.** If a future version adds required fields to `WindowTemplate` or `TabTemplate`, existing auto-launch configs would skip with a "Skipped" notification on startup. **Mitigation:** the existing `LaunchConfig` deserialization already uses `#[serde(default)]` patterns; new fields should follow that convention. The V1 `auto_launch_at_startup` field itself uses `#[serde(default)]` so the reverse case (older Warp reading newer config) deserializes the field as `false`, which is safe.

## Follow-ups (out of this spec)

- Per-day or time-of-day scheduling for auto-launch configs.
- Conditional auto-launch based on launching CWD or git state.
- Workspace-folder-anchored auto-launch (driven by IDE / shell integration).
- A separate "Startup order" UI distinct from the main launch-configs list ordering.
- Auto-detection of overlap between restore-previous-tabs and auto-launch configs.
- "Disable all auto-launch this session" affordance for the rare case where the user wants a clean launch without flipping every flag.
