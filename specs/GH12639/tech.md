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

## Proposed changes

### 1. Enable Droid in the session listener

Update `app/src/terminal/cli_agent_sessions/listener/mod.rs`:

- Add `CLIAgent::Droid` to `is_agent_supported`.
- Add `CLIAgent::Droid` to the `DefaultSessionListener` match arm in
  `create_handler`.
- Keep `DefaultSessionListener` behavior unchanged: skip `SessionStart`, forward
  all other structured events to `CLIAgentSessionsModel`.

Droid should use the default listener because its hook integration emits the
same structured Warp CLI-agent events as other default-listener agents. It does
not need Codex's OSC 9 fallback handling.

### 2. Add a Droid plugin manager for manual instructions

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

### 3. Event mapping and payload shape

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

### 4. Keep other agents unchanged

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
3. Droid invokes `SessionStart`; the hook emits a structured OSC 777
   `session_start` payload with `agent: "droid"`.
4. Warp parses the event, recognizes `CLIAgent::Droid`, and creates a Droid
   default session listener.
5. `DefaultSessionListener` drops the setup-only `SessionStart` event.
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
   - `droid_is_supported`: `is_agent_supported(&CLIAgent::Droid)` is true.
   - Droid with `DefaultSessionListener` skips `SessionStart`.
   - Droid with `DefaultSessionListener` forwards `Stop`.
   - Droid with `DefaultSessionListener` forwards `PermissionRequest`.
   - Optionally cover `QuestionAsked` or `ToolComplete` if local test helpers
     make that clearer than relying on existing default-listener coverage.

2. `app/src/terminal/cli_agent_sessions/plugin_manager/mod_tests.rs`
   - `plugin_manager_for(CLIAgent::Droid)` returns `Some`.
   - Droid is removed from the unsupported-agents assertion.

3. `app/src/terminal/cli_agent_sessions/plugin_manager/droid_tests.rs`
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

4. Parser coverage, if not already covered elsewhere:
   - A structured v1 payload with `agent: "droid"` resolves to
     `CLIAgent::Droid`.
   - A malformed or unsupported Droid payload is ignored or treated as unknown
     without panicking.

Recommended commands before opening the implementation for review:

```bash
cargo fmt --check
cargo test -p warp --lib terminal::cli_agent_sessions::listener::tests
cargo test -p warp --lib terminal::cli_agent_sessions::plugin_manager::tests
cargo test -p warp --lib terminal::cli_agent_sessions::plugin_manager::droid::tests
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
