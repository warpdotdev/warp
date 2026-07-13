# Computer-Use Recording Action Overlays — Tech Spec
Product spec: `specs/recording-overlay/PRODUCT.md`

All `warp` code references are pinned to commit `b7430f40a9ef73a534f97bbc815944ebf17eedf8` (branch `varoon/va-blocklist-ui`, where the recording code currently lives). This feature **depends on and lands after** the recording-finalization change (`specs/recording-finalization/`); it fills in that spec's burn-in hook (its TECH.md §2 step 3).

## Context
Computer-use recording is a single-pass ffmpeg `x11grab` capture streamed straight to an on-disk mp4; the live handle is held in a runtime-global singleton and finalization (stop → upload) happens at the executor layer. Today nothing correlates the input actions the agent performs with the recording, and nothing composites anything onto the video. This feature (1) collects a timecoded action log while a recording is live and (2) burns compact bottom-center pill labels into the mp4 at finalize time, before upload. See `PRODUCT.md` for user-visible behavior.

### Relevant code
- [`crates/ai/src/agent/action/convert.rs:489 @ b7430f40a`](https://github.com/warpdotdev/warp/blob/b7430f40a9ef73a534f97bbc815944ebf17eedf8/crates/ai/src/agent/action/convert.rs#L489) — `UseComputer` → `UseComputerRequest` conversion; `action_summary` (server-authored) and the structured `actions: Vec<computer_use::TargetedAction>` are both carried onto the request.
- [`app/src/ai/blocklist/action_model/execute/use_computer.rs (34-68) @ b7430f40a`](https://github.com/warpdotdev/warp/blob/b7430f40a9ef73a534f97bbc815944ebf17eedf8/app/src/ai/blocklist/action_model/execute/use_computer.rs#L34-L68) — `UseComputerExecutor::execute`; clones `request.actions` (line 44) and dispatches the actor. `ctx: &mut ModelContext<Self>` is available (currently unused) — the collection site.
- [`app/src/ai/blocklist/action_model/recording_controller.rs (26-90) @ b7430f40a`](https://github.com/warpdotdev/warp/blob/b7430f40a9ef73a534f97bbc815944ebf17eedf8/app/src/ai/blocklist/action_model/recording_controller.rs#L26-L90) — `ActiveRecording { id, handle }` (28-31), `finish_start` (60-66), `take_handle_or_err` (76-89). No start timestamp, no action log.
- [`crates/computer_use/src/lib.rs (196-327) @ b7430f40a`](https://github.com/warpdotdev/warp/blob/b7430f40a9ef73a534f97bbc815944ebf17eedf8/crates/computer_use/src/lib.rs#L196-L327) — `create_recorder` with a mock backend gated on `test-util` **or** `debug + WARP_COMPUTER_USE_MOCK_RECORDER` (196-204); `Recorder` trait (211-221); `RecordingHandle` (mock + linux `path/started_at/process`; `width()/height()` getters only, `started_at` private/linux-only, 249-308); `RecordingOutput`/`RecordingCompletionStatus` (312-327).
- [`crates/computer_use/src/linux/recording.rs (38-173) @ b7430f40a`](https://github.com/warpdotdev/warp/blob/b7430f40a9ef73a534f97bbc815944ebf17eedf8/crates/computer_use/src/linux/recording.rs#L38-L173) — the ffmpeg `x11grab` spawn/stop; the module that already shells out to `ffmpeg` and is the natural home for `burn_in_action_log`.
- `specs/recording-finalization/TECH.md` §2 (`finalize_recording`, and **step 3 the burn-in hook**) — the single call site this feature plugs into, before `FileArtifactUploader` upload.
- `warp-server` `logic/ai/multi_agent/agents/computer_use/tools/anthropic_computer_use.go` — builds `UseComputer` with the per-action `action_summary` strings this feature maps to labels (`Key "…"` at the `key` builder, `Type "…"` at `type`, `Scroll <dir> …` at `scroll`, click/move/wait/screenshot/cursor/zoom summaries). Source of the label text for the Key case.
- `warp-agent-docker-video` `xvfb-sidecar/Dockerfile` (24-30) — installs `xvfb`, `xfonts-base` (bitmap X fonts), and `ffmpeg`. **No scalable TTF/OTF** — a hard gap for libass text rendering (see §5).

## Proposed changes

### 1. Action-log data model (`computer_use` crate)
Define the log entry in the `computer_use` crate so the renderer can consume it directly:
```rust
pub enum OverlayKind { Key, Type }
pub struct ActionLogEntry {
    pub offset: Duration,        // time from capture-live to this action
    pub kind: OverlayKind,
    pub label: String,           // "ctrl+a" (Key) or "typing…" (Type)
    pub show_duration: Duration, // fixed default (~1.5s); clamped at render time
}
```

### 2. Collect the log while a recording is live (app layer)
- Add `started_at: Instant` and `actions: Vec<ActionLogEntry>` to `ActiveRecording` (`recording_controller.rs:28`), set `started_at` at `finish_start` (`recording_controller.rs:60`). This is required because `RecordingHandle.started_at` is private and linux-only (`lib.rs:260`) — unreachable from the app layer where offsets must be computed and cross-platform for the mock path.
- Add `RecordingController::record_action(entry)` that pushes only when a recording is `Active` (no-op otherwise).
- In `UseComputerExecutor::execute` (`use_computer.rs:34`), before spawning the actor, reach the `RecordingController` via `ctx`; if a recording is active, derive `(kind, label)` from `request.actions` (+ `request.action_summary` for the Key label), compute `offset = started_at.elapsed()`, and `record_action`. One entry per `UseComputer` call; omitted kinds produce no entry. This is synchronous and additive; the async actor path is unchanged.
- Drain the entries alongside the handle at finalize: extend the finalization claim introduced by #2 (`begin_finalize`/`take_handle_or_err`) to return `(RecordingHandle, Vec<ActionLogEntry>)`.

#### Action-kind switch (data-driven on `request.actions`; label from `action_summary`)
Guiding principle: **annotate only input that isn't visible on screen.** Keyboard input (`Key`, `Type`) is invisible and rendered; everything pointer/cursor/scroll-driven produces visible on-screen motion and is omitted. Derive **kind** from the structured actions (authoritative and redaction-safe); use the summary only for the Key combo text. Never render the typed payload.

| Server `action_summary` (`anthropic_computer_use.go`) | Structured `computer_use::Action` | Kind | Label | Render |
| --- | --- | --- | --- | --- |
| `Key "<combo>"` | `KeyDown`/`KeyUp` | `Key` | `<combo>` (strip `Key "`…`"`) | yes |
| `Type "<text>"` | `TypeText` | `Type` | `typing…` (payload dropped) | yes |
| `… click at …`, `Left mouse down/up …` | `MouseDown`/`MouseUp` | — | — | no (cursor visible) |
| `Mouse moved to …` | `MouseMove` | — | — | no (cursor visible) |
| `Scroll <dir> …` | `MouseWheel` | — | — | no (visible content motion) |
| `Left click drag from … to …` | `MouseDown`+`MouseMove`+`MouseUp` | — | — | no (cursor visible) |
| `Wait …`, `Screenshot`, `Cursor position`, `Zoom …` | `Wait(0)` | — | — | no (no-op/meta) |

### 3. Render: `.ass` burn-in via a post-stop re-encode pass
Add `computer_use::burn_in_action_log(input: &Path, entries: &[ActionLogEntry], capture: (u32,u32)) -> Result<PathBuf, RecordingError>` in `linux/recording.rs` (noop/mock backends return the input path unchanged). It:
1. Generates an `.ass` subtitle file from `entries`: one `Dialogue` per entry, `Start = offset`, `End = min(offset + show_duration, next.offset)` (PRODUCT invariant 4), text = `label` (ASS-escaped). `PlayResX/Y` = capture width/height (`RecordingHandle::width()/height()`), so positioning matches the frame.
2. Runs `ffmpeg -y -i <input.mp4> -vf "subtitles=<overlay.ass>" -c:v libx264 -preset ultrafast -pix_fmt yuv420p -movflags +faststart <input.overlay.mp4>`. This **demuxes the on-disk mp4 frame-by-frame and never buffers the whole recording in memory** (matches the finalization design's no-buffering rule). libass is present in stock apt ffmpeg — no `libzmq` / custom build needed.
3. Returns the overlay path on success; on any error, returns `Err` and the caller falls back to the original (PRODUCT invariant 11).

Bottom-center pill style — concrete example (`ctrl+a` at 3.0–4.5s, then `typing…`):
```
[Script Info]
ScriptType: v4.00+
PlayResX: 1920
PlayResY: 1080
ScaledBorderAndShadow: yes

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Pill,DejaVu Sans,48,&H00FFFFFF,&H000000FF,&H00000000,&HB0000000,-1,0,0,0,100,100,0,0,3,16,0,2,40,40,90,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:03.00,0:00:04.50,Pill,,0,0,0,,ctrl+a
Dialogue: 0,0:00:05.20,0:00:06.70,Pill,,0,0,0,,typing…
```
`Alignment=2` = bottom-center; `MarginV=90` lifts it off the edge; `BorderStyle=3` + `BackColour=&HB0000000` = semi-transparent dark pill (ASS alpha inverted: `00`=opaque, `FF`=clear); `Outline=16` sets box padding.

### 4. Wire into #2's centralized finalize (burn-in hook)
At the burn-in hook in `finalize_recording` (recording-finalization TECH §2 step 3), after `recorder.stop()` yields `RecordingOutput { path, width, height, .. }` and before upload:
1. Take the drained `Vec<ActionLogEntry>` (§2).
2. If non-empty, call `burn_in_action_log(&output.path, &entries, (output.width, output.height))`.
3. On `Ok(overlay_path)`, upload `overlay_path`; on `Err`, log at `warn` and upload the original `output.path` (best-effort; invariant 11).
4. #2's unconditional temp-file cleanup extends to remove the overlay file too.
Because burn-in sits inside the single finalize path, it covers every terminal cause (StoppedByAgent, AgentFinished, LimitReached, FfmpegExited) uniformly (PRODUCT invariant 13); a `StoppedEarly` recording is annotated with whatever entries were logged.

### 5. Required infra: scalable font for libass (`warp-agent-docker-video`)
libass renders **no text** from `xfonts-base` (bitmap) alone. Add a scalable font — `fonts-dejavu-core` (+ `fontconfig`, usually pulled in by ffmpeg) — to whichever image runs the burn-in ffmpeg (the `xvfb-sidecar` image, and the agent image if `ffmpeg` executes there). If fontconfig can't resolve the family, pass `subtitles=<file>:fontsdir=<dir>` or `:force_style='FontName=DejaVu Sans'`. This is a **hard dependency**: without it burn-in produces empty pills (and per invariant 11 still publishes the original video, so the failure mode is "no labels," not "no video"). Also confirm `ffmpeg` is on `PATH` for the client process that runs the recorder/burn-in.

### 6. Redaction
`action_summary` and `TypeText.text` are `(sensitive)=true`. The `Type` case drops the payload and renders `typing…`; only key combos and the generic `typing…` indicator are rendered. Burned-in text is user-visible in the artifact by design, but `ActionLogEntry` values must never be written to non-artifact logs (no `log::*` of labels/summaries).

## Testing and validation
### Pure unit tests (no recorder, no ffmpeg)
- Switch mapping (PRODUCT 5–7): table-driven over representative `UseComputer` requests (structured actions + summary) asserting `(kind,label)` or omission; explicitly assert `Type` → `typing…` and never the payload (invariant 10); and that clicks, mouse-moves, scroll, drag, waits, screenshot, cursor-position, and zoom all produce no entry.
- Clamp/overlap logic (invariant 4): entries closer than `show_duration` ⇒ earlier `End` clamped to next `Start`; single pill at a time.
- `.ass` generation: `Dialogue` timecode formatting (`H:MM:SS.cs`), ASS escaping of labels, style/`PlayRes` from capture dims, empty-entry list ⇒ no burn-in (invariant 8).

### Mock-recorder path (local, cross-platform)
`create_recorder` already returns a mock under `test-util` or `debug + WARP_COMPUTER_USE_MOCK_RECORDER` (`lib.rs:196-204`). Extend the mock to emit a real minimal playable mp4 so `burn_in_action_log` can be exercised on macOS; assert the overlay mp4 is produced when ffmpeg+font are available and that a missing ffmpeg/font degrades to the original path without failing (invariant 11). Collection tests (record_action gating, offsets monotonic from `started_at`) run against the controller directly.

### Real-capture e2e (oz-local + custom Docker with the font added)
Real `x11grab` only runs on Linux. With a `warp-agent-docker-video` image that includes `fonts-dejavu-core`, run a computer-use flow that issues key + type actions (plus some clicks/scroll), let #2 finalize publish the artifact, pull the mp4, and eyeball bottom-center pills at the right timecodes; confirm pointer/scroll-only flows have no pills. The overlay is **not inspectable headlessly** — verification requires downloading the artifact. Also verify a mixed flow shows only keyboard pills (invariant 6).

## Parallelization
Not beneficial. This is a small, tightly-coupled slice (log struct + collection + `.ass`/burn-in + one finalize hook + a font line in the Docker image) that must land after #2 exists to hook `finalize_recording`. Recommend a single agent on `varoon/recording-overlay`, worktree branched from the #2 branch **after it merges**; one client PR plus a small companion `warp-agent-docker-video` PR for the font.

## Risks and mitigations
- **Font dependency (hard).** No scalable font ⇒ empty pills. Mitigation: ship §5 in the same rollout; burn-in failure/absence still publishes the original video (invariant 11). Add a startup/log check that a usable font resolves.
- **Re-encode cost at finalize.** One extra encode proportional to duration; use `-preset ultrafast` (matching capture) and keep it inside #2's teardown-drain budget so it can't wedge run exit.
- **`action_summary` parsing brittleness (Key label).** Kind is derived from structured actions (stable); only the Key combo text is parsed from the summary. If the summary shape changes, the label degrades gracefully but the kind classification does not break.
- **Mock mp4 realism.** The mock must emit an mp4 ffmpeg can re-encode for the local burn-in test; otherwise the test only covers the no-op fallback.
- **Rapid action bursts.** Aggressive clamping can make some pills very brief; acceptable and matches "newest wins" (invariant 4).

## Follow-ups
- Pointer/scroll annotations (click pulses, drag trails, scroll indicators, cursor highlight) — intentionally omitted in V0 (visible on screen); could be revisited if product wants them, and would need coordinate capture and a compositor beyond libass text.
- Single-pass live burn-in via `drawtext textfile=…:reload=1` — a fallback only if a no-re-encode constraint later arises; rejected for V0 (fragile app↔ffmpeg file coupling, atomic-write races). A **live X11 overlay on the Xvfb display is rejected outright**: perception screenshots read the same `$DISPLAY` root the recorder grabs, so it would corrupt model input.
- Recording-usage/finalization metrics — separate PR (out of scope).
