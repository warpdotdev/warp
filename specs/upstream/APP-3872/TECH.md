# Integration Test Recording Overlays Technical Spec

## Summary
This spec describes the implementation of the integration test recording overlay feature described in `specs/APP-3872/PRODUCT.md`.

The current branch already contains the core implementation for annotated integration test recordings. This document treats that implementation as the starting point, but makes the product spec the source of truth. Where the branch behavior diverged from the intended product behavior, the implementation has been adjusted and this document reflects the corrected design.

## Goals
- Provide an opt-in API for annotated integration test video recordings.
- Capture mouse and keyboard event metadata during a recording session.
- Composite those annotations into the final recorded video rather than requiring a separate viewer.
- Keep the feature understandable and ergonomic for test authors.

## Non-Goals
- Building a generic event-inspection UI outside the generated recording artifacts.
- Changing how integration tests express events today.
- Supporting every input event type in the first version beyond click, click-and-drag, and keyboard events.

## Current Branch Foundation
The branch already implements the main pieces needed for this feature:
- A `VideoRecorder` abstraction for start/stop recording, frame capture, screenshot export, and MP4 finalization
- An `OverlayLog` that stores timestamped visual annotation events
- Integration-step hooks that observe mouse and keyboard input as tests dispatch events
- Final compositing that draws overlays onto recorded frames at encode time
- A manual end-to-end exercise test in `integration/src/test/video_recording.rs`

This tech spec keeps that architecture and refines behavior where needed to better match the product spec.

## API Surface

### Test Author API
The recording API is exposed through `TestStep` helpers:
- `with_start_recording()`
- `with_stop_recording()`
- `with_take_screenshot(...)`

This means annotated recording is opt-in at the test level. A test author does not need to change how clicks, drags, or keystrokes are authored once recording has started.

### Environment-Based Debugging API
The existing environment variable `WARP_INTEGRATION_TEST_VIDEO` remains available as a convenience override for automatic recording during test runs. This is useful for debugging, but the primary product-facing API remains the explicit step-based recording controls.

## High-Level Architecture

### 1. Event capture in the integration test runner
Integration events are intercepted in the step runner before they are dispatched into the app. Relevant mouse and keyboard events are translated into overlay events and appended to an in-memory `OverlayLog`.

The recorded overlay event types are:
- `MouseDown`
- `MouseMove`
- `MouseUp`
- `KeyPress`

Keyboard events are derived from parsed `Keystroke` values and rendered using a display formatter that includes modifier keys and the main key in a single visual token.

Important behavior:
- Overlay events are recorded only while the `VideoRecorder` is actively recording.
- This prevents pre-recording or post-recording interactions from leaking into the visual annotation timeline.

This active-recording gating is an intentional correction to the earlier branch behavior.

### 2. Frame capture loop
When integration tests are running with the `integration_tests` feature enabled, the driver starts a foreground capture loop backed by the existing window frame-capture path. The loop:
- Sleeps when recording is off
- Requests a frame capture while recording is on
- Stores captured frames together with wall-clock timestamps

This keeps recording overhead effectively limited to periods where recording is enabled.

### 3. Final video compositing
At finalization time, captured frames are encoded into MP4. Before each frame is handed to the encoder, the overlay renderer advances an `OverlayState` to the frame timestamp and draws any active annotations onto the frame buffer.

This approach has a few benefits:
- It avoids mutating the live app UI just to show recording-only annotations.
- It keeps the overlay logic isolated from the product UI.
- It allows overlay rendering to be deterministic from the captured timeline.

If MP4 encoding fails, the implementation falls back to writing PNG frames.

### 4. How the overlay pixels are actually produced
The implementation does not ask WarpUI to render a second overlay scene and it does not add overlays after MP4 encoding. Instead, the compositor mutates each captured frame's RGBA pixel buffer in memory before the frame is converted to RGB/YUV and handed to OpenH264.

The concrete flow is:
- `VideoRecorder::finalize(...)` drains the captured `TimestampedFrame` list
- `encode_to_mp4(...)` clones the frame's RGBA byte buffer when overlay events are present
- `OverlayState::advance_to(...)` applies all overlay events whose timestamps are at or before the current frame time
- `OverlayState::render_onto(...)` rasterizes the active click, drag, and keyboard visuals directly into that RGBA buffer
- the composited RGBA data is converted to RGB, then YUV, and then encoded into the output MP4

This means the overlays are effectively burned into the frame pixels before encoding. The original capture remains unchanged except for the pixels touched by the overlay primitives.

