# Tech Spec: Support rich CLI agent status for Droid

**Issue:** [warpdotdev/warp#12639](https://github.com/warpdotdev/warp/issues/12639)
**Product spec:** `specs/GH12639/product.md`

## Context

Droid is already modeled as a first-class `CLIAgent`, but rich session-status
handling currently stops before Droid events can update the shared CLI-agent
session model.

Relevant current code:

- `app/src/terminal/cli_agent.rs` - defines `CLIAgent::Droid`, the `droid`
  command prefix, display name, icon, brand color, and Droid skill providers.
- `app/src/terminal/cli_agent_sessions/event/v1.rs` - parses structured
  `warp://cli-agent` OSC 777 payloads and resolves the `"agent"` string through
  `CLIAgent::command_prefix()`. This already allows `"droid"` to resolve to
  `CLIAgent::Droid`.
- `app/src/terminal/cli_agent_sessions/listener/mod.rs` - `is_agent_supported`
  currently allows Claude, OpenCode, Codex, Gemini, Auggie, and Pi. Droid is not
  included, and `create_handler` explicitly returns `None` for
  `CLIAgent::Droid`.
- `app/src/terminal/view.rs` - `handle_cli_agent_notification` parses
  `warp://cli-agent` notifications, registers a listener from the first
  structured event, then applies that event to the sessions model.
- `app/src/terminal/cli_agent_sessions/mod.rs` - `register_listener` currently
  creates or upgrades sessions with `CLIAgentSessionStatus::InProgress`.
- `app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs` -
  `plugin_manager_for_with_shell` currently returns `None` for
  `CLIAgent::Droid`, so the plugin-instructions UI has no Droid-specific setup
  flow.

Droid's documented hooks system can run user-defined commands for lifecycle
events such as `SessionStart`, `UserPromptSubmit`, `Notification`, `Stop`, and
`PostToolUse`. Hook commands receive structured JSON on stdin, including common
fields such as `session_id`, `transcript_path`, `cwd`, and
`hook_event_name`. That is enough to bridge Droid hooks to Warp's existing
structured CLI-agent event protocol without adding a new Warp-side protocol.

## Current state

The structured parser can already parse a payload like this into
`CLIAgent::Droid`:

```json
{
  "v": 1,
  "agent": "droid",
  "event": "prompt_submit",
  "session_id": "abc123"
}
```

The event is then blocked by the support gate:

1. A raw OSC 777 notification reaches Warp.
2. `parse_event` / `event::v1::parse` can resolve `agent: "droid"`.
3. The listener support check does not include Droid.
4. `create_handler(CLIAgent::Droid)` returns `None`.
5. The session listener is not registered, so future Droid rich-status events do
   not update the session model.

Separately, the plugin manager has no Droid manager. Even if Droid can emit the
right protocol through hooks, Warp does not show the user Droid-specific manual
setup instructions from the plugin UI.

One additional registration detail matters for this implementation:
`handle_cli_agent_notification` registers the listener before it calls
`update_from_event`. `register_listener` currently initializes or upgrades the
session as `InProgress`, while `DefaultSessionListener` and
`CLIAgentSession::apply_event` both treat `SessionStart` as setup metadata. Droid
must not inherit a false running state from listener registration alone.

Command detection can also create a Droid session before the hook emits its
first structured notification. Those command-detected sessions are currently
`InProgress`, have no listener, and have not received any status-bearing rich
event. A setup-only Droid `SessionStart` must clear that false running state
rather than preserve it.

## Proposed changes

### 1. Enable Droid in the session listener

Update `app/src/terminal/cli_agent_sessions/listener/mod.rs`:

- Add `CLIAgent::Droid` to `is_agent_supported` behind the existing
  `FeatureFlag::HOANotifications` check.
- Add `CLIAgent::Droid` to the `DefaultSessionListener` match arm in
  `create_handler`.
- Keep `DefaultSessionListener` behavior unchanged: skip `SessionStart`, forward
  all other structured events to `CLIAgentSessionsModel`.

Droid should use the default listener because its hook integration emits the
same structured Warp CLI-agent events as other default-listener agents. It does
not need Codex's OSC 9 fallback handling.

### 2. Keep `SessionStart` registration neutral

Update `app/src/terminal/cli_agent_sessions/mod.rs` and the
`handle_cli_agent_notification` registration path so listener registration can
distinguish setup-only `SessionStart` from status-bearing events.

The implementation should add a non-running state to the CLI-agent session
model, named `CLIAgentSessionStatus::Idle` in this spec. `Idle` means a rich
listener is registered and context may be known, but the agent has not emitted a
status-bearing event for the current turn.

Required behavior:

- Track whether the current Droid session has already applied a status-bearing
  rich event. This can be a new boolean such as
  `has_received_status_bearing_rich_event`; it must be separate from the
  existing `received_rich_notification` concept because `SessionStart` is a rich
  notification but is not status-bearing.
- Status-bearing rich events are the events that produce an
  `InProgress`, `Blocked`, or `Success` session status after
  `CLIAgentSession::apply_event`: `PromptSubmit`, `PermissionRequest`,
  `QuestionAsked`, `PermissionReplied` after blocked, `ToolComplete` after
  blocked, and `Stop`. `SessionStart`, `IdlePrompt`, malformed events, and
  ignored unknown events do not set the status-bearing marker.
- When `register_listener` is triggered by Droid `SessionStart`, a new Droid
  session is created with `CLIAgentSessionStatus::Idle` instead of
  `InProgress`.
- When Droid `SessionStart` updates an existing command-detected Droid session,
  it attaches listener/plugin context and changes the session to `Idle`, even if
  the existing status is `InProgress`.
- An existing status may be preserved on Droid `SessionStart` only when the
  stored session already applied a status-bearing rich event for the same Droid
  session. The same-session check requires both stored and incoming
  `session_id` values to be present and equal. If either side lacks a
  `session_id`, or if the ids differ, treat the `SessionStart` as setup for a
  fresh session: reset the status-bearing marker and set the status to `Idle`.
  No other existing status, including command-detected `InProgress`, may be
  preserved across a setup-only `SessionStart`.
- `SessionStart` continues to seed `cwd`, `project`, `session_id`, and
  `plugin_version`.
- Droid `SessionStart` never emits an `InProgress`, `Blocked`, or `Success`
  status and never updates agent conversation history to
  `ConversationStatus::InProgress`.
- If a setup-only `SessionStart` changes an existing command-detected session
  from `InProgress` to `Idle`, the model must notify status surfaces so the
  running spinner is cleared. This can be done with `StatusChanged(Idle)` or an
  equivalent `SessionUpdated` notification, but the UI projection for `Idle`
  must be no conversation status (`None`), not
  `ConversationStatus::InProgress`.
- Update the projection used by status surfaces, such as
  `CLIAgentSessionStatus::to_conversation_status` or a wrapper around it, so
  `Idle` returns `None`. Do not map `Idle` to `InProgress`, `Blocked`, or
  `Success`.
- `PromptSubmit`, `ToolComplete` after blocked, `PermissionReplied` after
  blocked, `PermissionRequest`, `QuestionAsked`, and `Stop` remain
  status-bearing and move the session out of `Idle`. The status-bearing marker
  is set only after one of these events actually changes the session status.

This is intentionally a model/UI change, not just a listener change. Dropping
`SessionStart` inside `DefaultSessionListener` is not sufficient because the
initial registration path runs before the default listener observes future
events.

### 3. Add a Droid plugin manager for manual instructions

Add `app/src/terminal/cli_agent_sessions/plugin_manager/droid.rs` and register
it in `plugin_manager/mod.rs`.

The Droid manager should implement `CliAgentPluginManager` with:

- `minimum_plugin_version() -> "1.0.0"` or another initial version chosen during
  implementation.
- `can_auto_install() -> false`.
- `supports_update() -> false` unless implementation later provides a reliable
  on-disk version check.
- `install_instructions()` and `update_instructions()` returning the same manual
  instruction set.
- no override for `install()` / `update()`, so the default unsupported
  auto-install behavior remains in place.

`plugin_manager_for_with_shell` should return `Some(DroidPluginManager)` only
when `FeatureFlag::HOANotifications` is enabled. Do not add a new
Droid-specific feature flag for this first version; Droid shares the existing
rich CLI-agent notification rollout gate.

Manual instructions should guide the user to:

1. Create a local POSIX shell hook script that reads Droid's hook JSON from
   stdin and uses `jq` for JSON parsing, sanitization, truncation, and payload
   construction.
2. Exit without emitting anything when Droid is not running inside Warp, such as
   when `WARP_CLI_AGENT_PROTOCOL_VERSION` is absent.
3. Map supported Droid hook events to Warp CLI-agent events.
4. Write an OSC 777 notification to the terminal in Warp's existing
   `warp://cli-agent` format.
5. Register the hook command in Droid's documented hooks configuration.
6. Restart Droid or start a new Droid session so the hooks are active.

The implementation should align the configuration snippet with Droid's current
documented hooks schema. As of the referenced Factory docs, user hooks live in
`~/.factory/hooks.json` and are structured under a top-level `"hooks"` object,
with event names such as `UserPromptSubmit`, `Notification`, `Stop`, and
`PostToolUse` under that object. Events that do not use matchers can omit the
`matcher` field.

### 4. Event mapping, payload shape, and output safety

The hook script should emit Warp event names using this mapping:

- `SessionStart` -> `session_start`
- `UserPromptSubmit` -> `prompt_submit`
- `Notification` -> `permission_request` or `question_asked`
- `Stop` -> `stop`
- `PostToolUse` -> `tool_complete`

The emitted JSON payload should include:

- `v`: the supported protocol version, clamped or defaulted to `1`
- `agent`: `"droid"`
- `event`: the mapped Warp event name
- `session_id`: Droid `session_id`, when present
- `cwd`: Droid `cwd`, when present
- `project`: best-effort project name derived from `cwd`, when present
- `plugin_version`: the Droid hook bridge version

Event-specific fields should be populated conservatively:

- `prompt_submit`: include `query` from Droid's `prompt`, when present.
- `permission_request` / `question_asked`: include `summary` from Droid's
  notification `message`, when present.
- `tool_complete`: include `tool_name` and a bounded tool-input preview when
  useful. Do not forward Droid's raw `tool_input` object. If a preview is
  available, emit a small `tool_input` object with only one parser-supported
  string key, preferring `command` and falling back to `file_path`.
- `stop`: no additional payload is required.

For `Stop`, the hook should respect Droid's `stop_hook_active` field and avoid
emitting recursive or misleading stop notifications when Droid is continuing as
part of a stop-hook flow.

The hook bridge must treat all Droid-provided strings and JSON fields as
untrusted input:

- Build the v1 manual hook's OSC 777 payload with `jq`, not with
  hand-concatenated JSON strings.
- Remove terminal control characters from every string that can reach the OSC
  777 output, including prompt text, notification messages, paths, project
  names, tool names, and tool-input previews.
- Ensure the final write to `/dev/tty` cannot emit extra BEL/ST terminators or
  nested escape sequences sourced from Droid-controlled fields.

Use this exact sanitization and bounding policy for every string emitted by the
hook:

1. Convert only the known fields below to strings. Do not serialize arbitrary
   Droid objects into the Warp payload.
2. Remove Unicode control characters in `U+0000..U+001F` and `U+007F..U+009F`
   before truncation. This specifically removes raw ESC, BEL, and C1 string
   terminator characters from Droid-controlled data.
3. Truncate at the last complete Unicode scalar value at or before the listed
   limit. Do not append an ellipsis or other marker after truncation.
4. JSON-encode the sanitized values with compact `jq -c` output, using
   `jq --arg` and `jq --argjson` for value injection.
5. After JSON encoding, the body portion of the OSC 777 notification must be at
   most 8192 UTF-8 bytes. If the body is larger, drop optional fields in this
   order and re-encode after each step: `tool_input`, `transcript_path`, then
   event-specific text (`query` or `summary`). If the required fields still do
   not fit, skip emitting that notification rather than writing an oversized
   payload.

Per-field limits:

| Payload field | Source | Limit |
| --- | --- | --- |
| `session_id` | Droid `session_id` | 128 Unicode scalar values |
| `cwd` | Droid `cwd` or `FACTORY_PROJECT_DIR` | 1024 Unicode scalar values |
| `project` | basename derived from sanitized `cwd` | 128 Unicode scalar values |
| `transcript_path` | Droid `transcript_path` | 1024 Unicode scalar values |
| `plugin_version` | hook bridge constant | 32 Unicode scalar values |
| `query` | Droid `prompt` | 2048 Unicode scalar values |
| `summary` | Droid notification `message` | 2048 Unicode scalar values |
| `tool_name` | Droid `tool_name` | 128 Unicode scalar values |
| `tool_input.command` or `tool_input.file_path` | bounded preview extracted from Droid tool input | 2048 Unicode scalar values |

`agent` and `event` are fixed hook-generated constants, not Droid-controlled
strings. The hook should not emit `response` in v1.

The final terminal write must contain raw ESC and BEL only as the OSC wrapper
delimiters: `ESC ] 777 ; notify ; warp://cli-agent ; <json> BEL`. The encoded
JSON body must not contain raw bytes in `0x00..0x1F` or `0x7F`. JSON escape
sequences such as `\u001b` are not sufficient by themselves; Droid-controlled
control characters must be removed before encoding so Warp never stores or
renders those controls after JSON parsing.

### 5. Keep other agents unchanged

Do not change:

- Codex's `CodexSessionHandler` and OSC 9 fallback behavior.
- Existing plugin managers for Claude, OpenCode, Codex, or Gemini.
- Auggie and Pi listener-only support.
- Droid command detection, display metadata, skill providers, or bash-mode
  support.

## End-to-end flow

1. User installs/configures the Droid hook integration from Warp's manual
   instructions.
2. User starts Droid inside Warp.
3. Droid invokes `SessionStart`; the hook emits a safely encoded structured OSC
   777 `session_start` payload with `agent: "droid"`.
4. Warp parses the event, recognizes `CLIAgent::Droid`, and creates a Droid
   default session listener.
5. Warp registers the listener in `Idle` state, seeds context from
   `SessionStart`, and does not show the running spinner. If command detection
   had already created an `InProgress` Droid session for the same terminal, this
   step changes that session to `Idle` and clears the spinner.
6. User submits a prompt. Droid invokes `UserPromptSubmit`; the hook emits
   `prompt_submit`.
7. Warp forwards the event to `CLIAgentSessionsModel`, and the Droid session is
   shown as in progress.
8. If Droid needs input or permission, Droid invokes `Notification`; the hook
   emits either `permission_request` or `question_asked`, and Warp shows
   attention needed.
9. If Droid completes a tool, Droid invokes `PostToolUse`; the hook emits
   `tool_complete`, and Warp can return the session to in-progress.
10. When Droid finishes, Droid invokes `Stop`; the hook emits `stop`, and Warp
    shows the session as done.

## Testing and validation

Implementation tests should include:

1. `app/src/terminal/cli_agent_sessions/listener/mod_tests.rs`
   - `droid_is_supported_when_hoa_notifications_enabled`:
     `is_agent_supported(&CLIAgent::Droid)` is true when
     `FeatureFlag::HOANotifications` is enabled.
   - `droid_is_unsupported_when_hoa_notifications_disabled`:
     `is_agent_supported(&CLIAgent::Droid)` is false when
     `FeatureFlag::HOANotifications` is disabled.
   - Droid with `DefaultSessionListener` skips `SessionStart`.
   - Droid with `DefaultSessionListener` forwards `Stop`.
   - Droid with `DefaultSessionListener` forwards `PermissionRequest`.
   - Optionally cover `QuestionAsked` or `ToolComplete` if local test helpers
     make that clearer than relying on existing default-listener coverage.

2. `app/src/terminal/cli_agent_sessions/mod_tests.rs`
   - Registering a listener from a Droid `SessionStart` creates or upgrades a
     session in `Idle`, not `InProgress`.
   - If command detection already created an `InProgress` Droid session with no
     status-bearing rich event, registering from Droid `SessionStart` changes it
     to `Idle` and notifies status surfaces so the running spinner clears.
   - If the stored session has already applied a status-bearing rich event with
     the same non-empty `session_id`, a later duplicate Droid `SessionStart`
     keeps that status for that same session.
   - If a Droid `SessionStart` has a different `session_id` from the stored
     session, the status-bearing marker resets and the session becomes `Idle`.
   - Applying that same `SessionStart` seeds context/plugin version but emits no
     `InProgress`, `Blocked`, or `Success` status and does not project to
     `ConversationStatus::InProgress`.
   - A later `PromptSubmit` moves the session from `Idle` to `InProgress`.
   - A later `PermissionRequest` or `QuestionAsked` moves the session from
     `Idle` or `InProgress` to `Blocked`.
   - The status-bearing marker remains false for `SessionStart` and `IdlePrompt`
     and becomes true only after an applied rich event changes status.

3. `app/src/terminal/view_tests.rs`
   - A Droid `SessionStart` with rich notifications enabled creates the listener
     but leaves the vertical-tab status indicator without an in-progress
     spinner.
   - A command-detected Droid session that was showing an in-progress spinner
     clears that spinner when the first rich event is setup-only `SessionStart`.
   - A later Droid `prompt_submit` shows the in-progress spinner.

4. `app/src/terminal/cli_agent_sessions/plugin_manager/mod_tests.rs`
   - `plugin_manager_for(CLIAgent::Droid)` returns `Some` when
     `FeatureFlag::HOANotifications` is enabled.
   - `plugin_manager_for(CLIAgent::Droid)` returns `None` when
     `FeatureFlag::HOANotifications` is disabled.
   - Droid is removed from the unsupported-agents assertion.

5. `app/src/terminal/cli_agent_sessions/plugin_manager/droid_tests.rs`
   - `can_auto_install()` is false.
   - `supports_update()` is false.
   - minimum version is stable and explicit.
   - install instructions contain the required Droid hook events:
     `SessionStart`, `UserPromptSubmit`, `Notification`, `Stop`, and
     `PostToolUse`.
   - install instructions mention or encode the documented hooks configuration
     shape.
   - install instructions mention the required `jq` dependency and the hook
     script exits non-fatally when `jq` is unavailable.
   - hook payload construction uses compact JSON encoder output and sanitizes
     terminal control bytes from Droid-controlled strings before writing OSC 777.
   - hook payload tests cover the exact per-field limits for `session_id`, `cwd`,
     `project`, `transcript_path`, `query`, `summary`, `tool_name`, and
     tool-input preview.
   - hook payload tests verify truncation happens at a complete Unicode scalar
     boundary and does not append an ellipsis.
   - hook payload tests verify the final encoded JSON body is at most 8192 UTF-8
     bytes or no notification is emitted.
   - hook payload tests verify raw Droid `tool_input` objects are not forwarded;
     only a sanitized bounded preview under `tool_input.command` or
     `tool_input.file_path` is emitted.
   - hook payload tests inject ESC, BEL, C1 controls, and embedded OSC
     terminators into prompt text, notification text, paths, tool names, and
     tool input, then assert the final bytes written to `/dev/tty` contain no raw
     terminal control bytes except the required OSC wrapper delimiters.

6. Parser coverage, if not already covered elsewhere:
   - A structured v1 payload with `agent: "droid"` resolves to
     `CLIAgent::Droid`.
   - A malformed or unsupported Droid payload is ignored or treated as unknown
     without panicking.

Recommended commands before opening the implementation for review:

```bash
cargo fmt --check
cargo test -p warp --lib terminal::cli_agent_sessions::listener::tests
cargo test -p warp --lib terminal::cli_agent_sessions::tests
cargo test -p warp --lib terminal::cli_agent_sessions::plugin_manager::tests
cargo test -p warp --lib terminal::cli_agent_sessions::plugin_manager::droid::tests
cargo test -p warp --lib terminal::view::tests
```

Manual validation:

- Run WarpOss locally.
- Configure the Droid hook integration from the instructions.
- Start Droid inside Warp.
- Submit a prompt and confirm the vertical tab enters the running state.
- Trigger a permission/input notification and confirm the tab indicates
  attention needed.
- Allow Droid to complete a tool and continue.
- Let Droid finish and confirm the tab returns to done/completed.
- Attach a short screen recording to the implementation PR.

## Risks and mitigations

- Risk: Droid's hooks configuration schema changes or differs across versions.
  Mitigation: keep setup instructions aligned with Droid's documented
  `~/.factory/hooks.json` schema and include manual validation in the
  implementation PR.

- Risk: `Notification` does not expose a perfect machine-readable distinction
  between permission requests and generic input-needed prompts. Mitigation: use
  conservative heuristics and map ambiguous notifications to an attention-needed
  state rather than completion. Revisit the distinction if Droid adds a more
  specific field.

- Risk: the hook script fails because `jq` is unavailable or `/dev/tty` cannot
  be written. Mitigation: document the dependency, fail non-fatally, and avoid
  breaking the Droid session when rich status cannot be emitted.

- Risk: Droid-controlled text could inject malformed JSON or terminal control
  sequences into OSC 777 output. Mitigation: build payloads with a JSON encoder,
  apply the per-field limits above, remove terminal control characters before
  encoding, avoid forwarding raw `tool_input`, enforce the 8192-byte JSON body
  cap, and test the final bytes written to `/dev/tty`.

- Risk: a setup-only `SessionStart` could make Warp display Droid as running.
  Mitigation: add an explicit idle registration state, track whether a
  status-bearing rich event has actually applied to the current Droid
  `session_id`, and test both new sessions and command-detected `InProgress`
  sessions being reset to `Idle` by setup-only `SessionStart`.

- Risk: adding Droid to the default listener forwards unexpected event types.
  Mitigation: the event parser already normalizes known event names, and the
  default listener behavior is shared by other structured-event agents. Add
  Droid-specific listener tests for key status events.

- Risk: users expect one-click install because other agents support it.
  Mitigation: `can_auto_install()` remains false and the UI presents this as a
  manual setup flow.

## Follow-ups

- Consider a Droid plugin package if Droid's plugin hook mechanism is preferred
  over user-managed hook scripts.
- Add version detection and update prompts if the hook bridge becomes
  auto-installable or has a stable on-disk location.
- Revisit permission-vs-question classification if Droid exposes a structured
  notification type.
- Evaluate remote, SSH, and tmux behavior separately from this local Droid hook
  integration.
