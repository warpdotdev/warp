# QUALITY-887 — Enter on empty input sends the top queued prompt

See `specs/QUALITY-887/PRODUCT.md` for behavior. QUALITY-887 is a copy of [APP-4717](https://linear.app/warpdotdev/issue/APP-4717); the behavior it asks for is already implemented and shipped under the existing `QueueSlashCommand` gate. This spec documents where that behavior lives, mapped to the current tree at commit `4c39fe77364abb5297a6076dc0338c83ef3bdba6`.

## Context
The queued prompts panel and its host input are the two surfaces involved. The panel owns the "would Enter send?" decision and the header hint; the host input owns the Enter dispatch chain and the shared send helper.
- `app/src/terminal/input.rs` — `Input::input_enter`. The else-if chain that routes Enter has a dedicated branch (before `maybe_launch_cloud_handoff_request` / `maybe_queue_input_for_in_progress_conversation`) that, when `panel.enter_sends_queued_prompt(ctx)` holds, looks up the head row of the active conversation's queue (`BlocklistAIHistoryModel::active_conversation_id` + `QueuedQueryModel::queue(...).first()`, skipping a `is_locked()` head) and dispatches it. A locked head means Enter does nothing.
- `app/src/terminal/input.rs` — `Input::send_queued_row_immediately`: the shared dispatch helper used by both the panel's `SendNow` event arm (`handle_queued_prompts_panel_event`) and the Enter path. It reads the row origin, dispatches (`execute_queued_command` for command rows, `submit_queued_prompt_for_active_pane` for prompts), emits the `QueuedPromptSentNow` telemetry event with the trigger, removes the fired row via `QueuedQueryModel::remove_fired_row`, and refocuses the input.
- `app/src/terminal/view/queued_prompts_panel.rs` — `QueuedPromptsPanelView`:
  - `enter_sends_queued_prompt(ctx)` = `should_render` + `can_send_prompt` + host editor empty (read live from the held `host_editor` handle) + CLI-agent rich input closed (`CLIAgentSessionsModel::is_input_open`).
  - `should_show_enter_hint(ctx)` = `enter_sends_queued_prompt` + no row in inline edit (`editing_row`) + head row sendable (`!row.is_locked()`), so the hint can never advertise an Enter that wouldn't fire.
  - `can_send_prompt` is host-pushed via `set_can_send_prompt` (false for read-only shared-session viewers); it gates the Send-now buttons (`update_send_now_availability`), empty-Enter sends, and the hint. Host-input emptiness is not pushed — a subscription to the host editor's `Edited`/`BufferReplaced` events (`handle_host_editor_event`, damped by `host_editor_was_empty`) only re-renders on empty↔non-empty transitions; decisions always read the editor live.
- `app/src/terminal/input.rs` — at panel construction, `Input` seeds `can_send_prompt` from `model.lock().shared_session_status().is_reader()` and passes the host editor handle into the panel. `TerminalView::on_self_role_updated` pushes `set_can_send_prompt(role.can_execute())` when the shared-session role changes (panel reached via `Input::queued_prompts_panel()`).
- `app/src/server/telemetry/events.rs` — `QueuedPromptSentNow { origin, trigger }` with `QueuedPromptSendNowTrigger { SendNowButton, EnterOnEmptyInput }`. Gated on `FeatureFlag::QueueSlashCommand`; serialized as `"QueuedPrompt.SentNow"`.

No new feature flag: the behavior ships under the existing `QueueSlashCommand` gate the panel already requires.

## Testing and validation
This behavior is already covered by tests; QUALITY-887 adds no new code, so the existing coverage is the validation baseline:
- `app/src/terminal/input_tests.rs` — `send_now_event_submits_through_active_pane_and_preserves_draft` and `send_now_command_event_executes_command_and_arms_in_flight` exercise the shared dispatch helper (prompt and command rows) including row removal and untouched draft buffer.
- `app/src/terminal/view/queued_prompts_tests.rs` — `can_send_prompt_gates_buttons_and_hint_while_nonempty_input_gates_only_the_hint` and `enter_hint_hidden_during_inline_edit_and_for_locked_head` cover the hint/Enter gating (read-only viewer, non-empty buffer, inline edit, locked head).
- Build/lint per repo workflow: `cargo check`, `./script/format`, and `cargo clippy` before any PR.

## Parallelization
Not beneficial: this is a documentation-only spec for an already-implemented feature. A single agent authors the spec.
