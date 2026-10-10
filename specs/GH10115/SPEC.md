# Per-Window Zoom Level (GH-10115)

## Summary

Make `Cmd +` / `Cmd -` / `Cmd 0` (and equivalent Command Palette / menu actions) affect ONLY the focused OS window. Today these shortcuts mutate a single global zoom value that re-renders every open Warp window. After this change, each window owns its own zoom-level state. New windows open at the user's configured global default. Per-window zoom is in-memory only in V1 — closing and reopening the app resets all windows to the global default. Tabs and splits inside a single window continue to share that window's zoom.

## Problem

`Cmd +` in any Warp window currently zooms ALL Warp windows simultaneously because the zoom level is a single global setting. Users with multi-monitor setups, comparison workflows, or accessibility needs (e.g., a reading-focused window at higher zoom alongside a code-focused window at default zoom) have no way to keep one window large without enlarging text everywhere. The issue author has confirmed in-memory-only scoping is acceptable for V1 — persistence is not required to deliver value.

## Goals

- Keyboard shortcuts (`zoom_in`, `zoom_out`, `zoom_reset`) and any explicit `zoom_to_level` action mutate ONLY the focused window's zoom state.
- Command Palette zoom commands and View menu zoom items behave identically — focused-window-scoped.
- Pinch / trackpad zoom gestures mutate ONLY the focused window's zoom — same scoping as keyboard and palette inputs.
- New windows open at the user's global zoom default (the value in Settings → Appearance → Zoom).
- All tabs and splits inside a single window share that window's zoom level. Switching tab or focused split inside the same window does not change the zoom.
- The Settings → Appearance → Zoom UI label is updated to disambiguate it as the default for new windows, not a global override.
- A REQUIRED "Apply to all open windows" button gives users a fast one-click way to re-sync every open window's zoom to the global default.

## Non-Goals

- Per-pane / per-tab zoom inside a single window (V2 candidate at earliest).
- Per-monitor adaptive zoom that auto-scales when a window moves between displays.
- Persisting per-window zoom across app restart (V1.5 candidate — see Open Questions).
- Changing the underlying zoom mechanism, range, or step size.

## Behavior Contract

### B1. Zoom actions are focused-window-scoped

ALL zoom inputs are focused-window-scoped. This includes:

- Keyboard shortcuts: `zoom_in`, `zoom_out`, `zoom_reset`, and any `zoom_to_level(value)` action.
- Command Palette zoom actions and View menu zoom items.
- Pinch / trackpad zoom gestures (two-finger pinch on macOS trackpads, equivalent gestures on other platforms).

Each input mutates the zoom-level field on the FOCUSED window's per-window state object only. Other open windows observe no change.

### B2. New windows inherit the global default

When a new Warp window is opened (via `Cmd N`, "New Window", or any other entry point), it initializes its zoom-level to the value of `appearance.zoom_level` (the global default). It does NOT inherit the focused window's current zoom.

### B3. Tabs and splits share window zoom

All tabs and splits inside a window share that window's zoom-level value. Creating a new tab or split, switching the active tab, or moving focus between splits does not change the rendered zoom.

Tab moves between windows:

- **Move to existing window**: when a tab is moved (drag-to-window or shortcut "move tab to window") from window A (zoom 1.5x) into existing window B (zoom 1.0x), the moved tab adopts window B's zoom (1.0x). The tab content re-renders at the destination window's zoom. There is no per-tab zoom memory; zoom belongs to the containing window, not the tab.
- **Drag-out spawns a new window**: when a tab is dragged OUT of a window in a way that spawns a NEW window (rather than dropping it into an existing one), the new window starts at the GLOBAL default zoom — NOT the source window's zoom. This matches B2 (new windows always start at the global default).

### B4. In-memory only in V1

Per-window zoom state lives in the in-memory window state object. Closing and reopening the app resets every window to the global default. This matches the issue author's "in-memory only is acceptable" allowance and keeps V1 small.

### B5. Settings label disambiguation

The Settings → Appearance → Zoom control is relabeled to "Default zoom for new windows" (or equivalent localized string) so users understand changing it does not retro-apply to open windows.

### B6. Settings change does not retro-apply

Changing the global default in Settings ONLY affects subsequently-opened windows. Already-open windows keep their current per-window zoom.

