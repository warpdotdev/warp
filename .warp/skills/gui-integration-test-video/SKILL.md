---
name: gui-integration-test-video
description: 'GUI desktop app only. Run and author Warp Rust integration tests (the `crates/integration` harness) that capture screenshots and video via `TestStep::with_start_recording()` / `with_take_screenshot()`, including mouse and keyboard event overlays. TRIGGER only for the integration-test recording pipeline: recording or screenshotting a named integration test, authoring a test that captures video/screenshots, or reviewing artifacts a test produced. SKIP for any general request to screenshot or record the running Warp app or a UI flow — capture those with the computer use tool''s built-in screen recording / screenshots, not this skill.'
---

# Integration Test Video Recording

**Scope — GUI desktop app only.** This skill applies to Warp's **GUI** desktop front-end (the `app/` crate on the WarpUI pixel/GPU framework). It does **not** apply to the headless **TUI** front-end (`crates/warp_tui`; cell-grid `TuiElement` library under `crates/warpui_core/src/elements/tui`), which has its own components, tests, and change-verification workflow. For TUI work, see the `tui-ui-guidelines`, `tui-testing`, and `tui-verify-change` skills instead.

Use this skill when working with Warp's integration test recording pipeline on this branch.

## When NOT to use this skill

This skill is **only** for capturing screenshots/video from within the Rust
integration-test harness (`crates/integration`). It is **not** the way to fulfill
a general "take a screenshot" or "record a video" request against the running
Warp app or a live UI flow. For those, use the **computer use** tool, which
exposes screenshot and screen-recording (start/stop recording) capabilities that
drive the real app — don't reach for this integration-test tooling just because a
request mentions "recording" or "screenshot."

The relevant implementation lives in:
- `integration/src/bin/integration.rs`
- `integration/src/test/video_recording.rs`
- `integration/tests/integration/ui_tests.rs`
- `ui/src/integration/driver.rs`
- `ui/src/integration/step.rs`
- `ui/src/integration/video_recorder.rs`
- `ui/src/integration/artifacts.rs`
- `ui/src/integration/overlay.rs`

## Command to invoke a test

For a single manually-invoked recording test, prefer the integration binary:

```bash
WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
cargo run -p integration --bin integration -- test_video_recording
```

That is the command shown by the sample test in `integration/src/test/video_recording.rs`.

If you want the driver to auto-record a test or set of tests, add `WARP_INTEGRATION_TEST_VIDEO`:

```bash
WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
WARP_INTEGRATION_TEST_VIDEO=test_video_recording \
cargo run -p integration --bin integration -- test_video_recording
```

For broader integration test runs, the same env vars work with the normal test runner:

```bash
WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
WARP_INTEGRATION_TEST_VIDEO=test_foo,test_bar \
cargo nextest run --no-fail-fast --workspace test_foo
```

## Environment variables

### `WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS`
- Set this to `1` when you need real frame capture.
- Use it for screenshot/video workflows and manual visual verification.
- Without a real display, expect recording workflows to be incomplete or unusable.

### `WARP_INTEGRATION_TEST_VIDEO`
This is the main env var that controls driver-managed video recording in `ui/src/integration/driver.rs`.

Behavior:
- Unset or empty: auto-recording is disabled.
- `1` or `all`: auto-record every test in the run.
- Comma-separated test names: auto-record only those tests.

Examples:

```bash
# Record every test in the run
WARP_INTEGRATION_TEST_VIDEO=all
```

```bash
# Record only specific tests
WARP_INTEGRATION_TEST_VIDEO=test_foo,test_bar
```

Important nuance:
- You do not need `WARP_INTEGRATION_TEST_VIDEO` if the test itself explicitly calls `with_start_recording()` and `with_stop_recording()`.
- Use the env var when you want whole-test recording without changing the test code.

### `WARP_INTEGRATION_TEST_ARTIFACTS_DIR`
This controls the root artifact directory used by `TestArtifacts` in `ui/src/integration/artifacts.rs`.

If unset, artifacts go under:

```text
$TMPDIR/warp_integration_test_artifacts
```

Each run gets a timestamped directory:

```text
<artifacts_root>/<test_name>/<timestamp>/
```

This is the main directory to inspect for screenshots, logs, and the final `recording.mp4`.

### `WARP_INTEGRATION_TEST_VIDEO_DIR`
This env var exists in `ui/src/integration/video_recorder.rs` as the lower-level recorder output root helper, defaulting to:

```text
$TMPDIR/warp_integration_video_captures
```

On this branch, the normal integration driver flow writes the finalized video into the test artifacts directory instead, so `WARP_INTEGRATION_TEST_ARTIFACTS_DIR` is the one you usually care about when reviewing results.

## How to specify which tests to record

There are two modes:

