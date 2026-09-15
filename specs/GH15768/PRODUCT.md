# PRODUCT.md — First-class Muse Code CLI agent support

GitHub issue: https://github.com/warpdotdev/warp/issues/15768

Figma: none provided. Muse Code reuses the existing third-party CLI-agent chrome (footer/toolbelt, tab/pane brand tile, rich input). There is no new layout.

## Summary

Recognize Meta’s Muse Code (`muse`) as a first-class third-party CLI coding agent in Warp, at the same depth as **Grok Build as shipped** (`specs/GH11727/`): native identity, toolbelt, rich input, review/attach, vertical tabs, Tab Configs, and Remote Control. Warp does **not** install Muse hooks or ship a notification plugin in this change. A future agent-owned plugin (separate repo) can emit OSC 777; this work only listens.

## Problem

Muse Code sessions currently render as a generic terminal: Warp’s hardcoded `CLIAgent` registry does not include `muse`, so tabs keep the generic glyph, the agent toolbelt never appears, and none of the CLI-agent enhancements activate. That matches the reporter’s original icon gap and the later request for full agent parity.

## Goals / Non-goals

**In scope**

- Native detection of the `muse` binary (including aliases whose expansion’s first token is `muse`, and leading env-var assignments when shell parsing is available — the same rules as other `CLIAgent` prefixes).
- Brand identity in every surface that already uses `CLIAgent` chrome: footer tile, tab/pane icon, vertical tabs.
- The standard CLI-agent toolbelt, rich input, image attach, code-review comments, attach-code-as-context, Tab Configs, and Remote Control — whatever those surfaces already do for a recognized agent, with no Muse-only layout.
- Session listener support for structured OSC 777 events with `"agent":"muse"` **if** Muse or an agent-owned integration emits them. Warp does not install that integration.
- Telemetry: a distinct `CLIAgentType` for Muse Code.
- Unit coverage for detection, identity, listener support, and the **absence** of a Warp-managed plugin flow.

**Out of scope**

- Installing, updating, or writing Muse hooks, `managed_hooks_path`, `.muse/hooks.json`, or a `plugin_manager/muse.rs` install chip from the Warp client. Reviewers stripped the equivalent Grok in-client hook writer; notifications belong in a separate `muse-code-warp` (or similar) plugin, not this PR.
- Cloud / orchestration `Harness::Muse`, ambient spawn of Muse Code on workers, or treating Muse as an Automation Platform harness.
- ACP / `muse serve` (Muse Session Protocol) as a backend for Warp’s built-in Agent Mode.
- Shipping the Muse binary inside Warp, or changing Muse Code itself.
- Consumer **Meta Muse** (the personal assistant) branding, wordmark, or handwritten-M consumer mark.
- The public docs-site “supported third-party CLI agents” list. That lives in warpdotdev/docs and is a follow-up, same as #11727.
- A Muse-specific skill provider. Muse already loads shared Agents / Claude / Codex skill roots; Warp’s slash menu should list those, not invent a new root.
- Windows-only Muse installer work. Meta documents macOS and Linux; Warp still detects `muse` on any OS if the binary is present.

## Branding

