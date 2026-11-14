*Spec: Windows video recording for computer use (REMOTE-2064)*

Linear: [REMOTE-2064](https://linear.app/warpdotdev/issue/REMOTE-2064/investigate-windows-screen-recording-for-computer-use)

== PRODUCT ==
*Summary:* Add real video recording for computer use on Windows by supervising an ffmpeg
`gdigrab` process. Input and GDI screenshots already work on Windows. Recording is the remaining
platform gap because `create_recorder()` currently resolves to the no-op recorder through
`crates/computer_use/src/windows/mod.rs`. The first usable Windows recording captures the
composited full virtual desktop in physical pixels and returns a finalized, playable MP4. A later
stacked PR adds the existing smart cut and action overlays.

*Key design choices:*
1. *Use gdigrab for this delivery.* The recorder uses ffmpeg `gdigrab` with `desktop` input. This
   matches the current GDI `GetDC(NULL)` screenshot source and the subprocess architecture used by
   Linux `x11grab` and macOS `avfoundation`. It is the smallest path to cross-platform behavior
   parity.
2. *Do not implement WGC or Media Foundation now.* The older REMOTE-2064 investigation
   recommended a future Windows.Graphics.Capture (WGC) or DXGI Desktop Duplication source feeding
   Media Foundation. That design offers native GPU capture and better handling for hardware
   overlays. It also requires a larger capture-source, encoding, and multi-monitor project.
   `gdigrab` is intentionally chosen for this narrower delivery. This choice does not reject a
   later native rewrite.
3. *Match full virtual-screen screenshots.* Recording uses the same
   `SM_XVIRTUALSCREEN`/`SM_YVIRTUALSCREEN` origin and
   `SM_CXVIRTUALSCREEN`/`SM_CYVIRTUALSCREEN` dimensions as the Windows screenshot path. A
   `DpiAwarenessGuard` makes these values physical pixels. The origin can be negative. Width and
   height are rounded down to even values for H.264/yuv420p.
4. *Use ffmpeg's stdin control protocol.* Windows has no Unix SIGINT path equivalent to the
   current Linux/macOS implementation. The recorder pipes stdin and writes `q\n` to request a
   graceful ffmpeg shutdown. It then waits at most 15 seconds for MP4 finalization.
5. *Land the work as three stacked PRs.* The order is shared Windows plumbing, raw playable
   Windows recording, then Windows smart-cut and overlay parity. The middle PR is independently
   useful and publishes an unannotated playable MP4. The final PR adds pointer collection,
   synthetic cursor/click/drag overlays, and gap cutting.

*Behavior* (numbered, testable invariants):
1. On Windows, `create_recorder()` returns the Windows recorder when `test-util` is not enabled.
   `Recorder::start` launches ffmpeg `gdigrab` capture and resolves only after the output file has
   grown beyond zero bytes.
2. `Recorder::start` checks that `ffmpeg` is launchable and that the installed build exposes the
   `gdigrab` input demuxer. A missing executable, failed probe, or build without `gdigrab` returns
   `RecordingError::Environment`. The error names `ffmpeg` or `gdigrab` and contains a concise
   diagnostic. The recorder does not fall back to a no-op, WGC, DDA, or Media Foundation.
3. An ffmpeg process that launches but exits before output growth returns `RecordingError::Start`
   with the exit status and a bounded stderr tail. A process that remains alive without output for
   15 seconds is force-killed, reaped, and returns `RecordingError::Start` for readiness timeout.
   Failed starts remove the partial MP4 and log file.
4. Screen geometry is queried under per-monitor-v2 DPI awareness. Capture uses physical-pixel
   virtual-screen coordinates. `-offset_x` and `-offset_y` receive the exact signed virtual origin,
   including negative values for monitors above or left of the primary display.
5. Capture width is `SM_CXVIRTUALSCREEN & !1`. Capture height is
   `SM_CYVIRTUALSCREEN & !1`. The origin does not move when an odd dimension loses its final
   rightmost or bottommost pixel. Non-positive source dimensions or zero dimensions after even
   rounding return `RecordingError::Environment`.
6. The initial Windows recorder always captures the full virtual desktop. `Target::Window` does
   not select a window because Windows background/window-targeted computer use is unsupported.
   The behavior matches the Windows actor, which treats actions as screen/foreground actions.
7. The raw recording uses the existing cross-platform encode contract: H.264 through `libx264`,
   `ultrafast`, `yuv420p`, `+faststart`, configured frame rate, input `-t` for
   `RecordingConfig.max_duration`, and output `-fs` for `max_size_bytes`. The raw-recorder PR
   explicitly enables gdigrab cursor capture so the unannotated intermediate delivery remains
   understandable.
8. `Recorder::stop` first checks whether ffmpeg already exited. An already-exited process produces
   `RecordingCompletionStatus::StoppedEarly` and is still subject to output validation. A live
   process receives `q\n` through its piped stdin, stdin is closed, and the process gets 15 seconds
   to exit and write the MP4 trailer.
9. If writing `q\n` fails or ffmpeg misses the stop deadline, the recorder force-kills and reaps
   the child, deletes the output and log, and returns `RecordingError::Finalize`. It never
   publishes a file that may lack its MP4 trailer. A successful stop returns only a non-empty MP4
   that ffmpeg can inspect for a valid video duration.
10. `RecordingOutput` reports the real output path, capture elapsed duration, even capture width
    and height, file size, and completion status. The caller owns upload and cleanup after a
    successful stop.
11. Dropping a live `RecordingHandle` without `stop` terminates ffmpeg through
    `kill_on_drop(true)` and removes partial MP4/log files. Every explicit failure path also waits
    for or reaps the child. No ffmpeg child or temporary output is left behind.
12. The final overlay PR applies the current Linux post-stop sequence on Windows: capture a 1x
    source, retain meaningful action segments, remap action times, and burn labels plus synthetic
    pointer annotations into the upload candidate. Processing remains best-effort: a processing
    failure uploads the valid raw source.
13. Windows pointer annotations use recording-frame coordinates. A virtual-screen point
    `(x, y)` maps to `(x - virtual_origin_x, y - virtual_origin_y)` before clipping to the
    even-rounded output bounds. This preserves placement when the virtual origin is negative.
14. The overlay PR disables gdigrab's native cursor before enabling the synthetic pointer
    renderer. It must not display both cursors. It mirrors the Linux click, drag, scroll, keyboard,
    typing-redaction, smart-cut, and cleanup behavior. The raw-recorder PR keeps the native cursor
    enabled until that replacement lands.
15. No audio is captured. No public API, wire type, recording approval flow, screenshot bytes,
    input behavior, feature flag, or server behavior changes.

== TECH ==
*Context:* All Warp source references below are pinned to client commit
`4143c09ff8be80ff73165f36085cf4a013295a01`.

- [`crates/computer_use/src/lib.rs:236-388 @ 4143c09ff`](https://github.com/warpdotdev/warp/blob/4143c09ff8be80ff73165f36085cf4a013295a01/crates/computer_use/src/lib.rs#L236-L388)
  defines `create_recorder`, the `Recorder` trait, `RecordingConfig`, `RecordingHandle`, process
  polling, and drop cleanup. The exact trait contract is:
  `start(&self, RecordingConfig) -> Result<RecordingHandle, RecordingError>` and
  `stop(&self, RecordingHandle) -> Result<RecordingOutput, RecordingError>`.
- [`crates/computer_use/src/windows/mod.rs:1-17 @ 4143c09ff`](https://github.com/warpdotdev/warp/blob/4143c09ff8be80ff73165f36085cf4a013295a01/crates/computer_use/src/windows/mod.rs#L1-L17)
  implements Windows input and screenshots but re-exports `crate::noop::Recorder`.
- [`crates/computer_use/src/windows/screenshot.rs:20-92 @ 4143c09ff`](https://github.com/warpdotdev/warp/blob/4143c09ff8be80ff73165f36085cf4a013295a01/crates/computer_use/src/windows/screenshot.rs#L20-L92)
  enters per-monitor-v2 DPI awareness, reads signed virtual-screen origin and dimensions, and
  captures the composited desktop from `GetDC(NULL)`.
- [`crates/computer_use/src/windows/dpi.rs:21-47 @ 4143c09ff`](https://github.com/warpdotdev/warp/blob/4143c09ff8be80ff73165f36085cf4a013295a01/crates/computer_use/src/windows/dpi.rs#L21-L47)
  provides the thread-local `DpiAwarenessGuard`.
- [`crates/computer_use/src/mac/recording.rs:50-278 @ 4143c09ff`](https://github.com/warpdotdev/warp/blob/4143c09ff8be80ff73165f36085cf4a013295a01/crates/computer_use/src/mac/recording.rs#L50-L278)
  is the structural recorder model: bounded ffmpeg spawn, file-growth readiness, process ownership,
  finalization, diagnostics, and output construction.
- [`crates/computer_use/src/linux/recording.rs:42-224 @ 4143c09ff`](https://github.com/warpdotdev/warp/blob/4143c09ff8be80ff73165f36085cf4a013295a01/crates/computer_use/src/linux/recording.rs#L42-L224)
  provides the parallel `x11grab` path and a separable command builder/launcher shape.
- [`crates/computer_use/src/linux/recording.rs:489-607 @ 4143c09ff`](https://github.com/warpdotdev/warp/blob/4143c09ff8be80ff73165f36085cf4a013295a01/crates/computer_use/src/linux/recording.rs#L489-L607)
  owns Linux smart-cut/overlay post-processing, display geometry, readiness polling, and stderr-tail
  diagnostics.
- [`crates/computer_use/src/recording_metadata.rs:1-54 @ 4143c09ff`](https://github.com/warpdotdev/warp/blob/4143c09ff8be80ff73165f36085cf4a013295a01/crates/computer_use/src/recording_metadata.rs#L1-L54)
  verifies that ffmpeg can report a valid finalized media duration.
- [`app/src/ai/blocklist/action_model/recording_finalize.rs:100-184 @ 4143c09ff`](https://github.com/warpdotdev/warp/blob/4143c09ff8be80ff73165f36085cf4a013295a01/app/src/ai/blocklist/action_model/recording_finalize.rs#L100-L184)
  stops the recorder, applies platform post-processing, uploads the selected video, and removes
  local files.

*Design alternatives:*
- *WGC or DDA plus Media Foundation:* deferred. This is the older investigation's preferred
  long-term native architecture. It can improve GPU-path efficiency and capture content that GDI
  misses. It requires native frame acquisition, encoding, device-loss recovery, per-monitor
  stitching, and broader operational validation. It is not required to close the current recording
  parity gap.
- *ffmpeg gdigrab:* selected. It uses the same composited desktop family as screenshots, fits the
  existing ffmpeg lifecycle, and minimizes the initial implementation and review surface. It
  inherits GDI limits for protected content, hardware overlays, and some fullscreen surfaces.
- *Terminate or kill ffmpeg on stop:* rejected. Abrupt termination can omit the MP4 trailer and
  leave an unplayable artifact.
- *Send a Windows console control event:* rejected. It requires process-group and console
  attachment semantics that are fragile in GUI/cloud processes. ffmpeg's documented stdin `q`
  command is direct and does not depend on a console.
- *Treat output-file creation as readiness:* rejected. ffmpeg can create an empty path before it
  opens the capture source. Growth beyond zero bytes is the current cross-platform readiness
  contract and is selected for this delivery.
- *Ship recording and overlays in one PR:* rejected. The shared cfg changes, Windows process
  lifecycle, and overlay expansion have different failure modes. The three-PR stack makes each
  boundary reviewable and leaves a usable raw-video milestone.

*Proposed changes and stacked PR order:*

1. *PR 1 — shared Windows recording plumbing.*
   - Extend the `RecordingHandle` process/path/start-time/cleanup fields, `poll_exit`,
     `new_test`, and `Drop` cfgs from `any(linux, macos)` to
     `any(linux, macos, windows)`.
   - Extend `recording_metadata`, finalized-duration inspection, and video-thumbnail cfgs to
     Windows. Update comments that still call recording unsupported off Linux/macOS.
   - Add Windows-only `tokio` features `io-util`, `process`, and `time`, plus `uuid`, in
     `crates/computer_use/Cargo.toml`. Do not add `nix` on Windows.
   - Add Windows lifecycle tests for synthetic exit state and abandoned-output cleanup. This PR
     must compile without changing `windows/mod.rs` away from the no-op recorder.

2. *PR 2 — `windows/recording.rs` raw recorder.*
   - Add `mod recording; pub use recording::Recorder;` in `windows/mod.rs`.
   - Add a synchronous geometry helper that enters `DpiAwarenessGuard`, reads all four virtual
     screen metrics, validates positive dimensions, rounds width/height down with `& !1`, and
     drops the guard before the first async wait. Return `{origin_x, origin_y, width, height}`.
   - Before capture, run `ffmpeg -hide_banner -h demuxer=gdigrab`. A spawn failure, non-successful
     probe, or response that does not identify `gdigrab` is
     `RecordingError::Environment`. Preserve a bounded diagnostic, not the full tool output.
   - Build the capture command as
     `ffmpeg -y -f gdigrab -framerate <fps> -offset_x <signed-x> -offset_y <signed-y>
     -video_size <even-width>x<even-height> -draw_mouse 1 -t <seconds> -i desktop
     -c:v libx264 -preset ultrafast -pix_fmt yuv420p -movflags +faststart
     -fs <max-bytes> <path>`.
   - Keep `-t` before `-i` so it bounds wall-clock capture independently of output processing.
     Ignore `RecordingConfig.target` and `playback_speed_multiplier` for the Windows raw master.
     Windows captures 1x so PR 3 can perform the same smart cut as Linux.
   - Create a unique temporary MP4 and sibling log. Pipe stdin, discard stdout, send stderr to the
     log, and set `kill_on_drop(true)`. Wait for output growth with the existing 15-second timeout
     and 100-millisecond poll interval.
   - On stop, use `tokio::io::AsyncWriteExt` to write `q\n`, close/drop stdin, and wait up to
     15 seconds. Reap on every path. After graceful exit, reject an empty file and call the shared
     finalized-duration probe. Delete the file on invalid media. Transfer file ownership only
     after these checks pass.
   - Add `windows/recording_tests.rs`, included through the repository's separate-test-file
     convention.

3. *PR 3 — Windows smart cut and overlays.*
   - Extend `post_process_recording` and the overlay renderer cfgs to Windows. Reuse the Linux
     disk-to-disk smart-cut and ASS burn-in behavior rather than adding a second renderer.
   - Extend the Windows actor to populate `PointerSink` for down, move, up, and scroll actions
     while a recording is active. Convert signed virtual coordinates to frame coordinates by
     subtracting the recording's virtual origin. Clip or omit out-of-frame points using the
     even-rounded frame bounds.
   - Persist the virtual origin with the recording state or expose it through a Windows-specific
     mapping seam. Do not change public wire types.
   - Switch the gdigrab command to `-draw_mouse 0` in the same PR that enables the synthetic
     cursor. Preserve Linux overlay timing, redaction, click/drag classification, smart-cut
     margins, best-effort fallback, and temporary-file cleanup.
   - Add Windows coordinate-mapping, pointer-collection, ASS generation, smart-cut, fallback, and
     no-double-cursor tests.

*ffmpeg image prerequisite:*
- The Windows runtime image must put a compatible ffmpeg build on `PATH` before real recording can
  succeed. That build must include `gdigrab`, `libx264`, the MP4 muxer, and the filters/codecs used
  by the later overlay pass.
- Image packaging and license review are deployment work outside these three `warp` client PRs.
  They can land independently. Until they land, Windows `Recorder::start` returns the explicit
  environment error from invariant 2.
- The implementation PR must record the tested ffmpeg version and build configuration. It must not
  assume that every generic Windows ffmpeg package enables `gdigrab` or `libx264`.

*Out of scope:*
- WGC, DDA, Media Foundation, GPU zero-copy, HDR tone mapping, audio, protected-content bypass,
  and a unified screenshot/recording capture-source abstraction.
- Window-scoped Windows recording or background Windows computer use.
- Changes to the Windows input implementation, GDI screenshot bytes, server APIs, recording
  approval flow, or feature rollout.
- Fixing the shared negative-coordinate screenshot-region limitation. Full virtual-screen
  recording still includes monitors at negative origins.

*Interview decisions:*
- Missing or unusable ffmpeg/gdigrab fails fast with `RecordingError::Environment`; there is no
  native fallback in this delivery.
- The implementation stack is plumbing, then raw playable recording/upload, then Windows pointer
  collection and smart-cut/overlay parity.
- Graceful stop writes `q\n` and uses the existing 15-second timeout. A write failure or timeout
  force-kills/reaps ffmpeg, deletes the output, and returns `RecordingError::Finalize`.

*Validation & verification criteria* (must ALL pass before the relevant implementation PR merges):
1. *Command construction:* Windows unit tests assert exact relative ordering and values for
   `gdigrab`, frame rate, signed offsets, even video size, cursor mode, input `-t`, `desktop`
   input, H.264/yuv420p settings, `+faststart`, output `-fs`, and path. Tests include a negative
   origin and odd virtual dimensions.
2. *Availability:* hermetic tests use a fake ffmpeg executable for executable missing, successful
   gdigrab probe, probe failure, and a build without gdigrab. Only the usable probe reaches capture
   spawn. Failures are `RecordingError::Environment` and name the missing requirement.
3. *Readiness:* fake-process tests prove that start waits for output growth, succeeds after delayed
   growth, fails on early child exit with a bounded stderr tail, and times out after the configured
   deadline. Every failed attempt is killed/reaped and removes MP4/log files.
4. *Geometry:* Windows-only tests query a real multi-monitor virtual desktop when available and
   pure helper tests cover negative origins, mixed DPI, odd dimensions, invalid dimensions, and
   unchanged origin after even rounding. The returned handle and encoded stream report the
   expected even physical-pixel dimensions.
5. *Graceful finalization:* fake ffmpeg tests assert that stop writes the exact bytes `q\n`, closes
   stdin, waits for exit, and reports `Completed`. Separate tests cover already-exited
   `StoppedEarly`, stdin write failure, stop timeout, forced kill/reap, empty output, invalid MP4,
   and temporary-file cleanup.
6. *Playable media:* a Windows integration test records a known changing desktop for at least two
   seconds, stops it, inspects duration and dimensions with ffmpeg, and decodes at least the first
   and last frames. The frames must be non-empty and differ. The MP4 must open from its finalized
   path after the child exits.
7. *Virtual desktop:* on a real Windows host with at least two displays and one display left of or
   above the primary, compare a full GDI screenshot and a recording frame. Both must cover the
   same virtual-screen bounds. Evidence records the four `SM_*VIRTUALSCREEN` metrics, DPI settings,
   encoded dimensions, and any one-pixel even rounding.
8. *Bounds:* tests set small `max_duration` and `max_size_bytes` values and verify ffmpeg stops,
   yields `StoppedEarly`, and returns a playable bounded prefix when ffmpeg finalized it normally.
9. *Overlay stack:* Windows tests mirror the Linux segment, label, pointer, redaction, fallback,
   and cleanup suites. A real Windows artifact includes the synthetic cursor, click ripple, drag
   trail, and labels at correct positions after smart cutting, with no second native cursor.
10. *Cross-platform regression:* existing Linux `recording_tests.rs`, macOS
    `recording_tests.rs`, shared recording-handle tests, overlay tests, metadata tests, and
    thumbnail tests remain green. Linux/macOS command arguments and stop signals do not change.
11. *Repository gates:* run `./script/format` and the full `./script/presubmit` from the Warp
    repository before each implementation PR is opened or updated. Run the Windows
    `computer_use` build and tests on a Windows runner; Linux-only presubmit is not Windows proof.
12. *Required real-Windows evidence:* before the raw-recorder PR is marked ready, attach a sample
    MP4 and state the Windows version, host type (Windows CI, physical machine, or Parallels VM),
    display topology, scale factors, ffmpeg version/build, exact test flow, and observed duration,
    dimensions, size, completion status, and playback/decode result. Before the overlay PR is
    marked ready, attach a recording of known pointer/keyboard actions and state which annotations
    and cuts were visually verified. Computer-use visual proof was not requested for this spec-only
    PR; it is mandatory for the future implementation PRs.
