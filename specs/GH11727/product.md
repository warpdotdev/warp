# PRODUCT.md — First-class Grok Build agent support

Issue: https://github.com/warpdotdev/warp/issues/11727

## Summary

Add Grok Build (SpaceXAI’s `grok` binary) as a first-class third-party CLI coding
agent in Warp, at parity with Claude Code, Codex, and OpenCode. Users who run
`grok` in a Warp pane get native agent identity (brand icon and color), the CLI
agent toolbar, rich input, image attach, skills/bash mode support, and
native notification/session status wiring.

## Goals / Non-goals

**In scope**

- Native detection of the `grok` command (including aliases that expand to
  `grok` as the first token, and leading env-var assignments when shell
  parsing is available — same rules as other `CLIAgent` prefixes).
- Brand icon and color in the CLI agent footer, vertical tabs, and other agent
  chrome that already uses `CLIAgent` identity.
- Full CLI agent toolbar / rich input / image paste paths used by other agents.
- Session listener support, including native OSC 9 notifications and structured
  OSC 777 events emitted through compatible agent-owned integrations.
- Unit tests for detection, listener support, and the absence of a Warp-managed
  plugin flow.
- Telemetry: a distinct `CLIAgentType` for Grok Build.

**Out of scope**

- ACP / `grok agent stdio` as a backend for Warp’s built-in Agent Mode.
- Cloud orchestration `Harness::Grok` / ambient spawn of Grok Build on workers.
- Treating the install alias `agent` as Grok Build (that basename is Cursor CLI).
- Changing Grok Build itself or shipping a Grok binary inside Warp.
- Docs site / marketing pages outside this repository (including the public
  “supported third-party CLI agents” list called out on #11727 — follow-up in
  warpdotdev/docs or equivalent).
- Installing or updating Grok plugins from Warp. Any future notification
  integration should use an agent-owned extension mechanism.

## Branding

Aligned with [#11727](https://github.com/warpdotdev/warp/issues/11727) community
guidance (official package filenames; logomark only):

- Use the official **Grok logomark** (not corporate mark, not wordmark, not full
  lockup; not X/Twitter bird; not Oz branding; no custom mascot), per
  [brand guidelines](https://x.ai/legal/brand-guidelines) and
  https://data.x.ai/logos/xAI_Grok_Assets.zip.
- Canonical package files for avatars: `Grok_Logomark_Light.svg` /
  `Grok_Logomark_Dark.svg` (issue recommendation for dark/light UI).
- **Warp adaptation:** reuse the existing monochrome SVG at
  `app/assets/bundled/svg/grok.svg` with `fill="#FF0000"` (icon red-channel
  alpha mask). Warp tints the mark at paint time for light/dark chrome — the
  same pipeline as Claude / Codex / OpenAI. Dual on-disk light/dark files would
  not match how Warp’s `Icon` system works and would break tinting. Geometry
  comes from the official logomark; only fill is converted to the mask color
  and the paths are padded into the existing circular avatar frame.

## Behavior invariants

1. When the user runs a long-running command whose resolved first token is
   `grok`, Warp creates a CLI agent session with display name **Grok Build** and
   shows the CLI agent toolbar when the third-party CLI agent toolbar setting is
   enabled.

2. The CLI agent footer always shows the Grok Build brand icon (when the SVG is
   present) and brand color treatment, consistent with Claude/Codex/OpenCode
   footers—not the generic terminal icon and not the Oz mark.

3. Vertical tabs / pane agent chrome that resolve identity from
   `CLIAgentSessionsModel` show the same Grok Build brand treatment while the
   session is active.

4. With rich input enabled, the user can open the CLI agent rich composer
   (footer control / Ctrl-G / auto-open when configured). Submitting non-empty
   text delivers the prompt to the running Grok Build process via the PTY using
   a submit strategy that does not drop or double-submit the prompt. Empty or
   whitespace-only submit is a no-op.

5. Image attach while rich input is open uses Warp attachment chips; on submit,
   images are delivered via the system clipboard and Grok Build’s paste chord
   (Ctrl/Cmd+V; Alt+V on Windows), matching other CLI agents. Image drop while
   rich input is closed uses the same clipboard + paste path.

6. Bash mode: when the rich input starts with `!`, Warp uses the same mode-switch
   prefix split used for Claude/OpenCode so Grok Build can enter shell mode.

7. Skills: the rich input slash/skills menu only lists providers Grok Build can
   use (at least Agents / shared skill roots; skill invocation prefix is `/`).

8. Command basename `agent` alone is **not** detected as Grok Build (remains
   Cursor CLI). An alias whose expansion’s first token is `grok` **is** detected
   as Grok Build.

9. The session provides toolbar, rich input, and images without any plugin
   installation. Native OSC 9 turn-complete/approval signals use the shared
   fallback listener.

10. If Grok or an agent-owned integration emits a compatible OSC 777 event
    (title `warp://cli-agent` and JSON body `"agent":"grok"`), Warp processes it
    through the shared listener infrastructure.

11. The Grok footer does not show an enable/update notification chip, and Warp
    does not write files into Grok’s configuration directories.

12. Telemetry that records CLI agent type uses a distinct Grok value (not
    Unknown) when the session agent is Grok Build.

13. Shared-session serialization round-trips the Grok agent name so viewers can
    restore identity.
