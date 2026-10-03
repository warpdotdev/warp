# PRODUCT.md — Opt-in native left-drag selection in TUIs

Issue: https://github.com/warpdotdev/warp/issues/10353

## Summary

Add an opt-in setting that lets users select text in full-screen terminal applications with a bare left-button drag, without holding Shift. When enabled, Warp treats an unmodified left-button down, drag, and up gesture in a mouse-reporting TUI as a native Warp text-selection gesture so macOS users can use familiar drag-to-select followed by `Cmd+C`.

The feature is disabled by default to preserve existing TUI mouse-reporting behavior for users who rely on bare left-button drags being delivered to the running application.

Figma: none provided.

## Problem

In full-screen TUIs such as Claude Code, vim, htop, tmux, and similar applications, Warp currently forwards bare left-button drag gestures to the application when mouse reporting is active. The application owns that mouse gesture and Warp does not create a native text selection. On macOS, `Cmd+C` is handled by the terminal app rather than being sent as stdin to the TUI, so pressing `Cmd+C` after a TUI-owned selection appears to do nothing from the user's perspective.

Warp already has a `Shift`-drag escape hatch that bypasses mouse reporting and starts native selection, but that shortcut is non-obvious for users accustomed to macOS apps where bare left-drag selects text. The result is repeated confusion and a discoverability problem, especially for users who primarily want to copy text out of TUIs.

## Goals

- Provide a persistent, user-configurable way to make bare left-drag in mouse-reporting TUIs create a native Warp selection.
- Preserve current behavior by default for all existing users.
- Preserve non-left-drag TUI mouse functionality when the new setting is enabled.
- Expose the setting through the same surfaces as the existing mouse, scroll, and focus reporting toggles.
- Make the behavior clear enough that users understand this is a selection-first mode for left-button drag gestures, not a global disablement of all TUI mouse support.

## Non-goals

- Do not make native left-drag selection the default behavior.
- Do not remove or change the existing `Shift`-drag native selection bypass.
- Do not implement a transient `Option`-key or other per-gesture bypass in this change. A modifier-based bypass is complementary but separate from this persistent opt-in.
- Do not change `Cmd+C` routing, stdin byte generation, or TUI clipboard integrations.
- Do not disable right-click reporting, scroll-wheel reporting, middle-click reporting, focus reporting, or other non-left-drag TUI mouse behavior.
- Do not introduce app-specific behavior for Claude Code, vim, htop, tmux, or any other TUI.

## Figma / design references

Figma: none provided.

No visual mock was provided. The settings UI should follow the existing Features → Terminal toggle rows used by "Enable Mouse Reporting", "Enable Scroll Reporting", and "Enable Focus Reporting".

## User experience

### Default behavior

1. The setting is named "Native Left-Drag Selection" in Settings → Features → Terminal.
2. The setting persists as `terminal.native_left_drag_select_enabled`.
3. The default value is `false`.
4. With the setting disabled, existing behavior is unchanged:
   - Bare left-button drag in a mouse-reporting TUI is forwarded to the application.
   - Warp native selection still requires the existing bypass behavior, such as `Shift`-drag.
   - `Cmd+C` only copies when Warp has a native selection to copy.

### Behavior when enabled

1. When a pane is running a full-screen or mouse-reporting terminal application and the setting is enabled, an unmodified left-button down inside the terminal grid starts Warp's native selection instead of forwarding that left-button press to the application.
2. Continuing the same unmodified left-button drag updates Warp's native selection.
3. Releasing the unmodified left button ends Warp's native selection.
4. After the selection exists in Warp, `Cmd+C` copies the selected text through Warp's normal clipboard behavior.
5. Existing native selection behavior, including `Shift`-drag, remains valid and should not regress.
6. Selection type behavior remains the same as existing native selection:
   - Single-click drag creates a normal text selection.
   - Multi-click and smart or semantic selection behavior follows existing Warp selection rules.
   - Rectangular selection behavior follows the existing feature-flagged rectangular selection rules.
7. The setting applies consistently in alt-screen rendering and in long-running active command blocks where Warp forwards mouse events to the active TUI.

