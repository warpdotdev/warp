# Spec: Auto-zoom for computer-use video recordings (QUALITY-auto-zoom)

Linear: TBD (auto-zoom follow-on to the recording stack) · Repository: `warpdotdev/warp` (client)
Base branch: `master`. Builds directly on the merged smart-cut (QUALITY-1112) and click/drag annotation (QUALITY-1169 + follow-up) work. There is no `warp-server` change.

## Scope note
This spec adds a **virtual camera** to published computer-use recordings: the video zooms into where the agent interacts, pans to follow the cursor, and eases back out during idle moments (the Devin/Screen Studio behavior). Constraints, chosen to mirror the two predecessor specs:
- **Linux only.** Post-stop processing exists only on the Linux x11grab path (`post_process_recording`). macOS still applies no post-stop processing (it uses the avfoundation live-capture path and `post_process_recording` returns the input unchanged in `lib.rs`), so auto-zoom is not added there. Adding a macOS post-process pipeline is explicitly out of scope.
- **Reuse the existing signal.** The camera is driven entirely by the `pointer_events` already recorded per `ActionLogEntry` for click/drag annotations (capture-space pixels + source-timeline offsets). No new capture, no server field, no new perception data.
- **Post-stop, after the cut and overlays.** Auto-zoom is a new best-effort pass appended to the existing disk-to-disk ffmpeg pipeline, operating on the already-cut, already-annotated video so click rings/pills ride inside the zoom and the existing source→output timeline remap is reused.
- **Feature-flagged and off by default** until the camera model is tuned, because motion feel is subjective and the master is captured at 1x physical pixels (zooming upscales).

## == PRODUCT ==
**Summary:** Published Linux computer-use recordings gain an automatic, deterministic camera that zooms and pans toward agent interactions and eases back to full-frame during idle time. The transform is client-side in the existing post-stop finalization path; the live display, the agent's screenshots, and all model-perception frames are byte-identical to today. Output geometry (dimensions, duration, frame count, frame cadence) is identical to the un-zoomed recording; only the framing changes.

**Key design choices:**
1. **Camera driven by recorded pointer events.** The client already records `PointerEvent { offset, kind, button, point }` in capture-space pixels on the source/1x timeline for every meaningful `UseComputer` group. Those points, remapped onto the compacted (post-cut) output timeline with the existing `remap_source_interval`, are the camera's target signal. No new input capture.
2. **Camera path computed deterministically in Rust, then rendered by ffmpeg.** A pure function turns the pointer stream into a per-output-frame `(zoom, center_x, center_y)` track using a bounded, eased/damped model (idle → zoom-in on activity → follow cursor → ease back out). ffmpeg only renders the track; all policy lives in testable Rust.
3. **Rendered with ffmpeg `zoompan`.** Prototyping (see Prototype evidence) showed `zoompan` correctly centers and clamps the viewport for every keyframe including frame-edge targets, whereas a naive per-frame `scale(eval=frame)+crop` mis-framed targets at the clamp boundary. `zoompan` is a single extra disk-to-disk pass at the same cost as the existing passes.
4. **Ordered after cut + overlay.** Auto-zoom runs on the already-cut, already-annotated video. Annotations (click rings, drag trails, pills) therefore scale and translate with the content and stay attached to their targets, and no annotation coordinate transform is needed. The camera track is computed on the output timeline the cut already defines.
5. **Best-effort, reuse the fallback contract.** Any failure (degenerate camera track, ffmpeg/zoompan error) logs a non-sensitive warning and publishes the un-zoomed (but still cut + annotated) video. Auto-zoom never blocks publication.
6. **Feature-flagged, Linux-only, off by default.** Gated so it can be dogfooded and tuned before rollout; macOS and every other platform are unchanged.

