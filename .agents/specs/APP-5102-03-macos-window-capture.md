# APP-5102 — 3 of 3: Bounded macOS window-scoped recording

**Dependency:** Implement after spec 1's shared command/filter seam. Implement after or together with spec 2 so window-local pointer overlays use the cropped frame's coordinate space.

## macOS vs. Linux delta

**Linux reference:** Linux resolves X11 window geometry, raises/verifies representative visible points, and gives `x11grab` a native `-window_id`; ffmpeg captures that window at fixed start dimensions.

**macOS difference:** AVFoundation has no window input and `mac/recording.rs` ignores `RecordingConfig.target`. macOS window IDs/bounds come from CoreGraphics in display points, while AVFoundation captures a display-sized physical-pixel frame. The bounded first implementation must convert and crop that display frame, and it inherits composed-display limitations that native X11 `-window_id` does not hide.

## Summary

Honor macOS `Target::Window` by resolving a fixed, even, main-display pixel rectangle at recording start, making the target foreground-visible, and applying an AVFoundation `crop=W:H:X:Y` filter. Fail closed when the target cannot satisfy the bounded contract.

## Default behavior contract

1. `Target::Screen` remains the main-display AVFoundation path.
2. `Target::Window` requires a non-zero window ID for a non-minimized, positive-sized window fully contained on the main display.
3. At start, Warp resolves `CGWindowBounds` (points), subtracts the main display origin, applies that display's backing scale, rounds to an in-frame integer rectangle, and rounds width/height down to even values for yuv420p.
4. Warp verifies the target is foreground-visible at representative center/corner points. If covered, it best-effort activates/raises the owning application and polls; if the target still is not topmost at those points, start fails rather than recording another surface.
5. ffmpeg captures the main display with AVFoundation and writes only the fixed crop. `RecordingHandle`/`RecordingOutput` report cropped dimensions, so window-local pointer coordinates and ASS `PlayRes` map to the encoded frame.
6. Missing, zero-sized, minimized, off-main-display, display-spanning, unmappable-scale, TCC-denied, or non-visible targets return actionable start/environment errors and produce no artifact.
7. The crop is fixed at start. Movement/resizing does not move or resize it. Occluders that appear after start are recorded. These are explicit deferred non-parities.
8. Screen Recording TCC remains the required capture permission. No separate “window recording” permission is invented.

## Technical approach

Code references are pinned to `warpdotdev/warp@7a6044bd5377d708ab1d3767ece505a49d232aed`.

- `crates/computer_use/src/mac/recording.rs:52-100` always chooses main-display dimensions and explicitly ignores `config.target`.
- `crates/computer_use/src/mac/window.rs:90-112` resolves `CGWindowID` to point-space bounds through the on-screen CoreGraphics window list.
- `crates/computer_use/src/mac/util.rs:46-86` already finds the scale of the display fully containing a window.
- `crates/computer_use/src/mac/mod.rs:56-102` already performs the matching window-local-pixel → global-point → screen-pixel conversion for actions.
- `crates/computer_use/src/mac/screenshot.rs:61-131` proves `screencapture -l <window_id>` can capture an occluded **still** window, but that CLI does not provide a continuous video stream.
- `crates/computer_use/src/mac/activation.rs:59-146` activates background windows without raising; video capture needs a separate best-effort foreground preparation path.

Add a macOS capture-preparation result such as:

- capture kind (screen/window);
- AVFoundation display selector and input dimensions;
- optional output crop in physical pixels;
- encoded width/height.

For a window:

1. Resolve the target and owner from the front-to-back `CGWindowListCopyWindowInfo` list.
2. Validate layer, positive bounds, on-screen/minimized state, requested PID, full containment in `CGDisplayBounds(CGMainDisplayID())`, and one known backing scale.
3. Compute `x/y/width/height` in AVFoundation's physical-pixel frame, clamp only harmless rounding error, and reject real out-of-frame geometry. Make output dimensions even without shifting outside the target.
4. Sample visibility from the same front-to-back window list. If needed, activate the owner application, poll for a bounded interval, then fail if the target still is not topmost at all samples. Do not silently capture an occluder.
5. Pass `crop=w:h:x:y` through spec 1's platform filter seam. Preserve `-t`, `-fs`, H.264/yuv420p, AVFoundation startup supervision, SIGINT finalization, and post-stop processing.

## Design alternatives

- **Chosen default — crop the AVFoundation display capture.** Reuses the existing ffmpeg recorder and shared core. Cost: the window must be visible; crop position/size are fixed; later occlusion is captured.
- **ScreenCaptureKit `SCWindow`.** Captures window content independently of normal occlusion and decouples capture from display position. Cost: macOS 12.3+ native stream callbacks, resize/content-scale handling, and either AVAssetWriter or piping frames to ffmpeg. It replaces AVFoundation as the input substrate rather than adding one filter.
- **Repeated `screencapture -l` frames.** Rejected: a shell loop has pacing/CPU/PTS problems and recreates the failed custom-frame-loop class of recorder.

