# PRODUCT.md — First-class `uta` CLI agent support

GitHub issue: https://github.com/warpdotdev/warp/issues/16688

Figma: none provided. `uta` reuses the existing third-party CLI-agent chrome (footer/toolbelt, tab/pane brand tile, rich input). There is no new layout.

## Summary

Recognize **`uta`** ([unleashtheagents/uta](https://github.com/unleashtheagents/uta), `brew install unleashtheagents/tap/uta`) as a first-class third-party CLI coding agent in Warp, at the same depth as **Grok Build** (`specs/GH11727/`), **Muse Code** (`specs/GH15768/`), and **OhMyPi**: native identity, toolbelt, rich input, review/attach, vertical tabs, Tab Configs, Remote Control, and native OSC 777 (`warp://cli-agent`) session listener support. Because `uta` emits `warp://cli-agent` v1 OSC 777 notifications directly when `TERM_PROGRAM=WarpTerminal`, Warp does **not** need to install hooks or ship a client-side notification plugin.

## Problem

`uta` sessions (`uta`, `uta shell`, `uta run "…"`) currently render as generic terminals: Warp's `CLIAgent` registry does not include `uta`, so tabs keep the generic terminal glyph, the CLI-agent toolbelt never appears, none of the CLI-agent enhancements activate, and `uta`'s native OSC 777 (`warp://cli-agent`) events resolve to `CLIAgent::Unknown`.

## Goals / Non-goals

**In scope**

- Native detection of the `uta` static Go binary (including aliases whose expansion's first token is `uta`, leading env-var assignments when shell parsing is available, and absolute path basenames — the same rules as other `CLIAgent` prefixes).
- Brand identity in every surface that already uses `CLIAgent` chrome: footer tile, tab/pane icon, vertical tabs, and notifications.
- The standard CLI-agent toolbelt, rich input, image attach, code-review comments, attach-code-as-context, Tab Configs, and Remote Control — whatever those surfaces already do for a recognized agent, with no `uta`-only layout.
- Session listener support via `DefaultSessionListener` for structured OSC 777 events with `"agent":"uta"` (emitted natively by `uta` when `TERM_PROGRAM=WarpTerminal`).
- Telemetry: `CLIAgent::telemetry_name()` returns `"Uta"` (no longer `"Unknown"`).
- Unit coverage for detection, identity, listener support, submit strategy, and the **absence** of a Warp-managed plugin flow.

**Out of scope**

- Installing, updating, or writing hooks into `~/.uta` from the Warp client (`plugin_manager_for(CLIAgent::Uta)` returns `None`, matching `OhMyPi` / `Grok` / `Muse`).
- Cloud / orchestration `Harness::Uta` or treating `uta` as an Automation Platform harness.
- Shipping the `uta` binary inside Warp.
- A `uta`-specific skill provider (`uta` natively discovers shared `.agents/skills` and `.claude/skills` roots).

## Branding

Aligned with [#16688](https://github.com/warpdotdev/warp/issues/16688) and official first-party assets at `https://unleashtheagents.ai/favicon.svg` (MIT-licensed, contributed by the upstream maintainers `unleashtheagents`):

- Official product display name: **`uta`** (lowercase, matching the CLI binary and upstream brand).
- Official mark: four corner L-brackets in `uta` purple (`#7C5CFF`, `ColorU { r: 124, g: 92, b: 255, a: 255 }`).
- **Warp adaptation:** one monochrome SVG stencil at `app/assets/bundled/svg/uta.svg` with `fill="#FF0000"` (icon red-channel alpha mask), 24×24 `viewBox`, single `<path>`. Warp tints that stencil at paint time — the same pipeline as Claude / Grok / Muse.
- **Claude / Grok / Muse chrome (two surfaces, one mark):**
  - CLI-agent footer / toolbelt: no disc. The four-corner bracket mark is painted in `uta` purple (`brand_color`).
  - Vertical tabs, pane chrome, and notifications: a `uta`-purple filled circle with a **white** four-corner bracket mark (`brand_icon_color`).

## Behavior

1. When the user runs a long-running command whose resolved first token's basename is `uta` (e.g. `uta`, `uta shell`, `uta run "…"`), Warp creates a CLI agent session with display name **`uta`** and shows the CLI-agent toolbelt when the third-party CLI-agent toolbar setting is enabled.
2. Detection uses the same first-token equality as other agents: alias expansion, leading env-var assignments when shell parsing is available, and basename after `/` or `\`. Because `uta` is a single static Go binary on macOS, Linux, and Windows (no `node` wrapper), `comm`/`argv[0]` and first-token matching work identically across platforms.
3. While the session is active, `uta` chrome follows the Claude / Grok / Muse pattern:
   - Footer / toolbelt: `#7C5CFF` purple bracket mark, no disc.
   - Vertical tabs, pane chrome, notifications: `#7C5CFF` purple disc with a white bracket mark.
4. Ending the `uta` command tears the session down and returns chrome to ordinary terminal treatment. Tab titles set by `uta` via OSC 0 (`uta — <goal>` / `uta shell`) are preserved.
5. With rich input enabled, submitting non-empty text delivers the prompt to the running `uta` process via the PTY using `RichInputSubmitStrategy::Inline` (matching `uta`'s raw-mode `bufio.NewReader` line editor in `internal/cli/lineedit.go`, which processes `\r` as a submit when it arrives in the same buffer as the preceding text).
6. Skills: when the rich input slash/skills menu is open, Warp lists `SkillProvider::Agents` and `SkillProvider::Claude` (the shared `.agents/skills` and `.claude/skills` roots that `uta` natively loads in `internal/skills/skills.go`). Skill invocation uses `/`.
7. Bash `!` mode is **on** for `uta` (`supports_bash_mode() == true`), matching `uta shell` where lines prefixed with `!` execute locally in the working directory (`internal/cli/shell.go`).
8. Native OSC 777 notifications (`title = "warp://cli-agent"`, JSON body `"agent":"uta"`) emitted by `uta` when `TERM_PROGRAM=WarpTerminal` (`internal/cli/title.go`) are processed through `DefaultSessionListener`. Opaque OSC 9 text is ignored.
9. The `uta` footer does **not** show an enable/update notification chip (`plugin_manager_for(CLIAgent::Uta)` returns `None`), and Warp writes nothing into `~/.uta`.
10. Telemetry records `"Uta"` via `CLIAgent::telemetry_name()`, and Settings → Agents → Third-party CLI agents lists **`uta`** automatically via `enum_iterator::Sequence`.
