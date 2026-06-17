# Always Show Comment Buttons Regardless of AI State

## Summary
Code review comment and add-as-context buttons should always be visible in the code review panel, even when global AI is disabled. When AI is disabled, users can still send comments to CLI agent terminals (Claude Code, Gemini, etc.). The disabled "Send to Agent" button should clearly communicate *why* it is disabled.

## Problem
When the global AI toggle is off (`is_any_ai_enabled` returns false), all comment and add-as-context buttons vanish from the code review panel — header dropdown, per-file headers, and editor gutter buttons. This prevents users from creating or submitting review comments to CLI agents, which do not depend on Warp AI. There is no way to distinguish between "AI is disabled" and "all terminals are busy" when the send button is unavailable.

## Goals
- Comment and add-as-context buttons are always visible when their feature flags are enabled, regardless of AI state.
- When AI is disabled, only CLI agent terminals are considered valid review comment destinations.
- The disabled "Send to Agent" button displays a tooltip that differentiates between "AI is disabled" and "all terminals are busy."

## Non-Goals
- Changing the behavior of the AI toggle itself.
- Enabling non-CLI terminals for review comments when AI is disabled.
- Changing the "Send to Agent" button's enabled/disabled logic beyond what the destination already controls.

## Figma
Figma: none provided

## User Experience

### Button visibility
All of the following buttons are always visible when their respective feature flags are enabled, regardless of whether global AI is on or off:

- **Header dropdown** (wide and compact layouts): The `⋮` dropdown containing "Add comment" and "Add diff set as context."
- **Per-file header**: The per-file "add as context" button.
- **Editor gutter buttons**: Inline comment and add-as-context buttons on diff lines.
- **Editor gutter actions**: The `NewCommentOnLine` and `RequestOpenSavedComment` actions are always available. Comments can always be created; the AI gate applies only at submission time.

### Terminal availability when AI is disabled
When global AI is disabled:
- Only terminals running a CLI agent are considered available for review comments.
- Non-CLI, non-executing Warp terminals are **not** available as destinations.
- If AI is toggled on or off, terminal availability updates immediately.

When global AI is enabled:
- Behavior is unchanged from today. Any idle, non-executing terminal is a valid destination.

### Send button tooltip differentiation
The "Send to Agent" button tooltip communicates the specific reason it is disabled. Priority order:

1. CLI agent destination is selected → existing CLI tooltip (enabled state).
2. AI is disabled and no CLI terminals available → **"AI must be enabled to send comments to Agent"**
3. No AI credits → "Agent code review requires AI credits" (existing).
4. All terminals are busy → **"All terminals are busy"**
5. No sendable comments → existing tooltip.
6. Default → existing tooltip.

### State transitions
- Toggling AI on: terminal availability recomputes immediately; non-CLI terminals become available; tooltip updates.
- Toggling AI off: terminal availability recomputes immediately; non-CLI terminals become unavailable; tooltip changes to the AI-disabled message if no CLI terminals exist.

## Success Criteria
1. With AI disabled: the header dropdown, per-file add-as-context button, and all editor gutter comment/context buttons are visible and functional for creating comments.
2. With AI disabled and a CLI agent running: comments can be sent to the CLI agent terminal.
3. With AI disabled and no CLI agents: the "Send to Agent" button is disabled with tooltip "AI must be enabled to send comments to Agent."
4. With AI enabled and all terminals busy: the "Send to Agent" button is disabled with tooltip "All terminals are busy."
5. Toggling AI on/off updates terminal availability and tooltip text without requiring any other user action.
6. With AI enabled: behavior is identical to current behavior (no regressions).

## Validation
- Manual testing: toggle AI on/off with and without CLI agents running; verify button visibility and tooltip text in each state.
- Code review: confirm all `is_ai_enabled` gates on button visibility are removed.
- Build verification: `cargo check`, `cargo fmt`, WASM build.

## Open Questions
None currently.
