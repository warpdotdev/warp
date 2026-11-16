# CLI Agent Rich Input: /skills Technical Spec

## Summary
This spec covers implementing `/skills` support in the CLI agent rich input composer. The approach filters the existing slash command menu and skill data sources to show only natively supported skills, and passes through the selected skill name to the CLI agent via PTY write.

## Relevant Code
- `app/src/terminal/input/slash_commands/data_source/mod.rs` — `SlashCommandDataSource`, `recompute_active_commands()`
- `app/src/terminal/input/slash_commands/mod.rs` — `handle_slash_commands_menu_event()`, skill selection handling
- `app/src/terminal/input/slash_commands/view.rs` — `InlineSlashCommandView`, mixer with 3 data sources
- `app/src/terminal/input/skills/view.rs` — `InlineSkillSelectorView`
- `app/src/terminal/input/skills/data_source.rs` — `SkillSelectorDataSource`
- `app/src/terminal/cli_agent_sessions/mod.rs` — `CLIAgentSessionsModel`, `CLIAgentInputState`
- `app/src/ai/skills/skill_manager.rs` — `SkillManager`, `skill_by_reference()`
- `ai/src/skills/skill_provider.rs` — `SkillProvider`, `SKILL_PROVIDER_DEFINITIONS`, provider-to-folder mapping
- `app/src/terminal/cli_agent.rs` — `CLIAgent` enum

## Current State
The CLI agent rich input (opened via Ctrl-G or the Compose button) is a plain text editor that writes its buffer to the PTY on submit. It reuses the same `Input` view and editor as the normal Warp input, but in a constrained mode. On enter, `input_enter()` detects `CLIAgentSessionsModel::is_input_open()` and emits `Event::SubmitCLIAgentInput` with the raw buffer text, which is written to the PTY.

The slash commands menu and skill selector already exist and work in the normal Warp input. The slash menu (`InlineSlashCommandView`) uses a `SearchMixer` with three data sources:
1. `SlashCommandDataSource` (sync) — static commands like `/agent`, `/new`, `/skills`. Stored in `active_commands_by_id`.
2. `saved_prompts_data_source` (async) — saved prompts from Warp Drive.
3. `ZeroStateDataSource` (sync) — zero-state items combining commands and skills.

Individual skills appear as `AcceptSlashCommandOrSavedPrompt::Skill` items, produced by the data source querying `SkillManager`. The `/skills` command opens a dedicated `InlineSkillSelectorView`.

Currently, all static commands and all skills are shown regardless of whether CLI agent input is active. Warp-specific commands like `/agent` and `/new` don't make sense for CLI agents, and non-native skills can't be interpreted by the CLI agent.

## Proposed Changes

### 1. Filter static slash commands for CLI agent input

**Approach**: Inside `recompute_active_commands()`, read `CLIAgentSessionsModel::is_input_open(terminal_view_id)` directly. When true, filter `active_commands_by_id` to only keep allowlisted commands — specifically `/skills` (so users can browse skills). All other static commands (`/agent`, `/new`, `/conversations`, `/cloud-agent`, etc.) are removed.

No stored boolean is needed. `CLIAgentSessionsModel` is the single source of truth, avoiding stale state.

**Plumbing**: `SlashCommandDataSource` already subscribes to `CLISubagentController` events and calls `recompute_active_commands()`. Add a subscription to `CLIAgentSessionsModel` for `InputSessionChanged` events to trigger `recompute_active_commands()` when the input session opens or closes.

### 2. Filter skills to native-only

**What changes**: Only natively supported skills are shown in the CLI agent input menu. When selected, `/{skill-name} ` is inserted into the buffer. The CLI agent handles argument parsing natively — no client-side parsing needed.

**Filtering skills**: Use the existing `SkillProvider` and `CLIAgent` types to build a mapping of which CLI agents support which skill providers:

```
CLIAgent::Claude  → [SkillProvider::Claude]
CLIAgent::Codex   → [SkillProvider::Agents, SkillProvider::Claude, SkillProvider::Codex]
CLIAgent::OpenCode → [SkillProvider::OpenCode, SkillProvider::Agents, SkillProvider::Claude]
CLIAgent::Gemini  → [SkillProvider::Agents, SkillProvider::Gemini]
CLIAgent::Amp     → [SkillProvider::Agents]
CLIAgent::Copilot → [SkillProvider::Agents, SkillProvider::Copilot]
CLIAgent::Droid   → [SkillProvider::Droid, SkillProvider::Agents]
CLIAgent::Unknown → [] (no skills shown)
```

The `SkillSelectorDataSource` and `SlashCommandDataSource` (which also surfaces skills) need to filter results based on the active CLI agent's supported providers. When `CLIAgentSessionsModel::is_input_open()` is true, look up the active agent, get its supported providers, and filter out skills whose `ParsedSkill::provider` is not in the list. Non-native skills (including bundled Warp skills) are hidden entirely.

**Selection behavior**: No branching needed — all skills in the menu are natively supported, so the existing behavior of inserting `/{skill-name} ` works as-is.

## End-to-End Flow
1. User types `/` in CLI agent rich input.
2. Slash commands menu opens, showing only `/skills` command and natively supported skills (static commands filtered out).
3. User selects a skill whose provider matches the active CLI agent.
4. `/{skill-name} ` is inserted into the buffer.
5. User types arguments and presses Enter → full text written to PTY.

## Risks and Mitigations

### Skills not appearing for a CLI agent
If the `CLIAgent → SkillProvider` mapping is wrong or incomplete, users won't see their skills. Mitigation: the mapping is derived from existing `SkillProvider` and `SKILL_PROVIDER_DEFINITIONS` which are already used for skill discovery. Keep the mapping in sync with `skill_provider.rs`.

### `/skills` command filtered out
The `/skills` command must be allowlisted when filtering static commands. If accidentally removed, users lose the skill browsing entry point. Mitigation: explicit allowlist check in `recompute_active_commands()`.

## Testing and Validation
- Verify `/` opens the menu with only `/skills` and natively supported skills (no `/agent`, `/new`, etc.).
- Verify only natively supported skills appear (e.g., `.claude/` skills for Claude Code, `.agents/` skills for Codex).
- Verify non-native skills (including bundled Warp skills) are hidden from the CLI agent input menu.
- Verify selecting a skill inserts `/{skill-name} ` for passthrough.
- Verify all inserted content submits correctly to the PTY.
- Verify no regressions in normal Warp agent input (slash menu and skills still work as before).

## Follow-ups
- Surface native CLI agent slash commands (e.g., Claude Code's `/compact`, `/model`) in the menu (APP-3641).
