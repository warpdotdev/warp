# Restore Fast Forward State Across Sessions — Product Spec

Figma: none provided

## Summary

Persist the last state of the fast-forward button for agent conversations across app restarts using Warp's existing session restoration flow. If a restored conversation had fast forward enabled before quit, it should come back enabled after launch; if it was disabled before quit, it should come back disabled.

## Problem

Fast forward is currently a per-conversation control during an active session, but that choice is lost when Warp restarts. Users who intentionally enabled or disabled fast forward for a conversation have to remember and re-apply that choice after session restoration, which makes restored agent conversations feel inconsistent and unreliable.

## Goals

- Preserve fast-forward state for restored agent conversations across app restarts.
- Keep the behavior per conversation rather than turning it into a global setting.
- Make the restored state reflect the user's last explicit choice, even if they toggled it shortly before quitting.
- Ship the behavior behind a feature flag that is enabled by default for dogfood builds.

## Non-goals

- Changing the user's default autonomy settings or global fast-forward preference.
- Changing the visibility or semantics of the existing fast-forward button.
- Persisting state for brand-new conversations that were never created/restored through the normal session restoration flow.
- Syncing this preference across machines or through server-side conversation storage.

## Figma

Figma: none provided

## User Experience

### Core behavior

- Fast-forward state is remembered per agent conversation.
- Session restoration is the source of truth for when this behavior applies. If Warp restores a conversation on launch, the restored conversation should use the same fast-forward state it had when the app last saved session state.
- The remembered state should apply whether the restored conversation is shown in the standard terminal/blocklist UI or reopened into fullscreen agent view.

### State rules

- If fast forward was on for a conversation before quit, it is on after launch.
- If fast forward was off for a conversation before quit, it is off after launch.
- Different restored conversations can come back with different fast-forward states.
- New conversations that do not yet have persisted state continue to use the current default behavior until the user explicitly toggles fast forward.

### Timing expectations

- If the user toggles fast forward and then quits or restarts Warp without taking another conversation action, the newly selected state should still be restored.
- The user should not need to send another message, receive another response, or otherwise advance the conversation for the new fast-forward state to stick.

### Feature flag behavior

- The behavior is gated by a dedicated feature flag for remembering fast-forward state across sessions.
- That flag is enabled by default for dogfood builds.
- If the flag is disabled, restored conversations fall back to current behavior instead of honoring the persisted fast-forward override.

## Success Criteria

1. A restored conversation that had fast forward enabled before app restart still shows fast forward enabled after restore.
2. A restored conversation that had fast forward disabled before app restart still shows fast forward disabled after restore.
3. Two restored conversations with different pre-restart fast-forward states each restore their own state correctly.
4. Toggling fast forward immediately before restart is enough to preserve the new state.
5. Restoring a conversation into fullscreen agent view preserves the same fast-forward state.
6. Conversations persisted before this feature ships continue to restore safely and default to the existing non-fast-forward behavior.
7. Disabling the remember-state feature flag restores the current behavior without breaking conversation restoration.

## Validation

- Unit tests covering persistence and restoration of the per-conversation fast-forward state.
- A restoration-focused test showing that a persisted conversation round-trips through SQLite-backed restore with the expected fast-forward mode.
- Manual verification:
  - enable fast forward, restart Warp, confirm it stays enabled
  - disable fast forward, restart Warp, confirm it stays disabled
  - repeat with multiple restored conversations and fullscreen agent view

## Open Questions

None.
