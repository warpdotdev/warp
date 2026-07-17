# Computer-Use Recording Action Overlays — Product Spec

## Summary
Burn compact, human-readable labels for keyboard and scroll input into published computer-use recordings, so a viewer can tell what drove each on-screen change without reading the agent transcript. Labels explain the agent's input when the resulting visual change alone may be ambiguous, while omitting internal observation steps and noisy pointer movement. Labels are rendered into the video pixels themselves, so they appear anywhere the mp4 is viewed (in-app, downloaded, attached to a PR or Slack) with no special player support.

## Problem
Computer-use recordings show the UI changing over time but don't communicate the input that caused each change. A viewer may not be able to tell whether a state change came from `⌘K`, text entry, or agent-driven scrolling. This makes recordings hard to use for debugging, review, and demos.

## Figma
Figma: none provided. Design references a screenshot shared by the user: small pill-shaped labels near the bottom-center of the frame (examples: `ctrl+a`, `typing…`), each shown briefly at the moment the action occurs.

## Goals
- Make published recordings self-explanatory for keyboard and scroll input without external narration.
- Keep annotations burned into the video so they survive download and re-sharing.
- Follow familiar screen-recording conventions (transient, unobtrusive input labels).
- Never leak sensitive typed content or corrupt the agent's own perception of the screen.

## Non-goals
- Annotating clicks, mouse movement, or drags. Coordinate-local click pulses, drag trails, or cursor highlighting may be considered separately.
- Player-side or toggleable overlays / separate caption tracks (superseded by burn-in).
- Configurable styling, density, or localization of labels.
- Recording-usage metrics/telemetry (separate change).
- Rendering the recording/ended card in the block list (feature #1). This feature only affects the video artifact's pixels.

## Behavior

### What renders
1. When a computer-use recording is published, the input actions that occurred during capture are rendered as burned-in text labels in the video's pixels. The labels are present in the mp4 itself and require no player, track, or app support to be seen.
2. Each label renders as a small pill near the **bottom-center** of the frame: light text on a semi-transparent dark rounded background, sized to be legible at normal playback, lifted off the very bottom edge so it does not sit flush against the frame border.
3. A label appears at the **timecode of its action** (relative to when capture went live) and remains visible for a short fixed window (~1.5s), then disappears.
4. **One action group at a time.** Each `UseComputer` call produces at most one overlay group. When that call contains multiple renderable semantic actions, its pills appear together in action order as a centered horizontal row (for example, `ctrl+a` · `typing…` · `Return`). If a new call occurs before the previous group's window elapses, the previous group is cut short so the newest group replaces it. Groups never overlap.
5. The rendered set is:
   - **Keyboard shortcut / key press** → a key pill showing the key combination as the agent expressed it, e.g. `ctrl+a`, `cmd+c`, `shift+enter`.
   - **Text entry** → a generic `typing…` pill. The actual typed text is **never** shown.
   - **Scroll** → a direction-only pill: `scroll ↑`, `scroll ↓`, `scroll ←`, or `scroll →`. Scroll distance and coordinates are omitted.
6. Low-level primitives that implement one semantic action are collapsed. For example, modifier/key down/up events for `ctrl+a` produce one `ctrl+a` pill, and mouse-down/move/up events for a drag do not produce three labels.

### What does not render
7. **Pointer actions are omitted**: left/right/middle/double/triple clicks, mouse-down/up, mouse-move, and click-drag produce no text pill. Pointer-local effects are deferred rather than represented as noisy, context-free labels.
8. **No-op and meta actions do not render**: waits, screenshot captures, cursor-position queries, and zoom/region captures produce no label (they are internal steps, not user-visible input). The zero-duration wait used as a screenshot placeholder is explicitly treated as a no-op.
9. A recording in which no renderable actions occurred (e.g. a click-only flow) publishes a normal video with **no pills**. Empty burn-in is a no-op, not an error.

### Integrity and safety
10. **The overlay never affects the agent's perception.** Labels exist only in the published recording, not on the live display; the model's own screenshots taken during the session are pixel-identical to what they would be without this feature.
11. **Typed text is never burned in.** Text-entry payloads and unmodified printable keypresses render only as `typing…`. Modifier combinations and non-printing keys such as `Return`, `Tab`, `Escape`, and arrow keys may render by name.
12. **Annotations are best-effort and never block publication.** If overlay compositing cannot run or fails, the original (un-annotated) recording is still published. A recording is never lost or failed because of the overlay step.
13. Overlays are **deterministic**: the same sequence of logged action groups with the same timecodes produces the same pills at the same times.

### Lifecycle coverage
14. The overlay applies to **every published recording regardless of how it terminated** — agent stop, capture limit/auto-stop, agent finished without stopping, or a recording cut short. Whatever actions were logged before termination are annotated; a truncated recording still shows pills for the actions that occurred before it ended.
15. Only actions that occurred **while capture was live** are annotated. Actions before capture started or after it stopped never appear.
