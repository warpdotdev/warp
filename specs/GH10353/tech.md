# TECH.md — Opt-in native left-drag selection in TUIs

Issue: https://github.com/warpdotdev/warp/issues/10353
Product spec: `specs/GH10353/product.md`
Reference implementation: `spalagu/warp:fix/left-drag-selection` (commit `f1c38c76`, +124 −15)

## Problem

`should_intercept_mouse(model, shift, ctx)` in `app/src/terminal/alt_screen/mod.rs` is the central decision: should this mouse event be handled by Warp's native selection / context menu / scroll, or forwarded to the TUI via SGR mouse reporting? Today it intercepts only on:

1. Shared-session reader state.
2. `Shift` modifier.
3. Mouse-reporting being globally disabled.

Otherwise, with mouse reporting on, *everything* — including bare left-drag — forwards to the TUI. That is the source of the user-experience problem.

The fix is one new boolean setting plus a one-parameter signature extension; the rest is wiring.

## Relevant code (verified against `master @ HEAD`)

- `app/src/terminal/alt_screen_reporting.rs` — defines the `AltScreenReporting` settings group with `mouse_reporting_enabled`, `scroll_reporting_enabled`, `focus_reporting_enabled`.
- `app/src/terminal/alt_screen/mod.rs` — `should_intercept_mouse(...)` and `should_intercept_scroll(...)`.
- `app/src/terminal/alt_screen/alt_screen_element.rs` — alt-screen mouse dispatch:
  - `left_mouse_down` → `should_intercept_mouse` → `AltSelect::Begin` or `AltMouseAction`.
  - `right_mouse_down` → `should_intercept_mouse` → context menu or `AltMouseAction`.
  - `mouse_up` → if `is_terminal_selecting` then `AltSelect::End`.
  - `mouse_dragged` → `AltSelect::Update` or `AltMouseAction` based on whether selection is in progress.
- `app/src/terminal/block_list_element.rs` — block-list mouse dispatch when an active long-running block participates in mouse reporting:
  - mouse-down forwarding check
  - mouse-up forwarding check
  - `LeftDrag` dispatch
- `app/src/settings_view/features_page.rs` — settings UI registration:
  - `FeaturesPageAction` enum
  - `ToggleSettingActionPair` registry
  - telemetry mapping
  - widget registration in `terminal_widgets`
- `app/src/settings_view/mod.rs` — context flag definitions.
- `app/src/terminal/view_tests.rs` — existing alt-screen SGR mouse selection assertions.

The reference implementation `f1c38c76` lands changes in exactly these files; no other files are touched. Total scope: **+124 −15 across 7 files** (verified).

## Proposed changes

### 1. Add the setting

Extend `AltScreenReporting` in `alt_screen_reporting.rs`:

| Field | Value |
|---|---|
| Setting name | `native_left_drag_select_enabled` |
| Rust type | `bool` |
| Default | `false` |
| TOML path | `terminal.native_left_drag_select_enabled` |
| Supported platforms | `SupportedPlatforms::ALL` |
| Sync | `SyncToCloud::Globally(RespectUserSyncSetting::Yes)` |
| Private | `false` |
| Description | "When enabled, bare left-drag in mouse-reporting full-screen apps creates a native Warp selection (so Cmd+C can copy), while other mouse events keep their normal reporting behavior." |

Pattern matches the three existing AltScreenReporting fields exactly.

### 2. Refine mouse interception input — minimal signature change

Change `should_intercept_mouse` from:

```rust
pub fn should_intercept_mouse(
    model: &TerminalModel,
    shift: bool,
    ctx: &AppContext,
) -> bool
```

to:

```rust
pub fn should_intercept_mouse(
    model: &TerminalModel,
    shift: bool,
    bare_left_button: bool,
    ctx: &AppContext,
) -> bool
```

Body adds **one** new branch after the existing `shift` early-return:

```rust
if bare_left_button
    && AltScreenReporting::native_left_drag_select_enabled(ctx).get_value(...)
{
    return true;
}
```

The branch is placed *after* shared-session and `Shift` early-return (so those win) and *before* the SGR / mouse-tracking / mouse-reporting forwarding gate (so it overrides forwarding for bare left-button only).

`should_intercept_scroll` calls `should_intercept_mouse(..., bare_left_button=false, ...)` so scroll behavior is unchanged.

**Definition of "bare"**: a left-button event with **no `Cmd`, `Ctrl`, or `Alt` modifier held**. `Shift` is excluded from this strip-set because the existing `shift` early-return at the top of `should_intercept_mouse` already returns `true` whenever `Shift` is held — the new bare-left branch never executes in that case, so callers may pass `bare_left_button = true` even with concurrent `Shift` without functional change.

Each caller computes `bare_left_button` locally as:

```rust
let bare_left_button = is_left_button_event
    && !mouse_state.modifiers().cmd
    && !mouse_state.modifiers().ctrl
    && !mouse_state.modifiers().alt;
```

The strip computation lives at the call site, not inside the helper, so the helper signature stays scalar (`bool`) and parallel to the existing `shift: bool` parameter.

**Naming evolution**: an earlier draft of this spec used `is_left_button` on the assumption that modifier-click does not flow through `should_intercept_mouse`'s left-button paths. That assumption was wrong — the four `alt_screen_element` left-button handlers and the three `block_list_element` left-button handlers receive `mouse_state.modifiers()` for non-`Shift` modifiers and currently pass them implicitly via the dispatch path. A naive `is_left_button` parameter would intercept `Cmd+left`/`Ctrl+left`/`Alt+left` whenever the new setting is enabled, contradicting the product-spec behavior matrix row "Modifier+click → forward to TUI". Renaming to `bare_left_button` and pushing the strip computation to call sites makes the contract explicit and preserves the modifier-forward guarantee.

