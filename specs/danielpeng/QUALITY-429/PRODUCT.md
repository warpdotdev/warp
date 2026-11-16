# Settings File Error Banner

## Summary

When the user's `settings.toml` file contains errors — either the entire file is syntactically invalid TOML, or individual setting values cannot be deserialized into their expected types — Warp shows a dismissible warning banner at the top of the workspace. The banner tells the user what went wrong and provides a button to open the file in Warp's editor so they can fix it.

## Problem

Today, when `settings.toml` has errors, the user gets no feedback. Settings silently fall back to defaults, and the user has no way to know that their customizations aren't being applied.

## Goals

- Surface settings file errors to the user in a clear, non-blocking way.
- Handle both error types: entire file unparsable (TOML syntax error) and individual values invalid (wrong type for a setting).
- Show a single banner regardless of how many errors exist.
- Automatically clear the banner when the user fixes the file.
- Provide a direct action to open the settings file for editing.

## Non-goals

- Per-setting inline error annotations (future work).
- Automatic correction or migration of invalid values.
- Surfacing errors for other config files (tab configs already have their own toast system).
- A settings file editor with TOML syntax highlighting or validation (uses the existing code editor).

## Figma

Figma: none provided. The banner follows the existing `WorkspaceBanner` visual pattern (colored bar at the top of the workspace with text and an optional button).

## User experience

### Banner appearance

The banner appears at the top of the workspace, below the tab bar, using the existing workspace banner system. It uses the warning color scheme (amber/orange background, white text) — the same style as the reauth banner.

### Banner priority

The settings error banner sits just below the reauth banner in priority. If the reauth banner is active, it takes precedence and the settings error banner is hidden until reauth is resolved. The settings error banner takes precedence over all other workspace banners (version deprecated, unable to update, crash recovery) because it's more important that the user is notified their settings file is broken than that they continue to see those other banners.

### Error messages

- **Entire file unparsable**: "Your settings file (settings.toml) could not be loaded: {parse_error}" — where `{parse_error}` is the specific TOML parse error (e.g. line number and description from the TOML parser).
- **Single invalid value**: "Invalid value for '{key}' in settings.toml. The default value is being used."
- **Multiple invalid values**: "Invalid values in settings.toml for: {key1}, {key2}, ... Default values are being used."

### "Open settings file" button

The banner includes an "Open settings file" button styled consistently with other workspace banner buttons. Clicking it opens `settings.toml` in a new editor pane (using Warp's built-in code editor, same as opening any other file).

### Dismiss behavior

- The banner has a close (✕) button for temporary dismissal.
- Clicking the close button hides the banner for the current session.
- If the user edits the settings file and introduces a **new** error (different from the one that was dismissed), the banner reappears.
- If the user fixes the file (hot-reload succeeds with no errors), the banner disappears automatically regardless of dismissal state.

### Startup behavior

- If `settings.toml` has a TOML syntax error on startup, all settings fall back to defaults and the banner is shown on the first frame.
- If `settings.toml` is syntactically valid but contains individual invalid values, those settings fall back to defaults and the banner is shown on the first frame.

### Hot-reload behavior

- When the user modifies `settings.toml` and the file watcher detects the change:
  - If the file cannot be parsed: the previous in-memory settings are kept and the banner appears (or updates).
  - If the file parses but some values are invalid: valid settings are updated, invalid ones fall back to defaults, and the banner appears (or updates).
  - If the file parses and all values are valid: settings are updated and the banner disappears.

## Success criteria

1. When `settings.toml` contains invalid TOML syntax on startup, a warning banner is visible after the workspace loads.
2. When `settings.toml` contains a valid TOML structure but with an invalid value for a known setting on startup, a warning banner is visible.
3. When a running Warp instance detects a file change that introduces a TOML syntax error, the banner appears within a few seconds.
4. When a running Warp instance detects a file change that introduces an invalid setting value, the banner appears within a few seconds.
5. When the user fixes the file and the watcher picks up the change, the banner disappears automatically.
6. The "Open settings file" button opens `settings.toml` in Warp's code editor.
7. The close (✕) button dismisses the banner until a new error event arrives.
8. The banner does not appear when there are no errors in the settings file.

## Validation

- **Unit tests**: Verify `reload_all_public_settings` returns failed keys for invalid values and empty for valid values. Verify `validate_all_public_settings` detects invalid stored values without modifying state.
- **Integration tests**: Four integration tests covering the startup × reload and whole-file × individual-value error matrix. Each test asserts the banner is visible (or not) by checking `has_settings_file_error_banner()` on the workspace view. The reload-with-fix test also verifies the banner clears.
- **Manual validation**: Break `settings.toml` with a syntax error, launch Warp, confirm the banner is visible with the correct message and button. Fix the file, confirm the banner clears. Repeat with an invalid value (e.g. `font_size = "abc"`).

## Follow-ups

- Not included in this PR, but once we have the skill for an agent to edit the settings file, we should have some kind of "Oz auto-fix" button.