**Behavior** (numbered, testable invariants from the viewer's perspective):
1. With auto-zoom disabled, the published artifact is byte-for-byte the current cut+annotated recording (pure no-op / pass-through). Enabling it only changes framing.
2. Output geometry is invariant: the auto-zoomed video has the same width, height, frame rate, frame count, and total duration as the un-zoomed cut recording it was produced from. The camera never changes playback speed, drops, or duplicates frames.
3. While there is no recent interaction, the camera is at rest: 1x zoom, full frame, centered. The recording opens and closes at 1x full-frame.
4. When an interaction cluster begins, the camera eases in to a bounded zoom factor `Z` (default target 2.0x, hard max configurable) centered on the cluster's interaction location, over a smooth ease-in (no instantaneous jumps).
5. While interacting, the camera center tracks the cursor with damped motion (no per-event jitter): rapid small cursor moves do not cause the frame to shake, and a deliberate move to a new region is followed smoothly.
6. After interaction stops for an idle threshold (default ~1.2 s), the camera eases back to 1x full-frame over a smooth ease-out.
7. The camera viewport is always fully inside the frame: at any zoom it is clamped so no out-of-bounds (black/edge) pixels are ever shown, including when the interaction is at or beyond the frame edge. A target that cannot be centered (near an edge) is framed as close as the clamp allows while keeping the interaction visible.
8. Zoom is bounded to `[1.0, Z_max]` (default `Z_max = 2.5`) and the camera's translational and zoom velocity are bounded, so motion is always smooth and bounded regardless of the pointer data.
9. Burned-in annotations remain correctly attached: a click ring/drag trail/pill drawn on a target stays on that target through the zoom and pan (they are composited before the zoom and transform with the content).
10. The camera operates on the compacted (post-cut) timeline: because the smart cut already removed idle/thinking gaps, camera keyframes are computed from pointer offsets remapped through the same segment map, so the camera is aligned with the retained footage and never references removed frames.
11. The transform is deterministic: identical `(pointer_events, segments, dimensions, frame_rate, config)` inputs always produce an identical camera track and identical ffmpeg invocation.
12. Degenerate inputs are safe: a recording with no pointer events, a single interaction, or pointer data that would produce a degenerate/empty track yields either a valid at-rest (no-zoom) camera or a clean pass-through of the un-zoomed video — never a crash or an out-of-bounds frame.
13. Failure is best-effort: if the auto-zoom pass fails for any reason after a valid cut+annotated video exists, the un-zoomed cut+annotated video is published, temporary intermediates are cleaned up, and the warning contains no coordinates or typed text.
14. No live/perception/API change: the on-screen app during capture, the agent's screenshots and action results, capture metadata (`RecordingOutput`/`StopRecordingResult` duration/size), and all server/wire contracts are unchanged. Typed payloads never appear (unchanged redaction).
15. Platform scope: auto-zoom applies only to the Linux real-recording finalization paths (agent stop, agent-finished, duration/size limit, ffmpeg early exit). macOS and no-op/mock/test-util recorders are unaffected and never panic.

## == TECH ==

### Context (pinned to `af6fc40d476d428637b265da60bf584512e4e1d4`)
- `crates/computer_use/src/linux/recording.rs:483-520` — `post_process_recording`: the post-stop pipeline. Today it runs `cut_to_segments` then writes ASS and runs `burn_overlays_into_cut`. This is where the new zoom pass is appended.
- `crates/computer_use/src/linux/recording.rs:377-423` — `cut_to_segments` (trim/`setpts=PTS-STARTPTS`/`concat`), and `:430-468` — `burn_overlays_into_cut` (`subtitles` ASS burn-in). Both are independent single-purpose disk-to-disk ffmpeg passes; the zoom pass follows the same shape.
- `crates/computer_use/src/linux/recording.rs:529-548` — `build_cut_only_filtergraph`: the reference for programmatically constructing an ffmpeg filtergraph string from Rust.
- `crates/computer_use/src/overlay.rs:17-30` — `ActionLogEntry { offset, finish_offset, labels, pointer_events }`; `:37-52` — `PointerEvent { offset, kind, button, point }` / `PointerEventKind`; `:279-285` — `KeepSegment { source_start, source_end, output_start }`; `:353-399` — `build_keep_segments`; `:410-436` — `remap_source_interval`; `:447-530` — `build_overlay_ass`. The camera reuses `build_keep_segments`/`remap_source_interval` and the `pointer_events` stream (already flattened + stable-sorted recording-wide by `append_recording_pointer_dialogues`, `:657-682`).
- `crates/computer_use/src/lib.rs:287-303` — `post_process_recording` cross-platform shim (`#[cfg(all(linux, not(noop)))]` real, else returns input unchanged); `:336-370` — `RecordingConfig` (`frame_rate`, `max_duration`, `max_size_bytes`, `playback_speed_multiplier`, `target`).
- `app/src/ai/blocklist/action_model/recording_finalize.rs:63-171` — `finalize_recording`: stops capture, calls `computer_use::post_process_recording` best-effort (`:109-128`), uploads, cleans up. Auto-zoom needs no change here beyond passing config/flag through; the extra pass is internal to `post_process_recording`.
- `crates/computer_use/src/mac/recording.rs:204-237` — macOS avfoundation command; unchanged (no post-process path).

### Prototype evidence (macOS ffmpeg 8.1.2; scripts/artifacts under `/tmp/az`)
A synthetic 1280x720@30fps "screen recording" was generated with a grid and a red cursor box that jumps between known points (center → top-left → bottom-right → center). A camera track was generated **programmatically from the cursor keyframes with smoothstep easing** — the same pattern the Rust code will use — and applied two ways:
- **`scale(eval=frame)+crop`**: encoded the 9 s clip in ~0.23 s; correct geometry (1280x720, 270 frames). Tracked the idle→zoom-in on the top-left target and mid-pan correctly, but the **bottom-right target (at the clamp boundary) was mis-framed** (target left the viewport). `crop` cannot vary its size per frame, and the dynamic per-frame `scale` size + `crop` interaction breaks down at the clamp edges.
- **`zoompan`** (using its own `time`/`zoom` variables): encoded in ~0.19 s; correct geometry (1280x720, 270 frames). **Centered and clamped every target correctly, including the frame-edge target.** Rejected the `t`-based expression syntax initially — it requires `zoompan`'s `time` variable, not the generic `t`.

Conclusions carried into this spec: (a) post-processing cost is negligible relative to the existing passes; (b) a Rust-generated eased keyframe track renders correctly; (c) **`zoompan` is the mechanism** because it handles centering + clamping (including edges) internally, where dynamic `scale+crop` does not. Caveat: `zoompan` is historically prone to integer-rounding jitter on slow zooms — mitigated by computing a smooth, bounded-velocity track and, if needed, rendering at a modest zoom-space supersample before the final downscale.

### Data model
No new capture types are required — the camera consumes the existing `PointerEvent` stream. Add a camera module under `crates/computer_use/src/` (for example `overlay_camera.rs` or a `camera` submodule), gated `#[cfg(any(linux, test))]` like the other post-stop code:
- `CameraKeyframe { t: Duration, zoom: f32, cx: f32, cy: f32 }` — a sample on the **output** timeline.
- `CameraTrack(Vec<CameraKeyframe>)` — the full per-considered-time track (sampled at `frame_rate`, or at keyframe boundaries if emitting expressions).
- `CameraConfig { enabled: bool, target_zoom: f32, max_zoom: f32, zoom_in: Duration, zoom_out: Duration, idle_timeout: Duration, follow_stiffness: f32 }` with defaults matching the behavior section. Derive from `RecordingConfig` (see Config).

### Camera model (pure, unit-tested)
A pure function `build_camera_track(entries, segments, dimensions, frame_rate, config) -> CameraTrack`:
1. Flatten all `pointer_events` across committed entries (reuse the existing recording-wide flatten + stable sort by offset), and remap each event offset from source to output time via `remap_source_interval`/`build_keep_segments`. Events wholly in removed gaps are dropped.
2. Segment the output timeline into **active** windows (spans with interaction activity, extended by `idle_timeout`) and **idle** windows. Merge nearby active windows so brief pauses don't cause a full zoom-out/zoom-in oscillation.
3. For each active window, compute a target center from its events (e.g. a smoothed centroid / most-recent-cluster point) and a target zoom (`target_zoom`, reduced if the cluster's spatial spread would push content out of frame). Idle windows target `zoom = 1.0`, center = frame center.
4. Produce a continuous track by easing between targets: smoothstep for zoom in/out envelopes and a damped follow (critically-damped spring or capped-velocity lerp) for the center while active, sampled per output frame.
5. Clamp every sample so the viewport `[cx - W/(2Z), cx + W/(2Z)] × ...` lies fully within `[0,W]×[0,H]`; clamp `zoom ∈ [1, max_zoom]`; enforce bounded per-frame center/zoom deltas.
The model must be deterministic and total (never panics; always yields an in-bounds track).

### ffmpeg mechanism (rendering the track)
Append a third pass `zoom_cut(input, track, frame_rate) -> PathBuf` to `post_process_recording`, mirroring `cut_to_segments`/`burn_overlays_into_cut` (single `-i`, one video filter, `libx264 -preset ultrafast -pix_fmt yuv420p -movflags +faststart`, `-r frame_rate`, disk-to-disk, best-effort). Render the track via `zoompan`:
- Emit `z`, `x`, `y` as expressions in `zoompan`'s variable space (`time`, `zoom`, `iw`, `ih`), constructed from the track exactly like `build_cut_only_filtergraph` builds its graph string. `x = cx - (iw/zoom)/2`, `y = cy - (ih/zoom)/2`, with `z` the eased zoom envelope; all clamped in-expression as a defense-in-depth backstop to the Rust clamp. Set `d=1:fps=frame_rate:s=WxH`.
- Because `zoompan` expressions grow with keyframe count, prefer a compact piecewise-smoothstep expression per segment (validated in the prototype). If expression length becomes unwieldy for long recordings, fall back to driving `zoompan`/`crop` params via a generated `sendcmd` script (also available in this ffmpeg build) — keep this as an implementation option, not a required path.
- Note the `zoompan` jitter caveat; if visible, render zoom-space at a small supersample (e.g. 1.5x) then `scale` down.

### Pass ordering
`cut_to_segments` → `burn_overlays_into_cut` → **`zoom_cut`** (new). Rationale: zooming the already-annotated video means click rings/pills/trails scale and translate with their targets and need no coordinate transform; the camera track is computed on the same post-cut output timeline the overlays were remapped onto. This adds one more `ultrafast` re-encode; the prototype shows per-pass cost is a fraction of a second, but the implementer should confirm on representative Linux captures. Folding zoom into the overlay pass (single `-filter_complex`) is a valid later optimization but is not required for correctness.

### Config & feature flag
- Add a `FeatureFlag::ComputerUseAutoZoom` (per `AGENTS.md` feature-flag guidance) and gate the new pass on it; default off (not in `RELEASE_FLAGS`), optionally in `DOGFOOD_FLAGS` for dogfooding. Add the matching Command Palette enable/disable entry per `AGENTS.md`.
- Thread an `auto_zoom` toggle (and optional `max_zoom`) through `RecordingConfig` so the server can gate/tune it later, defaulting to disabled for wire compatibility; when unset, behavior is today's (no zoom).
- `post_process_recording` reads the flag/config; when disabled it skips the zoom pass entirely (invariant 1).

### Platform
Keep the existing `#[cfg(all(linux, not(noop)))]` gating: the camera module and `zoom_cut` are Linux+test only; the `lib.rs` shim continues to return the input unchanged on macOS/other/noop. No `mac/recording.rs` change.

### Design alternatives
- **`scale(eval=frame)+crop`**: rejected — prototype mis-framed clamp-boundary targets; `crop` can't vary size per frame and the dynamic-scale interaction is fragile at edges.
- **`zoompan`**: selected — handles centering + clamping (incl. edges) correctly, same cost, accepts eased `time`-based expressions.
- **Per-frame CPU compositing in Rust** (crop+resample each RGBA frame like the integration-test overlay renderer): rejected as the primary path — reinvents a scaler, is slower, and duplicates ffmpeg; retain as a conceptual fallback only.
- **Live zoom during capture**: rejected — the camera needs the whole interaction timeline (future events) to ease sensibly, would fight the post-stop cut, and can't be tuned/replayed deterministically.
- **Server- or player-side zoom**: rejected for the same reasons as the predecessor specs — the artifact must be correct when downloaded and viewed in any player, and the client already owns the pointer stream + pipeline.

### Risks / mitigations
- **Upscaling sharpness**: the master is captured at 1x physical pixels, so zoom softens the image. Acceptable for a sanity-check video; note it, and consider higher-res or window-scoped capture as a separate follow-on if fidelity is insufficient.
- **`zoompan` jitter**: mitigate with a smooth bounded-velocity track and optional zoom-space supersample.
- **Camera feel is subjective**: keep all policy in the pure Rust model behind config/flag; tune via dogfood before rollout.
- **Interaction with the cut**: always compute keyframes on the remapped output timeline; add tests with removed gaps.
- **macOS parity gap**: documented and intentional; auto-zoom is Linux-only until macOS gains a post-process path.

### Open questions
- Exact defaults for `target_zoom`, `max_zoom`, `idle_timeout`, ease durations, and follow stiffness — to be set during dogfood tuning; start from the behavior-section defaults.
- Whether to cap total zoomed time or force periodic full-frame "establishing" moments for readability — deferred; not required for v1.

## Validation & verification criteria (all must pass before merge)
1. **Camera-model unit tests** (`cargo nextest run -p computer_use`) on synthetic pointer streams: assert (a) idle → 1x full-frame at start/end; (b) ease-in to `target_zoom` centered on an interaction; (c) damped follow (bounded per-frame center delta) across a cursor move; (d) ease-out to 1x after `idle_timeout`; (e) viewport always in-bounds for edge/corner targets; (f) `zoom ∈ [1, max_zoom]`. Each is a named regression test that fails before the model exists.
2. **Timeline-remap test**: with ≥2 retained segments and a removed gap, assert camera keyframes are computed on the output timeline (an interaction after the gap shifts left by the removed duration; nothing references removed frames).
3. **Determinism test**: identical inputs produce an identical `CameraTrack` and identical generated `zoompan` filter string.
4. **Degenerate-input tests**: no pointer events, a single event, and all-events-in-removed-gaps each yield an at-rest track or a clean pass-through, never an out-of-bounds sample or panic.
5. **ffmpeg filter-string test**: assert the generated `zoompan` expression uses `time`/`zoom`/`iw`/`ih` (not `t`), clamps x/y/z, and sets `d=1:fps=<rate>:s=<W>x<H>`; assert the pass is skipped entirely when the flag/config is disabled (invariant 1 pass-through).
6. **Fixture render test** (like the `/tmp/az` prototype, using a deterministic synthetic input): run the real `zoom_cut` pass and assert output geometry equals input (W, H, frame count, duration) and that at chosen frame indices the interaction target is within a tolerance of frame center while zoomed, and full-frame while idle.
7. **Ordering / annotation-survival test**: with a click annotation present, assert the zoom pass runs after overlay burn-in and the annotated target remains centered when zoomed (frame inspection on the fixture).
8. **Fallback / cleanup test**: an induced zoom-pass failure still publishes the cut+annotated video, removes all intermediates, and logs a warning containing no coordinates/typed text.
9. **Regression**: full `computer_use` suite + recording controller/finalize suites stay green; smart cut, annotations, redaction, cursor capture, and upload behavior unchanged; a representative screenshot/action result is byte-identical with recording enabled.
10. **Platform**: Linux applies the flagged zoom pass; macOS and no-op/mock recorders return the input unchanged without panicking.
11. **Real Linux artifact verification** (deferred to PR review): record a multi-click, multi-region computer-use session on Linux/X11 with the flag on, download the published artifact, and confirm via frame inspection: opens/closes at full-frame, zooms and centers on each interaction, follows the cursor smoothly, eases out on idle, annotations stay attached, and geometry matches the un-zoomed recording. Attach visual evidence to the PR; do not commit media.
12. **Presubmit**: `./script/format` and `cargo clippy` (per `AGENTS.md`) pass before opening/updating the PR.
