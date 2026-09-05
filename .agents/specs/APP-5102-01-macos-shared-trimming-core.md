# APP-5102 — 1 of 3: Shared recording core and macOS action trimming

**Order:** Land this spec first. Spec 2 (overlays) and spec 3 (window capture) depend on the shared 1× source timeline and post-processing seam defined here.

## macOS vs. Linux delta

**Linux reference:** Linux captures a 1× `x11grab` master, then `post_process_recording` uses the shared action log to build keep segments, cut with `trim`/`setpts=PTS-STARTPTS`/`concat`, and burn ASS overlays.

**macOS difference:** macOS currently applies a live `setpts=(1 / playback_speed_multiplier)*PTS` filter to its AVFoundation capture, so the source timeline is already compressed before the app supplies wall-clock action offsets. `computer_use::post_process_recording` is also a non-Linux no-op, while the cut implementation lives inside `linux/recording.rs`. Parity therefore requires a 1× AVFoundation master and one shared post-stop policy; it does **not** require macOS-specific cut thresholds or a different ffmpeg mechanism.

## Summary

Move the ffmpeg process and segment-cut behavior that is genuinely platform-neutral into a shared recording core. Keep platform adapters responsible for selecting and preparing capture input. Remove macOS's live speed filter, retain `playback_speed_multiplier` only for request compatibility, and run the same keep-segment/cut policy on macOS and Linux. Linux continues to add its existing ASS burn-in after the shared cut; spec 2 makes that post-cut stage shared and enables it on macOS.

## Behavior contract

1. macOS and Linux masters are recorded at 1×. Neither platform applies `playback_speed_multiplier` to live capture.
2. Both platforms use the existing shared action policy: one successful meaningful `UseComputer` group contributes its real start/finish window, expanded by the same 250 ms pre-action and 1000 ms post-action margins; overlapping/touching windows merge and other gaps are removed.
3. Retained frames remain at 1×. ffmpeg uses `setpts=PTS-STARTPTS` only to reset each retained strip before concatenation, not to speed up action footage.
4. The compacted output duration is the sum of retained segments. It is shorter than wall-clock capture when there are thinking gaps.
5. After this spec alone, Linux returns its annotated cut and macOS returns its unannotated cut. Spec 2 adds macOS overlays without changing segments or timing.
6. Empty-action, cancellation, duration/size bounds, finalization, upload, thumbnail, cleanup, and best-effort fallback semantics remain unchanged. A processing failure uploads the untouched 1× master.
7. AVFoundation input selection, Screen Recording TCC handling, macOS startup diagnostics/retries, and `x11grab`/`DISPLAY` handling remain platform-specific.

## Technical approach

Code references are pinned to `warpdotdev/warp@7a6044bd5377d708ab1d3767ece505a49d232aed`.

- `crates/computer_use/src/mac/recording.rs:204-243` builds the AVFoundation command and currently inserts the live `setpts` filter.
- `crates/computer_use/src/linux/recording.rs:383-607` owns cut, ASS burn-in, and `post_process_recording`, even though those ffmpeg operations are platform-neutral.
- `crates/computer_use/src/overlay.rs:218-526` owns the keep-segment policy and source-to-output mapping behind `cfg(any(linux, test))`.
- `crates/computer_use/src/lib.rs:264-287` exposes `post_process_recording` only for real Linux builds.
- `app/src/ai/blocklist/action_model/recording_finalize.rs:102-219` already calls the platform-neutral API before upload and falls back to the original.

Implement a shared module under `cfg(any(macos, linux))` with:

- temp path/log allocation, common ffmpeg output/encode settings, supervised launch/finalize/cleanup, and a platform-provided readiness hook;
- `cut_to_segments`, cut filtergraph construction, cut-file lifecycle, and the shared cut portion of `post_process_recording`;
- the existing `build_keep_segments` and source-to-output timing helpers compiled for `any(macos, linux, test)`.

The post-process orchestration has an explicit post-cut stage. Linux keeps invoking its current ASS burn-in there; macOS returns the cut directly until spec 2 moves ASS generation/burn-in into that shared stage. This is the shared prerequisite between the otherwise separate trimming and overlay changes.

Each platform adapter continues to own:

- input selection and arguments (`x11grab`/`DISPLAY`/`-window_id` versus AVFoundation/ScreenCaptureKit);
- dimensions and target preparation;
- cursor input flags;
- platform startup classification, permissions, and retry policy.

The shared launch API must allow macOS-specific AVFoundation readiness/retry behavior without forcing Linux to understand AVFoundation diagnostics. The shared post-process API remains exactly the call the app already uses.