### Mouse reporting interactions

1. The new setting affects only the left-button drag selection path that would otherwise be forwarded to a mouse-reporting TUI.
2. Right-click behavior is unchanged:
   - If Warp would show its native context menu today, it still does.
   - If right-click would be forwarded to the TUI today, it still is forwarded.
3. Scroll-wheel and trackpad scroll reporting are unchanged and continue to obey `terminal.scroll_reporting_enabled` and `terminal.mouse_reporting_enabled`.
4. Middle-click behavior is unchanged.
5. Modifier-assisted mouse gestures are not newly intercepted by this setting. Gestures that already have special Warp behavior, such as `Shift`-drag for native selection, keep that behavior; other modifier-click or modifier-drag combinations continue to follow the same reporting behavior they had before this setting.
6. If `terminal.mouse_reporting_enabled` is disabled, Warp already intercepts mouse input for native behavior; this setting does not need to add a separate behavior change in that state.

### Settings and command surfaces

1. The toggle appears in Settings → Features → Terminal near the existing mouse reporting controls.
2. The toggle uses the same local-only or sync indicator conventions as the other AltScreenReporting settings.
3. The setting is searchable from the settings page with terms including "native left drag select", "left drag", "native selection", and "mouse reporting".
4. The command palette exposes a toggle action discoverable by searching for "native left drag select".
5. Keybinding integration exposes a context flag so users can bind enable or disable actions consistently with other toggleable settings.
6. The setting is supported on all platforms unless a later product decision limits the UI by OS. The macOS pain point is primary, but the behavior itself is not macOS-specific.

## Success criteria

1. Existing users see no behavior change after updating because the setting defaults to `false`.
2. A user can enable "Native Left-Drag Selection" from Settings → Features → Terminal.
3. A user can toggle the setting from the command palette.
4. With the setting enabled, bare left-drag inside a mouse-reporting TUI creates a visible Warp native selection.
5. With the setting enabled, pressing `Cmd+C` on macOS after making that selection copies the selected text to the system clipboard.
6. With the setting disabled, the same bare left-drag is still forwarded to the TUI when mouse reporting is active.
7. `Shift`-drag still creates Warp native selection regardless of the new setting value.
8. Right-click, scroll-wheel, trackpad scroll, middle-click, and non-Shift modifier mouse gestures continue to follow their previous behavior when the new setting is enabled.
9. `terminal.mouse_reporting_enabled`, `terminal.scroll_reporting_enabled`, and `terminal.focus_reporting_enabled` retain their existing defaults, UI labels, command actions, and behavior.
10. The feature works both in full alt-screen TUIs and in active long-running command blocks that participate in mouse reporting.
11. No selected text is copied unless Warp has a native selection, preserving normal clipboard semantics.

## Validation

- Automated tests cover the mouse-interception decision logic for:
  - setting disabled with SGR mouse reporting active
  - setting enabled with bare left-button input
  - Shift bypass behavior
  - non-left-button input remaining reportable
  - scroll interception still following the scroll reporting setting
- Existing alt-screen selection tests continue to pass after the `should_intercept_mouse` signature and behavior changes.
- Settings tests or compile-time coverage verify the new setting, action, context flag, telemetry action, and widget are wired consistently with the other AltScreenReporting toggles.
- Manual validation on macOS:
  - Run a TUI that enables mouse reporting, such as Claude Code, vim, tmux, or htop.
  - Confirm bare left-drag does not create Warp native selection when the setting is disabled.
  - Enable "Native Left-Drag Selection".
  - Confirm bare left-drag creates a Warp native selection.
  - Confirm `Cmd+C` copies that selection.
  - Confirm right-click, scroll, and existing TUI interactions outside bare left-drag still behave as before.
- Manual validation on at least one non-macOS platform or a platform-agnostic automated test confirms the setting does not depend on macOS-only APIs.

## Open questions

- Should the UI description explicitly mention macOS `Cmd+C`, or keep the copy platform-neutral and rely on docs or release notes for the macOS motivation?
- Should a future follow-up add an `Option`-key transient bypass so users can temporarily forward a left-drag to the TUI while the persistent setting is enabled?
