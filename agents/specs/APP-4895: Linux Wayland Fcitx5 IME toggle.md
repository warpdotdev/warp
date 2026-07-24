# APP-4895: Linux Wayland Fcitx5 IME toggle

*Spec: Make the Fcitx5 Left Shift input-mode toggle work in Warp on Linux/Wayland without regressing X11 or duplicating IME events.*

## Evidence and current state

The canonical report is [Warp #11524](https://github.com/warpdotdev/warp/issues/11524). On Ubuntu/GNOME Wayland with Fcitx5, configuring Left Shift as the input-mode shortcut works in GNOME Terminal and other terminals but does not change the active input mode in Warp. Follow-up reports reproduce the same class of failure with Ctrl+Space and under X11.

Code-level investigation was attempted before this spec:

- The pinned client checkout is `ebe3d363e053f2791a6682c7f80b124319be1c86`. Comparing its parent with this commit reproduces the activation gap that motivated [#13819](https://github.com/warpdotdev/warp/pull/13819): before that merge, Linux only called `set_ime_allowed(true)` on the X11 path; the current commit enables it for every Linux window and adds Wayland cursor-area/event-loop guards.
- The focused existing modifier tests pass (`6 passed`), but they do not exercise the IME lifecycle or a modifier-only Fcitx5 shortcut. The current environment has no `fcitx5` binary and only a virtual X display, so a native Wayland/Fcitx5 run could not be performed here. The implementation must not claim the user-visible defect is fixed until the required Linux exercises below are run.
- The character-rendering report in the original issue is a separate concern and is intentionally excluded from this spec and implementation.

## PRODUCT

### Desired behavior

1. On a native Linux Wayland session, a focused Warp window activates Fcitx5 and Left Shift toggles the configured input mode exactly as it does in a reference terminal.
2. A Wayland IME session delivers each preedit and commit exactly once to the focused editor/terminal. Preedit updates marked text; commit clears marked text before inserting the committed text.
3. Repeated compositor `Enabled` notifications or identical cursor-area updates do not create an activation/update loop, duplicate committed text, or broken-pipe behavior.
4. Existing X11 IME behavior remains supported, including Left Shift and Ctrl+Space shortcuts. The fix does not alter ordinary Warp keybindings or shell input.

### Key design choices

- Preserve the merged winit text-input-v3 approach from #13819; do not add a second IME backend.
- Treat #13819's Linux enablement as the first candidate fix and add deterministic lifecycle coverage. Only change the Wayland event-loop/key-routing path if a native Fcitx5 reproduction still fails on the pinned commit.
- Keep the X11 cursor-position workaround separate from Wayland deduplication.

### Design alternatives

- **Add a direct Fcitx5/IBus DBus backend.** Rejected: it duplicates winit's native text-input-v3 integration, adds platform-specific lifecycle/dependency ownership, and does not validate the existing merged path.
- **Force XWayland/XIM.** Rejected: it avoids native Wayland rather than fixing it, depends on an X server, and does not match the reported GNOME/Wayland setup.
- **Handle Left Shift as an ordinary Warp keybinding.** Rejected: the shortcut belongs to Fcitx5; consuming or remapping it in Warp would interfere with normal key semantics and would not fix preedit/commit delivery.

## TECH

### Context

- `crates/warpui/src/windowing/winit/window.rs:1457 @ ebe3d363e053f2791a6682c7f80b124319be1c86` creates Linux windows and now calls `set_ime_allowed(true)` unconditionally.
- `crates/warpui/src/windowing/winit/event_loop/mod.rs:1270 @ ebe3d363e053f2791a6682c7f80b124319be1c86` converts modifier-only `KeyboardInput` events into `ModifierKeyChanged`; `:1526` handles `Ime::Enabled`, `Preedit`, `Commit`, and `Disabled`; `:1801` updates the active IME cursor area and distinguishes Wayland from X11.
- `crates/warpui/src/windowing/winit/event_loop/key_events.rs:58 @ ebe3d363e053f2791a6682c7f80b124319be1c86` converts ordinary key events. Its tests cover key normalization but not IME activation or event sequencing.
- Open reports [#5279](https://github.com/warpdotdev/warp/issues/5279) and [#11618](https://github.com/warpdotdev/warp/issues/11618) are coordination references for broader Linux IME/commit support; this ticket owns the Left Shift Wayland regression and must avoid duplicating their scope.

### Proposed changes

1. Keep Linux `set_ime_allowed(true)` for both X11 and Wayland windows, and retain #13819's disabled→enabled edge guard and Wayland cursor-area deduplication.
2. Add a deterministic event-loop test seam (a pure state helper or equivalent) covering:
   - `Ime::Enabled` reports the active cursor position once per disabled→enabled transition;
   - repeated `Enabled` events do not re-enter an update loop;
   - identical cursor areas are deduplicated on Wayland;
   - switching to another window with the same rectangle still updates that window;
   - preedit is ignored while disabled and dispatched as marked text while enabled;
   - commit dispatches `ClearMarkedText` before exactly one `TypedCharacters` event.
3. Run the native Wayland/Fcitx5 reproduction on the pinned #13819 code. If Left Shift still fails, make the smallest targeted correction in `event_loop/mod.rs` or `key_events.rs` that preserves the modifier-only event for Fcitx5 activation and consumes each preedit/commit sequence once. Do not add an alternate backend or change shell/keybinding semantics.
4. Keep the X11 nudge path and existing non-IME keyboard conversion unchanged except for testability.

### Open questions resolved

- **Does #13819 already fix the production activation gap?** It fixes the previously missing Linux Wayland `set_ime_allowed(true)` path, but the end-to-end Fcitx5 exercise and lifecycle regression tests are still required; any remaining failure is limited to the targeted event-loop/key-routing path above.
- **Which framework is in scope?** Native winit text-input-v3 on Wayland plus the existing X11 path. Fcitx5 is the acceptance framework; IBus is optional smoke coverage, not a new backend.
- **What is the relationship to #5279 and #11618?** Coordinate findings and avoid conflicting changes, but do not broaden this ticket into generic IME framework support or unrelated rendering work.
- **What if the environment cannot run Wayland/Fcitx5?** Record the exact mismatch and do not claim the user-visible defect is verified. Code-level tests and `./script/presubmit` remain mandatory, and a human must run the missing visual exercise before merge.

## Validation & verification criteria

All criteria must pass before merge:

1. **Exact regression reproduction.** On native Linux Wayland with Fcitx5 and Left Shift configured as the toggle, focus Warp, press Left Shift repeatedly, and compare the active input mode with a reference terminal on the same session. Warp must toggle on every press without requiring a restart or focus workaround.
2. **Preedit and commit path.** With Fcitx5 active, type an input sequence that produces preedit text, select/commit a candidate, and confirm the focused Warp editor/terminal receives one marked-text update followed by one committed-text insertion. No duplicate text, broken pipe, or event storm is allowed.
3. **IME lifecycle regression tests.** Add and pass focused tests for the state/event sequence described in TECH. The tests must fail against an equivalent pre-fix fixture (Linux IME disabled, repeated cursor updates, or duplicate commit dispatch) and pass on the implementation branch.
4. **Wayland cursor/update behavior.** Verify with a test double or instrumented winit window that identical cursor areas are sent once on Wayland, `Enabled` only triggers the cursor-position report on the disabled→enabled edge, and a focus switch to another window with the same rectangle still sends an update.
5. **X11 compatibility and coordination.** Under X11/Fcitx5, repeat Left Shift and Ctrl+Space input-mode switching, including the issue-reported `XMODIFIERS=@im=fcitx` setup. Candidate selection must commit text to Warp, and the existing X11 cursor-area nudge must remain active. Record any interaction with #5279/#11618 without expanding scope.
6. **Visual proof.** Exercise a running Warp window with computer use on native Wayland/Fcitx5 and capture screenshots or video showing (a) Left Shift toggling the input-mode indicator, (b) preedit text, and (c) committed text in the focused terminal/editor. Attach the proof to the task record and PR, with a reference-terminal comparison.
7. **No collateral damage.** Run the existing focused tests, including `cargo test --manifest-path Cargo.toml -p warpui --lib key_events` and the relevant event-loop tests, plus the new IME tests. Ordinary printable keys, modifier keybindings, focus changes, and X11 keyboard behavior must remain green.
8. **Full repository gate and final repro.** From the repository root, `./script/presubmit` passes. After it completes, repeat the Wayland/Fcitx5 reproduction in a fresh Warp process and confirm Left Shift still toggles input mode.