Aligned with [#15768](https://github.com/warpdotdev/warp/issues/15768):

- Official product name: **Muse Code**. User-facing copy uses that string, not “Muse”, “Meta Muse”, or “Muse Spark”.
- There is no Muse-Code-specific logomark. Meta’s own Muse Code materials use the **Meta company infinity loop**. Warp bundles that mark, adapted to the existing CLI-agent icon pipeline, from the SVG attached on the issue (`meta-loop-24.svg`).
- **Warp adaptation:** one monochrome SVG at `app/assets/bundled/svg/muse.svg` with `fill="#FF0000"` (icon red-channel alpha mask), 24×24 `viewBox`. The on-disk red is a stencil, not a color the user sees. Warp tints that stencil at paint time — the same pipeline as Claude / Grok. Do not ship separate light/dark files. Do not use the consumer Muse handwritten-M.
- **Claude / Grok chrome (two surfaces, one mark):**
  - CLI-agent footer / toolbelt: no disc. The infinity loop is painted in Meta blue (`brand_color`), the same way Claude’s star is orange and Grok’s mark is near-black in the footer.
  - Vertical tabs, pane chrome, and notifications: a Meta-blue filled circle with a **white** infinity loop (`brand_icon_color`), the same way Claude is an orange circle with a white star and Grok is a black circle with a white mark.
- Meta’s brand terms gate usage through Meta Brand Review. Geometry comes from the issue-attached official mark; bundling is an explicit legal/product call, not an invented mark.

## Behavior

1. When the user runs a long-running command whose resolved first token’s basename is `muse`, Warp creates a CLI agent session with display name **Muse Code** and shows the CLI-agent toolbelt when the third-party CLI-agent toolbar setting is enabled.

2. Detection uses the same first-token equality as other agents: alias expansion, leading env-var assignments when shell parsing is available, and basename after `/` or `\`. `muse --model muse-spark-1.2`, `muse exec "…"`, `muse resume`, and `FOO=1 muse` all detect as Muse Code. A wrapper whose first token is not `muse` (for example `npx muse`) does not. The basename `agent` remains Cursor CLI.

3. While the session is active, Muse Code chrome follows the Claude / Grok pattern — not the generic terminal glyph and not the consumer Muse mark:
   - Footer / toolbelt: Meta-blue infinity loop, no disc.
   - Vertical tabs, pane chrome, notifications: Meta-blue disc with a white infinity loop.
   If the SVG is missing, the session still identifies as Muse Code and uses the brand color with the same fallback glyph other icon-less agents use.

4. Ending the `muse` command (block complete or pane close) tears the session down. Brand chrome and the toolbelt return to ordinary terminal treatment. Tab titles the CLI itself set are unchanged by this work.

5. With rich input enabled, the user can open the CLI-agent rich composer (footer control / Ctrl-G / auto-open when configured). Submitting non-empty text delivers the prompt to the running Muse Code process via the PTY using a submit strategy that does not drop or double-submit. Empty or whitespace-only submit is a no-op.

6. Image attach while rich input is open uses Warp attachment chips; on submit, images are delivered via the system clipboard and Muse Code’s paste chord, matching other CLI agents. Image drop while rich input is closed uses the same clipboard + paste path.

7. Skills: when the rich input slash/skills menu is open, Warp lists only providers Muse Code can natively load (shared Agents roots plus Claude and Codex skill roots that Muse Code discovers). Skill invocation uses `/`. Warp does not invent a Muse-only skill directory.

8. Bash `!` mode is **on** for Muse Code. Confirmed in the interactive TUI: a leading `!` is bang-shell / `user_shell` (the composer escape labeled “Start a shell command from the composer”), matching Claude / Codex / Grok. Warp’s CLI-agent rich input must treat a leading `!` the same way.

9. Code review comments and attach-code-as-context work for an active Muse Code session under the same settings and feature flags as any other recognized CLI agent. Warp does not add Muse-specific review UI.

10. Tab Configs that record a Muse Code session restore by launching `muse` (the canonical command prefix). Remote Control / share uses the same CLI-agent share chip and flags as other agents.

11. The session provides toolbar, rich input, review, tabs, and remote control **without** any notification plugin or hook install. Those surfaces must work on a machine that has never had Warp write into Muse’s config.

12. If Muse Code or an **agent-owned** integration emits a compatible OSC 777 event (title `warp://cli-agent` and JSON body `"agent":"muse"`), Warp processes it through the shared listener. Warp does not install, update, or prompt to install that integration.

13. Muse Code does not emit OSC 9 or OSC 777 itself. Warp does not treat opaque OSC 9 text as Muse Code stop events. Until an external plugin exists, Muse sessions have identity and toolbelt but not Warp inbox/rich-status notifications.

14. The Muse Code footer does **not** show an enable/update notification chip. Warp does not write `managed_hooks_path`, `.muse/hooks.json`, hook scripts, or a version file.

15. Telemetry that records CLI agent type uses a distinct Muse value (not Unknown) when the session agent is Muse Code.

16. Shared-session serialization round-trips the Muse agent name so viewers restore Muse Code identity rather than Unknown.

17. Settings → Agents → Third-party CLI agents lists **Muse Code** automatically with the other recognized agents. There is no Muse-only settings page.
