# PRODUCT.md — Opt-in native left-drag selection in TUIs

Issue: https://github.com/warpdotdev/warp/issues/10353
Reference implementation: `spalagu/warp:fix/left-drag-selection` (commit `f1c38c76`)

## Summary

Add a new `terminal.native_left_drag_select_enabled` setting (default `false`) to the AltScreenReporting group. When enabled, left mouse button events (down, drag, up) inside a full-screen TUI bypass the mouse-reporting protocol and are handled by Warp's native selection — same effect as if `Shift` were held for that gesture.

This matches iTerm2's long-standing default of "left-button drag is always native selection, hold a modifier to forward to the TUI", and resolves the discoverability gap where macOS users expect bare left-drag to select but Warp instead forwards it to the running application.

The setting defaults to `false` so no existing user sees a behavior change.

## Problem

In full-screen TUIs (Claude Code, vim, htop, tmux, fzf overlays, etc.), Warp currently forwards bare left-button drag gestures to the application via mouse reporting. The application owns that gesture, Warp creates no native selection, and on macOS `Cmd+C` — which is intercepted by the terminal app and never reaches the TUI as stdin — therefore copies nothing visible to the user.

The existing `Shift`-drag bypass technically solves this, but:

- It is not surfaced anywhere in the UI; users have to read documentation to know about it.
- It conflicts with macOS muscle memory where bare left-drag selects in every other text-displaying app (Mail, Notes, Safari, native Terminal.app).
- For users who rely heavily on copy-from-TUI workflows (LLM chat output in TUI agents, log inspection in `htop` / `k9s`, vim yank-to-system-clipboard), holding Shift on every selection is friction.

The pain is amplified for CJK / pinyin users who already context-switch between IME and copy actions.

## Goals

- Provide a persistent, user-configurable opt-in that makes bare left-drag in mouse-reporting TUIs create a Warp native selection.
- Preserve current behavior by default — no change for users who don't enable the setting.
- Preserve all non-left-drag TUI mouse functionality (right-click, scroll, middle-click, modifier+click) regardless of the setting.
- Expose the toggle through the same surfaces as the other AltScreenReporting toggles (Settings UI, command palette, settings JSON, context flag for keybindings).
- Match the working reference implementation (`f1c38c76`) so the spec describes a known-good design rather than a new proposal.

## Non-goals

- **Not the new default.** Leave existing behavior intact for all current users.
- **Not removing `Shift`-drag.** The existing `Shift`-modifier bypass continues to work regardless of the new setting.
- **Not a transient `Option`-key bypass.** A modifier-based per-gesture bypass is complementary but separate (see #2990 / #3280) and out of scope here.
- **Not changing `Cmd+C` routing, stdin byte generation, or TUI clipboard integrations.** The setting only affects whether left-button events are intercepted by Warp's selection path before reaching mouse reporting.
- **Not changing right-click / scroll / middle-click / modifier+click behavior** — those continue to follow `terminal.mouse_reporting_enabled` / `terminal.scroll_reporting_enabled` exactly as today.
- **Not a per-app or per-TUI heuristic.** No special-casing of Claude Code / vim / tmux / etc.

## User experience

### Default (setting = `false`)

Identical to today's behavior. Bare left-drag in a mouse-reporting TUI is forwarded to the application. `Shift`-drag still creates Warp native selection.

### When enabled (setting = `true`)

| Mouse gesture inside mouse-reporting TUI | Behavior |
|---|---|
| Bare left-button down → drag → up | **Native Warp selection** (new) |
| `Shift`-modified left-drag | Native Warp selection (unchanged) |
| Right-click | Forwarded to TUI (unchanged) |
| Scroll wheel / trackpad scroll | Follows `terminal.scroll_reporting_enabled` (unchanged) |
| Middle-click | Forwarded to TUI (unchanged) |
| Modifier+click (Cmd / Ctrl / Option) | Forwarded to TUI (unchanged) |

After the native selection exists, `Cmd+C` copies through Warp's normal clipboard path. Selection rendering, multi-click semantics, and rectangular selection follow existing native-selection rules unchanged.

The setting takes effect uniformly in alt-screen rendering and in active long-running command blocks that currently participate in mouse reporting.

### Settings surfaces

- **Settings → Features → Terminal**: a new toggle row labeled **"Native Left-Drag Selection"**, placed after the existing "Enable Focus Reporting" row, using the same widget shape and sync indicator as other AltScreenReporting toggles.
- **Command palette**: searchable as `native left drag select`. The `ToggleSettingActionPair` registration mirrors the other AltScreenReporting toggles.
- **Settings search index**: terms include `native left drag select`, `cmd c copy`, `iterm`, `cjk` so users searching for the actual pain points reach the toggle.
- **`~/.warp/user_preferences.json`**: writable as `terminal.native_left_drag_select_enabled = true`.
- **Keybinding context flag**: `NATIVE_LEFT_DRAG_SELECT_CONTEXT_FLAG` so users can bind enable/disable actions consistently with other toggleable settings.

### Telemetry

A new `FeaturesPageAction::ToggleNativeLeftDragSelect` variant is recorded with the same shape as the other AltScreenReporting toggle telemetry (which-toggle + new-value).

## Success criteria

1. Setting defaults to `false`. Existing users see no behavior change after upgrade.
2. Toggle is reachable from Settings UI, command palette, and JSON.
3. With setting enabled: bare left-drag in Claude Code / vim / htop / tmux selects natively in Warp; `Cmd+C` copies that selection.
4. With setting disabled: bare left-drag still forwards to TUI as today.
5. Right-click / scroll / middle-click / modifier+click are unaffected by the setting in either state.
6. `Shift`-drag continues to create Warp native selection regardless of setting state.
7. Setting works identically in alt-screen rendering and in active long-running blocks.
8. `terminal.mouse_reporting_enabled`, `terminal.scroll_reporting_enabled`, `terminal.focus_reporting_enabled` retain their existing defaults, labels, command actions, and behavior.

## Validation

Reference impl already validates the following manually on macOS:

- Claude Code: bare drag selects + `Cmd+C` copies; right-click context menu still shows; scroll still scrolls TUI history.
- vim: bare drag selects across visual buffer + `Cmd+C` copies; `Shift`-drag also works; visual mode triggered via `v` / `V` is unaffected.
- htop: bare drag selects column text + `Cmd+C` copies; `F-key` interactions still work.
- tmux (default mouse mode): bare drag selects across panes + `Cmd+C` copies; pane-resize drag (which uses `option`+drag in tmux config) still forwards.

Automated test coverage in `app/src/terminal/view_tests.rs` is updated for the new `should_intercept_mouse` signature; existing alt-screen mouse tests continue to pass.

## Open questions

- **Setting label**: "Native Left-Drag Selection" matches the reference impl. Alternative: "iTerm2-Style Left-Drag Selection" — more discoverable for users coming from iTerm2 but introduces a brand-name dependency.
- **Default value, future**: keep `false` permanently for backward compatibility, or consider flipping to `true` in a future major version after sufficient telemetry?
- **Per-app override**: out of scope here; could be a follow-up if telemetry shows users want different behavior in specific TUIs (e.g., always select in Claude Code but always forward in tmux).
