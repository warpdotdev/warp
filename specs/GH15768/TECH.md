# TECH.md — First-class Muse Code CLI agent support

Issue: https://github.com/warpdotdev/warp/issues/15768
Product spec: `specs/GH15768/PRODUCT.md`

Researched against `3d789579294cf10600e9180c2940ac7fa9b78328`.

## Context

Warp’s third-party CLI agent stack is centered on `CLIAgent`. Long-running blocks are classified with `CLIAgent::detect`; a match creates a `CLIAgentSession` and drives footer, rich input, chrome icons, optional OSC listeners, and optional plugin chips. Muse Code is absent from that enum today, so `muse` sessions stay generic terminals.

Relevant systems at this SHA:

- Identity / detect / skills / bash / brand — [`app/src/terminal/cli_agent.rs` (148-454) @ 3d789579](https://github.com/warpdotdev/warp/blob/3d789579294cf10600e9180c2940ac7fa9b78328/app/src/terminal/cli_agent.rs#L148-L454)
- Telemetry enum — [`app/src/server/telemetry/events.rs` (456-476) @ 3d789579](https://github.com/warpdotdev/warp/blob/3d789579294cf10600e9180c2940ac7fa9b78328/app/src/server/telemetry/events.rs#L456-L476)
- OSC 777 protocol — [`app/src/terminal/cli_agent_sessions/event/mod.rs` (14-34) @ 3d789579](https://github.com/warpdotdev/warp/blob/3d789579294cf10600e9180c2940ac7fa9b78328/app/src/terminal/cli_agent_sessions/event/mod.rs#L14-L34); `agent` must equal a `command_prefixes()` entry
- Listeners — [`app/src/terminal/cli_agent_sessions/listener/mod.rs` (39-84) @ 3d789579](https://github.com/warpdotdev/warp/blob/3d789579294cf10600e9180c2940ac7fa9b78328/app/src/terminal/cli_agent_sessions/listener/mod.rs#L39-L84)
- Plugin install UI — [`app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs` (239-306) @ 3d789579](https://github.com/warpdotdev/warp/blob/3d789579294cf10600e9180c2940ac7fa9b78328/app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs#L239-L306)
- Rich-input submit — [`app/src/terminal/view/use_agent_footer/mod.rs` (124-146) @ 3d789579](https://github.com/warpdotdev/warp/blob/3d789579294cf10600e9180c2940ac7fa9b78328/app/src/terminal/view/use_agent_footer/mod.rs#L124-L146)
- Icons — [`crates/warp_core/src/ui/icons.rs` (276-278, 624-638) @ 3d789579](https://github.com/warpdotdev/warp/blob/3d789579294cf10600e9180c2940ac7fa9b78328/crates/warp_core/src/ui/icons.rs#L276-L278); assets in `app/assets/bundled/svg/`
- Detection trigger — `TerminalView` long-running path (~50ms) in `app/src/terminal/view.rs`

Closest shipped analog is **Grok Build as landed** (`specs/GH11727/`): registry + icon + DelayedEnter + listener, **no** Warp-managed plugin (`plugin_manager_for(CLIAgent::Grok)` is `None`). An earlier Grok attempt wrote hooks from the Warp client; review stripped that and asked for an agent-owned plugin instead. This spec follows the landed Grok shape, not the stripped one.

Muse Code facts that constrain the design (verified against the installed **Muse Code 1.2.1 / 1.2.1-R2847.1** binary and live `muse exec` runs on 2026-09-14):

- Interactive binary is `muse`; headless is `muse exec`. First-token detection covers both. `muse plugins` prints `plugins are not available in this build` — do not call it.
- Hooks are a Claude-style **HookConfig** document, not a free-form `settings.json` `hooks` object. Two install locations actually fire:
  - Project: `<workspace>/.muse/hooks.json`
  - User/machine: `managed_hooks_path` in `~/.config/muse/settings.json` pointing at a HookConfig file
  Putting matcher groups directly (or double-wrapped) under `settings.hooks` does **not** fire. A sibling `~/.config/muse/hooks.json` does **not** fire.
- HookConfig shape (validated): `{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "/abs/path/script"}]}]}}`. `command` is a string. Stdin is one JSON object with `hook_event_name`.
- Events observed live: `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `Stop`, `SessionEnd`. The binary’s `HookEventKind` also includes `Notification`, `PermissionRequest`, `StopFailure`, `PostToolBatch`, plus LLM/compact/subagent events. `Notification` is real in 1.2.1 (docs were stale); it did not fire on an unattended `--disable-approval` exec.
- Muse does not emit OSC 9 or OSC 777. Bang-shell is a first-class TUI action (`bang_shell` / `user_shell`); confirmed in the interactive composer, not only from binary strings.
- Official Muse Code mark is the Meta infinity loop; SVG attached on #15768.

User-visible behavior is in `PRODUCT.md`. This file is the implementation plan.

## Proposed changes

### 1. `CLIAgent::Muse`

Add the variant next to `Grok` and wire every exhaustive match (compiler will list them).

| API | Value |
|-----|--------|
| `command_prefixes` | `&["muse"]` |
| `display_name` | `"Muse Code"` |
| `icon` | `Some(Icon::MuseLogo)` |
| `brand_color` | Meta blue `ColorU { r: 0, g: 129, b: 251, a: 255 }` (`#0081FB`) — footer glyph tint; sidebar/notification disc fill |
| `brand_icon_color` | white (default arm) — glyph on the disc only, same as Claude / Grok |
| `supported_skill_providers` | `&[SkillProvider::Agents, SkillProvider::Claude, SkillProvider::Codex]` |
| `skill_command_prefix` | `"/"` |
| `supports_bash_mode` | `true` |
| `supports_cli_agent_footer` | `true` (default) |
| `From` → `CLIAgentType` | `Muse` |

Do **not** add `Harness::Muse`. Do **not** add `SkillProvider::Muse`. Identifiers stay short (`CLIAgent::Muse`); user-facing copy is **Muse Code**.

Detection is ordinary first-token basename equality. Absolute paths whose last segment is `muse` match the same way `/usr/bin/claude` does — no extra special case.

### 2. Icon asset

- Bundled stencil: `app/assets/bundled/svg/muse.svg` — 24×24 `viewBox`, single path, `fill="#FF0000"` (red-channel alpha mask). Geometry is the Meta infinity loop from the issue-attached `meta-loop-24.svg`; the source Meta-blue fill (`#0081FB`) is **not** stored in the asset (R=0 would make the mask invisible).
- Add `Icon::MuseLogo` in `crates/warp_core/src/ui/icons.rs` and map it to `bundled/svg/muse.svg`.
- Footer paints the stencil with `brand_color` (blue loop, no disc). `icon_with_status` paints a `brand_color` disc and the stencil with `brand_icon_color` (white loop). Same split as Claude / Grok.
- Do not ship light/dark pairs. Black-filled or Meta-blue-filled source art is invisible under the red-channel mask.

### 3. Listener

- Include `CLIAgent::Muse` in `is_agent_supported`.
- Handler: `DefaultSessionListener` (Claude / Gemini / OpenCode class). Live `muse exec` stdout contains no OSC 9 / OSC 777.
- Do **not** call `register_cli_agent_listener_without_session_start_event` (that path is Codex/Grok OSC 9).
- OSC 777 body `"agent"` must be `"muse"` (the command prefix), not `"Muse Code"`.

### 4. Rich input submit

- `RichInputSubmitStrategy::BracketedPaste` (Codex / Hermes class). Muse enables bracketed paste (`CSI ? 2004 h`). A raw burst + 50ms Enter (DelayedEnter, including kitty CSI u) inserts into the composer without submitting. Wrapping the text in paste markers then sending `\r` as a second write does submit (verified against Muse 1.2.1).

### 5. No Warp-managed plugin

Same contract as landed Grok:

- `plugin_manager_for_with_shell(CLIAgent::Muse, …)` returns `None`. Add `Muse` to the existing `None` arm; do not add `plugin_manager/muse.rs`.
- The footer does not render a notification install/update chip.
- Warp does not create hooks, scripts, manifests, version files, or `managed_hooks_path` entries in Muse’s configuration directories.
- The shared listener can still consume compatible OSC 777 events emitted by Muse or an **agent-owned** plugin, but Warp does not install that plugin.
- Do **not** add `FeatureFlag::MuseNotifications`.

Hook probing notes (what fires, HookConfig shape) live in `specs/GH15768/warp-hooks-sample.md` for a **future** `warpdotdev/muse-code-warp` (or similar) repo. They are not implementation work in this PR.

### 6. Telemetry + settings

- `CLIAgentType::Muse` and the `From<CLIAgent>` arm.
- Settings third-party dropdown picks the variant up via `enum_iterator::all`; no extra row.

### 7. Tests (in-repo)

- `cli_agent_tests`: `muse` and `muse exec "…"` detect as `CLIAgent::Muse`; public-config test (`command_prefix`, `display_name`, icon, bash on, skill prefix `/`).
- Listener: Muse is supported; OSC 777 `"agent":"muse"` parses; `"agent":"claude"` is rejected; OSC 9 is ignored.
- Plugin factory: `plugin_manager_for(CLIAgent::Muse)` returns `None` (same assertion as Grok).
- Exhaustive matches in `rich_input_submit_strategy` and `plugin_manager_for_with_shell` compile.

Do not add `muse` to `ONE_OFF_SHELL_COMMAND_KEYWORDS` unless NLD steals the command in manual testing (Grok was not added).

## Testing and validation

| Product invariant | Verification |
|-------------------|--------------|
| 1–4 Detect, identity, teardown | Unit detect + public-config tests; manual screenshot of footer and vertical tab with `muse` running, then after exit |
| 2 Alias / env / path / `npx muse` | Unit tests mirroring existing detect cases |
| 5 Rich input submit | Manual multi-line submit on real Muse TUI; DelayedEnter unit coverage if a table exists |
| 6 Images | Manual clipboard paste / drop |
| 7 Skills `/` | Manual slash menu; unit `supported_skill_providers` |
| 8 Bash `!` | Unit `supports_bash_mode() == true`; manual leading `!` enters bang-shell like Claude |
| 9 Review + attach | Manual under `HoaCodeReview`; no new review tests required beyond existing CLI-agent paths |
| 10 Tab Configs / Remote Control | Manual: restore a Muse tab config launches `muse`; share chip matches other agents |
| 11 Toolbelt without plugin | Manual on a machine with no Muse hooks and no Warp writes under `~/.config/muse` |
| 12 OSC 777 if something else emits it | Unit parse `"agent":"muse"` |
| 13 No OSC 9 | Listener unit test |
| 14 No install chip | Plugin factory unit test returns `None`; manual footer screenshot has no notifications chip |
| 15–16 Telemetry + serialization | Compile-time enum + existing serde round-trip test |
| 17 Settings list | Manual; `enum_iterator` |

Presubmit: `./script/format` and clippy per AGENTS.md. PR needs a footer screenshot; a recording is optional.

## Parallelization

Do **not** split this across sub-agents or stacked PRs for the registry work. Adding a `CLIAgent` variant touches one exhaustive enum in shared files (`cli_agent.rs`, telemetry, listener, plugin factory, submit strategy, icons). Parallel edits collide on those match arms.

Sequence:

1. Registry + icon + telemetry + detect tests (unblocks every chrome surface).
2. Listener + DelayedEnter + factory `None` test.
3. Manual GUI pass.

Do not land a `muse-code-warp` plugin or `plugin_manager/muse.rs` in this PR.

## Risks and mitigations

- **Meta mark licensing.** Issue thread already notes Brand Review. Spec ships the attached official loop; legal/product can still refuse the bundle. Fallback: ship without `Icon::MuseLogo` (Hermes/Vibe) and keep Meta blue on the tile — identity still works (PRODUCT.md 3).
- **Submit strategy.** Use BracketedPaste, not DelayedEnter. A 50ms delayed Enter after a raw burst leaves the prompt sitting in Muse’s composer.
- **Name collision.** Any other `muse` binary will be branded Muse Code. Accept the same risk as `agent` / `pi`; do not add extra heuristics in this PR.

## Follow-ups

- warpdotdev/docs: add Muse Code to the supported-agents table and a short setup page.
- A **separate** `warpdotdev/muse-code-warp` (or similar) plugin that emits OSC 777, using `specs/GH15768/warp-hooks-sample.md`. Not this Warp PR; not `plugin_manager/muse.rs` writing into `~/.config/muse`.
- `Harness::Muse` / ACP via `muse serve` for Agent Mode and orchestration.
