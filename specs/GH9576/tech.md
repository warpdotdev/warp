# TECH.md — Show current zoom level when zooming

Issue: https://github.com/warpdotdev/warp/issues/9576
Product spec: `specs/GH9576/product.md`

## Problem

Warp already has stepped UI zoom values, zoom actions, keybindings, menu entries, settings UI, and the runtime bridge that applies the zoom factor. The missing piece is a transient workspace-level surface that is shown after user-initiated zoom actions and displays the resulting `ZoomLevel` percentage without changing the zoom semantics.

## Relevant code

- `app/src/window_settings.rs:73` — `zoom_level` setting definition.
- `app/src/window_settings.rs (84-98)` — `ZoomLevel::VALUES` and `ZoomLevel::as_zoom_factor()`.
- `app/src/workspace/action.rs (179-188)` — `WorkspaceAction::IncreaseZoom`, `DecreaseZoom`, and `ResetZoom`.
- `app/src/workspace/mod.rs (304-329)` — fixed zoom bindings registered when `FeatureFlag::UIZoom` is enabled.
- `app/src/workspace/mod.rs (382-410)` — editable zoom keybindings and adjacent font-size fallback bindings.
- `app/src/workspace/view.rs (15075-15101)` — `WindowSettingsChangedEvent::ZoomLevel` currently updates titlebar height.
- `app/src/workspace/view.rs (15660-15694)` — `increase_zoom`, `decrease_zoom`, `reset_zoom`, and `adjust_zoom`.
- `app/src/workspace/view.rs (20065-20067)` — action dispatch routes zoom actions to the helpers above.
- `app/src/workspace/view.rs (22995-23005)` — Linux/Windows Ctrl+scroll dispatches zoom actions when `UIZoom` is enabled.
- `app/src/settings_view/appearance_page.rs (914-926)` — settings-page observer applies `ctx.set_zoom_factor()` when `ZoomLevel` changes.
- `app/src/settings_view/appearance_page.rs (2444-2458)` — Appearance dropdown formats zoom items as `{value}%`.
- `app/src/settings_view/appearance_page.rs (5110-5146)` — `ZoomLevelWidget` renders the Appearance → Window → Zoom setting.
- `app/src/workspace/view.rs (2882-2890)` — workspace creates `DismissibleToastStack` instances with four-second timeout.
- `app/src/workspace/view.rs (19052-19062)` — global workspace toast positioning is already centered near the top of tab content.
- `app/src/workspace/view.rs (22891-22902)` — workspace overlays render the global toast stack using that positioning.
- `app/src/view_components/dismissible_toast.rs (58-120)` — existing ephemeral toast timeout and update behavior.

## Current state

`ZoomLevel` is a local window setting with stepped percentage values. When `FeatureFlag::UIZoom` is enabled, workspace keybindings and menu actions dispatch `IncreaseZoom`, `DecreaseZoom`, or `ResetZoom`. `increase_zoom` and `decrease_zoom` call `adjust_zoom`, which looks up the current value in `ZoomLevel::VALUES`, clamps to the min or max step, and writes the resulting value to `WindowSettings`. `reset_zoom` writes `ZoomLevel::default_value()`.

The visual zoom factor is applied by the Appearance settings page's subscription to `WindowSettingsChangedEvent::ZoomLevel`, which calls `ctx.set_zoom_factor(WindowSettings::as_ref(ctx).zoom_level.as_zoom_factor())`. Workspace also listens to the same event to update titlebar height.

Workspace already has general toast infrastructure, but it is optimized for message toasts with close buttons, optional icons, links, object IDs, and a four-second timeout. A zoom percentage is better represented as a short-lived HUD: one centered percentage, no close affordance, no stacking, and a shorter lifetime.

## Proposed changes

### 1. Add a dedicated zoom HUD view

Introduce a small view component under the workspace or shared view-components area, for example `app/src/workspace/zoom_level_hud.rs` or `app/src/view_components/zoom_level_hud.rs`. Keep it focused on this feature rather than extending `DismissibleToastStack`.

Suggested shape:

- `ZoomLevelHud` view with state:
  - `visible_zoom_level: Option<u16>`
  - `dismiss_handle: Option<SpawnedFutureHandle>`
