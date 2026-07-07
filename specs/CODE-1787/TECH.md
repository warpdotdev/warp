# Windows Quake Mode: Focus and Sizing Fix — Tech Spec
Product spec: `specs/CODE-1787/PRODUCT.md`

## Context
Two independent bugs prevent quake mode from working correctly on Windows when triggered while a non-Warp application has foreground focus.

### Bug 1: focus not transferred
`WinitWindow::focus()` in `crates/warpui/src/windowing/winit/window.rs:1080` had two branches: if the window was already visible it called `focus_window()`, otherwise it called `set_visible(true)` and relied on visibility implying focus. On Windows, `set_visible(true)` does not steal foreground focus from another application — an explicit `SetForegroundWindow` (via winit's `focus_window()`) is required. The quake window was hidden via `set_visible(false)`, so re-showing it always took the `set_visible(true)` branch and never called `focus_window()`.

### Bug 2: incorrect window size
All Windows monitor queries in `crates/warpui/src/windowing/winit/window/windows_wm.rs` routed through `get_active_window_handle()`, which requires a focused + visible Warp window. When no Warp window has focus, this fails and `active_display_bounds()` falls back to a hardcoded `DEFAULT_WINDOW_SIZE` (1280×800). The quake window is then sized as a percentage of that default instead of the actual display dimensions.

### Relevant code
- `crates/warpui/src/windowing/winit/window.rs:1080-1092` — `WinitWindow::focus()`
- `crates/warpui/src/windowing/winit/window/windows_wm.rs` — all Windows monitor query methods
- `crates/warpui/src/windowing/winit/window.rs:222-227` — `WindowManager::show_window_and_focus_app` (calls `focus()`)
- `app/src/root_view.rs:1481-1507` — quake mode toggle, hidden→visible branch

## Proposed changes

### 1. Always call `focus_window()` in `WinitWindow::focus()`
Restructure `focus()` so `focus_window()` is called unconditionally after the window is made visible or un-minimized. The previous code only called it in the already-visible branch.

Before:
```
if visible → set_minimized(false); focus_window()
else       → set_visible(true)    // hoped this would also focus
```

After:
```
if visible → set_minimized(false)
else       → set_visible(true)
focus_window()                    // always, regardless of prior visibility
```

This fixes Behavior 1 and 2.

### 2. Decouple monitor queries from active-window requirement
Split the Windows monitor methods into two categories:

**Global queries** (don't care which monitor): `get_primary_monitor_handle`, `get_available_monitors`, `get_available_monitor_count`. These only need *any* winit window handle to access platform APIs. Add `get_any_window_handle()` which returns the first available window regardless of focus, and call it directly from these methods.

**Active-monitor queries** (need to know which monitor the user is on): `get_active_monitor`, `get_current_monitor_id`, `get_active_monitor_logical_bounds`. When a Warp window has focus, use its `current_monitor()`. When no Warp window has focus, fall back to `get_foreground_monitor()`, which uses Win32 `GetForegroundWindow` + `MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)` to find the monitor of the app that has keyboard focus. This is the window that triggered the global hotkey.

This fixes Behavior 3 and 4. The foreground-window approach is preferred over cursor position because the cursor may be on a different monitor than the window handling keypress events.

### 3. Add Win32 feature dependencies
Enable `Win32_Graphics_Gdi` (for `MonitorFromWindow`, `MONITOR_DEFAULTTONEAREST`) and `Win32_UI_WindowsAndMessaging` (for `GetForegroundWindow`) in `crates/warpui/Cargo.toml`.

## Testing and validation
- Manual: configure quake mode with 100% width, focus a non-Warp app, press the hotkey. Verify the quake window receives focus and spans the full display width. (Behavior 1, 3)
- Manual: with a Warp window focused, press the hotkey. Verify existing behavior is preserved. (Behavior 2)
- Manual (multi-monitor): focus an app on monitor B, press the hotkey. Verify the quake window appears on monitor B at the correct size. (Behavior 4)
- Manual: verify macOS quake mode is unaffected — changes are behind `#[cfg(windows)]` and winit-only code paths. (Behavior 6)
