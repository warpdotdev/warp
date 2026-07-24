# TECH.md — First-class Grok Build agent support

Issue: https://github.com/warpdotdev/warp/issues/11727  
Product spec: `specs/GH11727/product.md`

## Context

Warp’s third-party CLI agent stack is centered on `CLIAgent` in
`app/src/terminal/cli_agent.rs`. Long-running commands are detected via
`CLIAgent::detect`; matches create a `CLIAgentSession` and drive the CLI agent
footer, rich input, chrome icons, and optional plugins.

Relevant systems today:

- **Identity / detect / skills / bash / brand** — `app/src/terminal/cli_agent.rs`
- **Sessions + rich input state** — `app/src/terminal/cli_agent_sessions/`
- **OSC 777 protocol** — `cli_agent_sessions/event/` (`warp://cli-agent` sentinel;
  `agent` field must equal `command_prefix()`)
- **Listeners** — `cli_agent_sessions/listener/mod.rs` (`is_agent_supported`,
  DefaultSessionListener vs Codex OSC 9 fallback)
- **Plugin install UI** — `cli_agent_sessions/plugin_manager/` (Claude auto-install;
  OpenCode/Gemini/Codex managers)
- **Submit strategies** — `app/src/terminal/view/use_agent_footer/mod.rs`
  `rich_input_submit_strategy`
- **Proactive Codex listener** — `TerminalView` long-running path registers Codex
  without SessionStart
- **Icons** — `crates/warp_core/src/ui/icons.rs` + `app/assets/bundled/svg/`
- **Telemetry** — `CLIAgentType` in `app/src/server/telemetry/events.rs`
- **Chrome** — `app/src/ui_components/agent_icon.rs`

Grok Build (production `~/.grok/bin/grok`) already treats Warp as a first-class
terminal brand and emits **OSC 9** notifications. Lifecycle hooks can emit Warp’s
OSC 777 protocol without changing Grok Build source. Install alias `agent`
collides with Cursor’s `CLIAgent::CursorCli` prefix — detection must use `grok`
only.

## Proposed changes

### 1. `CLIAgent::Grok`

Add variant to `CLIAgent` and wire exhaustive matches:

| API | Value |
|-----|--------|
| `command_prefix` | `"grok"` |
| `display_name` | `"Grok Build"` |
| `icon` | `Icon::GrokLogo` |
| `brand_color` | near-black (similar to Codex/OpenAI) |
| `brand_icon_color` | white |
| `supported_skill_providers` | `&[SkillProvider::Agents]` |
| `skill_command_prefix` | `"/"` |
| `supports_bash_mode` | `true` |
| `From` → `CLIAgentType` | `Grok` |

Do **not** map bare `agent` → Grok. Do **not** add `Harness::Grok` in this PR.
Identifiers stay short (`CLIAgent::Grok`, `GrokPluginManager`) to match existing
agent enums (`Claude`, `Gemini`); user-facing copy uses **Grok Build**.

Detection uses the same first-token equality as other agents
(`resolved_first_word == command_prefix()`), including alias expansion and
leading env-var assignments when shell parsing is available. Absolute paths
whose last segment is `grok` are **not** special-cased (same as Claude/Codex).

### 2. Icon asset

- Add `app/assets/bundled/svg/grok.svg` — official Grok logomark paths, monochrome
  with `fill="#FF0000"` (icon red-channel alpha mask; runtime tint for light/dark).
- Do **not** ship separate light/dark SVGs; Warp recolors one asset (same as Claude
  / OpenAI). Black-filled source art is invisible under the icon shader.
- `Icon::GrokLogo` in `crates/warp_core/src/ui/icons.rs` → `bundled/svg/grok.svg`.

### 3. Listener + OSC 9

- Include `CLIAgent::Grok` in `is_agent_supported`.
- Handler: **Codex-style** dual path — parse OSC 777 `warp://cli-agent` when
  present; otherwise treat OSC 9 body as opaque `Stop` until rich notifications
  latch. OSC 9 events reuse `CLIAgentEventSource::CodexOsc9Fallback` (shared
  opaque fallback tag; only `RichPlugin` latches rich status).
- On command detection of Grok, call
  `register_cli_agent_listener_without_session_start_event(CLIAgent::Grok)`
  alongside Codex so OSC 9 works without SessionStart.

### 4. Rich input submit