- `show_zoom_level(&mut self, zoom_level: u16, ctx: &mut ViewContext<Self>)`
  - sets `visible_zoom_level` to the new value
  - aborts any previous dismiss timer
  - spawns a new `Timer::after(Duration::from_secs(1))` or `Duration::from_millis(1000)`
  - clears `visible_zoom_level` when the timer fires
  - calls `ctx.notify()`
- `render`
  - renders nothing when `visible_zoom_level` is `None`
  - otherwise renders a compact, non-interactive rounded container containing `format!("{zoom_level}%")`

Use theme-aware colors from `Appearance` rather than hard-coded colors. The UI should be high contrast enough across themes and should not use button themes or a close button. The existing UI guideline to reuse shared abstractions applies where relevant, but this HUD is not a button.

### 2. Store and render the HUD from `Workspace`

Add a field on `Workspace`:

- `zoom_level_hud: ViewHandle<ZoomLevelHud>`

Create it in `Workspace::new` near the existing toast-stack creation. Render it as a positioned overlay in `Workspace::render`, close to where `toast_stack` is currently rendered.

Positioning should follow the product direction:

- Prefer a helper such as `zoom_level_hud_positioning(&self) -> OffsetPositioning`.
- Anchor to `TAB_CONTENT_POSITION_ID` with `PositionedElementAnchor::TopMiddle` and `ChildAnchor::TopMiddle`.
- Use a small positive y offset below the tab/title bar, similar to or slightly larger than `global_toast_positioning`.
- Use `PositionedElementOffsetBounds::WindowByPosition` so it behaves consistently with existing workspace overlays.

Rendering this separately from `toast_stack` avoids stacking behavior and allows a one-second lifetime without altering general toasts.

### 3. Show the HUD from zoom actions after the value is known

Refactor the zoom helpers so they return or compute the final zoom value that should be shown.

Recommended approach:

- Add a helper on `Workspace`, for example `fn show_zoom_level_hud(&mut self, zoom_level: u16, ctx: &mut ViewContext<Self>)`.
- In `reset_zoom`, set the value to `ZoomLevel::default_value()` and call `show_zoom_level_hud(ZoomLevel::default_value(), ctx)` after attempting the setting update.
- In `adjust_zoom`, keep the existing `current_index` lookup. If the current value is not found, return without showing the HUD. Otherwise compute `next_index` as today, set that value, and call `show_zoom_level_hud(next_zoom, ctx)`.
- At min and max, `next_index` may equal `current_index`; still call `show_zoom_level_hud(current_zoom, ctx)` so the user sees the clamped value.

This keeps the HUD coupled to user-initiated zoom actions instead of every `WindowSettingsChangedEvent::ZoomLevel`. It also avoids showing the HUD for startup restoration, sync, or direct settings changes unless product later chooses to expand the behavior.

If implementation wants the HUD to appear for direct Appearance dropdown changes, add an explicit `WorkspaceAction::SetZoomLevelFromSettings`-style path rather than showing on every settings event. The product spec leaves that optional, but the safer first implementation is action-only.

### 4. Consider moving zoom-factor application out of the settings page

The current runtime bridge to `ctx.set_zoom_factor()` lives in `AppearanceSettingsPageView`'s settings observer. That means the code path is tied to a settings page view subscription even though zoom is a workspace-level behavior. The spec can be implemented without moving it, but implementation should evaluate whether this is already reliable when the Appearance page has never been opened.

If zoom factor changes do not apply unless the settings page exists, move or duplicate the zoom-factor bridge to an app- or workspace-owned observer that always exists for the window. A low-risk option is to update the existing workspace `WindowSettingsChangedEvent::ZoomLevel` branch so it calls both:

- `ctx.set_zoom_factor(WindowSettings::as_ref(ctx).zoom_level.as_zoom_factor())`
- `self.update_titlebar_height(ctx)`

Then remove the duplicate `ctx.set_zoom_factor()` call from the Appearance page observer only if tests confirm there is no regression in dropdown behavior. If the existing bridge is already globally reliable because the settings page view always exists, keep it unchanged and avoid unrelated churn.

### 5. Keep existing zoom settings and feature gating

Do not change:

