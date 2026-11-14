# Add Kimi CLI Agent Support — Tech Spec

Product spec: `specs/GH12618/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/12618
Implementation PR: https://github.com/warpdotdev/warp/pull/12616

## Context

Warp recognizes CLI agents running inside a pane via command-prefix matching and maps each known prefix to a `CLIAgent` variant plus a `Harness` variant. Each variant carries its brand presentation (display name, icon, brand color, skill providers) and its launch/orchestration behavior. The agent is surfaced in the sidebar and agent footer (`harness_display`), classified for the orchestration driver (`HarnessKind`), launchable as a local child process (`local_harness_launch`), and reported in telemetry (`CLIAgentType`).

Adding a new CLI agent is a mechanical but wide change: a new enum variant must be threaded through every exhaustive `match` over `Harness` and `CLIAgent`, plus the icon registry, telemetry enum, CLI parsing, and display module. The `OpenCode` agent is the closest precedent — it is recognized and locally launchable but classified as `HarnessKind::Unsupported` (no deep session integration). Kimi follows that exact shape.

Relevant code (all paths relative to repo root):

- `app/src/terminal/cli_agent.rs` — the `CLIAgent` enum and its methods: `identifier()`, `from_harness()`, `display_name()`, `icon()`, `skill_providers()`, `brand_color()`, and `From<CLIAgent> for CLIAgentType`. Also the per-brand color constants (`OPENAI_COLOR`, `GEMINI_BLUE`, `OPENCODE_COLOR`, etc.). Prefix detection maps a detected command to a `CLIAgent`.
- `crates/warp_cli/src/agent.rs` — the clap `Harness` enum surfaced by the `warp` CLI, plus `parse_local_child_harness`, `parse_orchestration_harness`, `Harness::display_name`, and the harness-label round-trip used for orchestration.
- `app/src/ai/harness_display.rs` — `display_name`, `icon_for`, `brand_color`, `circle_background`, `icon_fill_on_circle`: the sidebar / footer brand rendering for a `Harness`.
- `crates/warp_core/src/ui/icons.rs` — the `Icon` enum and `From<Icon> for &'static str`, which maps each logo icon to its bundled SVG path under `app/assets/bundled/svg/`.
- `app/src/ai/agent_sdk/driver/harness/mod.rs` — `harness_kind()` classifies each `Harness` into a `HarnessKind` (`FirstParty`, `ThirdParty`, or `Unsupported`); `harness_model_env_vars()` decides which env vars each harness injects.
- `app/src/ai/agent_sdk/mod.rs` — `resolve_orchestration_harness_label()`: the `Harness` → wire-label mapping used for orchestration.
- `app/src/pane_group/pane/local_harness_launch.rs` — `build_local_*_child_command`, `validate_cli_installed`, `local_child_task_config`, and `prepare_local_harness_child_launch`: the local child-process launch path for each harness.
- `app/src/server/telemetry/events.rs` — the `CLIAgentType` telemetry enum and `From<CLIAgent> for CLIAgentType`.
- `app/assets/bundled/svg/` — bundled brand SVGs (`claude.svg`, `gemini.svg`, `opencode.svg`, …).
- Supporting exhaustive matches touched for completeness: `app/src/ai/conversation_details_panel.rs`, `app/src/ai/harness_availability.rs`, `app/src/ai/local_harness_setup.rs`, `app/src/pane_group/pane/terminal_pane.rs`, `app/src/terminal/cli_agent_sessions/listener/mod.rs`, `app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs`, `app/src/terminal/view/ambient_agent/view_impl.rs`, `app/src/terminal/view/shared_session/cloud_conversation_continuation.rs`, `app/src/terminal/view/use_agent_footer/mod.rs`.
- `app/src/ui_components/agent_icon_tests.rs` — tests covering `CLIAgent::from_harness`.

See product spec for user-visible behavior.

## Proposed changes

### 1. New bundled asset

Add `app/assets/bundled/svg/kimi.svg` — a 24×24 vector logo: black circle background with the Kimi "K" mark. The circle fill in the SVG is visually irrelevant once rendered on the brand-color circle (the circle background is painted by `circle_background`, not the SVG), but the mark itself is white-on-dark so it composes correctly on the `#1883FF` circle.

### 2. `CLIAgent::Kimi` variant

In `app/src/terminal/cli_agent.rs`:

