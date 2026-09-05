# Tech Spec: First-class Grok CLI agent identity

**Issue:** [warpdotdev/warp#11727](https://github.com/warpdotdev/warp/issues/11727)

## Problem

`CLIAgent` enumerates every CLI agent Warp recognizes and has no `Grok` variant, so
`CLIAgent::detect` never matches `grok`. Because every CLI-agent surface keys off that
enum, a Grok session gets no identity, no brand icon and none of the agent affordances.

## Context

### Relevant code

- `app/src/terminal/cli_agent.rs:148` — the `CLIAgent` enum. Every per-agent behaviour is an
  exhaustive `match` on it, so adding a variant makes the compiler enumerate each site that
  needs a decision.
- `app/src/terminal/cli_agent.rs` — the six per-agent matches: `command_prefixes` (175),
  `display_name` (239), `icon` (263), `supported_skill_providers` (301), `brand_color` (351),
  and `From<CLIAgent> for CLIAgentType` (628).
- `crates/warp_core/src/ui/icons.rs:278,626` — `Icon::GrokLogo` and its asset path
  `bundled/svg/grok.svg`. **Both already exist.**
- `crates/ai/src/llm_provider.rs:26` — `LLMProvider::Xai => Some(Icon::GrokLogo)`. The model
  picker already resolves Grok models to that icon, so only the CLI-agent side is missing.
- `app/src/ui_components/icon_with_status.rs:291` — `render_circle`, which composes the brand
  circle. It applies one `icon_size` ratio (line 70) to every agent; there is no per-icon scale.
- `app/src/workspace/view/vertical_tabs.rs:4159` — `terminal_primary_line_data`, which resolves
  the tab's primary label in priority order.
- `app/src/terminal/cli_agent_sessions/listener/mod.rs`,
  `app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs` — the two surfaces that gate on
  plugin-backed session events, where Grok is registered as unsupported (see Non-goals).

### Current state

`grok` matches no variant, so `detect` returns `None` and the session is treated as a plain
long-running command.

Separately, two latent issues surface once Grok is registered:

1. **Optical sizing.** `grok.svg`'s bounding box reaches the frame, but only through two
   hairline diagonal tips that fall below one pixel at icon sizes. Measured at the 19px icon
   size used inside a 36px badge, its solid ink covers 14x13 where claude covers 19x18 — so it
   renders at the same size but reads ~26% smaller.
2. **Tab label.** Agents that report a title show their product name; Grok CLI reports none, so
   `terminal_primary_line_data` falls through to echoing the launch command, showing `grok` in
   monospace beside `Claude Code` in the UI font.

## Proposed changes

### 1. Register the agent — `app/src/terminal/cli_agent.rs`

Add a `Grok` variant and fill in the six exhaustive matches: prefix `grok`, display name
`Grok` (matching the convention where Codex/Gemini/Cursor drop the "CLI" suffix), icon
`Icon::GrokLogo`, brand colour a new `GROK_COLOR` (xAI black), skill providers
`[SkillProvider::Agents]`, and a new `CLIAgentType::Grok` telemetry variant in
`app/src/server/telemetry/events.rs`.

`to_serialized_name` is serde-driven and `detect` iterates `enum_iterator::all`, so both pick
the variant up with no further change.

In the three surfaces gated on plugin-backed session events
(`use_agent_footer`, `listener`, `plugin_manager`) Grok joins the conservative group, matching
how Vibe, Goose and Cursor are handled today.

### 2. Optically size the logo — `app/assets/bundled/svg/grok.svg`

Wrap the path in `translate(12,12) scale(1.2) translate(-12,-12)`, raising solid-ink coverage
from 74% to 89% — the same band as goose (89%) and gemini (95%).

Applied in the asset rather than at the call site because `render_circle` deliberately uses one
ratio for all agents, and because `Icon::GrokLogo` is shared with the model picker, which has
the identical imbalance. Renderer support is not a concern: the client uses resvg 0.47, which
is spec-compliant for group transforms.

### 3. Label tabs by agent name — `app/src/workspace/view/vertical_tabs.rs`

Thread the already-available `agent_text.cli_agent` into `terminal_primary_line_data` and, in
the last-completed-command branch, prefer `display_name()` in the UI font. The substitution is
narrowed to commands matching that agent's `command_prefixes()`, so a genuine command left on
screen is still shown verbatim.

## Testing and validation

Each invariant in the product spec maps to a test:

| Invariant | Coverage |
|---|---|
| 1-3 detection | `test_detect_known_agents` (extended with `grok`), plus existing alias/env-prefix cases |
| 4-9 identity | `test_grok_variant_properties` — prefix, display name, icon, brand and glyph colour, telemetry variant, serialized round-trip |
| 9 round-trip | also covered automatically by `test_serialized_name_round_trips_known_agents`, which iterates `enum_iterator::all` |
| 10 optical size | measured by rendering the asset and comparing solid-ink coverage against peer logos |
| 11-12 tab label | `terminal_primary_line_prefers_agent_display_name_over_launch_command` and `terminal_primary_line_keeps_launch_command_when_it_is_not_the_agent_command` |
| 13 footer | existing `use_agent_footer` coverage applies via the shared `supports_cli_agent_footer` path |

Manual validation: build with `./script/run`, run `grok`, and confirm the badge, glyph size and
tab label against a Claude Code session in the same window.

## Risks

- The tab-label change touches shared rendering used by all agents. It is confined to the
  fallback branch and gated on the command matching the agent, and existing
  `terminal_primary_line_*` tests all continue to pass.
- Scaling the logomark truncates the hairline tips very slightly. At icon sizes those pixels are
  already sub-threshold; at large sizes the taper is preserved.

## Follow-ups

- Grok plugin install flow and an OSC 777 session listener, which would give Grok the rich
  per-turn status Claude Code has (the remainder of this issue's ask).
- `copilot.svg` has the same optical imbalance (68% coverage) and could be corrected the same way.