- `ZoomLevel::VALUES`
- `ZoomLevel::default_value()`
- keybinding names
- `CustomAction` variants
- menu item registration
- `FeatureFlag::UIZoom` gating
- the font-size fallback path used when `UIZoom` is disabled

The HUD should only be reachable from zoom actions that are registered when `FeatureFlag::UIZoom` is enabled.

## End-to-end flow

1. User invokes a zoom shortcut, menu item, command-palette action, or Ctrl+scroll path.
2. The input path dispatches `WorkspaceAction::IncreaseZoom`, `DecreaseZoom`, or `ResetZoom`.
3. `Workspace::handle_action` routes the action to `increase_zoom`, `decrease_zoom`, or `reset_zoom`.
4. The helper writes the new `WindowSettings::zoom_level` value, preserving existing min/max clamping.
5. The helper calls `zoom_level_hud.show_zoom_level(new_value, ctx)`.
6. The HUD renders as a top-centered overlay in the active workspace, showing `{new_value}%`.
7. Any previous HUD dismissal timer is cancelled and replaced.
8. After about one second without another zoom action, the HUD clears itself and disappears.
9. Existing settings observers apply the zoom factor and update titlebar height as they do today, or the workspace observer takes ownership if the implementation moves the runtime bridge.

## Risks and mitigations

- Risk: using `DismissibleToastStack` creates stacked percentages, close buttons, icons, or a four-second lifetime that does not match the desired UX. Mitigation: implement a dedicated single-state HUD view.
- Risk: showing the HUD on every `ZoomLevel` settings event could flash on startup, sync, or settings initialization. Mitigation: trigger it from user zoom actions only for the first implementation.
- Risk: repeated keypresses leak timers or clear the new value when an old timer fires. Mitigation: store and abort the previous `SpawnedFutureHandle` before starting a new timer.
- Risk: the zoom factor bridge is currently owned by the Appearance settings page. Mitigation: verify the bridge exists even when settings has never been opened; if not, move it into the workspace zoom-settings observer.
- Risk: the HUD is positioned under modal dialogs or important overlays. Mitigation: render it with the workspace overlay stack near global toasts, and manually verify with settings, panels, full-screen, and dialog states.
- Risk: theme contrast is insufficient. Mitigation: use existing theme colors from `Appearance` and capture a design-review screenshot or video.

## Testing and validation

### Unit and view tests

- Add focused tests for zoom step calculation if the implementation extracts a helper. Cover increase, decrease, reset, min clamp, max clamp, and invalid current value.
- Add `ZoomLevelHud` tests if the view test framework supports checking render state:
  - `show_zoom_level(110)` makes the HUD visible with `110%`.
  - `show_zoom_level(125)` while visible replaces `110%` with `125%`.
  - the dismissal path clears the visible value.
- Add or extend `workspace/view_test.rs` coverage so dispatching `WorkspaceAction::IncreaseZoom`, `DecreaseZoom`, and `ResetZoom` updates the `WindowSettings::zoom_level` and the HUD state in the active workspace.

### Manual validation

- macOS: from `100%`, press `Cmd =` and confirm the HUD reads `110%`; press `Cmd -` and confirm it reads `100%`; reset and confirm it reads `100%`.
- Windows/Linux: with `UIZoom` enabled, hold Ctrl and scroll up/down over the workspace and confirm the HUD tracks the stepped values.
- Bounds: at `50%`, invoke zoom out and confirm the HUD still reads `50%`; at `350%`, invoke zoom in and confirm it still reads `350%`.
- Settings: open Appearance → Window → Zoom and confirm the dropdown still tracks the current value and changing it does not regress the applied UI scale.
- Layouts: verify the HUD over a single pane, split panes, vertical tabs, settings open, an AI/resource panel open, and fullscreen or hover-hidden tab-bar mode.
- Design artifact: attach a screenshot or short video of the HUD in the workspace because the issue comment requested designs and no Figma mock is available.

## Follow-ups

- Decide whether direct changes from the Appearance zoom dropdown should also show the HUD.
- Decide whether to add an accessibility announcement for the changed zoom percentage.
- If this design proves generally useful, consider extracting a reusable single-value HUD component for other transient workspace state changes rather than overloading general toasts.