- Add a `KIMI_BLUE: ColorU` constant (`r: 24, g: 131, b: 255, a: 255`) — `#1883FF` — alongside the existing brand-color constants.
- Add `Kimi` to the `CLIAgent` enum (before `Unknown`, after `Antigravity`) and update the doc-comment list of agents.
- Implement every `CLIAgent` method for the new arm:
  - `identifier()` → `"kimi"`.
  - `display_name()` → `"Kimi"`.
  - `icon()` → `Some(Icon::KimiLogo)`.
  - `skill_providers()` → `&[SkillProvider::Agents]`.
  - `brand_color()` → `Some(KIMI_BLUE)`.
  - `from_harness(Harness::Kimi)` → `Some(CLIAgent::Kimi)`.
  - `From<CLIAgent> for CLIAgentType`: `CLIAgent::Kimi` → `CLIAgentType::Kimi`.

### 3. `Icon::KimiLogo`

In `crates/warp_core/src/ui/icons.rs`:

- Add `KimiLogo` to the `Icon` enum.
- Map `Icon::KimiLogo => "bundled/svg/kimi.svg"` in `From<Icon> for &'static str`.

### 4. `CLIAgentType::Kimi`

In `app/src/server/telemetry/events.rs`, add `Kimi` to the `CLIAgentType` enum (before `Unknown`). The `From<CLIAgent>` impl added in change 2 is what actually produces it.

### 5. `Harness::Kimi` (warp CLI)

In `crates/warp_cli/src/agent.rs`:

- Add `Kimi` to the clap `Harness` enum with `#[value(name = "kimi")]`.
- `parse_local_child_harness`: include `Self::Kimi` in the accepted local-child set (alongside `Claude | OpenCode | Codex`).
- `Harness::display_name` → `"Kimi"`.
- `parse_orchestration_harness("kimi")` → `Some(Harness::Kimi)`.
- Harness-label round-trip: `Harness::Kimi` → `"kimi"`.

### 6. Harness display

In `app/src/ai/harness_display.rs`, add `Harness::Kimi` arms mirroring the grouped agents:

- `display_name` → `"Kimi"`.
- `icon_for` → `Icon::KimiLogo`.
- `brand_color` → `Some(KIMI_BLUE)`.
- `circle_background` → `WarpThemeFill::Solid(KIMI_BLUE)`.
- `icon_fill_on_circle`: add `Harness::Kimi` to the existing `Claude | Codex | Gemini | OpenCode` group → `WarpThemeFill::Solid(ColorU::white())`.

### 7. Orchestration classification — `Unsupported`, no env vars

In `app/src/ai/agent_sdk/driver/harness/mod.rs`:

- `harness_kind(Harness::Kimi)` → `Ok(HarnessKind::Unsupported(Harness::Kimi))`, identical to `OpenCode`. Kimi is recognized and locally launchable but not driven as a first-class third-party harness.
- `harness_model_env_vars`: add `Harness::Kimi` to the no-op group (`Oz | OpenCode | Gemini | Codex | Kimi | Unknown`).

In `app/src/ai/agent_sdk/mod.rs`, `resolve_orchestration_harness_label(Some(Harness::Kimi))` → `"kimi"`.

### 8. Local child launch

In `app/src/pane_group/pane/local_harness_launch.rs`:

- Add `build_local_kimi_child_command(prompt)` → `format!("kimi {quoted_prompt}")`, matching the shape of `build_local_opencode_child_command` / `build_local_codex_child_command`.
- `local_child_task_config`: add `Harness::Kimi` to the `Claude | OpenCode | Gemini | Codex` arm so a local Kimi child produces an `AgentConfigSnapshot`.
- `prepare_local_harness_child_launch`: add a `Harness::Kimi` arm that calls `validate_cli_installed("kimi", None)` (surfacing a clear error if the CLI is absent) and then `build_local_kimi_child_command`.

### 9. Exhaustive-match completions (no behavior change)

Add `Harness::Kimi` (or `CLIAgent::Kimi`) to the grouped arms in the supporting files listed in Context, assigning it the same behavior as the `OpenCode` arm in each. These exist only to keep every `match` exhaustive without a `_` wildcard, so a future agent addition stays a compile error:

