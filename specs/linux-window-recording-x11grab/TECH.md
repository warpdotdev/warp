# Linux Window Recording via Native X11 Grab — TECH.md

## Context
This implements the behavior in `PRODUCT.md` by replacing the Linux `Target::Window` recording path with native FFmpeg `x11grab -window_id` after a best-effort foreground-visibility check.

Relevant pre-change code researched at `d0ecbd031f64575f322e20aee649cf4d8bca49c8`:
- [`crates/computer_use/src/linux/recording.rs (1-59) @ d0ecbd031f64575f322e20aee649cf4d8bca49c8`](https://github.com/warpdotdev/warp/blob/d0ecbd031f64575f322e20aee649cf4d8bca49c8/crates/computer_use/src/linux/recording.rs#L1-L59) selects `Target::Screen` vs `Target::Window`.
- [`crates/computer_use/src/linux/recording.rs (107-177) @ d0ecbd031f64575f322e20aee649cf4d8bca49c8`](https://github.com/warpdotdev/warp/blob/d0ecbd031f64575f322e20aee649cf4d8bca49c8/crates/computer_use/src/linux/recording.rs#L107-L177) starts the existing full-display `x11grab` FFmpeg process and handles startup validation.
- [`crates/computer_use/src/linux/recording.rs (187-478) @ d0ecbd031f64575f322e20aee649cf4d8bca49c8`](https://github.com/warpdotdev/warp/blob/d0ecbd031f64575f322e20aee649cf4d8bca49c8/crates/computer_use/src/linux/recording.rs#L187-L478) previously implemented window recording through Composite/GetImage/rawvideo and a separate stdin finalization path.
- [`crates/computer_use/src/linux/x11/windows.rs (241-345) @ d0ecbd031f64575f322e20aee649cf4d8bca49c8`](https://github.com/warpdotdev/warp/blob/d0ecbd031f64575f322e20aee649cf4d8bca49c8/crates/computer_use/src/linux/x11/windows.rs#L241-L345) provides geometry, window-local-to-root coordinate conversion, hit-testing, and raising.
- [`crates/computer_use/src/linux/x11/mod.rs (135-169) @ d0ecbd031f64575f322e20aee649cf4d8bca49c8`](https://github.com/warpdotdev/warp/blob/d0ecbd031f64575f322e20aee649cf4d8bca49c8/crates/computer_use/src/linux/x11/mod.rs#L135-L169) already uses raise-and-poll hit-testing for pointer actions.
- [`app/src/ai/blocklist/action_model/execute/start_recording.rs (59-93) @ d0ecbd031f64575f322e20aee649cf4d8bca49c8`](https://github.com/warpdotdev/warp/blob/d0ecbd031f64575f322e20aee649cf4d8bca49c8/app/src/ai/blocklist/action_model/execute/start_recording.rs#L59-L93) only honors a window recording target when `BackgroundComputerUse` is enabled.

The key tradeoff is intentional: native FFmpeg window capture is cheaper and preserves wall-clock cadence better than the custom rawvideo loop, but it only records what X11/FFmpeg can capture for a visible target window. Covered-window video fidelity is no longer the recording contract.

## Implementation
1. `crates/computer_use/src/linux/recording.rs` uses an FFmpeg `x11grab` process for both `Target::Screen` and `Target::Window`.
   - `start_screen` remains the full-display capture path.
   - `start_window` now prepares the target for visible capture and starts native `x11grab -window_id`.
   - The Composite/rawvideo loop and its `run_capture_loop`, `capture_window_frame`, `finalize_window_capture`, and `exceeds_capture_cap` helpers are removed.
   - `stop` finalizes window and screen recordings through the same SIGINT path because both are FFmpeg `x11grab` captures.

2. A helper prepares a window for native recording:
   - Connect to X11.
   - Resolve the target geometry with `windows::geometry`.
   - Even-round width and height for libx264.
   - Reject zero dimensions.
   - Attempts to raise the window if representative points are not already visible.
   - Polls for visibility using local `RAISE_POLL_INTERVAL` and `RAISE_TIMEOUT` constants in the recording module.

3. A representative-point visibility check:
   - Use root-coordinate sample points derived from the current window geometry.
   - Always include the center.
   - Include near-corner points for windows large enough to sample them safely.
   - Uses `windows::window_hit_at_point` to verify each point resolves to the target or one of its descendants.
   - Returns an actionable `RecordingError::Environment` or `RecordingError::Start` if the target cannot be made visible.

4. The native window FFmpeg command uses:
   - `-f x11grab`
   - `-framerate <config.frame_rate>`
   - `-video_size <width>x<height>`
   - `-window_id <window_id>`
   - `-i <DISPLAY>`
   - existing codec, preset, pixel format, duration, size limit, stdout/stderr, and kill-on-drop behavior.

5. Result semantics stay aligned with screen recording.
   - `RecordingHandle` no longer carries capture-loop stop/task fields.
   - `RecordingOutput.width` and `height` should be the even-rounded window dimensions resolved before launch.
   - Startup still waits for the output file to grow and includes FFmpeg log tail on error.

6. Action/screenshot targeting is unchanged.
   - `UseComputer` and `RequestComputerUse` continue to advertise and use background window targets when supported.
   - Window screenshots can continue to use Composite capture for covered windows.
   - Only video recording changes to the native visible-window contract.

## Testing and validation
1. Product Behavior 1: keep `records_full_display_for_screen_target` or equivalent coverage to ensure screen recordings still report full display dimensions.

2. Product Behavior 2, 3, 4, and 11: add or update Linux/Xvfb recorder tests to create a target window, start a window-targeted recording, and verify:
   - recording starts successfully when the target is visible;
   - reported dimensions match the target's even-rounded initial dimensions;
   - encoded dimensions match the target dimensions.

3. Product Behavior 3, 4, 5, and 12: add a visibility-preparation test where a covered target can be raised and recorded in a WM-less Xvfb environment.
   - A fully covering window should be stacked above the target initially.
   - Starting recording should raise the target, pass visibility sampling, and produce a recording of the target window.
   - If a deterministic negative case is practical, use an invalid/non-viewable target and assert start fails before producing an artifact.

4. Product Behavior 6 and 7: document resize/obscuration limitations in tests or comments only if directly exercised. Do not add brittle tests for FFmpeg-specific resize behavior unless CI can run them reliably.

5. Product Behavior 8: retain existing finalization behavior by reusing the screen finalizer; no separate rawvideo EOF test is needed after the custom path is removed.

6. Run focused validation:
   - `cargo test --locked -p computer_use --lib -- recording`
   - `cargo clippy --locked --release -p computer_use --tests --no-deps`
   - `cargo fmt --all -- --check`

## Parallelization
No parallel sub-agents are proposed for implementation. The change is concentrated in one Linux recording module plus its tests, and splitting the work would add merge overhead without meaningfully reducing risk.

## Risks and mitigations
1. Native `x11grab -window_id` may record non-target pixels if the target is covered after recording starts.
   - Mitigation: define the product contract as foreground-visible recording and fail only when the target cannot be made visible at start.

2. Window managers can deny or delay raise requests.
   - Mitigation: poll for a bounded time and return a clear start error when visibility cannot be verified.

3. Visibility sampling is not a full-pixel proof.
   - Mitigation: sample center plus representative edges/corners and document best-effort semantics.

4. FFmpeg resize behavior can end or distort recordings when the target window changes dimensions.
   - Mitigation: keep output dimensions fixed at start and document native FFmpeg behavior as accepted for this iteration.

## Follow-ups
- Consider re-raising the target before each computer-use action while a native window recording is active if cloud recordings still frequently show non-target windows.
- Add a separate covered-window-capable recording mode only if users need true background video fidelity and we can fix rawvideo pacing.