Delete the live macOS `setpts` branch. Keep `RecordingConfig.playback_speed_multiplier` accepted and documented as unused by the two real 1× recorders until the wire field can be removed independently.

## Design alternatives

- **Chosen default — shared lifecycle/output/post-process core plus platform input adapters.** Removes meaningful duplication while leaving platform quirks explicit.
- **Share post-processing only.** Smaller Linux blast radius, but preserves nearly identical path/log/spawn/SIGINT/cleanup code and does not realize the requested recording-core boundary.
- **One fully generic recorder/backend trait.** Rejected for this step: pointer acquisition, window targeting, TCC/`DISPLAY`, and startup classification do not benefit from being forced through one abstraction.
- **Keep macOS live speedup and remap timestamps.** Rejected: it creates two timelines, speeds action footage, and makes the Linux keep policy impossible to share safely.

## Review-time open decision

**Default for review:** share lifecycle/output/post-processing while retaining a platform readiness hook. If review prefers the narrower “post-processing only” extraction, spec 1 can be reduced without changing its user-visible 1× and trimming contract. Specs 2 and 3 require the shared post-process and 1× timeline, not necessarily the full lifecycle extraction. If this draft is approved unchanged, the implementor follows the default and does not reopen the choice.

## Validation and verification criteria

1. Replace the macOS argv regression with `mac_capture_command_captures_at_1x_without_setpts`: a configuration with multiplier 4 contains no live `setpts` and no speed-only `-vf`, while preserving `-t` before `-i`, `-fs`, codec, pixel format, and movflags.
2. Run the same `build_keep_segments` table on macOS and Linux/test cfg. It covers empty, one, out-of-order, duplicate, adjacent/overlapping, same-frame, source-boundary, and long-gap cases with identical expected segments.
3. Move `build_cut_only_filtergraph_constructs_trim_setpts_concat` and `smart_cut_retains_only_selected_frames_in_order` to the shared module. They pass on macOS with the same selected frames, order, PTS, cadence, and summed duration as Linux.
4. Add a macOS post-process fixture with two action groups separated by a known gap. The output drops that gap, retains both groups at 1×, and reports the finalized media duration rather than the wall-clock capture duration.
5. Induce cut failure and assert finalization uploads the original 1× master and removes `.cut.mp4` intermediates. Existing Linux ASS/overlay cleanup remains green; macOS ASS cleanup is covered by spec 2.
6. Existing macOS capture startup/finalization and Linux full-screen/window recording tests remain green.
7. `./script/format`, `cargo clippy -p computer_use --all-targets --all-features --tests -- -D warnings`, and `cargo build -p computer_use` pass on the implementation branch.
8. Exercise the running macOS flow with computer use and attach video proof plus the produced recording artifact to the task and implementation PR.

## Verify locally on macOS

Run from the `warp` checkout with a sibling `warp-server` checkout:

```bash
WARP_REPO="$(git rev-parse --show-toplevel)"
WARP_SERVER_REPO="$(cd "$WARP_REPO/../warp-server" && pwd)"
brew install ffmpeg jq

cd "$WARP_REPO"
cargo test -p computer_use --lib mac_capture_command_captures_at_1x_without_setpts
cargo test -p computer_use --lib build_action_segments_uses_finish_offsets_and_drops_blocked_gaps
cargo test -p computer_use --lib smart_cut_retains_only_selected_frames_in_order
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

Grant Screen Recording permission to `WarpLocal`/the launching terminal when macOS prompts, then submit this local Oz prompt:

> Start a video recording. Open TextEdit and type `trim-before`. Do not interact with the computer for 8 seconds so there is a thinking gap. Then type `trim-after`, press Return, stop the recording, and publish it.

The uploaded MP4 appears in the local Oz run's **Artifacts** panel. The source/intermediate files use `/tmp/warp-recording-*.mp4` while processing and are deleted after upload. Download the artifact, then run:

```bash
VIDEO="$(ls -t "$HOME"/Downloads/*.mp4 | sed -n '1p')"
ffprobe -v error \
  -show_entries format=duration \
  -of default=noprint_wrappers=1:nokey=1 \
  "$VIDEO"
```

Eyeball both edits playing at normal speed and the 8-second idle gap being absent. The output must be materially shorter than wall clock; it must not be a uniformly 4×-sped video.

Linux regression check:

```bash
cd "$WARP_REPO"
cargo test -p computer_use --lib linux_capture_command_captures_at_1x_without_setpts
cargo test -p computer_use --lib build_cut_only_filtergraph_constructs_trim_setpts_concat
cargo test -p computer_use --lib smart_cut_retains_only_selected_frames_in_order
```
