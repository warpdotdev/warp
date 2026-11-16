# CLI Agent Composer Auto-Show & Auto-Dismiss — Tech Spec

## Problem
The PRODUCT.md spec requires three new settings that control the visibility lifecycle of the CLI agent rich input composer. The implementation spans settings definitions, the settings UI, the session model, and the terminal view's subscription to CLI agent session status changes.

## Relevant Code
- `app/src/settings/ai.rs (492–1144)` — `AISettings` group where new settings will be added, near the existing `should_render_cli_agent_footer` setting.
- `app/src/settings_view/ai_page.rs (4949–5067)` — `CLIAgentWidget` that renders the "Coding Agents" section in Settings > AI.
- `app/src/terminal/cli_agent_sessions/mod.rs` — `CLIAgentSessionsModel` singleton, `CLIAgentSession`, `CLIAgentSessionStatus`, `CLIAgentInputState`.
- `app/src/terminal/view.rs:10802` — `handle_cli_agent_sessions_event()` which reacts to `CLIAgentSessionsModelEvent::StatusChanged`.
- `app/src/terminal/view/use_agent_footer/mod.rs:486–524` — `submit_cli_agent_rich_input()` which currently always closes the composer after submission.
- `app/src/terminal/view/use_agent_footer/mod.rs:527–574` — `open_cli_agent_rich_input()` which opens the composer.

## Current State
- The composer is opened manually via Ctrl-G or the footer button (`open_cli_agent_rich_input`).
- After the user submits a prompt, `submit_cli_agent_rich_input` always calls `close_cli_agent_rich_input`.
- `handle_cli_agent_sessions_event` only handles `StatusChanged` for desktop notifications when the user is navigated away — it does not drive any composer visibility logic.
- The `CLIAgentSession` struct has a `listener: Option<ModelHandle<CLIAgentSessionListener>>` field that indicates whether the plugin is connected.

## Proposed Changes

### 1. New settings in `AISettings` (`settings/ai.rs`)

Add three new boolean settings inside the `define_settings_group!(AISettings, ...)` block, placed after `should_render_cli_agent_footer`:

- `auto_toggle_composer` (`AutoToggleComposer`): default `true`. Auto-hides the composer on `Blocked` and auto-shows on `InProgress`/`Success`, gated on plugin presence and the per-session `should_auto_toggle_input` flag.
- `auto_open_composer_on_cli_agent_start` (`AutoOpenComposerOnCLIAgentStart`): default `false`. Auto-opens the composer when a session is created or a plugin listener is registered. Also sets the session's initial `should_auto_toggle_input` flag.
- `auto_dismiss_composer_after_submit` (`AutoDismissComposerAfterSubmit`): default `false`. Auto-closes the composer after prompt submission, only when Setting 1 is not actively managing visibility.

### 2. Settings UI (`settings_view/ai_page.rs`)

Extend `CLIAgentWidget` to include two new `SwitchStateHandle` fields and render two new toggles inside the "Coding Agents" section, gated on `is_footer_enabled`:

- **Setting 1 toggle**: Label "Auto show/hide composer based on agent status" with an `AdditionalInfo` info tooltip saying "Requires the Warp plugin for your coding agent".
- **Setting 2 toggle**: Label "Auto dismiss composer after prompt submission" with a description explaining the behavior.

Add corresponding `AISettingsPageAction` variants (`ToggleAutoToggleComposer`, `ToggleAutoDismissComposerAfterSubmit`) and wire them to the settings.

### 3. Per-session `should_auto_toggle_input` flag (`cli_agent_sessions/mod.rs`)

Add a `should_auto_toggle_input: bool` field to `CLIAgentSession`. This flag controls whether auto-toggle is active for a given session:

- Initialized from `*AISettings::as_ref(ctx).auto_open_composer_on_cli_agent_start` when the session is created or a listener is registered.
- Set to `true` whenever the composer is opened (via `open_input`, which always passes `true`).
- Set to `false` when the user manually dismisses the composer (`close_cli_agent_rich_input_and_disable_auto_toggle` → `close_input` with `false`).
- Preserved as `true` when auto-close fires on Blocked (`close_cli_agent_rich_input` → `close_input` with `true`).

Threaded through `register_listener`, `open_input`, and `close_input` as a parameter.

### 4. Auto-show/hide on status changes (`terminal/view.rs`)

