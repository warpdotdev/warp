# Recoverable Shared-Session Viewer Initial Joins

## Summary

When a user opens a live shared session, including a Cloud Mode session, transient failures while joining should recover automatically when possible. If the session still cannot be joined, the pane should show a stable failure state rather than remaining indefinitely on `Loading session...`, with an in-pane retry action only when trying again may recover.

## Problem

A join can fail after a viewer pane has begun loading but before session content is available. In the reported incident, the user was left with a pane that continued showing a loading surface even though it could not recover without reopening the session.

## Figma

Figma: none provided. The screenshot associated with REMOTE-1661 documents the broken loading state, not a target visual design; exact visual styling of the recovery surface remains a design/implementation follow-up.

## Goals and non-goals

### Goals

- Recover transparently from transient connection interruptions during initial session join.
- Ensure an unsuccessful join always settles into an actionable, understandable state.
- Make retry possible without closing or recreating the pane when the failure is retryable.

### Non-goals

- Change what happens after a viewer has already joined a live session and subsequently disconnects.
- Mask permanent session, permission, or access failures with repeated automatic attempts.
- Define the final visual styling of the failure surface.

## Behavior

1. When a user opens a live shared session or Cloud Mode session, the pane may show a joining/loading state only while the app is actively trying to display that session.

2. If the initial attempt is interrupted by a failure that may be temporary before any live session content is shown, Warp automatically makes a bounded number of additional join attempts. The user is not required to close the pane or reopen the session to benefit from these attempts.

3. While automatic attempts remain in progress, the pane continues to communicate that it is joining the session. It must not appear to be an active interactive session or a completed/ended session.

4. Automatic recovery is bounded. A sequence of transient initial-join failures must eventually resolve either by displaying the joined session or by displaying a stable failure state; the pane must not remain on a loading state indefinitely.

5. If an automatic retry succeeds, the existing pane transitions to the successfully joined session with the same behavior the user would have received from a successful first attempt. In Cloud Mode/handoff flows, existing local pane context must not be discarded merely because joining required a retry.

6. If the system determines that the initial join was rejected for a terminal reason, such as a session that is unavailable or access that is not allowed, it does not continue automatic retries. The pane moves promptly to the failure state, communicates the applicable user-facing failure reason, and does not offer an explicit retry action.

7. If bounded recovery is exhausted for a transient failure, the pane moves to a failure state that communicates that the session could not be loaded and offers a retry action. It must not continue showing `Loading session...` as though work is still underway.

8. The failed-initial-join state is distinct from an ended live session: the user is not told that a viewed session ended when the session was never displayed, and ended-session affordances or history must not be shown solely because the initial join failed.

9. Activating retry from a retryable failure state begins a new attempt to load the same session in the same pane and returns the pane to the joining/loading state while that attempt is active. Retry may later succeed or return to an updated failure state under these same rules.

10. Repeated failures for one join attempt must not create duplicate viewer panes or stack multiple simultaneous recovery surfaces. The user sees one coherent state for the pane they opened.

11. If the user closes the pane while joining is still in progress, the pending network/join machinery for that pane is promptly cleaned up and does not continue attempting to join in the background.

12. Cloud Mode/handoff flows must preserve local `initial_load_mode` and pane-local context across join retries; a retry must not reset or discard context that was prepared before the initial join attempt began.

13. The same shared session may be opened in multiple panes. A failed join in one pane must not interfere with concurrent join attempts in other panes or a successfully established viewer in another pane.

14. Post-join reconnection semantics remain unchanged: an already-joined viewer that is later disconnected continues to use the existing reconnect and retry behavior, including the event loop and existing retry strategy.