The "Apply to all open windows" button (see B8) is the explicit user-driven mechanism to re-sync open windows to the current global default.

### B7. Clamping is per-window

Existing min/max zoom clamps (e.g., 0.5x..3.0x) apply per window. Each window can independently be at its own clamped value; reaching the max in one window does not affect another window's range.

### B8. "Apply to all open windows" button (REQUIRED)

The "Apply to all open windows" button is a REQUIRED V1 control. It lives in Settings → Appearance → Zoom, immediately adjacent to the global zoom-default field.

Behavior:

- **Action**: clicking the button sets every open window's per-window zoom to the current global default. After the click, every window — including any new ones opened later — renders at the global default. (Subsequent per-window zoom changes still scope to that window only, per B1.)
- **Enabled state**: the button is enabled ONLY when at least one open window is currently at a zoom level that differs from the global default.
- **Disabled state**: when every open window already matches the global default, the button is disabled (greyed out) with the tooltip "All windows match the default".
- **State updates**: the enabled/disabled state recomputes whenever any open window's zoom changes or when the global default changes.

## Settings / API surface

`appearance.zoom_level` (existing) is preserved in **schema, type, range,
storage location, and serde representation**. What changes is **only its
runtime role**, which this spec makes explicit and consistent across all
sections:

| Aspect | Before this spec | After this spec |
|---|---|---|
| TOML key | `appearance.zoom_level` | `appearance.zoom_level` (unchanged) |
| On-disk type / range / default | unchanged | unchanged |
| Storage backing | unchanged | unchanged |
| **Runtime semantics** | Live global zoom — every window reads it on every paint; mutating it via `Cmd +/-/0` updates this value and re-renders all windows. | **Default for new windows only.** Read **once** at window-creation time to seed that window's per-window `zoom_level: f32`. Never read again on subsequent paints for an already-open window. |
| Effect of mutating it (Settings UI) | Applies live to every window. | Applies only to **subsequently-opened** windows; already-open windows keep their per-window zoom (B6). |
| Effect of `Cmd +/-/0` and zoom action handlers | Wrote into `appearance.zoom_level`. | Do **NOT** touch `appearance.zoom_level`. They mutate the focused window's per-window `zoom_level: f32` only (B1). |
| User-facing label | "Zoom" | "Default zoom for new windows" (B5). |

**Resolves the apparent contradiction.** Earlier wording that said
"semantics are unchanged" is **superseded** by this table — schema and
storage are unchanged, but runtime semantics shift from a live global
override to a one-time seed value. Concrete consequences:

- Toggling `appearance.zoom_level` in Settings while window A and B are
  open does **not** change the rendered zoom of A or B (B6, A6).
- `Cmd +` in window A does **not** write to `appearance.zoom_level` and
  does **not** change the value any other window will use on next open;
  it only mutates A's per-window state (B1, A1).
- Opening a new window C reads `appearance.zoom_level` exactly once to
  seed C's per-window `zoom_level: f32`; subsequent zoom actions in C
  diverge from `appearance.zoom_level` without rewriting it.

Other surface details:

- No new persisted settings in V1.
- Internal: per-window state object gains a `zoom_level: f32` field
  initialized from `appearance.zoom_level` at window-creation time and
  mutated thereafter only by zoom action handlers and the "Apply to all
  open windows" button. Zoom action handlers do **not** write back to
  `appearance.zoom_level`.
- New UI control (REQUIRED in V1): "Apply to all open windows" button in
  Settings → Appearance → Zoom. Writes the current value of
  `appearance.zoom_level` into every open window's per-window
  `zoom_level: f32` (does not modify `appearance.zoom_level` itself).
  Enabled only when at least one open window differs from the default;
  otherwise disabled with tooltip "All windows match the default"
  (see B8).

## Acceptance Criteria

