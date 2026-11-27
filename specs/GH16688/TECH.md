# TECH.md — First-class `uta` CLI agent support

Issue: https://github.com/warpdotdev/warp/issues/16688
Product spec: `specs/GH16688/PRODUCT.md`

Researched against `5a55a2aca77339ad75637bed650352369ee7fcb9`.

## Context

Warp's third-party CLI agent stack is centered on `CLIAgent`. Long-running blocks are classified with `CLIAgent::detect`; a match creates a `CLIAgentSession` and drives footer, rich input, chrome icons, and OSC 777 listeners. `uta` ([unleashtheagents/uta](https://github.com/unleashtheagents/uta)) is absent from that enum today, so `uta` sessions remain generic terminals.

Relevant systems at this SHA:

- Identity / detect / skills / bash / brand — `app/src/terminal/cli_agent.rs`
- OSC 777 protocol (`warp://cli-agent`) — `app/src/terminal/cli_agent_sessions/event/mod.rs` and `v1.rs` (`resolve_agent` matches `"agent"` against `command_prefixes()`)
- Listeners — `app/src/terminal/cli_agent_sessions/listener/mod.rs`
- Plugin install UI — `app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs`
- Rich-input submit — `app/src/terminal/view/use_agent_footer/mod.rs`
- Icons — `crates/warp_core/src/ui/icons.rs`; assets in `app/assets/bundled/svg/`

Upstream `uta` facts (verified against [`unleashtheagents/uta@81299e0`](https://github.com/unleashtheagents/uta/commit/81299e05260a456a30bcdb48d29f8b8f2632b04d)):

- Single static Go binary named `uta` on macOS, Linux, and Windows (`brew install unleashtheagents/tap/uta` / `go install github.com/unleashtheagents/uta/cmd/uta@latest`). Bare `uta` opens the interactive agent shell (`uta shell`); `uta run "…"` runs one-shot fan-out orchestration. First-token basename equality covers all subcommands.
- Interactive line editor (`internal/cli/lineedit.go`) uses `bufio.NewReader` in raw mode without enabling bracketed paste (`CSI ? 2004 h`) and treats `\r` or `\n` as immediate line submission even when arriving in the same write buffer as preceding text — matching `RichInputSubmitStrategy::Inline`.
- Bang-shell (`!`) is a first-class shell action (`internal/cli/shell.go`: lines starting with `!` run locally in the workdir via `LocalExec`).
- Skills engine (`internal/skills/skills.go`) natively loads `.claude/skills` and `.agents/skills` (`SkillProvider::Agents`, `SkillProvider::Claude`) and uses `/` for slash commands in `uta shell`.
- Native Warp OSC 777 emission (`internal/cli/title.go`): when `TERM_PROGRAM=WarpTerminal` and stdout is a TTY, `uta` emits `\x1b]777;warp://cli-agent;{"v":1,"agent":"uta","event":"..."}\x07` for `session_start`, `prompt_submit`, `stop`, and `stop_failure`. No Warp-managed hook installer is required.

## Proposed changes

### 1. `CLIAgent::Uta` (`app/src/terminal/cli_agent.rs`)

Add `Uta` next to `Muse` and wire every exhaustive match:

| API | Value |
|-----|-------|
| `command_prefixes` | `&["uta"]` |
| `display_name` | `"uta"` |
| `icon` | `Some(Icon::UtaLogo)` |
| `brand_color` | `UTA_PURPLE` (`ColorU { r: 124, g: 92, b: 255, a: 255 }`, `#7C5CFF`) |
| `brand_icon_color` | white (default arm) |
| `supported_skill_providers` | `&[SkillProvider::Agents, SkillProvider::Claude]` |
| `skill_command_prefix` | `"/"` |
| `supports_bash_mode` | `true` |
| `supports_cli_agent_footer` | `true` (default) |
| `telemetry_name` | `"Uta"` |

### 2. Icon asset (`app/assets/bundled/svg/uta.svg` & `crates/warp_core/src/ui/icons.rs`)

- Bundled stencil: `app/assets/bundled/svg/uta.svg` — 24×24 `viewBox`, single `<path>`, `fill="#FF0000"` (red-channel alpha mask), converted from the MIT-licensed official mark at `https://unleashtheagents.ai/favicon.svg`.
- Add `Icon::UtaLogo` in `crates/warp_core/src/ui/icons.rs` and map it to `"bundled/svg/uta.svg"`.

### 3. Listener (`app/src/terminal/cli_agent_sessions/listener/mod.rs`)

- Include `CLIAgent::Uta` in `is_agent_supported`.
- Route `CLIAgent::Uta` to `DefaultSessionListener` (same class as `OhMyPi` and `Muse`). `v1::resolve_agent` automatically resolves `"agent":"uta"` via `CLIAgent::Uta.command_prefixes()`.

### 4. Rich input submit (`app/src/terminal/view/use_agent_footer/mod.rs`)

- Route `CLIAgent::Uta` to `RichInputSubmitStrategy::Inline` so Warp writes the prompt bytes followed by `\r` without bracketed-paste escape sequences (`\x1b[200~`), matching `uta`'s raw-mode `lineedit.go` reader.

### 5. No Warp-managed plugin (`app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs`)

- Add `CLIAgent::Uta` to the `None` arm in `plugin_manager_for_with_shell`. `uta` emits `warp://cli-agent` OSC 777 events natively when `TERM_PROGRAM=WarpTerminal`.

### 6. Unit tests

- `app/src/terminal/cli_agent_tests.rs`: `uta` and `uta run "plan and fan out"` detect as `CLIAgent::Uta`; `test_uta_public_configuration` asserts `command_prefix`, `display_name`, `supports_bash_mode`, `skill_command_prefix`, `icon`, and `supported_skill_providers`.
- `app/src/terminal/cli_agent_sessions/listener/mod_tests.rs`: `uta_is_supported_via_default_listener` verifies `is_agent_supported(&CLIAgent::Uta)`, parses `{"v":1,"agent":"uta","event":"stop","session_id":"abc12345"}`, and confirms opaque OSC 9 text is ignored.
- `app/src/terminal/cli_agent_sessions/plugin_manager/mod_tests.rs`: asserts `plugin_manager_for(CLIAgent::Uta).is_none()`.
- `app/src/terminal/view/use_agent_footer/mod_tests.rs`: asserts `rich_input_submit_strategy(CLIAgent::Uta) == RichInputSubmitStrategy::Inline`.
