# QUALITY-1169 follow-up: progressive drag trail (draw the line as it's dragged)

Repository: `warpdotdev/warp` (client). Linux burn-in only. No server/API change.
This continues the click/drag annotation work (`agents/specs/QUALITY-1169-richer-click-drag-annotations.md`, `.agents/specs/QUALITY-1169-click-drag-follow-up.md`).

## Problem
The burned-in drag annotation reads as unnatural. Concretely:
1. The **entire** drag path is drawn the instant the press happens, before the drag "moves". You see the whole future line up front.
2. A filled orange dot (the held indicator) then travels along that already-complete line.
3. The real OS cursor appears to vanish and be replaced by the animation.

The desired behavior: the trail should be **drawn progressively as the drag proceeds** — the line grows behind the moving cursor dot — rather than appearing all at once. The real cursor should remain.

## Root cause / key constraint (read this first)
The agent's drag is effectively a **teleport**, not a continuous gesture. The x11 actor dispatches each `MouseMove` as an instantaneous `screen_mouse.move_to(*to)` with no interpolation and records exactly one `PointerEvent` per action (`crates/computer_use/src/linux/x11/mod.rs (248-345)`). All of a drag's `Down`/`Move`/`Up` events therefore land within a few milliseconds of each other, and the real cursor jumps straight to the endpoint.

Consequences the implementer must account for:
- Pacing the reveal off the raw pointer-event offsets would reveal the whole line in a few ms — no visible improvement. The animation must be paced over a **synthetic duration**, the same way the click ring / drag fade durations are derived from the retained cut margin today (`click_ring_duration`, `drag_trail_fade_duration` in `crates/computer_use/src/overlay.rs (263-271)`).
- The "real cursor disappears" report is a side effect, not intentional: `-draw_mouse 1` stays on (invariant 9 of the base spec), so the OS cursor is still captured — it just teleports to the endpoint while the large synthetic dot dominates attention. Do **not** hide the OS cursor.

## Current rendering (what to change)
All in `crates/computer_use/src/overlay.rs`, function `append_drag` (~727-797):
- **Trail** (~762-771): one static `Cursor` dialogue containing the full `ass_trail_quads` polyline, shown across the whole `[press, release+fade]` interval. This is the "entire path up front" behavior.
- **Start anchor** (~772-779): filled circle at the press point.
- **Held indicator** (~782-796): a filled dot that `\move`s in a straight line from the **first** point to the **last** point, ignoring intermediate points, at constant velocity.
- Trail quads are built by `ass_trail_quads` (~823-852). Recording-level gesture classification is in `classify_pointer_gestures`; drags are dispatched from `append_recording_pointer_dialogues` (~658).

## Proposed change (stay in ASS/libass; no new pipeline)
Rework `append_drag` so the trail is revealed segment-by-segment in step with the moving dot, over a bounded synthetic duration.

1. **Introduce a synthetic drag-draw duration.** Pace the whole drag animation over a fixed, visible duration bounded by the retained post-action margin (mirror the existing `min(design, SEGMENT_MARGIN_POST - headroom)` pattern so it always survives the smart cut). Suggested target ~500-700 ms; the exact value is a tuning detail. This duration drives both the dot travel and the trail reveal, decoupling the animation from the near-zero real dispatch time.
2. **Progressive trail reveal.** Instead of one polyline dialogue, emit the trail as per-segment quad dialogues (or an equivalent staggered scheme). Each segment becomes visible when the dot reaches it: stagger each segment's `Start` timecode across the synthetic duration, distributed by cumulative path length so the reveal speed is uniform along the path. Once fully revealed, keep the complete trail, then fade over the existing `drag_trail_fade_duration` after release.
3. **Make the dot follow the actual path.** Replace the single straight-line `\move(first,last)` with piecewise `\move`s along consecutive points (sequential timing over the synthetic duration) so the dot stays on the line it is drawing.
4. **Keep everything else identical.** Real cursor stays (`-draw_mouse 1`); start anchor, orange color/opacity constants, `\an7` centering, `\clip` to frame, and `remap_source_interval` smart-cut remapping are unchanged. A drag still never emits a click ring.

Held press with no release (incomplete drag): keep current bounded behavior — reveal up to the last known point and hold until the retained window ends; no synthesized release.

## Explicitly out of scope (do NOT do here)
The bigger "Screen Studio-grade" ideas were considered and deferred:
- Interpolating the drag at dispatch time so the real cursor glides.
- A per-frame RGBA/synthetic-cursor compositor, spring physics, motion blur, or hiding the OS cursor.
Keep this change confined to the ASS renderer in `overlay.rs`.

## Testing
- `cargo nextest run -p computer_use`. Extend `crates/computer_use/src/overlay_tests.rs` sparingly:
  - A multi-segment drag emits **multiple** staggered trail dialogues (not one), with later segments starting later, and the full trail present by release.
  - The post-release fade still applies; a drag still emits zero click rings; geometry stays clipped to the frame and centered (`\an7`).
- Real Linux artifact check (per base spec's deferred verification): record a drag, download the mp4, confirm the line draws in behind the dot rather than appearing whole.
- Run `./script/format` and `cargo clippy` (per `AGENTS.md`) before the PR.