- `conversation_details_panel.rs`, `harness_availability.rs` (GraphQL harness → `None`), `local_harness_setup.rs` (product-disabled → `None`, setup state → `Ready`), `terminal_pane.rs` (`launch_remote_child` → `None`), `cli_agent_sessions/listener/mod.rs` (`create_handler` → `None`), `cli_agent_sessions/plugin_manager/mod.rs` (`plugin_manager_for_with_shell` → `None`), `view/ambient_agent/view_impl.rs` (`Harness::Kimi` matches `CLIAgent::Kimi`), `view/shared_session/cloud_conversation_continuation.rs` (→ `AIAgentHarness::Unknown`), `view/use_agent_footer/mod.rs` (submit strategy → `Inline`).

### 10. Test

In `app/src/ui_components/agent_icon_tests.rs`, extend `cli_agent_from_harness_maps_known_harnesses` with `assert_eq!(CLIAgent::from_harness(Harness::Kimi), Some(CLIAgent::Kimi))`.

## Testing and validation

Each numbered invariant in `specs/GH12618/product.md` maps to verification below.

### Unit / compile-time tests

- `cargo check -p warp_cli` and `cargo check -p warp_core` confirm the new `Harness`/`Icon` variants compile and every exhaustive `match` over them is complete (invariants 11–12). The absence of `_` wildcards means a missed arm is a compile error.
- `cli_agent_from_harness_maps_known_harnesses` in `agent_icon_tests.rs` asserts `Harness::Kimi` ↔ `CLIAgent::Kimi` (invariants 8). Guards the from-harness mapping.
- Run the full agent-icon test module to confirm no existing-variant assertion changed (invariant 12).

### Manual validation

Requires `kimi` installed on `PATH` and Warp built locally via `./script/run`.

- In a Warp pane, run `kimi`. Confirm the sidebar / agent footer shows the Kimi logo on a `#1883FF` circle with a white mark, and the display name reads "Kimi" — not the generic CLI-agent icon (invariants 1, 3, 4, 5).
- Confirm no other running agent (e.g. `claude`, `codex`, `gemini`) changes its icon, color, or name after this change (invariant 12).
- Run `warp agent ... --harness kimi` with `kimi` installed: confirm Warp spawns `kimi <prompt>` as a local child (invariant 9, happy path).
- With `kimi` not on `PATH`, retry `--harness kimi`: confirm the launch fails with the installed-CLI validation error rather than spawning a broken process (invariant 9, error path).
- Open the telemetry / agent-type surface (or inspect a logged `CLIAgentType`) after running `kimi` and confirm it reports `Kimi`, not `Unknown` (invariant 8).
- Provide before/after screenshots of the sidebar showing a `kimi` pane (generic icon → Kimi brand) as PR proof, per the repo's manual-testing requirement for visual changes.

## Risks and mitigations

### Risk: prefix collision with an unrelated `kimi` binary

Any executable named `kimi` on `PATH` is classified as `CLIAgent::Kimi` regardless of whether it is the Kimi CLI. This is an accepted limitation of prefix-based detection shared by every other recognized agent (invariant 13); it is not specific to Kimi.

Mitigation: none required — consistent with existing behavior. Documented as a non-goal to set reviewer expectations.

### Risk: brand-color / logo licensing

The Kimi logo and `#1883FF` are sourced from Kimi's brand. If the brand owner requires a different licensed asset, only `app/assets/bundled/svg/kimi.svg` and the `KIMI_BLUE` constant need to change — no logic change.

Mitigation: the SVG is isolated behind `Icon::KimiLogo` and the color behind a single named constant, so a brand-asset swap is a one-file change.

### Risk: `HarnessKind::Unsupported` may be misread as "broken"

Classifying Kimi as `Unsupported` matches `OpenCode`, but a reviewer unfamiliar with that precedent might read `Unsupported` as "this agent does not work." It specifically means "not driven as a first-class third-party harness" — sidebar recognition and local launch still work.

Mitigation: called out explicitly in invariants 10 and the non-goals; the `OpenCode` precedent is referenced in the tech spec so the classification is unambiguous.

## Follow-ups

- If deep Kimi session integration becomes desired later (structured output parsing, a `CLIAgentSessionHandler`, plugin-manager integration), graduate `Harness::Kimi` from `HarnessKind::Unsupported` to `HarnessKind::ThirdParty(KimiHarness)` and add the session handler — out of scope for this recognition-only change.
- Consider a Warp cloud / Oz harness for Kimi if cloud-continuation demand appears; currently maps to `AIAgentHarness::Unknown` / GraphQL `None` (non-goal).
- Track localization of agent display names (`"Kimi"`) once agent names move to a localizable string source; today they are inline literals, consistent with all other agents.
