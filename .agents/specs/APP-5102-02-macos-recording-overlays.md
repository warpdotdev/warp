# APP-5102 — 2 of 3: macOS action and pointer overlays

**Dependency:** Implement after spec 1 provides a shared 1× post-processing timeline. This spec reuses that core; it does not create a second macOS compositor.

## macOS vs. Linux delta

**Linux reference:** Linux records resolved pointer events into `PointerSink`, builds ASS pills plus click/drag/cursor vector dialogues, and burns them after smart cutting.

**macOS difference:** The app already constructs the same `PointerSink`, but the macOS actor ignores it. ASS generation and burn-in are Linux/test-gated. macOS also mixes point-based CoreGraphics window geometry with physical-pixel AVFoundation frames, and AVFoundation currently captures its own cursor/click visualization. macOS must record actor-resolved capture-space pixels, run the shared libass pass, and avoid double cursors/indicators.

## Summary

Provide semantic—not pixel-identical—overlay parity on macOS. Reuse the existing ASS renderer for key/type/scroll pills, click rings, drag trails, and the synthetic cursor. Teach the macOS actor to populate `PointerSink` in the recording target's pixel space.

## Behavior contract

1. macOS recordings render the same semantic labels as Linux: shortcuts/non-printing keys, redacted `typing…`, and direction-only scroll pills. Typed payloads never appear.
2. Completed clicks render one ring each. Drags render a continuous held indicator/anchor/trail and no click ring on release. Split-call drags retain session continuity.
3. Functional parity is required; exact glyph metrics, font rasterization, and pixel-identical placement across operating systems are not.
4. Screen recordings use resolved physical display pixels. Window recordings use window-local physical pixels for actions targeting the recorded window; mismatched surfaces are omitted and clear pointer session state.
5. macOS native cursor/click compositing is disabled when the shared synthetic cursor/annotations are enabled, preventing duplicate or unrelated pointers—especially for PID-targeted background actions that do not move the real cursor.
6. Missing libass/font/ffmpeg or burn-in failure preserves the existing best-effort behavior: upload the untouched 1× source and remove intermediates.

## Technical approach

Code references are pinned to `warpdotdev/warp@7a6044bd5377d708ab1d3767ece505a49d232aed`.

- `crates/computer_use/src/overlay.rs:218-1315` contains the complete ASS renderer behind `cfg(any(linux, test))`.
- `crates/computer_use/src/lib.rs:684-783` defines `PointerSink`/`PointerSession` and still documents Linux as the only producer.
- `crates/computer_use/src/linux/x11/mod.rs:193-560` is the reference event-resolution and session implementation.
- `crates/computer_use/src/mac/mod.rs:56-102,147-220` already remaps window-local physical pixels through CoreGraphics points for dispatch, but ignores `options.pointer_sink`.
- `crates/computer_use/src/mac/util.rs:1-86` exposes main/display scale conversions.
- `crates/computer_use/src/mac/recording.rs:204-243` enables AVFoundation cursor/click capture.

Changes:

1. Compile the ASS renderer for `cfg(any(macos, linux, test))` and invoke spec 1's shared post-process on macOS.
2. In `mac::Actor::perform_actions`, take the sink once and record `Down`, `Move`, `Scroll`, and `Up` at dispatch time using the existing `PointerSession` methods.
3. Preserve both local and resolved points:
   - recording `Target::Screen` → use the post-remap global physical pixel;
   - recording the same `Target::Window` → use the original window-local physical pixel;
   - recording one window while acting on another surface → omit and clear the session.
4. Use the same recording start `Instant`, stable event order, button matching, split-call state, clamping, and redaction rules as Linux.
5. Set AVFoundation `-capture_cursor 0 -capture_mouse_clicks 0` once shared synthetic pointer rendering is active. This aligns screen and background-window artifacts and prevents duplication.
6. Keep the current ASS geometry/timing. Choose a fontconfig-resolved generic monospace family (or verified macOS fallback) instead of requiring DejaVu's exact metrics.

## libass and font reality

There is no libass blocker in the shipped macOS sidecar. The exact pinned `ffmpeg-static` b6.1.1 arm64 binary was inspected during spec research: its embedded build configuration includes `--enable-libass --enable-libfreetype --enable-fontconfig`, and the binary contains the `subtitles` filter. The sidecar does not bundle DejaVu fonts, so semantic parity should rely on a resolvable macOS monospace family. Bundle a font/`fontsdir` only if the real sidecar smoke test proves system-font discovery fails.

## Design alternatives

