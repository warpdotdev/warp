# Foreground window control for headless computer-use recordings — SPEC
Branch: `varoon/cu-background-cursor-fix` (off `master` @ `af6fc40d4`). Combined PRODUCT + TECH. Linux X11 only.
## == PRODUCT ==
**Summary:** On Linux X11, window-scoped computer-use recordings render the wrong cursor. Window control runs through a separate background "agent seat" cursor that the recorder never captures, so the video shows a stray/stationary core cursor while the cursor actually acting is invisible. In environments with **no interactive user** (headless/cloud), stop using the background seat: bring the target window to the foreground and drive the real (core) cursor, so the recorded cursor is the one actually acting.
**Problem:**
- The recorder composites the core X cursor via ffmpeg `-draw_mouse 1` (`crates/computer_use/src/linux/recording.rs:162`). Background window actions are performed by a second XInput2/MPX master — the "agent seat" (`crates/computer_use/src/linux/x11/seat.rs`) — whose cursor the recorder never draws. Window recordings therefore show the wrong cursor. The burned-in click/drag ripples still land correctly (they are coordinate-based), which makes the missing/stray cursor more jarring, not less.
- Screenshots composite no cursor at all today (`crates/computer_use/src/linux/x11/screenshot.rs`, the cursor TODO), so the bug is *latent* there but shares the exact root cause.
- Background computer use exists solely to avoid disturbing a human at the machine (their cursor, focus, modifiers). A headless/cloud session has no such human, so the seat delivers zero benefit while causing the cursor mismatch and carrying extra complexity (second cursor, seat lifecycle/leak reaping).
**Goals:**
- Correct, legible cursor in Linux window-scoped recordings in headless environments.
- Keep full-screen recording and screen-targeted actions unchanged everywhere.
- Keep window-local coordinates and window-scoped capture (`x11grab -window_id`, Composite screenshots).
- Remove the latent wrong-cursor risk for window screenshots.
**Non-goals:**
- Changing background computer use on a real local desktop with a user present — the agent seat stays there.
- macOS (events post directly to the owning pid with no second cursor; recording uses a different avfoundation path).
- Correct-cursor *local* recordings during background CU (accepted limitation; see Follow-ups).
- Covered-window video fidelity (already out of scope; window recording is a foreground-visible capture contract).
**Behavior (testable invariants):**
1. With no interactive user, a window-targeted action raises + focuses the target and is performed with the core pointer/keyboard; window-local coordinates still map onto the captured window exactly as today.
2. With no interactive user, a window-scoped recording shows the real acting cursor (native `-draw_mouse`), consistent with the click/drag overlays.
3. With a user present (local desktop), behavior is unchanged: window actions use the background agent seat and never move the user's cursor or steal focus.
4. Full-screen recordings and screen-targeted actions are byte-identical in all environments.
5. Window screenshots keep window-relative results and covered-window Composite capture; no cursor is drawn into screenshots (unchanged), and any future cursor compositing inherits the correct core cursor in headless envs.
6. If a window recording target cannot be made foreground-visible, start fails as today.
## == TECH ==
**Context (this branch):**
- `recording.rs:162` `-draw_mouse 1` composites the core pointer; `start_window` raises + verifies visibility (`ensure_window_visible_for_recording`) then runs `x11grab -window_id`.
- `x11/mod.rs:182` `perform_actions` reads `options.background_enabled`. When false it forces `Target::Screen` for every action; when true, `Window` targets lazily create and drive the agent seat. Pointer actions raise a covered target via `ensure_window_clickable_at`; window-local→root uses `windows::window_local_to_root`.
- `app/src/ai/agent/api/impl.rs` advertises `supports_background_computer_use = FeatureFlag::BackgroundComputerUse.is_enabled() && computer_use::background_supported()`. `background_supported()` / `probe_background_support` (`x11/mod.rs:93`) is only an XI2/MPX probe — so it is **true on cloud Xvfb**.
- `use_computer.rs` / `request_computer_use.rs` set `background_enabled = FeatureFlag::BackgroundComputerUse.is_enabled()`.
- `start_recording.rs` honors a `Window` recording target only when `BackgroundComputerUse` is enabled (else full-screen).
- `overlay.rs` + `recording.rs` post-process: a `PointerSink` records capture-space pointer events; click/drag rings and text pills are burned in post-stop and are already correct for window targets.
**Design:**
1. **Introduce one "interactive user present" signal** (proposed `computer_use::interactive_user_present()` on Linux, sourced from the existing headless/cloud environment indicator; conservative default = true so desktops are unaffected). The effective decision to use the seat becomes "background feature on **and** a user is present **and** MPX available." Fold the user-present check into `supports_background_computer_use` (`impl.rs`) so a headless session is told background is *not* in use, rather than being advertised a covert capability we actually implement overtly. This is what resolves the "same tool, different behavior by environment" concern: `set_computer_use_target` honestly degrades to plain overt focus when background is unavailable.
2. **Add a foreground window-control branch** in `x11/mod.rs::perform_actions`. When an action carries a `Window` target but the seat is not in use, instead of forcing `Target::Screen`: raise the window (reuse `ensure_window_clickable_at` for pointer; raise + `SetInputFocus` for keyboard), drive the core `screen_mouse`/`screen_keyboard`, and convert window-local→root via `windows::window_local_to_root`. This is a third mode, distinct from both "seat" and "screen": the window stays the coordinate/capture unit but is driven by the real cursor.
3. **Decouple honoring the window recording target from the covert flag** in `start_recording.rs`: honor a `Window` recording target whenever window control is available in *either* mode (seat or foreground), so headless window recordings keep working with the seat off.
4. **Screenshots:** no functional change; the Composite covered-capture path is unchanged. The foreground mode keeps window screenshots working, and the acting cursor being the core cursor removes the latent bug.
**Semantics note:** at the interaction level this is the classic "focus the window and use the one real cursor" — not a new mechanism. Background CU is *covert* focus; this is *overt* focus, chosen only when there is no user to stay covert for. The seat's benefits (no raise, no cursor move, no focus steal) are unobservable when nobody is watching.
**Testing & validation:**
- Unit (`perform_actions`): a `Window` target with the seat off drives the core masters, raises/focuses, and converts coordinates; no `AgentSeat` is created. Seat-on + user-present path is unchanged.
- Recording (Linux/Xvfb): a headless window recording captures the target with the moving core cursor; dimensions/finalization unchanged; overlays still remap through the smart cut.
- Capability: `supports_background_computer_use` is false with no interactive user, true on a user-present desktop with MPX.
- Regression: screen recording + screen actions byte-identical; seat leak reaping unaffected on desktops; presubmit (`./script/format`, `cargo clippy` per `AGENTS.md`) green.
**Risks:**
- Reliability of the "interactive user" signal: a wrong-true on a headless host reintroduces the bug; a wrong-false on a desktop disturbs the user. Default conservatively (assume a user is present) and derive from the same indicator the app already uses for cloud/headless execution.
- Raise/focus side effects under a real WM are intended for the headless/WM-less path only; the seat still covers local desktops.
**Follow-ups:**
- Correct-cursor *local* recordings (opt into foreground control while recording even with a user present) — deferred.
- macOS recording cursor — separate path, not addressed here.
- Implement the screenshot cursor-compositing TODO once foreground mode lands (now safe in headless envs).
