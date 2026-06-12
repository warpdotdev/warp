# QUALITY-839 — Auto-queue prompts during agent-controlled long-running commands

See `specs/QUALITY-839/PRODUCT.md` for behavior. Researched at commit `8e984f0d784f38684472054978db10f39ff7ea5c` (branch `harry/quality-839-auto-enable-prompt-queueing-during-lrc`, stacked on the APP-4717 empty-input-Enter send-now work).

## Context

All read sites for "is queue mode on?" already funnel through one method, so the core of this feature is making that method LRC-aware:

- [`app/src/ai/blocklist/queued_query.rs:366 @ 8e984f0d`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/ai/blocklist/queued_query.rs#L366) — `QueuedQueryModel::is_queue_next_prompt_enabled`: per-conversation override falling back to the cached `AISettings::default_prompt_submission_mode`. `ConversationQueueState` (L152-164) holds the per-conversation override; `toggle_queue_next_prompt` (L377) flips it.
- [`app/src/terminal/input.rs:13778 @ 8e984f0d`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/terminal/input.rs#L13778) — `maybe_queue_input_for_in_progress_conversation`: the submission intercept; consults `is_queue_next_prompt_enabled` and conversation in-progress/blocked status. During an agent-controlled LRC the conversation status is `InProgress` (or `Blocked`), so no change is needed to its status gating.
- [`app/src/terminal/input.rs:6141 @ 8e984f0d`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/terminal/input.rs#L6141) — `agent_mode_hint_text`: ghost text switches to the queue hint (`AGENT_MODE_AI_ENABLED_QUEUE_HINT_TEXT_*`, L453-455) when `is_queue_next_prompt_enabled` is true and the conversation is in progress. PRODUCT §10 falls out automatically.
- [`app/src/ai/blocklist/block/status_bar.rs:838 @ 8e984f0d`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/ai/blocklist/block/status_bar.rs#L838) — the queue chip (`queue_next_prompt_button`) renders accent-colored when `is_queue_next_prompt_enabled` is true. PRODUCT §9 falls out automatically.
- [`app/src/terminal/model/block/interaction_mode.rs:102 @ 8e984f0d`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/terminal/model/block/interaction_mode.rs#L102) — `Block::is_agent_in_control`: the trigger condition (PRODUCT §1-2). Covers the blocked-on-approval state (`LongRunningCommandControlState::Agent { is_blocked, .. }`); excludes user-in-control and tagged-in-only.
- [`app/src/terminal/view.rs:27035 @ 8e984f0d`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/terminal/view.rs#L27035) — `ToggleQueueNextPrompt` handler (chip click + `Cmd-Shift-J`): resolves the active conversation and calls `QueuedQueryModel::toggle_queue_next_prompt`. `TerminalView` holds `self.model`, so it can check LRC control state when routing the toggle.
- Re-render on LRC transitions is already wired: the status bar notifies on `CLISubagentEvent::UpdatedControl` and `ModelEvent::BlockCompleted` ([`status_bar.rs:231-248, 312-330`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/ai/blocklist/block/status_bar.rs#L231-L248)), and its warping-indicator render already locks the terminal model and reads `is_agent_in_control` (L752-770). The input likewise already locks `self.model` on hot paths (e.g. `is_input_mode_toggle_disabled`, [`input.rs:14409`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/terminal/input.rs#L14409)).
- [`app/src/settings/ai.rs:496-533 @ 8e984f0d`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/settings/ai.rs#L496-L533) — `PromptSubmissionMode` setting; the new bool setting sits next to it. Bool-setting pattern: `submit_on_ctrl_enter` (L1291-1299).
- [`app/src/settings_view/ai_page.rs:5771-5790 @ 8e984f0d`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/settings_view/ai_page.rs#L5771-L5790) — `AIInputWidget::render` places the "Default prompt submission mode" dropdown under `FeatureFlag::QueueSlashCommand`; the new toggle goes directly below it. Palette wiring pattern for the sibling setting: `init_actions_from_parent_view` (L367-388) + context flags in [`settings_view/mod.rs:521-522`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/settings_view/mod.rs#L521-L522) + flag computation in [`workspace/view.rs:22371`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/workspace/view.rs#L22371).

No new feature flag (per decision): everything ships under the existing `QueueSlashCommand` gate that already wraps the chip, the queue panel, and the submission intercept.

## Proposed changes

### 1. Setting (`app/src/settings/ai.rs`)

New bool setting in `AISettings`:

```
auto_queue_prompts_during_long_running_commands: AutoQueuePromptsDuringLongRunningCommands {
    type: bool,
    default: true,
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "agents.warp_agent.other.auto_queue_prompts_during_long_running_commands",
    description: "Whether prompts submitted while an agent controls a long-running command are queued instead of sent immediately.",
}
```

The settings macro generates the matching `AISettingsChangedEvent` variant used below.

### 2. LRC-aware enablement, computed at the call sites (`queued_query.rs`, `input.rs`, `status_bar.rs`)

No LRC state is pushed into `QueuedQueryModel`; each call site determines the LRC context itself from the terminal model it already holds, and the model only answers the enablement question given that context.

- `QueuedQueryModel::is_queue_next_prompt_enabled` gains a `lrc_auto_queue_active: bool` parameter (true exactly when the active block's `is_agent_in_control()` is true AND the new setting is on; computed via the shared `is_lrc_auto_queue_active` helper). When true, return the LRC-scoped override if set, else `true` (auto-enabled); when false, existing logic (persistent override → cached default). The persistent override is never consulted or written while the LRC branch is in effect, which yields the revert-on-LRC-end semantics for PRODUCT §11, §14.
- One new field on `ConversationQueueState`: `queue_next_lrc_prompt_override: Option<bool>` — a manual toggle made during an agent-controlled LRC. It is explicitly cleared when the command ends: `TerminalView` calls `clear_queue_next_lrc_prompt_override` on `CLISubagentEvent::FinishedSubagent` (PRODUCT §12-13, §19). Also dropped with the conversation's queue state.
- Call sites that compute `lrc_auto_queue_active` (each already holds the terminal model):
  - `maybe_queue_input_for_in_progress_conversation` (`input.rs`) — the routing decision stays in the input, as today.
  - `agent_mode_hint_text` (`input.rs`) — ghost text (PRODUCT §10).
  - `render_warping_indicator_for_latest_exchange` (`status_bar.rs`) — the chip's `is_active` (PRODUCT §9); this render already reads `is_agent_in_control` from the locked terminal model.
- The setting is read directly from `AISettings` at each call site (no cache), so mid-LRC setting flips take effect on the next render/submission (PRODUCT §18). For chip/hint re-render on the setting change, extend `QueuedQueryModel`'s existing `AISettingsChangedEvent` subscription to also re-emit `DefaultModeChanged` for the new setting's variant.

### 3. Toggle routing (`app/src/terminal/view.rs`)

In the `ToggleQueueNextPrompt` handler (view.rs:27035), check the active block: if the agent is in control and the setting is on, call a new `QueuedQueryModel::toggle_queue_next_prompt_during_lrc(conversation_id, ctx)`, which writes `queue_next_lrc_prompt_override = Some(!current_effective)`; otherwise the existing `toggle_queue_next_prompt`. Both emit `QueueNextPromptToggled`, which the status bar and input already subscribe to. `TerminalView`'s existing `CLISubagentEvent::FinishedSubagent` handler clears the override when the command ends. Re-render on control transitions themselves (agent takes/loses control) is covered by the status bar's existing `UpdatedControl`/`BlockCompleted` notifies; the input additionally subscribes to `CLISubagentEvent` (`SpawnedSubagent`/`UpdatedControl`/`FinishedSubagent`/`ControlHandedBackAfterTransfer`) to refresh the ghost text, since its hint subscriptions did not previously cover control transitions.

### 4. Queued-row origin (`queued_query.rs`, `input.rs`, `server/telemetry/events.rs`)

New `QueuedQueryOrigin::LrcAutoQueue` variant (and matching `TelemetryQueuedQueryOrigin` value). `maybe_queue_input_for_in_progress_conversation` uses it instead of `AutoQueueToggle` when the LRC branch was the effective enabler (`lrc_auto_queue_active` was true and the non-LRC logic would have returned false), so existing `QueuedPrompt*` telemetry distinguishes auto-queued-during-LRC rows. Exhaustive matches on the enum get the new arm.

### 5. Settings UI (`app/src/settings_view/ai_page.rs`)

Inside the existing `FeatureFlag::QueueSlashCommand.is_enabled()` block in `AIInputWidget::render`, after the dropdown: a toggle row labeled "Auto-queue prompts during long-running commands" with an info tooltip carrying the in-depth explanation (PRODUCT §15), built via `render_body_item_label` with `AdditionalInfo { tooltip_override_text }` + `build_toggle_element` — the same pattern as the "Auto show/hide Rich Input" toggle ([`ai_page.rs:6683-6714`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/settings_view/ai_page.rs#L6683-L6714)). New `SwitchStateHandle` + `MouseStateHandle` on `AIInputWidget`, and a new `AISettingsPageAction::ToggleAutoQueuePromptsDuringLrc` handled via `toggle_and_save_value` (the `submit_on_ctrl_enter` pattern). Add LRC terms to `AIInputWidget::search_terms`.

### 6. Command palette (`settings_view/mod.rs`, `workspace/view.rs`, `ai_page.rs`)

- New context flag `AUTO_QUEUE_PROMPTS_DURING_LRC_FLAG` in `settings_view/mod.rs` flags.
- Set it from `*ai_settings.auto_queue_prompts_during_long_running_commands` in the workspace context computation (`workspace/view.rs:22284`-style).
- Register a `ToggleSettingActionPair::custom` in `ai_page::init_actions_from_parent_view` ("Enable/Disable auto-queue prompts during long-running commands") gated `.with_enabled(|| FeatureFlag::QueueSlashCommand.is_enabled())`, in the `IS_ANY_AI_ENABLED` context.

## Testing and validation

- `app/src/ai/blocklist/queued_query_tests.rs` (model-level, maps to PRODUCT invariants):
  - `is_queue_next_prompt_enabled` with `lrc_auto_queue_active` → enabled by default; without → existing behavior unchanged (§1, §11, §16).
  - `toggle_queue_next_prompt_during_lrc` writes the LRC-scoped override, leaves the persistent override untouched, and re-toggling re-enables (§12, §13); `clear_queue_next_lrc_prompt_override` (command end) restores auto-enable for the next LRC (§19) and the pre-LRC state is what the non-LRC path returns afterward (§11, §14).
- `app/src/terminal/input_tests.rs` (host-level, next to the existing queue host tests): with the active block's agent in control and the setting on, a non-empty AI submission queues instead of submitting, with `LrcAutoQueue` origin (§5); ghost text returns the queue hint (§10); setting off → submission routes as today (§16, §18).
- Chip state (§9) is a pure read of `is_queue_next_prompt_enabled` — covered by the model tests; verify visually in the manual smoke.
- Manual smoke: run a dev-server-style command via the agent, let the agent take control, submit two prompts (both queue), press Enter on empty input (head row fires to the subagent), toggle the chip off (submission steers immediately), let the command finish (queue mode reverts), and flip the setting in Settings → AI mid-LRC.
- `cargo check` + `./script/format`; full presubmit before PR per repo workflow.

## Parallelization

Not beneficial: the change is a single coupled chain (setting → model API → call sites → settings UI) where each step consumes the previous one's types. A single agent implements it on this branch (`harry/quality-839-auto-enable-prompt-queueing-during-lrc`).
