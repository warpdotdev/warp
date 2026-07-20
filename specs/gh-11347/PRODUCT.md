# GH-11347 — Clarify when SSH warpification takes effect

Issue: [#11347 — SSH warpification toggle does not apply to current session](https://github.com/warpdotdev/warp/issues/11347)

## Summary

Changing **Warpify SSH Sessions** does not affect an SSH connection that is already running. Warp will explain in Settings that users must open a new terminal tab or pane and reconnect before the new value takes effect.

## Problem

The toggle currently has no timing guidance. A user can enable it while viewing an unwarpified SSH session, return to that session, and reasonably conclude that the setting is broken because the connection remains unchanged. Disabling it is similarly ambiguous: an already-warpified connection cannot be safely converted back to a plain SSH session in place.

## Goals

- State where the setting takes effect at the point where the user changes it.
- Give users a concrete, safe next step when they want the new value to apply.
- Preserve running terminals and SSH connections without reconnecting, terminating, or partially reconfiguring them.

## Non-goals

- Warpifying or unwarpifying an already-running SSH connection.
- Opening a new terminal tab or pane, reconnecting SSH, or installing/removing the SSH extension automatically.
- Applying the new value to terminal tabs or panes that were already open when the setting changed.
- Changing host eligibility, denylist, extension installation-mode, ControlMaster, or remote-server fallback behavior.

## Figma

Figma: none provided

## Behavior

1. The **Warpify SSH Sessions** setting row always includes the following supporting text: **“Changes apply to new terminal sessions. Open a new tab or pane, then reconnect to SSH.”**

2. Turning the setting on saves the enabled value immediately, but it does not alter, reconnect, or interrupt any local terminal or SSH connection that was already running when the value changed.

3. After turning the setting on, an eligible SSH connection launched from a newly opened terminal tab or pane follows the existing SSH warpification and extension-installation flow. Existing host eligibility, denylist, installation-mode, platform, and fallback rules continue to decide the outcome.

4. Turning the setting off saves the disabled value immediately, but it does not remove Warp features from an already-warpified SSH connection and does not terminate or reconnect that connection.

5. After turning the setting off, SSH connections launched from newly opened terminal tabs or panes do not enter Warp's SSH warpification or extension-installation flow.

6. Exiting SSH and rerunning `ssh` in a terminal tab or pane that was already open when the setting changed does not adopt the new value. The user must open a new tab or pane and reconnect to SSH.

7. The setting continues to control whether its dependent SSH extension installation-mode and ControlMaster controls are interactive; adding the supporting text does not change those controls or their saved values.

8. The supporting text is visible in enabled and disabled states, uses the standard Settings description style, remains readable at supported Settings widths and text scales, and is exposed to assistive technologies as descriptive text rather than as an interactive control.

9. No success toast, warning modal, confirmation dialog, or terminal-side banner is shown when the value changes. The persistent supporting text is the single explanation of when the change takes effect.
