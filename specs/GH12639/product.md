# Product Spec: Support rich CLI agent status for Droid

**Issue:** [warpdotdev/warp#12639](https://github.com/warpdotdev/warp/issues/12639)
**Figma:** none provided

## Summary

Droid is already recognized as a CLI agent in Warp, but Droid sessions do not
currently participate in Warp's rich CLI-agent status flow. When Droid is
running inside Warp and emits supported Warp CLI-agent notifications, Warp should
track Droid sessions with the same vertical-tab and agent status model used for
other supported CLI agents.

## Problem

Today, Droid receives basic CLI-agent treatment in Warp: it has a known command
prefix, display name, icon, color, and skill providers. However, structured rich
status events for Droid are not wired into the listener and plugin-instructions
path that powers in-progress, completed, and attention-needed states.

As a result, a Droid session can be recognized as a Droid CLI-agent session, but
the user does not get the richer feedback that other supported agents can show
while work is running or blocked on user input. This is especially noticeable in
vertical tabs and other status surfaces where users expect long-running agent
work to advertise whether it is still active, done, or waiting.

## Goals

- Droid sessions can emit Warp's existing structured CLI-agent protocol events
  and have those events update rich session status in Warp.
- Droid uses the same status semantics as other supported CLI agents wherever
  the same Warp CLI-agent events are emitted.
- Warp provides clear manual setup instructions for enabling the Droid hook
  integration.
- The integration does not require a new Warp protocol.
- Existing rich-status behavior for Claude Code, OpenCode, Codex, Gemini,
  Auggie, and Pi remains unchanged.

## Non-goals

- Adding a new CLI-agent notification protocol.
- Adding one-click auto-install or auto-update for Droid hooks in this change.
- Changing Droid itself or requiring a Droid release.
- Changing Droid's command detection, display metadata, icon, color, or skill
  providers.
- Supporting every Droid hook event. Only events needed for Warp's existing
  rich status model are in scope.
- Changing the UX of the plugin install modal beyond Droid-specific manual
  instructions.

## Behavior

1. When a user runs Droid inside Warp without installing or configuring the Warp
   hook integration, Droid continues to behave as it does today. Basic agent
   recognition still works, and no new rich-status events are produced.

2. When a user opens the plugin instructions for Droid, Warp shows Droid-specific
   manual setup instructions for installing a hook script and registering that
   script with Droid's hooks configuration.

3. The Droid setup instructions use Droid's documented hooks system. They do not
   require Warp to install files automatically or modify the user's Droid
   configuration without user action.

4. The setup flow tells the user that the hook emits Warp CLI-agent events only
   when Droid is running inside Warp. Outside Warp, the hook should be inert.

5. After the user completes setup and restarts or starts a Droid session in
   Warp, a Droid prompt submission is reflected as an in-progress/running agent
   status.

6. When Droid finishes responding normally, Warp reflects the session as
   completed/done using the existing completed status model.

7. When Droid requests permission or otherwise notifies that it needs user
   input, Warp reflects the session as attention-needed/blocked using the
   existing attention-needed status model.

8. When Droid completes a tool after an attention-needed state, Warp can return
   the session to in-progress if Droid emits a supported tool-complete event.

9. Droid `SessionStart` events can be used to activate the rich listener path,
   but they do not by themselves mark the session as running, completed, or
   blocked. This matches the default behavior for agents whose `session_start`
   event is setup metadata rather than user-visible status.

10. Event payloads that declare `agent: "droid"` and use Warp's existing
    `warp://cli-agent` OSC 777 payload format are parsed as Droid events, not as
    unknown-agent events.

11. Unsupported or malformed Droid hook payloads are ignored rather than
    crashing the terminal session or changing the session status incorrectly.

12. Existing rich-status behavior for other CLI agents is unchanged. In
    particular, Codex's OSC 9 fallback behavior and existing structured
    plugin-event handling continue to work as before.

13. If Droid's hook configuration is present but the hook script cannot emit an
    event, the failure is non-fatal to Droid and to Warp. The user may lose rich
    status updates, but the Droid session itself should continue.

14. The integration applies to local Droid sessions running inside Warp. Remote,
    SSH, or tmux-specific notification forwarding behavior is not changed by
    this issue.

## Event mapping

The Droid hook integration should map Droid lifecycle signals to Warp's existing
CLI-agent event names:

- `SessionStart` -> `session_start`
- `UserPromptSubmit` -> `prompt_submit`
- `Notification` -> `permission_request` or `question_asked`, depending on the
  notification content available to the hook
- `Stop` -> `stop`
- `PostToolUse` -> `tool_complete`

The exact hook script can use conservative heuristics for distinguishing
permission requests from generic questions when Droid exposes both through
`Notification`. If the distinction is ambiguous, the implementation should favor
a safe attention-needed state rather than reporting completion.

## Success criteria

1. A configured Droid session emits a `prompt_submit` event and Warp shows the
   session as in progress.
2. A configured Droid session emits a `stop` event and Warp shows the session as
   completed/done.
3. A configured Droid session emits a permission-like `Notification` event and
   Warp shows the session as attention-needed/blocked.
4. A configured Droid session emits a question-like `Notification` event and
   Warp shows the session as attention-needed/blocked.
5. A configured Droid session emits a `PostToolUse` event after an
   attention-needed state and Warp can return the session to in-progress.
6. A `SessionStart` event with `agent: "droid"` activates the rich listener path
   but does not create a false running/completed/blocked status on its own.
7. A valid structured OSC 777 `warp://cli-agent` payload with `agent: "droid"`
   is parsed as `CLIAgent::Droid`.
8. Existing Claude Code, OpenCode, Codex, Gemini, Auggie, and Pi session-status
   tests continue to pass.
9. Droid setup instructions are visible from the same plugin-instructions UI
   used by other CLI-agent notification integrations.
10. The setup instructions are manual-only; Warp does not claim to auto-install
    or auto-update the Droid hook integration.

## Validation

- Unit tests cover Droid support in the rich session listener, including support
  gating and default forwarding behavior for `stop`, `permission_request`, and
  other status events.
- Unit tests cover the Droid plugin manager or instructions provider, including
  manual-only install behavior and presence of the required Droid hook events.
- Unit or parser tests confirm a structured event with `agent: "droid"` resolves
  to `CLIAgent::Droid`.
- Manual validation runs Droid in Warp with the documented hooks configured and
  records prompt-submit, running, attention-needed, tool-complete, and done
  states.
- Regression validation runs the existing listener and plugin-manager tests for
  other supported CLI agents.

## Open questions

- Should Warp prefer a user-managed hook script in `~/.factory/hooks.json`, or a
  Droid plugin-based hook package if Droid's plugin hook mechanism is considered
  stable enough for this integration?
- Does Droid expose a more reliable notification field for distinguishing
  permission requests from generic input-needed notifications, beyond inspecting
  the notification message?
- Should Droid rich-status support be guarded by an existing feature flag such
  as `HOANotifications`, or is listener support acceptable whenever Droid emits
  the established Warp CLI-agent protocol?
