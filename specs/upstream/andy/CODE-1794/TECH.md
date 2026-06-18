# TECH — Distinguish left vs right Alt on Windows and Linux

Linear: [CODE-1794](https://linear.app/warpdotdev/issue/CODE-1794/windowslinux-right-alt-isnt-recognized-breaking-right-alt-as-meta)

See `PRODUCT.md` for user-visible behavior.

## Context

The extra-meta-keys setting is consumed by the `apply_extra_meta_keys` event munger in `app/src/lib.rs:461-480`. It reads `details.left_alt` and `details.right_alt` from the `KeyEventDetails` attached to a `KeyDown` event and rewrites the keystroke (strips `alt`, sets `meta`) when the corresponding side is enabled.

On macOS, those flags are populated by the platform-native NSEvent path (`crates/warpui/src/platform/mac/event.rs`) which reads `NSEvent.modifierFlags` and correctly distinguishes the two Option keys.

On Windows and Linux, events flow through winit (`crates/warpui/src/windowing/winit/event_loop/...`). Before this change, `convert_keyboard_input_event` in `crates/warpui/src/windowing/winit/event_loop/key_events.rs:125-134` populated `KeyEventDetails` as:

```rust path=null start=null
details: KeyEventDetails {
    left_alt: window_state.modifiers.alt_key(),
    right_alt: false,
    key_without_modifiers,
},
```

`winit::keyboard::ModifiersState` is side-agnostic: `alt_key()` returns true whenever any Alt is held, and there is no corresponding `left_alt_key()` / `right_alt_key()` on that type. Per-side state on `winit::event::Modifiers` (`lalt_state()` / `ralt_state()`) is unreliable on some Linux backends. As a result, right Alt was never reported as right Alt, and left Alt was set to "any Alt is held", so `apply_extra_meta_keys` could never distinguish the two.

Winit does reliably report the physical key of each individual `KeyboardInput` event via `event.physical_key`, which is a `PhysicalKey::Code(KeyCode)`. `KeyCode::AltLeft` and `KeyCode::AltRight` are distinct variants and are already used elsewhere in this file (`try_from_winit_keycode`, `event_loop/mod.rs:91-107`) to produce side-aware `ModifierKeyChanged` events for voice input and similar consumers.

Relevant files:

- `app/src/lib.rs:458-480` — `apply_extra_meta_keys`, the consumer of `details.left_alt` / `right_alt`.
- `app/src/settings/mod.rs:181-199` — `ExtraMetaKeys` struct with `left_alt` / `right_alt` bools.
- `crates/warpui/src/windowing/winit/event_loop/mod.rs` — `WindowState` and event dispatch for the winit platform.
- `crates/warpui/src/windowing/winit/event_loop/key_events.rs` — winit → warpui keyboard event conversion.
- `crates/warpui_core/src/event.rs` — `KeyEventDetails` definition.

## Proposed changes

Track per-side Alt press state in `WindowState` based on `PhysicalKey::Code(AltLeft/AltRight)` from `KeyboardInput` events, and surface those flags through `KeyEventDetails` so the existing `apply_extra_meta_keys` munger distinguishes the two sides without any other changes.

1. `crates/warpui/src/windowing/winit/event_loop/mod.rs` — Add two booleans to `WindowState`:
   - `left_alt_pressed: bool`
   - `right_alt_pressed: bool`
   Initialize both to `false` in `WindowState::new`. These are per-window state to match the existing `modifiers` field.

2. In `convert_window_event`, inside `WindowEvent::KeyboardInput`, update the flags before the existing modifier-key early return:
   - Match on `event.physical_key` for `KeyCode::AltLeft` / `KeyCode::AltRight`.
   - Set the corresponding flag to `event.state == ElementState::Pressed`.
   - Do this before the `try_from_winit_keycode` short-circuit so the flag is always updated, even when the event is forwarded as `ConvertedEvent::ModifierKeyChanged` instead of flowing through `convert_keyboard_input_event`.

3. Add two belt-and-suspenders resets so dropped release events can't leave a side "stuck":
   - `WindowEvent::ModifiersChanged`: if the resulting `state.alt_key()` is false, clear both flags.
   - `WindowEvent::Focused(false)`: clear both flags before the existing focus-out bookkeeping.
   These mirror how the synthetic-event guard already protects against Alt+Tab races in `convert_keyboard_input_event` (`key_events.rs:81-83`).

4. `crates/warpui/src/windowing/winit/event_loop/key_events.rs` — In `convert_keyboard_input_event`, populate `KeyEventDetails` from the tracked flags instead of from `ModifiersState`:
   ```rust path=null start=null
   details: KeyEventDetails {
       left_alt: window_state.left_alt_pressed,
       right_alt: window_state.right_alt_pressed,
       key_without_modifiers,
   },
   ```

5. `app/src/lib.rs` — Tag the existing `log::info!("Treating option as meta")` with which side triggered the conversion (`left alt`, `right alt`, or `left+right alt`). This is a small log change that makes future bug reports triageable from logs alone.

No new public types or cross-crate API changes. No setting migration. No feature flag: the change is bounded to the Windows/Linux winit path and strictly narrows existing incorrect behavior (the old code treated any Alt as `left_alt: true` and never reported `right_alt`).

### Tradeoffs considered

- **Use `winit::event::Modifiers::lalt_state()` / `ralt_state()` in `ModifiersChanged`.** Simpler to write, but per-side state on that API is documented as unreliable on some Linux/X11 backends (can return `Unknown`). `PhysicalKey::Code` is the portable, stable signal.
- **Query the Windows API directly (`GetKeyState(VK_LMENU/VK_RMENU)`).** Works on Windows but is platform-specific and redundant with what winit already delivers in `KeyboardInput`.
- **Populate `KeyEventDetails` from `event.physical_key` at `convert_keyboard_input_event` time.** `physical_key` on a non-Alt event tells us which character key is being pressed, not which Alt side is currently held. We need the accumulated per-side modifier state, which is what the `WindowState` flags give us.

## Testing and validation

Invariant references are to the numbered behaviors in `PRODUCT.md`.

- Unit tests in `crates/warpui/src/windowing/winit/event_loop/key_events_tests.rs` (existing file) that drive `convert_keyboard_input_event` with a `WindowState` set to combinations of `left_alt_pressed` / `right_alt_pressed` and assert `KeyEventDetails.left_alt` / `right_alt` round-trip correctly. Covers invariants 2, 3, 4, 5, 10.
- Unit test(s) for `apply_extra_meta_keys` in `app/src/lib.rs` exercising the four `(left_alt, right_alt)` × `(ExtraMetaKeys.left_alt, ExtraMetaKeys.right_alt)` combinations. Covers invariants 2, 3, 4, 5, 11 (via the tagged log message if captured).
- Manual verification on Windows (primary risk surface):
  - With `ExtraMetaKeys { left_alt: true, right_alt: false }`: `LeftAlt+b` sends ESC-b to the PTY; `Ctrl+RightAlt+R` fires the Resume conversation keybinding (invariant 2, 6).
  - With `ExtraMetaKeys { left_alt: false, right_alt: true }`: `RightAlt+b` sends ESC-b; `Ctrl+LeftAlt+R` fires the Resume conversation keybinding (invariant 3, 6).
  - Toggling one setting on the Keys page without touching the other and re-testing (invariant 1, 8).
  - Alt+Tab out of Warp with Alt held, release outside the window, refocus: next character key reports plain Alt (invariant 9).
- Manual verification on Linux (X11 and Wayland) for invariants 2, 3, 5, 9. Confirms the per-side tracking works regardless of the per-side `Modifiers` state reliability caveat.
- Manual verification on macOS that the Option-as-meta path is unchanged (invariant 7) — the change is gated to the winit path and should be a no-op on macOS, but a smoke test is cheap.

## Risks and mitigations

- **Dropped key-release events leave a side "stuck" as pressed.** Mitigated by clearing both flags on `WindowEvent::Focused(false)` and whenever `ModifiersChanged` reports no Alt is held. The common Alt+Tab path hits both.
- **Synthetic focus-in key events.** The existing synthetic-event guard in `convert_keyboard_input_event` (`key_events.rs:81-83`) drops synthetic press events so they cannot re-arm the flag when refocusing. The new tracking runs in `convert_window_event`, before that guard; that is intentional so the flag reflects physical state, but synthetic press events for AltLeft/AltRight on focus-in could briefly set a flag that `Focused(true)` doesn't clear. Mitigated by the `ModifiersChanged` safety net, which fires whenever the real modifier state differs from our tracked state.
- **Other modifier keys (Shift/Ctrl/Cmd).** This change intentionally does not add per-side tracking for those; `ExtraMetaKeys` only exposes Alt sides today, and extending to other modifiers is out of scope.