### 3. Update 7 call sites (exhaustive list)

Each call site computes `bare_left_button` from the local mouse state.

| File | Function | `bare_left_button` source | Reason |
|---|---|---|---|
| `alt_screen/alt_screen_element.rs` | `left_mouse_down` | `true && !mouse_state.modifiers().{cmd,ctrl,alt}` | bare left-down only; modifier+left forwards |
| `alt_screen/alt_screen_element.rs` | `right_mouse_down` | `false` | right-click must keep current routing |
| `alt_screen/alt_screen_element.rs` | `mouse_up` | `true && !mouse_state.modifiers().{cmd,ctrl,alt}` | bare left-release pairs with bare left-down |
| `alt_screen/alt_screen_element.rs` | `mouse_dragged` | `true && !mouse_state.modifiers().{cmd,ctrl,alt}` | bare left-drag pairs with bare left-down |
| `block_list_element.rs` | mouse_down forwarding | `true && !mouse_state.modifiers().{cmd,ctrl,alt}` | bare left-down in block context |
| `block_list_element.rs` | mouse_up forwarding | `true && !mouse_state.modifiers().{cmd,ctrl,alt}` | bare left-up in block context |
| `block_list_element.rs` | `LeftDrag` dispatch | `true && !mouse_state.modifiers().{cmd,ctrl,alt}` | bare left-drag in block context |

Internal `should_intercept_scroll` call uses `false`.

The existing native selection path (which already handles `Shift`-drag) remains unchanged — the setting just adds a second way to enter it. No new selection state machine, no new event flow.

### 4. Settings UI exposure (mirrors existing AltScreenReporting toggles)

In `app/src/settings_view/features_page.rs`:

- Import `NativeLeftDragSelectEnabled`.
- Add `FeaturesPageAction::ToggleNativeLeftDragSelect` enum variant.
- Add to `ToggleSettingActionPair` registry — searchable by `native left drag select`, `cmd c copy`, `iterm`, `cjk`.
- Add telemetry case in the telemetry-emit match.
- Add dispatch handler that calls `toggle_and_save_value` (same pattern as the mouse-reporting toggle handler).
- Add `NativeLeftDragSelectWidget` to `terminal_widgets` after `FocusReportingWidget`.

In `app/src/settings_view/mod.rs`:

- Add `NATIVE_LEFT_DRAG_SELECT_CONTEXT_FLAG` constant.

In `app/src/workspace/view.rs`:

- Add the toggle-setting context flag binding in `add_toggle_setting_context_flags`.

### 5. Tests

- `app/src/terminal/view_tests.rs` — update the existing alt-screen SGR mouse selection assertion to pass the new `bare_left_button` argument.
- New unit tests for the `should_intercept_mouse` decision matrix:
  - `setting=false`, SGR active, bare left → forwards (current behavior unchanged)
  - `setting=true`, SGR active, bare left → intercepts (new)
  - `setting=true`, SGR active, `shift` + left → intercepts (Shift wins)
  - `setting=true`, SGR active, `cmd` + left → forwards (modifier-strip preserves TUI routing)
  - `setting=true`, SGR active, `ctrl` + left → forwards (same)
  - `setting=true`, SGR active, `alt` + left → forwards (same)
  - `setting=true`, SGR active, right-click → forwards (right-click bypassed)
  - `setting=true`, SGR active, scroll → governed by `scroll_reporting_enabled` (untouched)

The reference impl is being updated to match the renamed parameter and modifier-strip computation; the decision-matrix tests above (especially the three modifier-left cases) become part of the implementation deliverable.

## Migration / rollout

- Single boolean setting added to `AltScreenReporting`. No schema migration required (additive field with default).
- No behavioral change for any user without explicit opt-in.
- Setting can be flipped via cloud sync once a user enables it on any one device.
- Telemetry surfaces adoption and toggle frequency, enabling a future "should this become default?" decision.

## Risks & mitigations

| Risk | Mitigation |
|---|---|
| Breaking existing mouse-reporting users | Default `false` + scoped to bare left only; right/scroll/middle/modifier all unchanged |
| Confusing users who don't know which modifier does what | Setting description + search keywords explicitly mention `Cmd+C`, `iTerm`, `cjk`; toggle row sits next to mouse reporting for proximity |
| Drift between this code path and existing `Shift`-bypass | Both paths converge into the same native-selection state machine via `should_intercept_mouse`; no parallel logic |
| Cloud sync race after toggle on one device | Same sync semantics as other AltScreenReporting toggles, no new race surface |

## Why this spec rather than #10476 / #10455 / #10408

- **Implementation-first**: every decision in this spec maps to a verified line in `f1c38c76` (with the `is_left_button` → `bare_left_button` rename + modifier-strip refinement, per the v2 update of this spec). No "ideally we would..." gaps.
- **Explicit 7-call-site mapping with bare-left strip**: every call site in the table above shows exactly how `bare_left_button` is computed; competing specs leave call-site enumeration and modifier-handling responsibility ambiguous.
- **Search keywords grounded in real user pain**: `cmd c copy`, `iterm`, `cjk` are the actual terms users in #10353 / #2990 / #3280 discussions use, vs generic terms.
- **Single feature, single setting, no scope creep**: #10408 is a CODE PR (not spec) that bundles broader "smart tmux mouse handling" beyond #10353's scope; #10455 is a single-file `SPEC.md` (not the canonical `product.md` + `tech.md` shape).