- Use `RichInputSubmitStrategy::DelayedEnter` (Claude/OpenCode class) as default;
  validate against production Grok Build. Bracketed paste remains a one-line switch
  if manual testing requires it.

### 5. Plugin manager

- Add `plugin_manager/grok.rs` with **one-click auto-install** (file write, not a
  marketplace CLI). User-facing copy matches Claude/Codex (**Enable … notifications**,
  **Warp plugin installed/updated**):
  - `can_auto_install() -> true`
  - `install` / `update` write under `$GROK_HOME/hooks` (or `~/.grok/hooks`):
    - `warp-plugin.json` (SessionStart / UserPromptSubmit / Stop / StopFailure)
    - `bin/warp-plugin.sh` (OSC 777 `warp://cli-agent`, `"agent":"grok"`)
    - `warp-plugin.version` (semver for `needs_update`)
  - Naming: Claude/Codex use marketplace plugin id `warp@…-warp` (separate repos
    `claude-code-warp` / `codex-warp` — not checked into this monorepo). Grok has
    no hosted package yet, so the same conceptual **warp** plugin is written as
    `warp-plugin.*` hooks files.
  - `is_installed` / `needs_update` read those files; honors `$GROK_HOME`.
  - Manual instructions remain for sandboxed / remote sessions that force
    instruction mode.
  - **Limitation vs Claude/Codex/Gemini:** those call the agent’s own
    `plugin`/`extensions` CLI against a published Warp marketplace package
    (`claude-code-warp`, `gemini-cli-warp`, etc.). Grok Build has a plugin
    marketplace, but Warp does not yet ship a hosted Grok plugin package — so
    auto-install materializes the hooks-based Warp plugin on disk. A future
    `grok plugin install …` path can replace the file write if Warp publishes
    one.
  - Wire in `plugin_manager_for_with_shell` without a new feature flag.

### 6. Telemetry + settings

- `CLIAgentType::Grok` in telemetry events.
- Settings third-party agent dropdown picks up enum via `enum_iterator::all`.

### 7. Tests

- `cli_agent_tests`: detect `grok`, args; **not** bare `agent`; identity helpers
  (display name, icon, bash, skills prefix).
- Listener tests: Grok supported; OSC 9 / OSC 777 paths; drop OSC 9 after rich.
- Plugin manager tests: install instructions non-empty; `can_auto_install` true;
  install under `$GROK_HOME`; `needs_update` when missing/old version; env tests
  use `#[serial_test::serial]`.

## Data flow

```text
User runs `grok`
  → long-running detect → CLIAgent::Grok session
  → proactive listener (OSC 9 + optional OSC 777)
  → footer icon + toolbar + optional rich input
  → submit/images → PTY / clipboard paste into Grok TUI
  → optional hooks → OSC 777 warp://cli-agent → rich status
```

## Testing and validation

| Product invariant | Verification |
|-------------------|--------------|
| 1–3 Detect + icon | Unit detect tests; manual screenshot of footer/tabs |
| 4 Rich input submit | Manual multi-line submit; DelayedEnter unit coverage if present |
| 5 Images | Manual clipboard paste / drop |
| 6 Bash `!` | Manual |
| 7 Skills `/` | Manual slash menu filter |
| 8 No `agent` collision | Unit test |
| 9 OSC 9 without plugin | Manual unfocus turn complete |
| 10 OSC 777 rich | Unit parse body with `"agent":"grok"`; manual after install + restart |
| 11 Plugin chip | Unit install under `$GROK_HOME`; manual chip UI |
| 12 Telemetry | Compile-time enum + From mapping |
| 13 Serialization | Existing serde name helpers (variant `Grok`) |

Presubmit: `./script/format` and clippy per CONTRIBUTING / AGENTS.md.  
Manual proof required on the PR (screenshots + short recording).

## Risks

- Official SVG must stay monochrome `#FF0000` for the icon red-channel mask.
- Submit strategy may need BracketedPaste if DelayedEnter flakes on Grok.
- Hooks plugin uses lightweight JSON field extraction in bash; prompts with
  complex quoting may omit optional fields until python3 builds the body.

## Follow-ups (not this PR)

- Hosted `warpdotdev/grok-warp` (or similar) marketplace package +
  `grok plugin install …`.
- Optional rename of `CLIAgentEventSource::CodexOsc9Fallback` to an agent-neutral
  name shared by Codex and Grok.
- `Harness::Grok` for cloud/orchestration.
- ACP client for Warp Agent Mode.