- **Chosen — shared ASS/libass renderer.** Lowest divergence; timings, redaction, cut remapping, and gesture classification stay identical.
- **CoreAnimation/CoreGraphics compositor.** Rejected: a second renderer would duplicate gesture/timing behavior and require a new frame pipeline.
- **Keep AVFoundation's native cursor/clicks plus ASS annotations.** Rejected: background PID-targeted actions do not move that cursor and shared ASS would add a second pointer.
- **Bundle DejaVu for exact parity.** Deferred unless font discovery fails; the requester selected semantic parity.

## Open questions resolved

- “Capture scale” means coordinate units, not overlay visual scale. AVFoundation frames and action coordinates are physical pixels; CGWindow/CGEvent geometry is in points. Existing conversion helpers provide the required one-display mapping.
- Pixel-identical Linux/macOS typography is out of scope.
- The shared renderer is authoritative; macOS does not get platform-specific gesture thresholds or animation timing.

## Validation and verification criteria

1. Add `mac_actor_records_screen_pointer_events_in_capture_space` and `mac_actor_records_window_pointer_events_in_capture_space`; assert Down/Move/Scroll/Up ordering, source offsets, matching-window local pixels, screen global pixels, mismatched-target omission/session clearing, and split-call release recovery.
2. Existing overlay tests run under macOS cfg without duplicated expectations: key/type/scroll labels, redaction, click count, drag exclusivity/path, cursor motion, clipping, cut remapping, and cross-entry continuity.
3. Add a macOS argv test asserting AVFoundation native cursor/click compositing is disabled when shared annotation rendering is used.
4. Add a real libass smoke test that generates one pill and one vector ring, burns them into a tiny MP4 with the shipped/brew ffmpeg, and verifies non-background pixels exist in both expected regions. It must fail clearly if no font resolves.
5. Induced ASS/font failure uploads the original 1× source and cleans `.ass`, cut, and overlay files without logging labels, coordinates, or typed content.
6. A real macOS artifact visibly contains the expected pills, one ring per click, one trail for a drag, and no typed payload or duplicate cursor.
7. `./script/format`, `cargo clippy -p computer_use --all-targets --all-features --tests -- -D warnings`, and `cargo build -p computer_use` pass on the implementation branch.
8. Exercise the running macOS flow with computer use and attach video proof plus the produced annotated recording to the task and implementation PR.

## Verify locally on macOS

Run from the `warp` checkout with a sibling `warp-server` checkout:

```bash
WARP_REPO="$(git rev-parse --show-toplevel)"
WARP_SERVER_REPO="$(cd "$WARP_REPO/../warp-server" && pwd)"
brew install ffmpeg jq

ffmpeg -hide_banner -buildconf | grep -E -- '--enable-libass|--enable-fontconfig|--enable-libfreetype'
ffmpeg -hide_banner -filters | grep -E '[[:space:]]subtitles[[:space:]]'

cd "$WARP_REPO"
cargo test -p computer_use --lib maps_semantic_labels_in_action_order
cargo test -p computer_use --lib single_click_emits_one_expanding_ring
cargo test -p computer_use --lib drag_emits_trail_anchor_held_and_no_ring
cargo test -p computer_use --lib mac_actor_records
cargo test -p computer_use --lib
./script/format
cargo clippy -p computer_use --all-targets --all-features --tests -- -D warnings
cargo build -p computer_use

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

Grant Screen Recording and Accessibility/Input Monitoring permission when prompted, then submit:

> Start a full-screen video recording. Open TextEdit. Press command+a, type `APP-5102-secret`, press Return, and scroll down. Click once in the document, then perform a visible drag selection across the text. Pause for 5 seconds, click once more, stop the recording, and publish it.

The uploaded MP4 appears in the local Oz run's **Artifacts** panel; temporary `/tmp/warp-recording-*` files are removed after upload. Download and play it frame-by-frame. Assert:

- pills show `cmd+a`/`typing…`/`Return`/scroll direction, never `APP-5102-secret`;
- each click has one centered ring;
- the drag has a continuous trail/held marker and no release ring;
- there is one synthetic cursor, not native + synthetic duplicates;
- overlays remain aligned after the 5-second thinking gap is cut.

Linux regression check:

```bash
cd "$WARP_REPO"
cargo test -p computer_use --lib capture_command_disables_cursor_compositing_for_screen_and_window
cargo test -p computer_use --lib single_click_emits_one_expanding_ring
cargo test -p computer_use --lib drag_emits_trail_anchor_held_and_no_ring
cargo test -p computer_use --lib
```