- A1. With windows A and B open, `Cmd +` while A is focused increases A's rendered zoom; B's rendered zoom is unchanged (visual diff).
- A2. With A at 1.5x and B at 1.0x, `Cmd 0` while A is focused resets A to global default; B remains at 1.0x.
- A3. Opening a new window C while A is at 1.5x and global default is 1.0x produces C at 1.0x.
- A4. Inside window A: switching tabs, switching splits, and creating new splits do not change A's rendered zoom.
- A5. Quitting and relaunching the app produces all windows at the global default.
- A6. Changing the global default from 1.0x to 1.25x does not change zoom in any already-open window.
- A7. Clicking "Apply to all open windows" resets every open window to the current global default.
- A8. Min/max clamps work per window: A can be at 3.0x while B is at 1.0x, and `Cmd +` in B still increases B without affecting A.
- A_pinch_per_window_scope. With windows A and B open at 1.0x, performing a pinch-zoom-in gesture inside window A increases A's rendered zoom only; B's rendered zoom is unchanged. Pinch-zoom-out is equivalently scoped.
- A_apply_to_all_button_resets_all_open. With three open windows at 1.5x, 1.25x, and 1.0x respectively and a global default of 1.0x, clicking "Apply to all open windows" resets all three windows to 1.0x. After the click, opening a new window also produces 1.0x. The button transitions from enabled (because windows differed) to disabled (all match default) and exposes the tooltip "All windows match the default" in its disabled state.

## Implementation Pointers

Verified in this codebase:

- `crates/warpui_core/src/zoom.rs` — current zoom implementation. Likely owns the global zoom state today.
- `app/src/appearance.rs` — appearance settings; holds `zoom_level` default.
- `app/src/settings_view/appearance_page.rs` — Settings UI page where the relabel and "Apply to all open windows" button land.
- `crates/warp_core/src/ui/appearance.rs` — appearance theming surface.

Likely-touched (verify during implementation):

- Per-window state container — where window-scoped state already lives. Add `zoom_level: f32` field initialized from the global default.
- Zoom action handlers — change call sites from "mutate global zoom" to "mutate focused window's zoom".
- Window-creation entry points — read `appearance.zoom_level` at creation time and seed the per-window field.

Net-new modules: none expected. The change is mostly relocating where the zoom value is stored and read, plus a small Settings UI addition.

## Tests

- T1. Unit: `zoom_in` action with multiple registered windows mutates only the focused window's zoom field.
- T2. Unit: opening a new window seeds its zoom from the global default, not from the currently-focused window.
- T3. Integration: tab switch and split focus changes inside a single window do not emit zoom updates.
- T4. Integration: app restart starts every window at the global default.
- T5. Integration: changing the global default does not emit zoom updates for open windows.
- T6. Integration: "Apply to all open windows" emits zoom updates for every open window.
- T7. Unit: per-window clamping — independent windows can sit at independent clamped values.
- T8. Visual / snapshot: render two windows at different zooms and assert they differ.
- T_pinch_zoom_per_window. Integration: simulate a pinch-zoom gesture in window A while window B is also open. Assert window A's zoom-level field changed and window B's did not. Repeat for pinch-out (zoom-decrease) gesture.
- T_tab_move_adopts_dest_zoom. Integration: with window A at 1.5x and window B at 1.0x, move a tab from A to B (drag-to-window or "move tab to window" shortcut). Assert: the tab now renders at 1.0x; the tab carries no per-tab zoom memory; further zoom changes in B affect the moved tab in lockstep with the rest of B.
- T_tab_move_to_new_window_uses_global_default. Integration: with window A at 1.5x and the global default at 1.0x, drag a tab OUT of A in a way that spawns a NEW window. Assert: the new window opens at 1.0x (global default), NOT 1.5x (source-window zoom).
- T_apply_to_all_button_enabled_state. Unit: with all open windows matching the global default, the button is disabled and exposes tooltip "All windows match the default". When any window's zoom is changed to a non-default value, the button becomes enabled. After clicking the button, all windows match the default and the button returns to disabled.

## Open Questions

- V1.5: should per-window zoom persist across app restart by mapping a stable window identity (e.g., saved-window-config ID, or display ID) to a stored zoom? Suggest yes, keyed by saved window/workspace config rather than ephemeral window instance.
- Should the "Apply to all open windows" button ALSO appear in the View menu (in addition to Settings)? Suggest Settings only for V1 to keep menu surface area small; revisit in V1.5 if users ask for menu access.

## Telemetry

No new events. If `zoom.changed` (or equivalent) already exists, extend its payload with a `scope: "window"` field so future analytics can distinguish per-window vs legacy global behavior.