## Review-time open decision

**Default for review:** bounded crop-from-display. Switch this spec to ScreenCaptureKit before implementation if either occlusion-independent capture or move/resize tracking is merge-blocking. Those requirements cannot honestly be satisfied by a static AVFoundation crop. If this draft is approved unchanged, the implementor follows the bounded crop contract and does not choose between substrates.

## Explicit deferred non-parity

- A window moved/resized after start does not update the crop.
- A window covered after start records the covering pixels.
- Minimized/off-main-display/multi-display-spanning windows are rejected.
- Multi-display recording remains out of scope.
- True background window video is deferred to ScreenCaptureKit.

## Validation and verification criteria

1. Add pure macOS geometry tests for Retina and 1× displays, non-zero display origins, even rounding, edge clamping, and rejection of zero/minimized/off-main/spanning/mixed-scale windows.
2. Replace `ignores_window_target_until_window_scoped_recording_lands` with argv tests proving screen target has no crop and window target contains the exact `crop=w:h:x:y`, correct AVFoundation selector, and cropped reported dimensions.
3. Add target-preparation tests for missing ID, PID mismatch, zero bounds, TCC/window-list failure, already-visible target, successful activation/visibility poll, and still-occluded failure. No failing case launches ffmpeg.
4. Add a real macOS recorder test that captures a known visible window with distinctive content; ffprobe dimensions equal the target's even start dimensions and decoded center/corner samples come from the target.
5. Move/resize and post-start occlusion tests document the bounded behavior: fixed crop remains fixed and an occluder is visible. They are not asserted as parity.
6. Screen target argv/dimensions, post-process overlays, cleanup, and Linux native `-window_id` tests remain unchanged.
7. `./script/format`, `cargo clippy -p computer_use --all-targets --all-features --tests -- -D warnings`, and `cargo build -p computer_use` pass on the implementation branch.
8. Exercise screen and window targets in the running macOS UI with computer use and attach video proof plus the cropped recording artifact to the task and implementation PR.

## Verify locally on macOS

Run from the `warp` checkout with a sibling `warp-server` checkout:

```bash
WARP_REPO="$(git rev-parse --show-toplevel)"
WARP_SERVER_REPO="$(cd "$WARP_REPO/../warp-server" && pwd)"
brew install ffmpeg jq

cd "$WARP_REPO"
cargo test -p computer_use --lib mac_window_crop
cargo test -p computer_use --lib mac_window_target
cargo test -p computer_use --lib
./script/format
cargo clippy -p computer_use --all-targets --all-features --tests -- -D warnings
cargo build -p computer_use

cargo run -p computer_use --bin use_computer -- windows
```

Open a TextEdit window with distinctive text. Copy its `window#` and `owner_pid` from the last command:

```bash
WINDOW_ID="<window#>"
WINDOW_PID="<owner_pid>"
screencapture -x -o -l "$WINDOW_ID" /tmp/app-5102-window-reference.png
sips -g pixelWidth -g pixelHeight /tmp/app-5102-window-reference.png
```

Build and start local direct mode:

```bash
cd "$WARP_REPO"
./script/run \
  --features with_local_server,with_local_session_sharing_server \
  --host-id local-dev \
  --dont-open
WARP_BIN="$WARP_REPO/target/debug/bundle/osx/WarpLocal.app/Contents/MacOS/warp"

cd "$WARP_SERVER_REPO"
./script/oz-local up --detach --wait \
  --worker-backend direct \
  --oz-path "$WARP_BIN"
open http://localhost:3002
```

Grant Screen Recording permission, then submit:

> Use background computer use to select the TextEdit window titled with `APP-5102`. Start a recording targeted to that same window, click and drag inside it, type `window-crop`, wait 5 seconds, then stop and publish the recording. Do not move or resize the window during this baseline run.

Download the MP4 from the run's **Artifacts** panel:

```bash
VIDEO="$(ls -t "$HOME"/Downloads/*.mp4 | sed -n '1p')"
ffprobe -v error \
  -select_streams v:0 \
  -show_entries stream=width,height \
  -of default=noprint_wrappers=1 \
  "$VIDEO"
```

Assert the video contains only the TextEdit window, dimensions match the reference image after even rounding, window-local click/drag overlays align, and the 5-second gap is cut. Then cover or move the window during a second run and confirm the documented non-parity: the crop stays fixed and composed occluding pixels may appear.

Linux regression check:

```bash
cd "$WARP_REPO"
cargo test -p computer_use --lib records_window_target_via_native_x11grab_after_raise
cargo test -p computer_use --lib records_full_display_for_screen_target
```
