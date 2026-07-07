# Codex Warp Plugin Tech Spec

## Context
Warp already supports CLI agent status via structured OSC 777 plugin notifications. Codex is special because it previously only had native OSC 9 desktop notifications with plain text. This branch adds a feature-flagged Codex Warp plugin path while preserving OSC 9 as fallback.
The key invariant changes:
- `listener.is_some()` no longer means “structured plugin connected” for Codex. It can mean OSC 9 fallback listener.
- Structured Codex plugin events must win over OSC 9 once seen. Otherwise Warp would process both OSC 777 and OSC 9 and notify twice.
- UI that needs trustworthy state should use `CLIAgentSession::supports_rich_status()`, not `listener.is_some()`.
Relevant code:
- [`app/src/terminal/cli_agent_sessions/listener/mod.rs:14`](https://github.com/warpdotdev/warp/blob/68acdf7601cc1824d4f4dd250485aca8329efc17/app/src/terminal/cli_agent_sessions/listener/mod.rs#L14) — `CLIAgentSessionHandler` now mutably parses notifications so Codex can remember structured-plugin activation.
- [`app/src/terminal/cli_agent_sessions/listener/mod.rs:87`](https://github.com/warpdotdev/warp/blob/68acdf7601cc1824d4f4dd250485aca8329efc17/app/src/terminal/cli_agent_sessions/listener/mod.rs#L87) — `CodexSessionHandler` parses both OSC 9 fallback and structured OSC 777 Codex events.
- `app/src/terminal/cli_agent_sessions/mod.rs:155` — `has_structured_plugin()` distinguishes structured OSC 777 from Codex OSC 9 fallback.
- `app/src/terminal/cli_agent_sessions/mod.rs:164` — `supports_rich_status()` centralizes the “safe to show fine-grained status” check.
- [`app/src/terminal/cli_agent_sessions/plugin_manager/codex.rs:17`](https://github.com/warpdotdev/warp/blob/68acdf7601cc1824d4f4dd250485aca8329efc17/app/src/terminal/cli_agent_sessions/plugin_manager/codex.rs#L17) — Codex plugin constants: marketplace, user plugin key, platform plugin key, config dirs, min version.
- [`app/src/terminal/cli_agent_sessions/plugin_manager/codex.rs:57`](https://github.com/warpdotdev/warp/blob/68acdf7601cc1824d4f4dd250485aca8329efc17/app/src/terminal/cli_agent_sessions/plugin_manager/codex.rs#L57) — plugin manager gates install/update/inspection on `FeatureFlag::CodexPlugin`.
- [`app/src/terminal/cli_agent_sessions/plugin_manager/claude.rs:17`](https://github.com/warpdotdev/warp/blob/68acdf7601cc1824d4f4dd250485aca8329efc17/app/src/terminal/cli_agent_sessions/plugin_manager/claude.rs#L17) — Claude manager structure copied for Codex, with CLI/config differences.
- [`app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs:222`](https://github.com/warpdotdev/warp/blob/68acdf7601cc1824d4f4dd250485aca8329efc17/app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs#L222) — Codex manager now receives shell path/type/PATH like Claude/Gemini.
- `app/src/terminal/view.rs:11680` — command-detected Codex still proactively registers a listener, but without seeding a plugin version.
- `app/src/terminal/view.rs:12753` — OSC 777 Codex events are ignored when `CodexPlugin` is disabled.
- `app/src/terminal/view.rs:12825` — listener registration without `SessionStart` never seeds a plugin version. Codex remains OSC 9 fallback until a real structured plugin event reports version.
- [`crates/warp_features/src/lib.rs:789`](https://github.com/warpdotdev/warp/blob/68acdf7601cc1824d4f4dd250485aca8329efc17/crates/warp_features/src/lib.rs#L789) and [`app/src/features.rs:480`](https://github.com/warpdotdev/warp/blob/68acdf7601cc1824d4f4dd250485aca8329efc17/app/src/features.rs#L480) — new `CodexPlugin` flag is wired into shared/app feature plumbing.

## Proposed changes
### 1. Add `CodexPlugin` flag
Add `FeatureFlag::CodexPlugin` and enable it for dogfood builds.
When disabled:
- Codex keeps the existing native OSC 9 behavior.
- structured Codex events are ignored.
- install instructions remain the old “enable native Codex notifications” steps.
- auto-install/update are disabled.
When enabled:
- Warp can install/update `warp@codex-warp`.
- structured OSC 777 events unlock rich status.
- OSC 9 remains fallback for older Codex clients.

This means we can test this before releasing into the wild.

### 2. Make Codex listener protocol-aware
`CLIAgentSessionHandler::try_parse` now takes `&mut self`. Codex uses that state to remember when it has seen a structured Codex plugin event.
Codex parsing rules:
- Try `parse_event(title, body)` first.
- If it is a Codex structured event and `CodexPlugin` is enabled, mark `structured_plugin_active = true` and forward it.
- If it is a Codex structured event but the flag is disabled, drop it and leave OSC 9 fallback active.
- If it is a structured event for another agent, drop it.
- If it is OSC 9 (`title == None`) and no structured plugin has been seen, convert text to `Stop`.
- If structured plugin is active, ignore later OSC 9 so Warp does not emit duplicate status/notifications.

### 3. Move rich-status checks onto session state
The old `agent_supports_rich_status(agent)` helper was static and could not distinguish Codex plugin from Codex OSC 9 fallback.
`CLIAgentSession` now owns the distinction:
- `has_structured_plugin()` is true when a listener exists and, for Codex, a plugin version exists.
- `supports_rich_status()` delegates to `has_structured_plugin()`.
This works because the Codex structured plugin reports a version on `session_start`, while OSC 9 fallback does not.
`apply_event()` now records `payload.plugin_version` from any event, and `update_from_event()` emits `SessionUpdated` when the version changes. This lets footer/UI surfaces re-evaluate chip/status state after the structured plugin connects.

### 4. Keep Codex command detection fallback
Codex command detection still registers a listener immediately, because OSC 9 has no `SessionStart` sentinel.
Important distinction:
- Command-detected Codex calls `register_cli_agent_listener_without_session_start_event(CLIAgent::Codex, ctx)`.
- `RegisterPluginListener` after install/update uses the same helper and also does not seed a plugin version.
- Installing the plugin mid-session does not make the running Codex process emit OSC 777. Codex must be restarted to load hooks and start structured notifications.
- Until restart, Warp should keep treating the session as OSC 9 fallback. Disk install state can drive install/update/restart UI, but not `has_structured_plugin()`.

### 5. Update UI consumers to use rich-status semantics
All surfaces that previously treated `listener.is_some()` plus static agent support as rich-status now use `session.supports_rich_status()`:
- footer plugin-chip suppression and update-chip logic
- auto show/hide of rich input
- close-on-submit behavior
- terminal icon status
- vertical tabs summary/detail status
This prevents OSC 9 fallback Codex sessions from showing precise statuses or suppressing plugin install chips too early.

### 6. Add Codex plugin manager
The Codex manager follows the Claude manager shape:
- shell-aware `LocalCommandExecutor`
- PATH override support for tools installed by shell managers
- marketplace add/remove/install/update flow
- local install detection
- cached manifest version detection
- platform plugin install hook

Key Codex differences:
- CLI binary is `codex`, not `claude`.
- User plugin command is `codex plugin add warp@codex-warp`, not `claude plugin install warp@claude-code-warp`.
- Update removes marketplace `codex-warp`, re-adds `warpdotdev/codex-warp`, then runs `codex plugin add warp@codex-warp`.
- User plugin key is `warp@codex-warp`.
- Config root is `$CODEX_HOME` or `~/.codex`.
- Install state reads `[plugins."warp@codex-warp"].enabled = true` from `config.toml`.
- Version state reads cached manifests under `plugins/cache/codex-warp/warp/*/.codex-plugin/plugin.json`.
- Success copy says restart Codex, not `/reload-plugins`.

## Testing and validation
Unit coverage added/updated:
- `CodexSessionHandler` parses OSC 9 text as `Stop`.
- empty OSC 9 bodies are ignored.
- titled non-structured notifications are ignored.
- structured Codex events are ignored when `CodexPlugin` is disabled.
- after one structured Codex event, later OSC 9 is ignored.
- structured events for other agents are ignored by the Codex handler.
- Codex plugin manager toggles install/update support by `CodexPlugin`.
- plugin install/update instruction commands match Codex command names.
- native fallback instructions remain when `CodexPlugin` is off.
- install detection reads `config.toml`.
- version detection picks latest cached plugin manifest version.
- `CODEX_HOME` overrides the default `~/.codex` path in install/update checks.

Suggested local validation:
- `cargo test -p warp --features test-util codex`
- `cargo test -p warp --features test-util cli_agent_sessions`
- Run Warp with `CodexPlugin` disabled, start Codex with native notifications, verify one completion notification and no rich status.
- Run Warp with `CodexPlugin` enabled and plugin active after Codex restart, verify permission requests show rich blocked state.
- In plugin mode, verify Codex emitting both OSC 777 and OSC 9 does not produce duplicate notifications.
- Verify footer install/update/restart UI remains visible for OSC 9 fallback, then hides after structured plugin connects after restart.
- Verify vertical tabs and agent icon do not show rich status for OSC 9 fallback.

## Parallelization
No sub-agents recommended for implementation. The change is tightly coupled around one invariant: whether a Codex session is OSC 9 fallback or structured OSC 777 plugin-backed.
Best review split:
- Protocol/session review: `listener/mod.rs`, `cli_agent_sessions/mod.rs`, `terminal/view.rs`.
- Plugin manager review: `plugin_manager/codex.rs`, `codex_tests.rs`, `plugin_manager/mod.rs`.
- UI surface review: footer, rich input, agent icon, vertical tabs.
These can be reviewed in parallel, but changes should land as one PR because the protocol invariant must stay consistent across all consumers.
