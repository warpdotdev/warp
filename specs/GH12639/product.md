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
- Droid rich-status support follows Warp's existing rich CLI-agent notification
  rollout gate.
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
- Adding a new Droid-specific rollout flag in this change.

## Behavior

1. When a user runs Droid inside Warp without installing or configuring the Warp
   hook integration, Droid continues to behave as it does today. Basic agent
   recognition still works, and no new rich-status events are produced.

2. When Warp's rich CLI-agent notification infrastructure is disabled, Droid
   rich-status support is disabled as well: Warp does not register the Droid
   rich listener and does not show the Droid hook instructions entry point.

3. When Warp's rich CLI-agent notification infrastructure is enabled and a user
   opens the plugin instructions for Droid, Warp shows Droid-specific manual
   setup instructions for installing a hook script and registering that script
   with Droid's hooks configuration.

4. The Droid setup instructions use Droid's documented hooks system. They do not
   require Warp to install files automatically or modify the user's Droid
   configuration without user action.

5. The setup flow tells the user that the hook emits Warp CLI-agent events only
   when Droid is running inside Warp. Outside Warp, the hook should be inert.

6. After the user completes setup and restarts or starts a Droid session in
   Warp, a Droid prompt submission is reflected as an in-progress/running agent
   status.

7. When Droid finishes responding normally, Warp reflects the session as
   completed/done using the existing completed status model.

8. When Droid requests permission or otherwise notifies that it needs user
   input, Warp reflects the session as attention-needed/blocked using the
   existing attention-needed status model.

9. When Droid completes a tool after an attention-needed state, Warp can return
   the session to in-progress if Droid emits a supported tool-complete event.

10. Droid `SessionStart` events can be used to activate the rich listener path
    and seed context, but they do not by themselves mark the session as running,
    completed, or blocked. Registering a listener from a setup-only
    `SessionStart` must leave the session in a neutral/idle state until Droid
    emits a status-bearing event such as `prompt_submit`, `permission_request`,
    `question_asked`, `tool_complete` after an attention-needed state, or
    `stop`.

11. If Warp has already created a Droid session from command detection before
    the hook emits `SessionStart`, that setup-only `SessionStart` still clears
    the false running state and leaves the session neutral/idle. The existing
    state is preserved only after a status-bearing rich Droid event has already
    updated the same non-empty Droid `session_id`.
    Clearing this false running state is a UI/session refresh only: it must not
    behave like completion, attention-needed, or resumed-work status.

12. Event payloads that declare `agent: "droid"` and use Warp's existing
    `warp://cli-agent` OSC 777 payload format are parsed as Droid events, not as
    unknown-agent events.

13. Unsupported or malformed Droid hook payloads are ignored rather than
    crashing the terminal session or changing the session status incorrectly.

14. Existing rich-status behavior for other CLI agents is unchanged. In
    particular, Codex's OSC 9 fallback behavior and existing structured
    plugin-event handling continue to work as before.

15. If Droid's hook configuration is present but the hook script cannot emit an
    event, the failure is non-fatal to Droid and to Warp. The user may lose rich
    status updates, but the Droid session itself should continue.

16. If Droid hook input contains prompt text, notification text, paths, tool
    names, or any other Droid-controlled field, the hook encodes it safely before
    writing the OSC 777 notification. Untrusted text cannot inject additional
    terminal control sequences or malformed JSON into Warp. The hook emits only
    sanitized strings bounded by the technical spec and reduces tool input to a
    bounded preview rather than forwarding Droid's raw tool input object.

17. The integration applies to local Droid sessions running inside Warp. Remote,
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
   but leaves the session neutral/idle and does not create a false
   running/completed/blocked status on its own.
7. If command detection created a Droid session as in-progress before the first
   Droid `SessionStart`, that setup-only `SessionStart` changes the session to
   neutral/idle and removes any running indicator without creating completed,
   attention-needed, rich-input auto-toggle, desktop notification, or agent task
   lifecycle side effects.
8. A valid structured OSC 777 `warp://cli-agent` payload with `agent: "droid"`
   is parsed as `CLIAgent::Droid`.
9. Existing Claude Code, OpenCode, Codex, Gemini, Auggie, and Pi session-status
   tests continue to pass.
10. Droid setup instructions are visible from the same plugin-instructions UI
   used by other CLI-agent notification integrations when rich CLI-agent
   notifications are enabled.
11. The setup instructions are manual-only; Warp does not claim to auto-install
    or auto-update the Droid hook integration.
12. Droid listener registration and hook instructions are unavailable when the
    rich CLI-agent notification infrastructure is disabled.
13. Every Droid-controlled string emitted by the hook is sanitized, truncated to
    the concrete per-field limits in the technical spec, JSON-encoded, and free
    of raw terminal control bytes before being emitted in OSC 777.
14. Droid tool input is not forwarded as an unbounded object; the hook emits only
    a sanitized, bounded preview field that Warp can display.

## Validation

- Unit tests cover Droid support in the rich session listener, including support
  gating, neutral `session_start` registration for new and command-detected
  sessions, and default forwarding behavior for `stop`, `permission_request`,
  and other status events.
- Unit or integration tests cover the command-detected `session_start` reset as a
  non-status update: the running indicator clears, but no completed/blocked
  conversation status, rich-input auto-toggle, desktop notification, agent
  notification, or local task lifecycle update is produced.
- Unit tests cover the Droid plugin manager or instructions provider, including
  rollout gating, manual-only install behavior, and presence of the required
  Droid hook events.
- Unit tests cover safe hook payload construction for JSON encoding and terminal
  control-byte sanitization, concrete per-field truncation limits, final payload
  size enforcement, and bounded tool-input previews.
- Unit or parser tests confirm a structured event with `agent: "droid"` resolves
  to `CLIAgent::Droid`.
- Manual validation runs Droid in Warp with the documented hooks configured and
  records prompt-submit, running, attention-needed, tool-complete, and done
  states.
- Regression validation runs the existing listener and plugin-manager tests for
  other supported CLI agents.

## Follow-ups

- Consider a Droid plugin-based hook package in a later version. V1 uses a
  user-managed hook script registered in `~/.factory/hooks.json`.
- Revisit permission-vs-question classification if Droid exposes a structured
  notification type. V1 maps ambiguous notifications to an attention-needed
  state.