### 5. Software rasterization model
Overlay rendering is implemented as a small software rasterizer in `ui/src/integration/overlay.rs`.

The renderer works on a flat RGBA byte slice and uses alpha blending per destination pixel:
- `blend_pixel(...)` mixes overlay color into the destination image
- `draw_filled_circle(...)` is used for the held-mouse indicator and drag anchor
- `draw_click_ring(...)` paints the animated post-click ring by filling only the annulus
- `draw_thick_line(...)` stamps circular samples along each drag segment
- `draw_key_overlay(...)` paints the rounded pill background for keyboard events
- `draw_text(...)` writes glyph pixels directly into the same RGBA buffer

There is no separate retained overlay surface. We simply replace some destination pixels with alpha-blended overlay colors.

### 6. Coordinate scaling
Overlay events are recorded in logical coordinates, while captured frames are stored in device pixels. The driver records the window backing scale factor into `OverlayLog`, and the compositor multiplies event coordinates by that scale factor before drawing circles, lines, and key pills. This is how the overlay positions line up with the captured image content.

## Rendering Model

### Clicks
Clicks are represented by:
- A filled pointer indicator while the mouse button is held
- An animated expanding ring after mouse up

This matches the familiar click pulse pattern used by screen recording and demo tools.

### Click and drag
Drag gestures are represented by:
- A visible drag start anchor
- A trail connecting the recorded drag points
- A visible end-state ring on mouse up
- A live pointer indicator while the drag is in progress

The earlier branch implementation already had a drag trail and end-state ring, but it did not emphasize the drag start strongly enough. The implementation has been adjusted to render an explicit drag-start anchor so the gesture is easier to read in playback.

### Keyboard events
Keyboard events are rendered as transient pill overlays near the bottom of the frame. Each pill contains:
- Any modifier keys involved in the event
- The main key being fired

Examples:
- `⌘C`
- `⌃A`
- `⇧Enter`

To better match the product requirement that rapid keyboard sequences remain readable, the implementation now renders multiple recent key events as a small stack instead of showing only the most recent keypress. This is another intentional correction relative to the earlier branch behavior.

### Keyboard glyph rendering details
The current implementation does not use system fonts or vector glyph rasterization for keyboard overlays. Instead, it uses:
- an embedded 8x16 bitmap font table for printable ASCII characters
- handwritten 8x16 bitmap glyphs for modifier and special-key symbols such as `⌘`, `⌃`, `⌥`, `⇧`, `↩`, `⇥`, and the arrow keys
- integer upscaling (`FONT_SCALE`) to enlarge those bitmaps inside the pill

`keystroke_display_text(...)` still formats the keyboard shortcut text as the expected Unicode symbols, but `draw_text(...)` does not shape or rasterize them with a font engine. `get_glyph(...)` maps each character to either the ASCII bitmap table or one of the custom symbol bitmaps and then writes the glyph pixels directly into the frame buffer.

This is why the modifier keys appear correctly today, but also why the resulting text can look pixelated in videos.

To improve readability, the current implementation now prefers plain-text labels for ambiguous non-modifier special keys such as `Enter`, `Tab`, `Backspace`, and `Delete` instead of rendering those as symbolic glyphs. Modifier keys still use symbolic forms such as `⌘`, `⌃`, `⌥`, and `⇧`.

### Improving glyph quality
Yes, we can improve the key and modifier glyph quality by switching to higher-resolution source bitmaps. That is the lowest-risk follow-up if the current 8x16 glyphs scaled by `FONT_SCALE` look too blocky in the final MP4.

The simplest version of that change would be:
- replace the 8x16 ASCII and modifier-symbol bitmaps with a larger source set such as 16x32 or 24x48
- either reduce the amount of integer upscaling or keep a smaller scale factor
- keep the same direct-to-buffer rendering model and alpha blending pipeline

This would preserve the current deterministic, cross-platform rendering behavior while making the keyboard overlays look cleaner. It is a much smaller change than introducing a full font rasterization stack.

## Event-to-Overlay Mapping

### Mouse mapping
- `LeftMouseDown` and the other supported mouse-down variants create `MouseDown`
- `LeftMouseDragged` creates `MouseMove` in the overlay timeline
- `LeftMouseUp` creates `MouseUp`
- Saved-position click helpers also emit matching synthetic overlay events so helper-driven interactions look the same as hand-authored events in recordings

