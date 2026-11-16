# CLI Agent Composer Auto-Show & Auto-Dismiss Settings

## Summary
Add three new user-facing settings that control the automatic visibility of the CLI agent rich input composer. The first setting auto-hides the composer whenever a CLI agent is blocked (requiring direct keyboard interaction) and auto-shows it when the agent resumes work, gated on having the Warp plugin installed for rich status information. The second setting auto-opens the composer when a CLI agent session starts or a plugin listener is registered. The third setting controls whether the composer auto-dismisses after the user submits a prompt, applying whenever Setting 1 is not actively managing composer visibility (either because it is disabled or because there is no plugin listener).

A per-session `should_auto_toggle_input` flag tracks whether auto-toggle is active for a given session. Opening the composer (manually or automatically) opts the session in; manually dismissing (Escape, Ctrl-G toggle, footer button) opts it out. Auto-close on Blocked preserves the flag so auto-open can fire when the agent resumes.

## Problem
Today, users interacting with CLI agents (Claude, Codex, Gemini, etc.) must manually open the rich input composer via Ctrl-G or the footer button every time they want to send a message. There is no way to have the composer appear automatically when the agent is waiting for input. Similarly, after submitting a prompt, the composer remains open, which may not be desired for users who prefer a minimal terminal view when the agent is actively working.

## Goals
- Let users opt into auto-hiding the composer whenever a CLI agent enters a "blocked" state (requiring direct keyboard interaction), and auto-showing it when the agent resumes work, so the interaction feels seamless.
- Let users opt into auto-dismissing the composer after sending a prompt, reducing visual clutter when the agent is working.
- Gate the auto-show behavior on having rich conversation status (i.e., the Warp plugin listener is active), since without it we cannot reliably detect when the agent is blocked.
- Gate the auto-dismiss (post-submission) behavior on Setting 1 not actively managing visibility — when the plugin is present and Setting 1 is enabled, auto-show/hide handles visibility; otherwise the user can choose to have the composer close after submission.

## Non-goals
- Changing the existing manual Ctrl-G / footer button flows.
- Auto-installing the plugin or prompting for installation from these settings.
- Changing behavior in the regular agent conversation view (these settings apply only to CLI agent sessions).

## Figma / Design References
Figma: none provided

## User Experience

### Setting 1: "Auto show/hide composer based on agent status" (`auto_toggle_composer`)
- **Location**: Settings > AI > Coding Agents section, below existing "Show coding agent toolbar" toggle.
- **Label**: `Auto show/hide composer based on agent status`
- **Info tooltip** (ⓘ icon next to label): "Requires the Warp plugin for your coding agent"
- **Default**: `true` (on)
- **Behavior when enabled**:
  - When a CLI agent session has a plugin listener (`session.listener.is_some()`), the session's `should_auto_toggle_input` flag is true, and the session status transitions to `Blocked` (permission request, idle prompt), the composer automatically closes (the agent requires direct keyboard interaction in the terminal).
  - When the session status transitions away from `Blocked` (to `InProgress` or `Success`), the composer automatically opens.
  - If the user manually dismisses the composer (Escape, Ctrl-G toggle, footer button), `should_auto_toggle_input` is set to `false` for that session, disabling auto-toggle until the composer is opened again.
  - If there is no plugin listener on the session, this setting has no effect.
- **Behavior when disabled**: No automatic composer visibility changes based on status.

### Setting 2: "Auto open composer when a CLI agent session starts" (`auto_open_composer_on_cli_agent_start`)
- **Location**: Settings > AI > Coding Agents section, below Setting 1.
- **Label**: `Auto open composer when a CLI agent session starts`
- **Default**: `false` (off)
- **Behavior when enabled**:
  - When a CLI agent session is created (command detection) or a plugin listener is registered, the composer automatically opens.
  - Also sets the session's initial `should_auto_toggle_input` flag to `true`, enabling auto-toggle from Setting 1 immediately.
- **Behavior when disabled**: The composer does not auto-open on session start. The session's `should_auto_toggle_input` starts as `false`, so auto-toggle from Setting 1 remains dormant until the user manually opens the composer.

### Setting 3: "Auto dismiss composer after prompt submission" (`auto_dismiss_composer_after_submit`)
- **Location**: Settings > AI > Coding Agents section, directly below Setting 2.
- **Label**: `Auto dismiss composer after prompt submission`
- **Default**: `false` (off)
- **Behavior when enabled**:
  - After the user submits a prompt through the CLI agent composer, the composer automatically closes.
  - This setting is a no-op only when the plugin IS present, Setting 1 is enabled, AND `should_auto_toggle_input` is true (because Setting 1's status-driven logic manages visibility in that case). In all other scenarios (no plugin, or plugin present but Setting 1 disabled), this setting controls post-submission behavior.
- **Behavior when disabled**: The composer remains open after submission.

### Edge Cases
- **All settings enabled, plugin present**: Setting 1 governs visibility (auto-hide on blocked, auto-show on resume). Setting 2 auto-opens the composer on session start. Setting 3 is effectively a no-op because the plugin provides rich status.
- **All settings enabled, no plugin**: Setting 2 has no effect (requires plugin for reliable status). Setting 3 closes the composer after submission. Setting 1 has no effect (no rich status to react to).
- **Settings 1 on, Setting 2 off, plugin present**: Auto-toggle is enabled but dormant until the user manually opens the composer (which sets `should_auto_toggle_input = true`). After that, auto-hide on blocked and auto-open on resume are active.
- **Session ends while composer is open**: Existing behavior already handles this (composer closes when session is removed).
- **User manually dismisses composer**: `should_auto_toggle_input` is set to `false`, disabling auto-toggle for that session. The user must re-open the composer to re-enable it.
- **Multiple terminals with different CLI agents**: Settings are global; auto-show/hide applies per-terminal based on each terminal's session state and its own `should_auto_toggle_input` flag.

## Success Criteria
1. A new "Auto show/hide composer based on agent status" toggle appears in Settings > AI > Coding Agents with an (ⓘ) tooltip reading "Requires the Warp plugin for your coding agent". Defaults to on.
2. When enabled and the plugin is present, the composer closes automatically when the CLI agent enters a blocked state and opens when it resumes (once `should_auto_toggle_input` is true for the session).
3. A new "Auto open composer when a CLI agent session starts" toggle appears below the first setting. Defaults to off.
4. A new "Auto dismiss composer after prompt submission" toggle appears below the second setting. Defaults to off.
5. When enabled and no plugin is present, the auto-dismiss setting closes the composer after the user submits a prompt.
6. When the plugin IS present and auto-toggle is active, the auto-dismiss setting has no observable effect (auto-show/hide from setting 1 takes precedence).
7. All three settings persist via the standard settings infrastructure (cloud-synced).
8. All three settings are only effective when AI is enabled and the coding agent toolbar is enabled.

## Validation
- Manual testing: Enable each setting independently and in combination, with and without the Warp plugin, to verify correct auto-show/hide behavior.
- Unit tests: Verify that `CLIAgentSessionsModel` status transitions trigger the correct open/close calls when settings are enabled.
- Settings persistence: Verify settings survive app restart and cloud sync.

## Open Questions
- Should there be a brief delay before auto-showing the composer to avoid flicker for very brief blocked states? (Recommend: no delay initially, iterate if needed.)
