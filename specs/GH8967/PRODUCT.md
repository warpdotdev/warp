# Requested Command Details Expanded by Default

GitHub issue: #8967

## Summary

Add a user preference that makes Agent Mode requested-command details open
expanded by default. Users who frequently inspect generated commands can see
the command body and execution details immediately, while the current collapsed
default remains unchanged for everyone else.

## Problem

Requested commands are often the highest-risk and most useful part of an Agent
Mode turn to inspect. Today, users who want to review command details must
expand each requested command manually, which adds friction during debugging,
auditing, and long agent runs with multiple commands.

## Figma

Figma: none provided.

## Behavior

1. Warp exposes a user-facing preference named "Expand requested command details
   by default" or equivalent wording. The preference is off by default.

2. When the preference is off, requested-command cards behave as they do today:
   newly displayed requested commands use the existing collapsed/expanded
   defaults, and any existing automatic expansion behavior remains unchanged.

3. When the preference is on, each newly displayed Agent Mode requested-command
   card starts with its details expanded. This applies before execution, while
   a command is waiting for approval, while it is running, after it succeeds,
   after it fails, and after it is cancelled.

4. Expanded-by-default is only an initial presentation state. It must not accept,
   reject, execute, cancel, retry, or otherwise change the command lifecycle.
   A command that requires approval still requires the same approval action.

5. The expanded state reveals only details that the requested-command card
   already makes available when manually expanded today. The preference must not
   fetch additional command data, expose hidden secrets, bypass redaction, or
   show output that the current user would not otherwise be allowed to inspect.

6. Manual user action wins over the preference for an individual card. If the
   user collapses a card that was expanded by default, that card stays collapsed
   across ordinary rerenders. If the user expands a card while the preference is
   off, that card stays expanded across ordinary rerenders.

7. Changing the preference affects requested-command cards created after the
   setting change. It does not retroactively expand or collapse cards that are
   already visible, so the UI does not unexpectedly move while the user is
   reading or selecting text.

8. Restored or historical conversations preserve any saved per-card
   expanded/collapsed state when such state exists. If no per-card state exists
   for a restored requested-command card, Warp uses the current preference as
   that card's initial state.

9. Requested-command cards produced in parallel within the same Agent Mode turn
   each apply the same rule independently. Manually toggling one card does not
   change the state of sibling command cards.

10. The preference applies to requested shell-command details in Agent Mode. It
    does not change the default presentation of unrelated inline actions such
    as code diffs, MCP tool calls, summaries, plan items, or general terminal
    command blocks.

11. If existing delayed auto-expand behavior would normally expand a running
    requested command, enabling this preference makes that card start expanded
    immediately. The delayed behavior must not later collapse a card that the
    preference or the user already expanded.

12. The preference is available from the same settings surface used for Agent
    Mode behavior preferences. The setting label and description must make clear
    that it affects requested-command details only, not automatic command
    execution or command approval.

13. The setting is keyboard-accessible and screen-reader discoverable anywhere
    it is exposed in UI. The requested-command card's existing expand/collapse
    control remains keyboard-accessible regardless of the default state.

14. The setting behaves consistently across windows for the same user. If Warp
    syncs the preference between devices, the synced value applies only to new
    requested-command cards created after the local client observes the updated
    value.

15. If the setting cannot be loaded, Warp falls back to the current default-off
    behavior rather than expanding command details unexpectedly.
