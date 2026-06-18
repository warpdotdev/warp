# APP-3858: Change CLI Sharing Entrypoint to Remote Control

## Summary
Rebrand the CLI agent footer "Share session" button as "Remote control" and streamline the interaction so that a single click starts sharing, auto-copies the link to the clipboard, and shows a toast. The share modal is bypassed entirely. Once sharing is active, the same button becomes a "Stop sharing" button.

## Problem
The current "Share session" label doesn't communicate the primary use case: accessing a CLI agent session on your phone to track progress or steer it. Users don't equate "sharing" with mobile access. The current flow also requires an extra step through the share modal, which is unexpected for this use case.

## Goals
- Rename the feature to "Remote control" across the CLI agent footer entrypoint.
- One-click start: clicking the chip starts sharing immediately (without scrollback, just the current block), auto-copies the link, and shows a toast.
- Once sharing is active, the chip switches to a "Stop sharing" chip.
- Replace the `Icon::Share` icon with a phone icon (`phone-01.svg`). Use the existing `Icon::StopFilled` for the stop state.
- Add a `/remote-control` slash command that triggers the same action as clicking the chip.

## Non-goals
- Renaming the feature in contexts outside the CLI agent footer (pane header overflow menu, right-click context menu, the share modal itself). Those are separate entry points with different audiences.
- "Text link to my device" functionality (stretch goal for a follow-up).
- Changes to the viewer experience or the sharing protocol.
- Changes to the ambient agent / cloud mode share button.

## User Experience

### Before sharing
- The CLI agent footer renders a chip with the phone icon (`Icon::Phone01`) and label `/remote-control`.
- Typing `/remote-control` in the input triggers the same start-sharing action.
- The chip is only visible when `FeatureFlag::CreatingSharedSessions` and `ContextFlag::CreateSharedSession` are enabled (same gating as today).

### Starting a share (single click or slash command)
1. User clicks the `/remote-control` chip or types `/remote-control` in the input.
2. The session begins sharing immediately without scrollback (`SharedSessionScrollbackType::None`). This must be allowed even when agent conversations exist (bypassing the normal guard that blocks no-scrollback shares when `FeatureFlag::AgentSharedSessions` is enabled and conversations are present).
3. As soon as the session is established, the sharing link is auto-copied to the user's clipboard.
4. A toast appears: "Remote control link copied."
5. The share modal does **not** open.

### While sharing is active
- The chip changes to show a filled stop square (`Icon::StopFilled`) with label `Stop sharing`.
- Clicking the chip stops the sharing session.

### After sharing ends
- The chip reverts to its pre-sharing state (phone icon, `/remote-control` label).

### Edge cases
- If the session is already being shared (e.g. started from the pane header menu), the footer button should reflect the active sharing state (show stop button).
- If another share was started via a different entry point that used the share modal, the footer chip still shows "Stop sharing" while the session is active.
- If sharing fails to start (e.g. network error), the button should revert to its default state. Existing error handling for `SharePending` → failure applies.

### Banners
When sharing is started from the remote control entrypoint, the inline banners use different copy than the default:
- Start banner: "Remote control active" (instead of "Sharing started")
- End banner: "Remote control stopped" (instead of "Sharing ended")

Shares started from other entry points (pane header, share modal, etc.) keep the default "Sharing started" / "Sharing ended" copy. This intentionally deviates from the Figma mocks, which show the generic phrasing.

## Success Criteria
1. The CLI agent footer chip displays the phone icon and `/remote-control` label when no session is being shared.
2. Clicking the chip starts sharing without scrollback and does **not** open the share modal.
3. After sharing starts, the link is in the user's clipboard and a "Remote control link copied." toast is visible.
4. While sharing is active, the chip shows `StopFilled` icon with label "Stop sharing".
5. Clicking the stop chip ends the sharing session and the chip reverts to its default state.
6. The chip correctly reflects sharing state even when sharing was started/stopped from a different entry point.
7. Typing `/remote-control` in the agent input triggers the same start-sharing flow.

## Validation
- Build the app and verify the button icon, tooltip, and click behavior in a CLI agent session.
- Verify that clicking the button copies a valid sharing link to the clipboard.
- Verify the toast appears.
- Verify the button toggles to stop state while sharing is active.
- Verify the button reverts after stopping.
- Existing shared session tests should continue to pass since the underlying sharing mechanism is unchanged.

## Open Questions
- Should the agent view (non-CLI) share button also be rebranded to "Remote control", or only the CLI agent footer? (Current decision: only CLI agent footer for now.)
