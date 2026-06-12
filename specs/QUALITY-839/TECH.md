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
- [`app/src/settings/ai.rs:496-533 @ 8e984f0d`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/settings/ai.rs#L496-L533) — `PromptSubmissionMode` setting; the new enum setting is defined next to it and follows the same `implement_setting_for_enum!` pattern.
- [`app/src/settings_view/ai_page.rs:5771-5790 @ 8e984f0d`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/settings_view/ai_page.rs#L5771-L5790) — `AIInputWidget::render` places the "Default prompt submission mode" dropdown under `FeatureFlag::QueueSlashCommand`; the new dropdown goes directly below it. Palette wiring pattern for the sibling setting: `init_actions_from_parent_view` (L367-388) + context flags in [`settings_view/mod.rs:521-522`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/settings_view/mod.rs#L521-L522) + flag computation in [`workspace/view.rs:22371`](https://github.com/warpdotdev/warp/blob/8e984f0d784f38684472054978db10f39ff7ea5c/app/src/workspace/view.rs#L22371).

No new feature flag (per decision): everything ships under the existing `QueueSlashCommand` gate that already wraps the chip, the queue panel, and the submission intercept.

## Proposed changes

### 1. Setting (`app/src/settings/ai.rs`)

New enum setting in `AISettings`, defined next to `PromptSubmissionMode`:

```
pub enum LongRunningCommandSubmissionMode {
    SendImmediately,
    #[default]
    QueueUntilCommandCompletes,
}
```

registered via `implement_setting_for_enum!` (cloud-synced, `toml_path: "agents.warp_agent.other.long_running_command_submission_mode"`, `feature_flag: FeatureFlag::QueueSlashCommand`) and stored as the `long_running_command_submission_mode` field. `display_name()` returns "Send immediately" / "Queue until command finishes"; `command_palette_description()` the matching "Set long-running command submission: …" strings. The settings macro generates the matching `AISettingsChangedEvent::LongRunningCommandSubmissionMode` variant used below.

### 2. LRC-aware enablement, computed at the call sites (`queued_query.rs`, `input.rs`, `status_bar.rs`)

No LRC state is pushed into `QueuedQueryModel`; each call site determines the LRC context itself from the terminal model it already holds, and the model only answers the enablement question given that context.

- `QueuedQueryModel::is_queue_next_prompt_enabled` gains a `lrc_auto_queue_active: bool` parameter, computed via the shared `is_lrc_auto_queue_active` helper (`queued_query.rs`): true exactly when the queue feature flag is on, `default_prompt_submission_mode == Interrupt`, `long_running_command_submission_mode == QueueUntilCommandCompletes`, and the active block's agent controls the command for this conversation. The Interrupt requirement keeps the whole LRC machinery inert in Queue mode (PRODUCT §5). When true, return the LRC-scoped override if set, else `true` (auto-enabled); when false, existing logic (persistent override → cached default). The persistent override is never consulted or written while the LRC branch is in effect, which yields the revert-on-LRC-end semantics for PRODUCT §13, §16.
- One new field on `ConversationQueueState`: `queue_next_lrc_prompt_override: Option<bool>` — a manual toggle made during an agent-controlled LRC. It is explicitly cleared when the command ends: `TerminalView` calls `clear_queue_next_lrc_prompt_override` on `CLISubagentEvent::FinishedSubagent` (PRODUCT §14-15, §22). Also dropped with the conversation's queue state.
- Call sites that compute `lrc_auto_queue_active` (each already holds the terminal model):
  - `maybe_queue_input_for_in_progress_conversation` (`input.rs`) — the routing decision stays in the input, as today.
  - `agent_mode_hint_text` (`input.rs`) — ghost text (PRODUCT §12).
  - `render_warping_indicator_for_latest_exchange` (`status_bar.rs`) — the chip's `is_active` (PRODUCT §11); this render already reads `is_agent_in_control` from the locked terminal model.
- The settings are read directly from `AISettings` at each call site (no cache), so mid-LRC setting flips take effect on the next render/submission (PRODUCT §21). For chip/hint re-render on the setting change, `QueuedQueryModel`'s `AISettingsChangedEvent` subscription also re-emits `DefaultModeChanged` for the `LongRunningCommandSubmissionMode` variant.

### 3. Toggle routing (`app/src/terminal/view.rs`)

In the `ToggleQueueNextPrompt` handler, check `is_lrc_auto_queue_active`: when true, call `QueuedQueryModel::toggle_queue_next_prompt_during_lrc(conversation_id, ctx)`, which writes `queue_next_lrc_prompt_override = Some(!current_effective)`; otherwise the existing `toggle_queue_next_prompt`. Both emit `QueueNextPromptToggled`, which the status bar and input already subscribe to. Re-render on control transitions themselves (agent takes/loses control) is covered by the status bar's existing `UpdatedControl`/`BlockCompleted` notifies; the input additionally subscribes to `CLISubagentEvent` (`SpawnedSubagent`/`UpdatedControl`/`FinishedSubagent`/`ControlHandedBackAfterTransfer`) to refresh the ghost text, since its hint subscriptions did not previously cover control transitions.

### 4. Queued-row origin (`queued_query.rs`, `input.rs`, `server/telemetry/events.rs`)

New `QueuedQueryOrigin::LrcAutoQueue` variant (and matching `TelemetryQueuedQueryOrigin` value). `maybe_queue_input_for_in_progress_conversation` uses it instead of `AutoQueueToggle` when the LRC branch was the effective enabler (`lrc_auto_queue_active` was true and the non-LRC logic would have returned false) and the submission is a prompt — command rows always keep `AutoQueueToggle` since they cannot be delivered to the agent (PRODUCT §10). The origin drives both the send-on-command-finish behavior (§5 below) and the panel row suffix (§6 below), and distinguishes the rows in `QueuedPrompt*` telemetry. Exhaustive matches on the enum get the new arm.

### 5. Send queued prompts when the command finishes (`app/src/terminal/view.rs`)

New `TerminalView::send_lrc_queued_prompts(conversation_id, ctx)`: collects the conversation's queued rows with `LrcAutoQueue` origin, and for each (in queue order) dispatches it via `Input::submit_queued_prompt_for_active_pane` + `QueuedQueryModel::remove_fired_row` — the same path the panel's send-now button uses. Called from the `CLISubagentEvent::FinishedSubagent` handler, right after `clear_queue_next_lrc_prompt_override` (PRODUCT §9). `FinishedSubagent` fires when the command block completes regardless of who held control at that moment, which gives the fire-after-manual-takeover behavior of §9. Rows with other origins are skipped and keep the existing end-of-response drain (`drain_queued_prompts`).

### 6. Queued row suffix (`app/src/terminal/view/queued_prompts_panel.rs`)

In `render_row`, non-command rows with `LrcAutoQueue` origin render an italic `sub_text_color` suffix — `"(queued until the command finishes)"` (`LRC_AUTO_QUEUE_ROW_SUFFIX`) — after the preview text, mirroring the model picker's "(selected)" treatment. The preview is wrapped in `Shrinkable::new(1., …)` so it shrinks to its text (clipping with an ellipsis when long) and the suffix hugs it.

### 7. Settings UI (`app/src/settings_view/ai_page.rs`)

Inside the existing `FeatureFlag::QueueSlashCommand.is_enabled()` block in `AIInputWidget::render`, after the "Default prompt submission mode" dropdown: a second `render_dropdown_item` labeled "Default long-running command submission mode", rendered only when `default_prompt_submission_mode == Interrupt` (PRODUCT §18). The dropdown handle (`lrc_submission_mode_dropdown`) lives on `AISettingsPageView`, is built by `OtherAIWidget::create_lrc_submission_mode_dropdown` (the `create_default_prompt_submission_mode_dropdown` pattern), and re-syncs its selection on `AISettingsChangedEvent::LongRunningCommandSubmissionMode`. A new `AISettingsPageAction::SetLongRunningCommandSubmissionMode(mode)` persists via `set_value` (the `SetPromptSubmissionMode` pattern). LRC terms live in `AIInputWidget::search_terms`.

### 8. Command palette (`settings_view/mod.rs`, `workspace/view.rs`, `ai_page.rs`)

- New context flags `LRC_SUBMISSION_SEND_IMMEDIATELY` / `LRC_SUBMISSION_QUEUE_UNTIL_COMMAND_COMPLETES` in `settings_view/mod.rs` flags, set from `ai_settings.long_running_command_submission_mode` in the workspace context computation (the `PROMPT_SUBMISSION_*` pattern).
- Per-mode `FixedBinding`s registered in `ai_page::init_actions_from_parent_view` next to the `PromptSubmissionMode` bindings, additionally gated on `PROMPT_SUBMISSION_INTERRUPT` so the entries hide when the setting is hidden (PRODUCT §20).

## Testing and validation

- `app/src/ai/blocklist/queued_query_tests.rs` (model-level, maps to PRODUCT invariants):
  - `is_queue_next_prompt_enabled` with `lrc_auto_queue_active` → enabled by default; without → existing behavior unchanged (§1, §13, §19).
  - `toggle_queue_next_prompt_during_lrc` writes the LRC-scoped override, leaves the persistent override untouched, and re-toggling re-enables (§14, §15); `clear_queue_next_lrc_prompt_override` (command end) restores auto-enable for the next LRC (§22) and the pre-LRC state is what the non-LRC path returns afterward (§13, §16).
- `app/src/terminal/input_tests.rs` (host-level, next to the existing queue host tests): with the active block's agent in control and the default settings, a non-empty AI submission queues instead of submitting, with `LrcAutoQueue` origin (§6); ghost text returns the queue hint (§12); "Send immediately" → submission routes as today (§19, §21); Queue default mode → row queues with the generic origin (§5); `send_lrc_queued_prompts` fires `LrcAutoQueue` rows in order and leaves other rows queued (§9).
- Chip state (§11) is a pure read of `is_queue_next_prompt_enabled` — covered by the model tests; verify visually in the manual smoke.
- Manual smoke: run a dev-server-style command via the agent, let the agent take control, submit two prompts (both queue, with the row suffix), let the command finish (both fire to the agent in order, queue mode reverts), toggle the chip off mid-LRC (submission steers immediately), and flip the dropdown in Settings → AI mid-LRC (including hiding it by switching the default mode to Queue).
- `cargo check` + `./script/format`; full presubmit before PR per repo workflow.

## Parallelization

Not beneficial: the change is a single coupled chain (setting → model API → call sites → settings UI) where each step consumes the previous one's types. A single agent implements it on this branch (`harry/quality-839-auto-enable-prompt-queueing-during-lrc`).
