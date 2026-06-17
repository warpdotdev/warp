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

- When `register_listener` is triggered by `SessionStart`, a new Droid session is
  created with `CLIAgentSessionStatus::Idle` instead of `InProgress`.
- When a `SessionStart` updates an existing Droid session, the update attaches
  listener/plugin context but preserves the existing status. Only a newly
  created setup-only session starts in `Idle`.
- `SessionStart` continues to seed `cwd`, `project`, `session_id`, and
  `plugin_version`.
- `SessionStart` does not emit `StatusChanged` and does not update agent
  conversation history to `ConversationStatus::InProgress`.
- Vertical tabs and other status surfaces render an idle listener registration
  without the running spinner. If they need a conversation-status projection,
  `Idle` should project to no status (`None`) rather than to
  `ConversationStatus::InProgress`.
- `PromptSubmit`, `ToolComplete` after blocked, `PermissionReplied` after
  blocked, `PermissionRequest`, `QuestionAsked`, and `Stop` remain
  status-bearing and move the session out of `Idle`.

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

1. Create a local hook script that reads Droid's hook JSON from stdin.
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

- `prompt_submit`: include a bounded `query` from Droid's `prompt`, when
  present.
- `permission_request` / `question_asked`: include a `summary` from Droid's
  notification `message`, when present.
- `tool_complete`: include `tool_name` and a tool-input preview when useful and
  already supported by the parser.
- `stop`: no additional payload is required.

For `Stop`, the hook should respect Droid's `stop_hook_active` field and avoid
emitting recursive or misleading stop notifications when Droid is continuing as
part of a stop-hook flow.

The hook bridge must treat all Droid-provided strings and JSON fields as
untrusted input:

- Build the OSC 777 payload with a JSON encoder such as `jq`, not with
  hand-concatenated JSON strings.
- Bound long string fields such as `query` and `summary` before including them
  in the payload.
- Strip or escape terminal control bytes from every string that can reach the
  OSC 777 output, including prompt text, notification messages, paths, project
  names, tool names, and tool-input previews.
- Ensure the final write to `/dev/tty` cannot emit extra BEL/ST terminators or
  nested escape sequences sourced from Droid-controlled fields.

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
   777
   `session_start` payload with `agent: "droid"`.
4. Warp parses the event, recognizes `CLIAgent::Droid`, and creates a Droid
   default session listener.
5. Warp registers the listener in `Idle` state, seeds context from
   `SessionStart`, and does not show the running spinner.
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
   - Applying that same `SessionStart` seeds context/plugin version but emits no
     `StatusChanged` and does not project to `ConversationStatus::InProgress`.
   - A later `PromptSubmit` moves the session from `Idle` to `InProgress`.
   - A later `PermissionRequest` or `QuestionAsked` moves the session from
     `Idle` or `InProgress` to `Blocked`.

3. `app/src/terminal/view_tests.rs`
   - A Droid `SessionStart` with rich notifications enabled creates the listener
     but leaves the vertical-tab status indicator without an in-progress
     spinner.
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
   - install instructions mention required dependencies such as `jq`, if the
     hook script uses them.
   - hook payload construction uses a JSON encoder and sanitizes terminal
     control bytes from Droid-controlled strings before writing OSC 777.

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
  bound long fields, and strip or escape terminal control bytes before writing
  the notification.

- Risk: a setup-only `SessionStart` could make Warp display Droid as running.
  Mitigation: add an explicit idle registration state and tests proving
  `SessionStart` does not produce an in-progress conversation status.

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
