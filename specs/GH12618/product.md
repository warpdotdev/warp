# Add Kimi CLI Agent Support — Product Spec

GitHub issue: https://github.com/warpdotdev/warp/issues/12618

## Summary

Recognize the **Kimi CLI agent** (the `kimi` command) in the Warp terminal sidebar and local-harness launch path, the same way existing CLI agents (Claude Code, Codex, Gemini CLI, OpenCode, Amp, Cursor, etc.) are recognized today. When a user runs `kimi` inside a Warp pane, the sidebar / agent footer shows the Kimi brand logo on its brand-color circle instead of a generic terminal icon, and the `warp` CLI can delegate a prompt to `kimi` via `--harness kimi`.

This is a pure additive change: a new `Harness::Kimi` / `CLIAgent::Kimi` variant threaded through the existing agent-discovery, display, telemetry, and local-launch surfaces. No existing agent's behavior changes.

## Problem

Warp identifies CLI agents running inside a pane by matching the command prefix against a known set (`claude`, `codex`, `gemini`, `opencode`, …) and, on a match, renders that agent's brand icon and brand color in the sidebar and agent footer. The `kimi` command is not in that set, so a user running Kimi sees a generic CLI-agent icon with no brand color, and `warp agent ... --harness kimi` is not a valid invocation. This is purely a recognition gap — Kimi is otherwise a standard interactive CLI that already works inside a Warp pane.

## Goals

- Detect the `kimi` command and map it to a dedicated `CLIAgent::Kimi` / `Harness::Kimi`, following the same pattern as existing agents.
- Render the Kimi brand logo (`bundled/svg/kimi.svg`) on a brand-color circle (`#1883FF`) in the sidebar and agent footer.
- Allow `warp` CLI local-child delegation to `kimi` via `--harness kimi`, including installed-CLI validation, matching the Codex/OpenCode local-launch path.
- Report `CLIAgentType::Kimi` in telemetry so Kimi usage is distinguishable from `Unknown`.
- Keep the change fully additive: no regression to any existing agent, no breaking change to enums (the `Unknown` fallback continues to cover any agent not yet recognized).

## Non-goals

- Deep session integration with Kimi — no `CLIAgentSessionHandler`, no plugin-manager integration. Kimi is treated as an "unsupported harness" for orchestration purposes (sidebar recognition + local launch only), exactly as `OpenCode` is today. This is consistent with how Warp treats CLIs it can recognize but does not yet deeply integrate.
- A Warp cloud / Oz harness for Kimi. `Harness::Kimi` maps to `None` in the GraphQL harness surface and `AIAgentHarness::Unknown` in cloud-continuation, same as `OpenCode`.
- Modifying the Kimi CLI itself, its installation, or its configuration.
- Cloud conversation continuation for Kimi sessions.

## Behavior

1. When a process matching the `kimi` command prefix is detected in a Warp pane, the pane's CLI agent is classified as `CLIAgent::Kimi` (not `CLIAgent::Unknown`).

2. The `CLIAgent::Kimi` identifier string (used for matching / serialization) is `kimi`.

3. The user-visible display name for the agent is `Kimi`.

4. The sidebar / agent-footer icon for `CLIAgent::Kimi` is the bundled Kimi logo SVG (`Icon::KimiLogo` → `bundled/svg/kimi.svg`).

5. The brand color for `CLIAgent::Kimi` is Kimi blue, `#1883FF` (RGB 24, 131, 255). The agent's circle background is solid `#1883FF`, and the logo fill rendered on that circle is white — matching the visual treatment of Claude, Codex, Gemini, and OpenCode.

6. `CLIAgent::Kimi` advertises `SkillProvider::Agents`, the same skill-provider set used by the other recognized third-party CLI agents (Hermes, Vibe, etc.).

7. `Harness::Kimi` round-trips through the harness label surface as the string `kimi` (both `parse_orchestration_harness("kimi")` → `Harness::Kimi` and `Harness::Kimi` → `"kimi"`).

8. `Harness::Kimi` maps to `CLIAgent::Kimi` via `CLIAgent::from_harness`, and `CLIAgent::Kimi` maps to `CLIAgentType::Kimi` for telemetry.

9. `warp agent ... --harness kimi` is accepted as a local child harness (alongside `claude`, `opencode`, `codex`). When launched:
   - Warp validates that the `kimi` CLI is installed (same `validate_cli_installed` path as OpenCode). If it is missing, the launch fails with a clear error before spawning.
   - If installed, Warp spawns `kimi <quoted-prompt>` as the child process.

10. For orchestration / driver purposes, `Harness::Kimi` is classified as `HarnessKind::Unsupported(Harness::Kimi)` — i.e. Warp does not attempt to drive it as a first-class third-party harness. It contributes no model env vars. This matches the existing treatment of `OpenCode`.

11. In every exhaustive `match` over `Harness` or `CLIAgent` that previously enumerated the known set, `Kimi` is handled explicitly (no `_` wildcard) so that adding a future agent still triggers a compile error forcing the author to decide. The behavior assigned to the `Kimi` arm is the same as the `OpenCode` arm wherever the two are grouped (e.g. local-harness product-disabled message returns `None`, local harness setup state is `Ready`, submit strategy is `Inline`).

12. No existing `Harness` / `CLIAgent` variant changes its identifier, display name, color, icon, skill providers, or launch behavior. Adding `Kimi` does not alter the classification of any other command prefix.

13. A `kimi` process running inside a pane that is not the recognized Kimi CLI (e.g. a hypothetical unrelated `kimi` binary with different behavior) is still classified as `CLIAgent::Kimi` based on the command prefix — this matches how every other prefix-based agent is detected and is an accepted limitation of prefix-based detection.

14. Serialization is stable: clients/servers that do not yet know `Kimi` fall back to the existing unknown-enum-value handling (the `Unknown` variant) rather than failing to deserialize — no schema migration required.