### Keyboard mapping
- `KeyDown` events are converted to `KeyPress`
- The display text is derived from the parsed `Keystroke`, not the raw typed character stream
- This ensures modifier state is preserved in the overlay output

## Timing Model
All overlay events and captured frames use wall-clock timestamps from the test process.

During encoding:
- Each captured frame advances overlay state up to that frame timestamp
- Expiring visual effects fade out according to fixed durations
- Gaps between captured frames are normalized into repeated output frames at the target FPS

This means annotations are synchronized to the recorded interaction timeline rather than to test-step boundaries.

## Data Flow
1. The driver creates a `VideoRecorder`, `ActionLog`, `OverlayLog`, and artifacts directory for the test run.
2. A test step enables recording.
3. The capture loop begins accumulating timestamped frames while the recorder is active.
4. Integration events are dispatched as normal.
5. Relevant events are mirrored into the `OverlayLog`.
6. Finalization encodes the captured frames and composites overlay annotations frame-by-frame.
7. Artifacts are written, including:
   - `recording.mp4`
   - screenshots requested by the test
   - `recording.log`

## Artifacts
The implementation produces:
- Annotated video output
- Requested screenshots
- A plain-text action log for timestamped event auditing

The action log is useful for debugging and complements the overlay video, but it is not the primary user-facing product surface.

## Implementation Details by Module

### `ui/src/integration/driver.rs`
Responsible for:
- Creating per-test recording state
- Starting the frame capture loop
- Initializing the overlay scale factor from the active window backing scale
- Handling screenshot capture after steps
- Finalizing the recording and writing artifacts

### `ui/src/integration/step.rs`
Responsible for:
- Exposing the step-level recording API
- Intercepting integration events
- Translating supported input events into overlay events
- Ensuring overlay events are only recorded while recording is active

### `ui/src/integration/overlay.rs`
Responsible for:
- Defining overlay event types
- Converting keystrokes into display strings
- Tracking transient overlay state across the event timeline
- Rendering click, drag, and keyboard overlays into RGBA frame buffers via direct pixel mutation and alpha blending

### `ui/src/integration/video_recorder.rs`
Responsible for:
- Managing recording lifecycle
- Running the timestamped frame capture loop
- Encoding MP4 output
- Invoking overlay compositing during finalization
- Falling back to PNG frames if video encoding fails

### `integration/src/test/video_recording.rs`
Acts as the manual end-to-end validation flow for:
- screenshots
- start/stop recording
- click overlays
- drag overlays
- keyboard overlays

## Decisions and Corrections

### Keep overlays composited at encode time
We should continue rendering overlays into captured frames during finalization instead of drawing them live in the app. This keeps the production UI untouched and keeps the feature scoped to integration recording.

### Keep recording opt-in
The explicit start/stop recording API matches the product spec and keeps the feature ergonomic for tests that only want overlays for a narrow slice of execution.

### Correct overlay collection to honor recording state
Overlay collection must be tied to active recording. This avoids stale annotations from appearing in the output video and better matches the product requirement that overlays appear when the feature is enabled.

### Render short keyboard history, not just the latest key
Showing a short stack of recent key events better matches the product goal of making recordings understandable at normal playback speed, especially for shortcuts or rapid key sequences.

### Make drag start visible
The drag path alone is not always enough to communicate where a gesture began. A dedicated start anchor makes the recording easier to interpret.

### Prefer higher-resolution bitmaps before introducing real font rendering
If the current keyboard overlays are too pixelated, the best next step is to increase the source bitmap resolution rather than immediately moving to system fonts. Higher-resolution bitmaps would improve visual quality without adding font discovery, shaping, rasterization libraries, or platform-specific output differences to the integration test pipeline.

## Validation
Implementation was validated with:
- `cargo check -p warpui_core --features integration_tests --manifest-path /Users/zach/Projects/warp_5/Cargo.toml`
- `cargo check -p integration --manifest-path /Users/zach/Projects/warp_5/Cargo.toml`

For manual validation, the existing `integration/src/test/video_recording.rs` flow should be used to inspect:
- click visibility
- drag readability
- keyboard modifier rendering
- stacked key annotations during rapid sequences

## Future Extensions
- Add richer styling configuration if test authors need different overlay density or durations
- Add support for more pointer event types if product requirements expand
- Replace the current 8x16 keyboard bitmaps with a higher-resolution glyph set to reduce visible pixelation in encoded videos
- Optionally export structured overlay metadata alongside the rendered video if downstream tooling needs it