### 1. Record in test code
Use `TestStep::with_start_recording()` and `TestStep::with_stop_recording()` inside the test itself. This is best when you only want to capture a specific span of the test.

### 2. Record from the environment
Set `WARP_INTEGRATION_TEST_VIDEO` to:
- `all`
- `1`
- or a comma-separated list like `test_a,test_b`

This starts recording at the beginning of matching tests and writes the video when the test completes.

## How overlays work

There is no separate overlay env var on this branch.

Overlay annotations are produced from the input events the test dispatches while recording is active. The overlay pipeline is implemented in `ui/src/integration/overlay.rs`, and the event capture hooks live in `ui/src/integration/step.rs`.

To get useful overlays in the final video, drive the test with APIs that emit mouse and keyboard events, such as:
- `with_event(...)`
- `with_event_fn(...)`
- `with_click_on_saved_position(...)`
- `with_keystrokes(...)`

Overlay types currently exercised by the sample test:
- mouse click indicators
- drag trails
- keyboard shortcut pills

In practice:
- Mouse down / drag / mouse up events create click and drag overlays.
- KeyDown events create keyboard overlay pills.
- If a test only records frames and never dispatches relevant input events, the resulting video will not show these annotations.

## How to write a test that takes screenshots

Use `TestStep::with_take_screenshot("filename.png")`.

Example pattern:

```rust
TestStep::new("Take screenshot after bootstrap")
    .with_take_screenshot("after_bootstrap.png")
```

The screenshot request is stored during the step and written by the driver after the step renders. The PNG lands in the test's timestamped artifacts directory.

## How to write a test that records video

### Minimum pattern
1. Use `Builder::new().with_real_display()`.
2. Add a step with `with_start_recording()`.
3. Run the actions/events you want captured.
4. Add a step with `with_stop_recording()`.

Example shape:

```rust
Builder::new()
    .with_real_display()
    .with_step(TestStep::new("Start recording").with_start_recording())
    .with_step(/* actions and events */)
    .with_step(TestStep::new("Stop recording").with_stop_recording())
```

### For overlay-friendly recordings
Prefer explicit UI-driving steps that emit mouse and key events:
- click with `with_click_on_saved_position(...)`
- dispatch raw mouse events with `with_event(...)` / `with_event_fn(...)`
- send keyboard shortcuts with `with_keystrokes(...)`

For drag overlays, send a sequence like:
- `LeftMouseDown`
- one or more `LeftMouseDragged`
- `LeftMouseUp`

### Optional validation
It is reasonable to add an `with_on_finish(...)` hook that checks for expected artifacts such as:
- `recording.mp4`
- `recording.log`
- screenshot PNGs

The sample test does exactly that.

## Where the video assets go

The normal output location is:

```text
${WARP_INTEGRATION_TEST_ARTIFACTS_DIR:-$TMPDIR/warp_integration_test_artifacts}/<test_name>/<timestamp>/
```

Common artifacts in that directory:
- `recording.mp4`
- `recording.log`
- any screenshots requested with `with_take_screenshot(...)`

For `test_video_recording`, the sample test expects:
- `after_bootstrap.png`
- `after_commands.png`
- `recording.mp4`
- `recording.log`

If MP4 encoding fails during finalization, the recorder falls back to per-frame PNGs in a sibling directory like:

```text
recording_frames/
```

with files such as:

```text
recording_0000.png
```

## How to review the assets

1. Open the latest timestamped artifact directory for the test.
2. Review `recording.mp4` first to confirm:
   - the UI state is correct
   - recording actually started and stopped in the intended window
   - overlay annotations appear at the right moments
3. Review any PNG screenshots captured by the test.
4. Check `recording.log` if the output looks incomplete or suspicious.
5. If `recording.mp4` is missing, look for fallback frame PNGs.

When summarizing results for the user, include the exact artifact directory path.

## Sample test for video recording

The sample manual test is `test_video_recording`.

It is:
- registered in `integration/src/bin/integration.rs`
- listed in `integration/tests/integration/ui_tests.rs`
- implemented in `integration/src/test/video_recording.rs`

Run it with:

```bash
WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
cargo run -p integration --bin integration -- test_video_recording
```

If you want full-test auto-recording from the environment as well, use:

```bash
WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
WARP_INTEGRATION_TEST_VIDEO=test_video_recording \
cargo run -p integration --bin integration -- test_video_recording
```

## Working pattern for agents

When asked to record or debug an integration test with video:
1. Identify the exact test name.
2. Decide whether recording should be explicit in the test or enabled via `WARP_INTEGRATION_TEST_VIDEO`.
3. Ensure the run uses `WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1`.
4. If the user wants visible interaction overlays, make sure the test dispatches mouse and keyboard events while recording is active.
5. After the run, inspect the timestamped artifact directory and report the output paths back to the user.
