# Spec: Richer click/drag annotations for computer-use video artifacts (QUALITY-1169)

Linear: [QUALITY-1169](https://linear.app/warpdotdev/issue/QUALITY-1169/richer-clickdrag-annotations-for-computer-use-video-artifacts-ripple) · Repository: `warpdotdev/warp` (client)
Base branch: `master` (the recording stack — the post-stop smart cut and overlay preservation, PR #14148 — is now merged to master). This is a normal client PR against `master`; there is no `warp-server` code change.

## Scope note (revised)
This revision narrows the original cross-repo spec to what we are actually shipping now:
- **Linux only.** Burn-in only runs on the Linux x11grab capture path. `computer_use::burn_in_action_log` already returns the input unchanged on macOS and every other target (`lib.rs`), so the artifact is never re-encoded off Linux. We are intentionally **not** adding a macOS path: macOS applies no post-stop processing today (it still uses the avfoundation playback-speed capture), and adding a second compositor there is out of scope.
- **Smart-cut aware.** The recording is already cut to effective action windows post-stop (`build_keep_segments` + `cut_to_segments`), which removes idle/thinking gaps and remaps overlays onto the compacted timeline. The click ring and drag trail must survive the cut like the existing pills. The cut already retains 1000 ms (`SEGMENT_MARGIN_POST`) after each action, which exceeds the 900 ms ring / 600 ms fade; rather than change the cut, the animation durations are *derived* from that margin so they always fit (see behavior invariant 7).

## == PRODUCT ==
**Summary:** Published computer-use video artifacts should make pointer input legible without reading the transcript. Add a deterministic, client-side click ripple and a distinct drag animation, burned into the uploaded Linux video only. The live display and every screenshot used for model perception are unchanged.
**Key design choices:**
1. **Client-side burn-in, reusing the existing ASS pass.** The client already owns the structured action stream, capture target/dimensions, platform coordinate mapping, the recording lifecycle, the post-stop smart cut, and the upload hook. We extend the existing ASS (`subtitles` filter) burn-in with vector drawings (`\p`) for rings, anchors, and trails, reusing the current disk-to-disk ffmpeg flow and the `build_keep_segments`/`remap_source_interval` machinery. No server composition, no second frame-buffer pipeline.
2. **One canonical pointer-event log in capture pixels.** Record resolved pointer events with monotonic offsets (source/1x timeline), button, and capture-space coordinates at dispatch time in the x11 actor, rather than reconstructing geometry from text summaries or the final cursor position. Keep the existing semantic text-label path additive and unchanged.
3. **Animation survives the cut by construction.** Derive each animation's maximum tail from the cut's retained post-action margin (`SEGMENT_MARGIN_POST`), so a ripple/trail always fits inside the footage the cut keeps and shrinks automatically if that margin is reduced — no `build_keep_segments` change is needed.
**Behavior** (numbered, testable invariants):
1. A left, right, or middle click produces one visible orange ripple centered on the click coordinate. Double- and triple-clicks produce one pulse per completed click, with overlapping pulses allowed at the same coordinate; no click is silently collapsed.
2. A drag is visually distinct from a click: while the button is held, a filled held indicator, a start anchor, and a trail follow the recorded path; on release the trail and anchor fade out. A drag release never also emits a click ripple for the same down/move/up sequence.
3. The animation model is deterministic and bounded, mirroring the integration-test reference renderer (`crates/warpui_core/src/integration/overlay.rs`): click ring duration 900 ms, radius 18 px→36 px with 4 px stroke; held indicator 16 px radius; drag trail 4 px wide with a 10 px start anchor; trail/anchor fade for 600 ms after release. Orange fill (`[255, 80, 40]`). All constants are centralized in the reusable renderer and all vector bounds are clipped to the video frame.
4. Pointer-only `UseComputer` groups still retain their timing window for smart cutting (they are already "meaningful" with empty labels), but their empty label list does not suppress pointer geometry. Mixed pointer + keyboard/typing/scroll groups render both the pointer geometry and the existing text pills in their current ordering. Wait-only, screenshot-only, and other no-op groups render neither.
5. Typed payloads never appear in the artifact. Existing `typing…` redaction, printable-key redaction, key/scroll labels, and ASS escaping are unchanged; coordinates live only in the in-memory artifact event log and are not emitted to ordinary logs or telemetry.
6. An event is drawn in the coordinate space of the captured artifact:
   - full-screen capture (`Target::Screen`): resolved physical root/screen pixels map directly to the output frame;
   - window capture (`Target::Window`): the window-local pixels used to drive the action map to the captured window's native dimensions;
   - a target mismatch (e.g. a window-targeted action while recording the screen) is resolved through the same platform helper used to dispatch the action (`window_local_to_root`), then normalized to the recording target;
   - negative, out-of-bounds, or non-finite values are clamped to the frame or omitted before rendering, never allowed to address outside the frame.
7. **The click/drag animation survives the smart cut.** The recording is cut to retained action windows and overlays are remapped onto the compacted output timeline. Each pointer event occurs within its group's action window (`offset..finish_offset`), and the cut already retains `SEGMENT_MARGIN_POST` (1000 ms) after `finish_offset`. The animation durations are capped at `SEGMENT_MARGIN_POST - safety` (so the effective ring/fade are ≤ the retained tail), which guarantees the whole animation falls inside a single retained segment and remaps linearly. If that margin were ever reduced below the design durations, the animations shrink with it rather than being clipped.
8. The feature applies to every supported Linux real recording finalization path (agent stop, agent-finished, duration/size limit, ffmpeg early exit). No-op, mock, and test-util recorders remain safe: burn-in returns the original path unchanged without panicking.
9. The live display, the agent's screenshots, and action results are byte-identical to current behavior. Only the post-stop Linux artifact changes. Existing cursor capture (`-draw_mouse 1`) stays enabled; the custom ripple is the burned-in indicator (x11grab has no native click highlight to disable).
10. If ASS generation, ffmpeg, libass, or fonts fail, the finalize path logs a non-sensitive warning, removes temporary intermediates, and uploads the original 1x recording (a no-cut video beats no video). Annotation failure never prevents publication. This preserves the existing best-effort contract in `recording_finalize.rs`.
11. No server-side change, artifact metadata field, or public API is introduced. The server continues to emit the existing `MouseDown`/`MouseMove`/`MouseUp` actions and coordinate data.

## == TECH ==
### Context (current code on the base branch)
- `crates/computer_use/src/overlay.rs`: `ActionLogEntry { offset, finish_offset, labels }`; `overlay_labels_for` omits pointer primitives; `build_keep_segments` (expands each group by `SEGMENT_MARGIN_PRE = 250 ms` / `SEGMENT_MARGIN_POST = 1000 ms`, merges, assigns `output_start`); `remap_source_interval`; `overlay_display_interval` (pill linger reuses `SEGMENT_MARGIN_POST`); `build_overlay_ass` (text pills, remapped through segments). All `.ass`/segment code is `#[cfg(any(linux, test))]`.
- `crates/computer_use/src/linux/recording.rs`: 1x x11grab capture (`-draw_mouse 1`, no live `setpts`); `burn_in_action_log` → `cut_to_segments` (trim/`setpts=PTS-STARTPTS`/`concat`) then `build_overlay_ass` + `burn_overlays_into_cut` (`subtitles` filter). Best-effort; empty segments → error → caller uploads the source.
- `crates/computer_use/src/linux/x11/mod.rs`: `Actor::perform_actions` dispatches `MouseDown{button,at}`/`MouseUp{button}`/`MouseMove{to}`. Screen-target coords are root pixels; window-target coords are window-local and converted via `windows::window_local_to_root`. `last_mouse_position` is tracked in root pixels.
- `crates/computer_use/src/lib.rs`: `Options { screenshot_params, background_enabled }`; `Target`; `MouseButton`; `burn_in_action_log` gated `#[cfg(all(linux, not(noop)))]`, otherwise returns `input` unchanged.
- `app/src/ai/blocklist/action_model/recording_controller.rs`: `ActiveRecording` (holds `frame_rate`, committed `actions`, `pending_group`); `begin_action_group` (returns capture-start `Instant`), `commit_action_group`, `discard_action_group`. Does not currently retain the recording `Target`.
- `app/src/ai/blocklist/action_model/execute/use_computer.rs`: begins a group before dispatch, commits with the finish offset on success or discards on failure/cancellation.
- `app/src/ai/blocklist/action_model/execute/start_recording.rs`: resolves the `Target` and frame rate at start.
- `app/src/ai/blocklist/action_model/recording_finalize.rs`: calls `burn_in_action_log` best-effort, then uploads.
- `crates/warpui_core/src/integration/overlay.rs`: integration-test-only raster renderer; the reuse reference for constants/behavior (900 ms ring, 600 ms fade, 16 px held, 10 px anchor, 4 px trail, orange). Not the production path.

### Data model
Add to `crates/computer_use/src/overlay.rs` (ungated; used by the app on every platform, like `ActionLogEntry`):
- `PointerEvent { offset: Duration, kind: PointerEventKind, button: Option<MouseButton>, point: Vector2I }`, where `point` is a capture-space pixel and `offset` is on the same source/1x clock as `ActionLogEntry.offset`.
- `PointerEventKind { Down, Move, Up }`.
- `ActionLogEntry` gains `pointer_events: Vec<PointerEvent>` (labels preserved unchanged).
Export `PointerEvent`/`PointerEventKind` from `lib.rs`.

### Capture (app + x11 actor)
- `ActiveRecording` retains the resolved `Target`; `start_recording.rs` passes it to `finish_start`. `begin_action_group` returns the capture-start `Instant` **and** the recording `Target` so the executor can build the sink.
- Add `Options.pointer_sink: Option<PointerSink>` where `PointerSink { started_at: Instant, recording_target: Target, events: Arc<Mutex<Vec<PointerEvent>>> }`. `None` on non-recording, CLI, `request_computer_use`, and test paths.
- `use_computer.rs`: when a meaningful group is begun during an active recording, create the shared events buffer, build `PointerSink`, and pass it in `Options`. After the actor future returns, drain the buffer and hand the events to `commit_action_group` (added parameter); discard on failure/cancellation exactly as labels are today.
- The **x11 actor** records a `PointerEvent` immediately before dispatching each `MouseDown`/`MouseMove`/`MouseUp`, resolving `point` into capture space:
  - `Target::Screen` recording: screen action → the root `at`/`to`; window action → the `window_local_to_root`-converted root point; `MouseUp` reuses the last recorded point.
  - `Target::Window { rec_id }` recording: window action targeting `rec_id` → the original window-local `at`/`to`; any other target (screen action, or a different window) → omit that event.
  - `offset = Instant::now() - sink.started_at`. Clamp/omit invalid points. Only the x11 actor populates the sink; macOS/Wayland/Windows/noop ignore it (no Linux burn-in there anyway).

### Renderer (ASS vectors) — `overlay.rs`, `#[cfg(any(linux, test))]`
- Constants centralized (design values): `CLICK_RING_DURATION = 900 ms`, `CLICK_RING_MIN_RADIUS = 18`, `CLICK_RING_MAX_RADIUS = 36`, `CLICK_RING_THICKNESS = 4`, `HELD_INDICATOR_RADIUS = 16`, `DRAG_ANCHOR_RADIUS = 10`, `DRAG_TRAIL_THICKNESS = 4`, `DRAG_TRAIL_FADE_DURATION = 600 ms`, orange `[255, 80, 40]`. Effective ring/fade durations are `min(design, max_animation_tail())` where `max_animation_tail() = SEGMENT_MARGIN_POST - ANIMATION_TAIL_SAFETY` (safety = 100 ms).
- Classify each entry's `pointer_events` into gestures: a `Down`…`Up` with no intervening `Move` is a **click** (emit a ring); a `Down` + one or more `Move` + `Up` is a **drag** (emit held indicator + start anchor + trail; no ring). Enforce drag-vs-click exclusivity (invariant 2).
- Emit ASS `\p` vector dialogues, positioned in `PlayResX/Y` space, each remapped through `build_keep_segments` via `remap_source_interval`:
  - **Ring**: a circle drawn with transparent fill + `\bord` outline (orange), `\an5\pos(cx,cy)`, animating `\fscx/\fscy` (radius 18→36) and `\alpha` (opaque→transparent) via `\t` over 900 ms.
  - **Trail**: a stroked polyline (`m … l …`, transparent fill + `\bord`) through the drag points, with a `\t` alpha fade over the last 600 ms after release.
  - **Held indicator** (16 px) + **start anchor** (10 px): filled orange circles.
  - Clip every path to the frame; reject malformed/non-finite coordinates before writing.
- **Smart-cut coverage**: no `build_keep_segments` change. The effective ring/fade durations are `min(design, SEGMENT_MARGIN_POST - ANIMATION_TAIL_SAFETY)` (via `max_animation_tail()`), so each animation ends within the 1000 ms the cut already retains after `finish_offset`, lies inside one contiguous retained segment, and remaps linearly. Text-pill timing/behavior is unchanged.
- `build_overlay_ass` emits pointer dialogues in addition to the existing pills; ordering, escaping, and pill positions are unchanged.

### Runtime prerequisite
Burn-in requires an ffmpeg with the `subtitles`/libass filter and a resolvable scalable font. The `warp-agent-docker` sidecar installs these for the `dev` channel only; stable/dogfood must be verified before enabling more broadly. Any font/package change is a companion `warp-agent-docker` PR, not a client or server change. Failure remains best-effort (invariant 10).

## Validation & verification criteria
1. **Renderer unit tests** (`cargo nextest run -p computer_use`): click (ring, 900 ms, 18→36 px, 4 px), double/triple clicks (one ring per completed click), straight and multi-segment drags (held/anchor/trail + 600 ms fade, no ring on release), mixed pointer + keyboard/scroll (pills unchanged), pointer-only vs wait/screenshot-only groups. Assert vector tags, remapped timecodes, clipping, drag-vs-click exclusivity, and that no typed payload appears in the ASS.
2. **Smart-cut coverage test**: a click at the group finish still remaps to a full-duration ring (not clipped) because it fits inside the retained 1000 ms margin; assert pointer events do not change `build_keep_segments` output and that existing pill remapping is unchanged.
3. **Coordinate resolution tests**: screen vs window recording targets, matched and mismatched action targets, negative/out-of-bounds clamped or omitted; assert no point lies outside `[0,width) × [0,height)`.
4. **Lifecycle/plumbing tests** (`recording_controller_tests`): `begin_action_group` returns the target; commit stores pointer events; failed/cancelled calls discard them; pointer-only empty-label groups still commit; no cross-conversation contamination.
5. **Fallback/cleanup**: missing ffmpeg/font/malformed ASS still uploads the original mp4; the warning contains no coordinates or typed text; all `.ass`/cut/overlay intermediates are removed.
6. **Regression**: full `computer_use` suite + recording controller/finalize suites stay green; existing keyboard/typing redaction, scroll labels, smart cut, cursor capture, and upload behavior unchanged.
7. **Perception isolation**: a representative screenshot/action result is byte-identical with and without recording enabled.
8. **Real Linux artifact verification** (deferred to PR review): record known left/right/middle/double/triple clicks and a ≥3-point drag, stop, download the published mp4, and inspect frames at the click/drag timecodes — one ripple per click, held/anchor/trail + fade on the drag, no ripple on drag release, correct coordinates, no typed payload, animations intact across the smart cut. Attach screenshots/video to the PR.
9. **Presubmit**: `./script/format` and `cargo clippy` (per `AGENTS.md`) pass before opening/updating the PR.
