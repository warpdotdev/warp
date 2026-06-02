# Harness-Specific Model Selection in Orchestration Config

Linear: [QUALITY-643](https://linear.app/warpdotdev/issue/QUALITY-643)

## Summary

When a user selects a non-Oz harness (Claude Code, Codex) in the orchestration config UI, the model picker should show models that the selected harness actually supports, and the chosen model should reach the harness process. Today the model picker always shows Warp's internal model catalog regardless of harness, and those IDs are not recognized by third-party harness CLIs.

Figma: none provided — changes are behavioral within existing orchestration config UI chrome (plan card and run_agents confirmation card).

## Behavior

### Harness picker content

1. The harness picker populates from the server-provided `availableHarnesses` list (via `HarnessAvailabilityModel`), not from a hardcoded client-side list. This ensures the desktop matches the web UI and respects admin-configured harness availability.

2. Each harness entry displays its `display_name` from the server with the corresponding brand icon (Warp logo for Oz, Claude logo for Claude Code, OpenAI logo for Codex, Gemini logo for Gemini CLI).

3. Enabled harnesses are selectable. Disabled harnesses (admin-disabled via org settings) appear in the list with a visual indicator (e.g. greyed text or a "disabled" badge) but cannot be selected.

4. Enabled harnesses appear before disabled harnesses in the list.

5. If the currently-selected harness becomes disabled (e.g. after a server refresh), the picker retains the selection but the UI should indicate the issue.

### Model picker content by harness

6. When the harness picker is set to **Oz** (or is empty/unset), the model picker shows the Warp LLM catalog — the same models shown in single-agent mode. This is the current behavior and must not regress.

7. When the harness picker is set to **Claude Code**, the model picker shows:
   - A **"Default model"** entry at the top (value: empty string) meaning "don't override — let the harness use its own default."
   - The server-provided Claude Code model catalog (e.g. `best`, `opus`, `sonnet`, `haiku`, `opus (1M context)`, `sonnet (1M context)`, pinned versions like `opus 4.7`, `sonnet 4.6`).
   This matches the Oz web UI's harness model selector.

8. When the harness picker is set to **Codex** and the execution mode is **Cloud**, the model picker shows:
   - A **"Default model"** entry at the top (value: empty string), same as Claude Code.
   - The server-provided Codex model catalog (e.g. `default`, `GPT-5.5`, `GPT-5.4`, `GPT-5.4 mini`). The `default` entry from the server explicitly skips writing the model key to config, which has the same practical effect as the "Default model" entry but is Codex-specific.
   This matches the Oz web UI's Codex model selector.

   When the execution mode is **Local**, the model picker shows only the **"Default model"** entry. The Codex CLI reads its model from `~/.codex/config.toml`, which is shared global state — writing to it from a child agent would clobber the user's existing config and race with parallel agents. Local Codex children inherit whatever model the user has configured.

9. When the harness picker is set to **Gemini** (currently disabled for orchestration), the model picker follows the same pattern: "Default model" at top, then server-provided Gemini models if any exist.

10. Each model entry displays its `display_name` from the server catalog. The raw `id` (e.g. `"opus"`, `"gpt-5.4"`) is the value stored as the selected model_id and passed to the harness process. No provider icons or model spec sidecars are shown for harness models (they lack that metadata).

### Defaults when no model is specified

11. When the orchestration config is created (via `create_orchestration_config`) or the harness changes and no model_id is specified (empty string), the model picker defaults to:
   - **Oz**: the orchestrator's current model (the model the parent agent is using).
   - **Non-Oz harnesses** (Claude Code, Codex, Gemini): the "Default model" entry (empty string), meaning the harness uses its own default.

### Model reset on harness change

12. When the user changes the harness, the model_id resets because each harness has its own disjoint model catalog. The reset target is "Default model" (empty string) for non-Oz harnesses, or the first available Warp LLM for Oz. The only exception is the empty string itself ("Default model"), which can persist across non-Oz harness changes.

### Loading and empty states

13. If the harness model catalog has not yet been fetched from the server when the user switches to a non-Oz harness, the model picker shows only the "Default model" entry. Once the catalog arrives, the picker repopulates with the full list, keeping "Default model" selected.

14. If the server returns an empty model list for a harness, the model picker shows only the "Default model" entry. The model_id remains empty.

### Consistency across UI surfaces

15. The orchestration config block (plan card) and the run_agents confirmation card must show the same harness-specific models, using the same server catalog, and behave identically when the harness changes.

16. The `sync_picker_selections` logic (which syncs picker UI state to the edit state) must correctly match harness-specific model IDs against the harness model list, not against Warp's internal LLM catalog, when a non-Oz harness is active.

### Model ID delivery to harness processes

17. When the user selects a model for **Claude Code** and agents are launched, the selected model_id (e.g. `"opus"`) must reach the Claude Code CLI process as the `ANTHROPIC_MODEL` environment variable. This applies to both local child agents and remote agents.

18. When the user selects a model for **Codex** and agents are launched in **Cloud** mode, the selected model_id (e.g. `"gpt-5.4"`) must be written to `~/.codex/config.toml` as the top-level `model` key before the Codex CLI starts. If the selected model is `"default"`, the `model` key must NOT be written (or must be removed if previously present), allowing Codex to use its own default. In **Local** mode, no model override is written — the user's existing `~/.codex/config.toml` is used as-is.

19. When the user selects a model for **Oz**, the existing model_id propagation behavior (Warp internal LLM preference) must not change.

20. When the "Default model" entry is selected (empty model_id) for any non-Oz harness, no model override is injected — the harness process uses its own default.

### Auto-launch behavior

21. The `matches_active_config` check (which determines whether a run_agents call auto-launches without user confirmation) must treat harness-specific model_ids the same as Warp model_ids: an exact string match between the request's model_id and the config's model_id.
