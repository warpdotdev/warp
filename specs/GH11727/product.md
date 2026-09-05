# Product Spec: First-class Grok CLI agent identity

**Issue:** [warpdotdev/warp#11727](https://github.com/warpdotdev/warp/issues/11727)
**Figma:** none provided

## Summary

Grok CLI (`grok`, built by xAI) should be a recognized third-party CLI coding agent in
Warp, the way Claude Code, Codex, Gemini CLI and Antigravity already are. Today `grok`
runs as an ordinary terminal command: Warp shows no Grok identity, no brand icon, and
none of the CLI-agent surfaces activate.

This spec covers the **identity layer** — detection, naming, icon and brand colour — which
is what every other CLI-agent surface keys off. Plugin-backed session events are called
out as a non-goal below.

## Problem

Warp enumerates the CLI agents it knows about. `grok` is absent from that list, so a Grok
session is indistinguishable from any other long-running command:

- No Grok logo in the vertical tab list or agent conversation list; peers show their brand mark.
- The tab is labelled with the raw command (`grok`) rather than a product name.
- The CLI-agent footer, rich input editor and agent toolbelt never activate.

The custom command-regex setting is only a partial workaround: it can force the toolbar to
appear, but it grants no brand identity, no metadata and no per-agent behaviour.

## Goals

- Warp auto-detects `grok` as a CLI agent, including with arguments, aliases and env-var prefixes.
- A Grok session shows the xAI Grok logomark, at the same optical weight as peer agent logos.
- A Grok session is named "Grok" consistently, rather than echoing the launch command.
- The CLI-agent footer and rich input activate for Grok as they do for peer agents.

## Non-goals

- **Plugin-backed session events.** Rich per-turn status ("In progress", tool-call summaries)
  arrives via OSC 777 events from Warp's notification plugin. Wiring a Grok plugin install
  flow and session listener is deliberately deferred to a follow-up; Grok is registered as
  having neither, exactly as Vibe, Goose and Cursor are today.
- Bundling or installing the Grok CLI binary itself.
- Grok-specific onboarding, docs or marketing surfaces.
- Changing how Grok models are offered in the model picker (`LLMProvider::Xai` already maps
  to the Grok icon).

## User experience

### Current behaviour

1. User installs Grok CLI and runs `grok` in Warp.
2. The tab shows `grok` in monospace, with the generic terminal glyph.
3. No CLI-agent footer or rich input appears.

### Expected behaviour

1. User runs `grok` in Warp.
2. Warp recognizes the session as the Grok agent.
3. The tab shows the Grok logomark on a black badge, and the name **Grok** in the UI font.
4. The CLI-agent footer and rich input activate, as for peer agents.

### Edge cases

- **Arguments:** `grok --model grok-4` is still detected as Grok.
- **Aliases and env prefixes:** `FOO=bar grok` and a shell alias resolving to `grok` are detected.
- **Unrelated commands:** a session whose last command is `cargo nextest run` keeps showing that
  command verbatim — the name substitution applies only to the agent's own launch command.
- **Agents that report a title:** Claude Code continues to show its reported title; the
  display-name fallback only applies when no title is reported.
- **Light and dark themes:** the logomark is tinted by the existing brand-icon colour rule
  (white glyph on the black brand circle), so it is legible in both.

## Success criteria

Numbered, testable behaviour invariants:

1. `CLIAgent::detect("grok", ..)` resolves to the Grok agent.
2. `CLIAgent::detect("grok --model grok-4", ..)` resolves to the Grok agent.
3. Detection also succeeds through an alias and through an env-var prefix.
4. The Grok agent's canonical command prefix is `grok`.
5. The Grok agent's display name is `Grok`.
6. The Grok agent's icon is the Grok logomark asset.
7. The Grok agent's brand colour is xAI black, and its glyph colour is white.
8. The Grok agent maps to a distinct telemetry variant, not `Unknown`.
9. The Grok agent's serialized name round-trips through the session-sharing protocol.
10. The Grok logomark's rendered solid ink covers a comparable share of the icon box to peer
    logos (>= 85%), so it does not read visibly smaller in the same badge.
11. A recognized agent whose session reports no title is labelled with its display name in the
    UI font, not the launch command in monospace.
12. A session whose last command is not that agent's launch command still shows the command verbatim.
13. The CLI-agent footer renders for a Grok session.
