# Computer-Use Recording Action Overlays — Product Spec

## Summary
Burn compact, human-readable labels for keyboard input (keypresses/shortcuts and typing) into published computer-use recordings, so a viewer can tell what drove each on-screen change without reading the agent transcript. The guiding principle is to **annotate only input that isn't visible on screen**: pointer and scroll actions produce visible on-screen motion (the cursor, or content scrolling), so only keyboard input is annotated. Labels are rendered into the video pixels themselves, so they appear anywhere the mp4 is viewed (in-app, downloaded, attached to a PR or Slack) with no special player support.

## Problem
Computer-use recordings show the UI changing over time but don't communicate the keyboard input that caused each change. Pointer and scroll actions produce visible on-screen motion (the cursor moving/clicking, or content scrolling), but keyboard shortcuts and typing leave no on-screen trace — a viewer can't tell whether a state change came from `⌘K` or a typed string. This makes recordings hard to use for debugging, review, and demos.

## Figma
Figma: none provided. Design references a screenshot shared by the user: small pill-shaped labels near the bottom-center of the frame (examples: `ctrl+a`, `typing…`), each shown briefly at the moment the action occurs.

## Goals
- Make published recordings self-explanatory for keyboard input without external narration.
- Keep annotations burned into the video so they survive download and re-sharing.
- Follow familiar screen-recording conventions (transient, unobtrusive input labels).
- Never leak sensitive typed content or corrupt the agent's own perception of the screen.

## Non-goals
- Annotating pointer or scroll actions (clicks, mouse-moves, scroll, drag) — they produce visible on-screen motion; only keyboard input is annotated.
- Animated click pulses, drag trails, or cursor highlighting (V0 is keyboard text labels only).
- Player-side or toggleable overlays / separate caption tracks (superseded by burn-in).
- Configurable styling, density, or localization of labels.
- Recording-usage metrics/telemetry (separate change).
- Rendering the recording/ended card in the block list (feature #1). This feature only affects the video artifact's pixels.

## Behavior

### What renders
1. When a computer-use recording is published, the input actions that occurred during capture are rendered as burned-in text labels in the video's pixels. The labels are present in the mp4 itself and require no player, track, or app support to be seen.
2. Each label renders as a small pill near the **bottom-center** of the frame: light text on a semi-transparent dark rounded background, sized to be legible at normal playback, lifted off the very bottom edge so it does not sit flush against the frame border.
3. A label appears at the **timecode of its action** (relative to when capture went live) and remains visible for a short fixed window (~1.5s), then disappears.
4. **One pill at a time.** If a new action occurs before the previous pill's window elapses, the previous pill is cut short so the newest label replaces it (each label's visible end is clamped to the next label's start). Labels never stack or overlap.
5. **Only keyboard input renders**, per the guiding principle (annotate only input that isn't visible on screen). The rendered set is exactly:
   - **Keyboard shortcut / key press** → a key pill showing the key combination as the agent expressed it, e.g. `ctrl+a`, `cmd+c`, `shift+enter`.
   - **Text entry** → a generic `typing…` pill. The actual typed text is **never** shown.

### What does not render
6. **All pointer and scroll actions are omitted** — they produce visible on-screen motion (the cursor moving/clicking, or content scrolling), so a label would be redundant. This includes left/right/middle/double/triple clicks, mouse-down/up, mouse-move, scroll, and click-drag.
7. **No-op and meta actions do not render**: waits, screenshot captures, cursor-position queries, and zoom/region captures produce no label (they are internal steps, not user-visible input).
8. A recording in which no renderable actions occurred (e.g. a click-only flow) publishes a normal video with **no pills**. Empty burn-in is a no-op, not an error.

### Integrity and safety
9. **The overlay never affects the agent's perception.** Labels exist only in the published recording, not on the live display; the model's own screenshots taken during the session are pixel-identical to what they would be without this feature.
10. **Typed text is never burned in.** Only key combinations and the generic `typing…` indicator appear. Key combinations are treated as non-sensitive; the typed payload is not.
11. **Annotations are best-effort and never block publication.** If overlay compositing cannot run or fails, the original (un-annotated) recording is still published. A recording is never lost or failed because of the overlay step.
12. Overlays are **deterministic**: the same sequence of logged actions with the same timecodes produces the same labels at the same times.

### Lifecycle coverage
13. The overlay applies to **every published recording regardless of how it terminated** — agent stop, capture limit/auto-stop, agent finished without stopping, or a recording cut short. Whatever actions were logged before termination are annotated; a truncated recording still shows pills for the actions that occurred before it ended.
14. Only actions that occurred **while capture was live** are annotated. Actions before capture started or after it stopped never appear.
