# PRODUCT — Distinguish left vs right Alt on Windows and Linux

Linear: [CODE-1794](https://linear.app/warpdotdev/issue/CODE-1794/windowslinux-right-alt-isnt-recognized-breaking-right-alt-as-meta)

## Summary

On Windows and Linux, the "Right Alt as meta" and "Left Alt as meta" settings under Keys in Warp should each independently control only their own physical Alt key. Today, right Alt is never recognized as right Alt, so enabling "Right Alt as meta" is a no-op and enabling "Left Alt as meta" incorrectly treats both Alt keys as meta.

## Problem

Warp users on Windows and Linux who rely on the extra-meta-keys settings to make a single Alt key behave as meta (for example, shell users who want right Alt to emit ESC-prefixed keys while left Alt still works as a regular modifier for keybindings like Ctrl+Alt+R "Resume conversation") cannot do so today. The setting either has no effect (right-alt-as-meta) or applies to both keys at once (left-alt-as-meta), breaking keybindings that need a plain Alt modifier.

## Behavior

1. The Settings → Keys page exposes two independent checkboxes, "Left Alt as meta" and "Right Alt as meta", each of which controls only its own physical key. Toggling one must not change the behavior of the other.

2. When "Left Alt as meta" is enabled and "Right Alt as meta" is disabled:
   - Pressing a character with left Alt held produces a meta-prefixed keystroke (Alt is stripped, meta is set), both for keybindings and for PTY input (ESC-prefixed text in the shell).
   - Pressing the same character with right Alt held produces a normal Alt-prefixed keystroke. Keybindings that use Alt (e.g. `ctrl-alt-r`) fire as expected, and Alt-only bindings see `alt: true, meta: false`.

3. When "Right Alt as meta" is enabled and "Left Alt as meta" is disabled:
   - Pressing a character with right Alt held produces a meta-prefixed keystroke.
   - Pressing the same character with left Alt held produces a normal Alt-prefixed keystroke.

4. When both settings are enabled, either Alt key produces a meta-prefixed keystroke (current combined behavior is preserved).

5. When both settings are disabled, neither Alt key is treated as meta and both behave as plain Alt modifiers. This is the default.

6. The existing Windows/Linux keybinding `Ctrl+Alt+R` ("Resume conversation"), and any other `ctrl-alt-*` keybinding, continues to work with either Alt key whenever that Alt side is not configured to be treated as meta.

7. On macOS, the existing Option-as-meta behavior, which already distinguishes left and right Option via the platform-native path, is unchanged.

8. Settings changes take effect on the next keystroke. The user does not have to relaunch Warp.

9. Alt state does not get "stuck":
   - If the user holds Alt, switches windows via Alt+Tab, releases Alt while Warp is not focused, and then refocuses Warp, Warp must not continue to believe that Alt is held. The next character key pressed in Warp reports `alt: false`.
   - If either Alt key release is lost for any other reason (dropped event, OS-level remap), Warp recovers whenever the OS next reports that no Alt is held.

10. The per-side distinction applies only to Alt for this feature. Other modifiers (Shift, Ctrl, Cmd/Super) continue to behave as before.

11. The diagnostic log line emitted when a key is rewritten to meta identifies which side triggered the conversion (left alt, right alt, or both), so bug reports of the form "my right Alt still acts like meta" can be triaged from logs without a repro.

## Success criteria

- On Windows, with "Left Alt as meta" enabled and "Right Alt as meta" disabled, `Ctrl + RightAlt + R` fires the Resume conversation keybinding, and `LeftAlt + b` sends ESC-b to the PTY.
- On Windows and Linux, toggling "Right Alt as meta" on its own changes right Alt behavior and leaves left Alt untouched, and vice versa.
- Existing macOS behavior for Option-as-meta is unchanged.
