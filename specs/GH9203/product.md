# Product Spec: Launch configs that open at app startup

**Issue:** [warpdotdev/warp#9203](https://github.com/warpdotdev/warp/issues/9203)
**Figma:** none provided

## Summary

Let users mark one or more saved Launch Configs as "open at startup" so the user's daily workflow layout (multiple tabs, panes, working directories, agent panes) materializes automatically when Warp launches. The data model already supports multi-window, multi-tab, multi-pane layouts via `LaunchConfig`; this feature adds a single per-config toggle and the startup hook that consumes it.

This addresses the issue's core ask — *"a default group of tab config panes that either open on launch or are a one click open"* — by leaning on the existing Launch Configs feature (which already covers the "one click open" case via the command palette) and filling the missing "open on launch" half.

## Problem

A user with a stable daily layout (the issue's reporter mentions ~5 panes for a typical workflow) currently has to:

1. Launch Warp.
2. Open the command palette.
3. Find their saved launch config by name.
4. Trigger it.

Every time. The friction compounds when multiple workflows are part of the daily setup (e.g. a "frontend" config plus a "backend" config). Users coming from iTerm2 windows sets, tmuxinator/teamocil sessions, or VS Code workspace restore expect their layout to come back when they relaunch the app.

The infrastructure to *describe* a multi-tab, multi-pane layout is fully present in `LaunchConfig` (windows → tabs → panes, with start commands, working directories, and pane-mode metadata). The feature gate is exclusively *when* to apply it.

## Goals

- A user can flip a single toggle on a Launch Config to make it auto-launch at startup, with no other configuration required.
- Multiple Launch Configs may be marked auto-launch; all of them open at startup, in the order users assign.
- The mechanism is opt-in. A user who never opens the toggle sees no behavior change — Warp launches with its current default (a single new tab).
- A misconfigured or partially-broken auto-launch config (e.g. a saved working directory that no longer exists) does not block app startup. It opens the parts it can and surfaces a non-blocking notification listing what failed.
- Auto-launch configs interact cleanly with Warp's existing **Restore previous tabs on startup** preference: if the user has both turned on, restore-previous wins for the previous-session tabs and the auto-launch configs open as additional windows alongside (not duplicating, not replacing).

## Non-goals (V1 — explicitly deferred to follow-ups)

- **Per-day or per-time-of-day scheduling.** "Open the morning config Mon–Fri 9am, the personal config evenings/weekends" is a richer scheduling system; out of scope.
- **Conditional auto-launch based on directory or git state.** "Only open this config when the launching CWD is under `~/code/foo`" — interesting but a separate feature.
- **Workspace-folder-anchored auto-launch.** "When opening Warp from `code .`-style integration, pick the matching saved config" — depends on shell/IDE integration that isn't part of this feature.
- **A new top-level "Startup" settings page.** The toggle lives on the existing Launch Config row, the same surface that already exposes Save / Edit / Delete affordances. No new section in Settings.
- **Programmatic startup-config switching from a CLI flag.** Out of scope; the existing `--launch-config` family of flags (if any) is unchanged.
- **Re-running auto-launch when a window crashes mid-session.** The hook fires once per app process startup. Crash recovery is the existing tab/restore-on-relaunch system's job.

## User experience

### Marking a config as auto-launch

1. User opens the Launch Configs list (existing surface — accessible via the command palette and the resource-center sections per `app/src/resource_center/sections.rs`).
2. Each row gains a small toggle labeled **Open at startup**, sitting alongside the existing per-row affordances.
3. Toggling it on persists the change to the launch-config record. No restart required to record the preference; the behavior takes effect on the *next* app launch.
4. The list visually surfaces auto-launch-enabled configs (e.g. a small "Startup" tag next to the config name) so users can see at a glance which configs will open next launch.

### App startup with auto-launch configs configured

1. User launches Warp.
2. Warp goes through its existing startup path (loading settings, restoring previous tabs if that preference is on, etc.).
3. After the existing startup completes, Warp enumerates the saved launch configs whose `auto_launch_at_startup` flag is set, sorted by the user-assigned order (see "Ordering" below), and applies each one — opening windows and tabs as if the user had triggered the config from the command palette.
4. The user sees their full workflow ready to go without any manual steps.

### Ordering multiple auto-launch configs

The Launch Configs list lets the user reorder rows via drag-and-drop (existing affordance, V0 simply respects the existing list ordering). Auto-launch configs open in list order on startup. There's no separate "startup order" UI in V1 — keeping the surface simple matches the issue's importance level (the reporter rated this 2/5).

### Failure scenarios

1. **Auto-launch config references a missing working directory** (e.g. user deleted `~/projects/foo` since saving the config). The config's other tabs/panes open normally; the broken pane opens with a fallback CWD (`$HOME` or the OS default — match whatever the existing manual launch-config flow does today). Warp surfaces a non-blocking notification: *"Auto-launch config `<name>`: 1 pane opened with fallback working directory."*
2. **Auto-launch config fails to apply entirely** (e.g. the config record is corrupt, or the saved windows array is empty). Warp logs the error, skips the config, surfaces a notification *"Auto-launch config `<name>` could not be opened. See settings."*, and continues with the next auto-launch config. App startup is never blocked.
3. **All auto-launch configs are missing or invalid** (e.g. user wiped their config storage). Warp falls through to its current default startup behavior (single new tab). Same as if the user had no auto-launch configs in the first place.
4. **Restore-previous-tabs is on AND auto-launch configs are present.** The two preferences compose: restore-previous restores the previous session's tabs into the existing default window (today's behavior), and auto-launch configs each open in *new* windows alongside. The user gets both. (No de-duplication: if a user has restore-previous on and the same workflow saved as an auto-launch config, they'll see duplicate panes and can disable one. This is documented but not auto-detected; auto-detection is a follow-up.)

## Configuration shape

Per-launch-config flag, persisted on the existing launch-config record:

```rust
pub struct LaunchConfig {
    pub name: String,
    pub windows: Vec<WindowTemplate>,
    pub active_window_index: Option<usize>,
    // V1 addition:
    #[serde(default)]
    pub auto_launch_at_startup: bool,
}
```

Defaults to `false` so existing saved configs are unchanged. No global setting; the feature is enabled per-config.

## Testable behavior invariants

Numbered list — each maps to a verification path in the tech spec:

1. With no launch configs marked `auto_launch_at_startup`, app startup behavior is byte-equivalent to today: existing default window with a single new tab, plus the existing restore-previous behavior if that preference is on.
2. With exactly one launch config marked `auto_launch_at_startup`, that config's windows and tabs open at app startup *in addition to* the default startup behavior.
3. With two configs marked `auto_launch_at_startup`, both apply on startup in the same order they appear in the launch-configs list.
4. Toggling the flag on a config from off → on records the change immediately and persists across app restarts. The current app session does *not* retroactively open the config — the toggle takes effect on next launch.
5. Toggling the flag from on → off prevents the config from auto-launching on next app startup; the config remains saved and can still be triggered manually.
6. An auto-launch config whose record is missing fields the current schema requires (a hypothetical migration mismatch) is skipped at startup with a non-blocking notification, and other auto-launch configs continue to apply.
7. An auto-launch config that references a working directory that no longer exists opens the rest of the config normally; the broken pane opens with the existing fallback CWD path used by manual launch-config invocation. Notification surfaces the count of fallback panes.
8. Disabling the flag on every auto-launch config restores today's startup behavior (invariant 1) on the next app launch.
9. Auto-launch behavior coexists with the **Restore previous tabs on startup** preference: when both are on, both apply. The auto-launch configs' windows are additive; they do not displace restored windows.
10. The auto-launch hook fires exactly once per app process startup. Subsequent in-session events (closing all windows then re-opening, switching workspaces, etc.) do not re-trigger auto-launch.

## Open questions

- **Should auto-launch configs share a flag with the existing "favorite" or "pinned" affordances on Launch Configs (if any)?** Recommend no — this is a separate semantic. Pin/favorite is for ordering and discovery; auto-launch is for behavior at app startup. Conflating them couples two unrelated concerns.
- **How does this interact with Quake mode?** `LaunchConfig::from_snapshot` already filters `quake_mode` windows out of saved configs, so auto-launch configs cannot create quake windows. This is fine for V1; quake-mode auto-launch is a separate feature.
- **Should the user reorder auto-launch configs via a dedicated "Startup order" UI rather than via the main list order?** Recommend no for V1 — adds a second list to maintain. If a third or fourth auto-launch config is common in practice, a dedicated list is a follow-up.