Extend `handle_cli_agent_sessions_event` to react to `StatusChanged` for the current terminal view (not just notifications). When all conditions are met:

- `auto_toggle_composer` is enabled
- The session has a plugin listener and `should_auto_toggle_input` is `true`
- AI is enabled and the CLI agent toolbar is enabled

Then:
- On transition to `Blocked`: call `close_cli_agent_rich_input` (preserves `should_auto_toggle_input = true`).
- On transition to `InProgress` or `Success`: call `open_cli_agent_rich_input(AutoShow)` if the composer isn't already open.

Additionally, `maybe_auto_open_cli_agent_composer` is called after session creation and listener registration to handle the auto-open-on-start setting.

### 5. Conditional close after submission (`terminal/view/use_agent_footer/mod.rs`)

A shared `maybe_close_composer_after_submit` method encapsulates the conditional close logic, called from both the synchronous path and the `DelayedEnter` timer callback in `write_cli_agent_text_then_submit`. It checks `has_plugin` (plugin present AND `should_auto_toggle_input`) and `auto_toggle_composer` to decide whether status events manage visibility or `auto_dismiss_composer_after_submit` should close the composer.

### 6. Close variants (`terminal/view/use_agent_footer/mod.rs`)

- `close_cli_agent_rich_input`: delegates to `close_cli_agent_rich_input_impl(true)` — preserves `should_auto_toggle_input` for auto-close on Blocked.
- `close_cli_agent_rich_input_and_disable_auto_toggle`: delegates to `close_cli_agent_rich_input_impl(false)` — disables auto-toggle when the user manually dismisses.

All manual close call sites (Escape, Ctrl-G toggle, footer button toggle, footer hide, block completion) use the `_and_disable_auto_toggle` variant.

### 7. New `CLIAgentInputEntrypoint::AutoShow` variant

Added to `cli_agent_sessions/mod.rs` to distinguish auto-opens from manual opens in telemetry.

## End-to-End Flow

### Auto-open on session start (Setting 2 enabled):
1. CLI agent command detected → session created with `should_auto_toggle_input = true`.
2. `maybe_auto_open_cli_agent_composer` fires → composer opens via `AutoShow` entrypoint.

### Auto-hide on blocked, auto-show on resume (Setting 1 enabled, plugin present):
1. CLI agent runs and enters `PermissionRequest` → `CLIAgentSession::apply_event` sets status to `Blocked`.
2. `CLIAgentSessionsModel` emits `StatusChanged { status: Blocked }`.
3. `TerminalView::handle_cli_agent_sessions_event` checks: setting on, plugin present, `should_auto_toggle_input` true → calls `close_cli_agent_rich_input` (preserves flag).
4. User interacts directly with the terminal (e.g., approves permission).
5. Agent resumes → status changes to `InProgress` → handler calls `open_cli_agent_rich_input(AutoShow)`.

### Manual dismiss breaks auto-toggle cycle:
1. During auto-toggle, user presses Escape → `close_cli_agent_rich_input_and_disable_auto_toggle` sets `should_auto_toggle_input = false`.
2. Subsequent status changes no longer trigger auto-open/close for this session.
3. User manually re-opens with Ctrl-G → `open_input` sets `should_auto_toggle_input = true` → auto-toggle resumes.

### Auto-dismiss on submit (Setting 3 enabled, no plugin):
1. User manually opens composer with Ctrl-G.
2. User submits text → `maybe_close_composer_after_submit` checks: no plugin (or `should_auto_toggle_input` false), setting 3 is on → closes the composer.

## Risks and Mitigations
- **Flicker from rapid status transitions**: If a CLI agent rapidly transitions Blocked→InProgress→Blocked, the composer could flicker open/close. Mitigation: unlikely in practice since permission requests have user-gated responses. Can add a debounce later if needed.
- **Race with manual open**: If the user manually opens the composer just before auto-close fires, it could feel jarring. Mitigation: the auto-close only fires on status transitions, not on a timer, so it maps to genuine agent state.

## Testing and Validation
- Add unit tests in `cli_agent_sessions/mod_tests.rs` verifying that `StatusChanged` events propagate correctly.
- Add integration test scenarios exercising auto-show on blocked and auto-dismiss on submit.
- Manual testing with and without the plugin to verify both settings behave correctly.

## Follow-ups
- Add telemetry for auto-show/auto-dismiss to track adoption.
- Consider debounce/delay on auto-show if rapid transitions prove to be an issue.
