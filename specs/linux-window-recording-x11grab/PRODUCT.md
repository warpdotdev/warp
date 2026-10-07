# Linux Window Recording via Native X11 Grab — PRODUCT.md

## Summary
Window-targeted Linux computer-use recordings should use the same native FFmpeg `x11grab` capture path as screen recordings whenever the target window can be made foreground-visible. This preserves low-overhead, real-time video recording for cloud/Xvfb computer-use sessions while clearly defining that native window recording is a visible-window capture contract, not an obscured-background-window capture contract.

## Problem
Background computer use can target a specific window for actions and screenshots, but the Composite/GetImage recording approach needed to preserve covered-window pixels adds significant CPU overhead and can produce time-compressed recordings when capture falls behind. For the current recording product surface, the safer user-visible behavior is a fast native recording of the target window after making it visible, with explicit failure when the window cannot be made visible enough to record.

## Goals / Non-goals
Goals:
- Keep full-screen recording behavior unchanged.
- Record `Target::Window` sessions as a native FFmpeg window capture when the target can be made foreground-visible.
- Avoid recording a different foreground surface when the agent intends to record a specific target window.
- Prefer clear failure over silently producing a misleading recording.

Non-goals:
- Guarantee that a target window remains fully unobscured for the entire recording across all window managers.
- Preserve true covered-window video capture semantics.
- Add adaptive video resizing when the target window is resized mid-recording.
- Change background computer-use action or screenshot targeting behavior.

## Behavior
1. When recording starts with a screen target, Warp records the full X11 display exactly as before.

2. When recording starts with a window target on Linux X11, Warp treats the recording as a foreground-visible window recording:
   - The target window is the recording subject.
   - The encoded video dimensions are based on the target window's dimensions at recording start.
   - Coordinates and actions may still be window-targeted, but the recording contract is that the target must be visible on the display.

3. Before a window-targeted recording begins, Warp attempts to make the target window visible enough to record:
   - If the target is already visible, recording starts without changing stacking order.
   - If the target is covered, Warp may raise it above other windows without changing user-facing recording controls.
   - Warp verifies visibility before reporting recording start success.

4. Visibility verification is best-effort, not a whole-window proof:
   - Warp must verify representative points within the target window, including the center and edges/corners when the window is large enough.
   - If those sampled points would not hit the target window after raising, recording fails with an error instead of starting.
   - Passing verification means the target is sufficiently foreground-visible for native recording, not that every pixel is guaranteed unobscured forever.

5. If Warp cannot resolve the target window, the target window has invalid dimensions, or the target window cannot be made foreground-visible, starting the recording fails and no artifact is uploaded.

6. If the target window is covered after recording has started, Warp does not guarantee covered pixels in the final video. Native window recording may show non-target pixels or otherwise follow FFmpeg/X11 behavior. This is accepted for this feature and should be treated as a limitation of visible-window recording.

7. If the target window is resized while recording is active, Warp does not dynamically resize the encoded video. The video follows native FFmpeg `x11grab -window_id` behavior for resize, including fixed initial dimensions and possible early termination or incomplete coverage when the window shrinks, grows, unmaps, or closes.

8. If FFmpeg exits before an explicit stop, reaches a configured duration/size limit, or cannot finalize the file, the user sees the same recording completion/error behavior as existing screen recordings.

9. Window-targeted recording must not change the behavior of window-targeted screenshots:
   - Screenshots may continue using the covered-window-capable capture path.
   - Screenshot result metadata remains window-relative.
   - A screenshot can still succeed for a covered target even when native recording would require the target to be visible.

10. Window-targeted recording must not change the behavior of background computer-use actions:
    - Keyboard-only actions can still target a background window.
    - Pointer actions can still raise the target as required by existing X11 action routing.
    - Recording's foreground-visible requirement applies only to video capture.

11. When recording starts successfully, the start result reports the window recording's fixed initial width and height.

12. When recording fails because the target cannot be made visible enough, the error must explain that the target window could not be made foreground-visible for native recording.

13. The behavior is Linux X11-specific. Unsupported platforms and non-X11 display servers keep their existing recording behavior and errors.
