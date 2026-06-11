# APP-4717 — Enter on empty input sends the top queued prompt

See `specs/APP-4717/PRODUCT.md` for behavior. Researched at commit `e367c9de8b9629600885e40b029c10c8915f9ec8`.

## Context

- [`app/src/terminal/input.rs:12808 @ e367c9de`](https://github.com/warpdotdev/warp/blob/e367c9de8b9629600885e40b029c10c8915f9ec8/app/src/terminal/input.rs#L12808) — `Input::input_enter`. CLI-agent rich input returns early at the top (L12809-12867), so the queue-send path never applies there (PRODUCT §10). The else-if chain at L12984-12988 (`maybe_launch_cloud_handoff_request` / `maybe_queue_input_for_in_progress_conversation` / …) is where the new empty-buffer check slots in; all existing branches in that chain require a non-empty buffer, so ordering is conflict-free.
- [`app/src/terminal/input.rs:3755-3793 @ e367c9de`](https://github.com/warpdotdev/warp/blob/e367c9de8b9629600885e40b029c10c8915f9ec8/app/src/terminal/input.rs#L3755-L3793) — `handle_queued_prompts_panel_event`: the existing Send-now dispatch (command vs prompt, `remove_fired_row`, refocus). This is the logic Enter must reuse.
- [`app/src/terminal/view/queued_prompts_panel.rs:580-620 @ e367c9de`](https://github.com/warpdotdev/warp/blob/e367c9de8b9629600885e40b029c10c8915f9ec8/app/src/terminal/view/queued_prompts_panel.rs#L580-L620) — `SendNow` action handler; skips rows where `row.is_locked()` (the initial cloud-mode prompt), which is exactly the head-row sendability condition (`update_send_now_availability`, L285-324, disables the head only when it is the locked initial cloud-mode row).
- [`app/src/terminal/view/queued_prompts_panel.rs:853-903 @ e367c9de`](https://github.com/warpdotdev/warp/blob/e367c9de8b9629600885e40b029c10c8915f9ec8/app/src/terminal/view/queued_prompts_panel.rs#L853-L903) — `render_header` ("N queued" label) where the "⏎ to send" hint goes. `should_render` (L548-563) already gates on flag, inline menus, and queue presence.
- [`app/src/terminal/input.rs:9756-9763 @ e367c9de`](https://github.com/warpdotdev/warp/blob/e367c9de8b9629600885e40b029c10c8915f9ec8/app/src/terminal/input.rs#L9756-L9763) — `Input` already detects empty↔non-empty buffer transitions on every `Edited` event (`is_editor_empty_on_last_edit`); the panel can be driven from here.
- [`app/src/server/telemetry/events.rs:2945-2963 @ e367c9de`](https://github.com/warpdotdev/warp/blob/e367c9de8b9629600885e40b029c10c8915f9ec8/app/src/server/telemetry/events.rs#L2945-L2963) — existing `QueuedPrompt*` telemetry events to extend.

## Proposed changes

1. Shared dispatch helper on `Input` (`app/src/terminal/input.rs`): extract the body of the `QueuedPromptsPanelEvent::SendNow` arm into `fn send_queued_row_now(&mut self, conversation_id, query_id, text, is_command, trigger, ctx)`. Both the panel-event arm and the new Enter path call it. It emits the new telemetry event (below) before dispatch.
2. Two-tier gating on `Input`:
   - `fn pane_can_send_prompt(&self, ctx) -> bool` — pane-level availability: prompt sending is possible at all (not a read-only/non-executor shared-session viewer). Gates the Send-now buttons, the Enter path, and the hint.
   - `fn can_enter_send_queued_prompt(&self, ctx) -> bool` — `pane_can_send_prompt` plus the Enter-only conditions: `self.editor.as_ref(ctx).is_empty(ctx)` and CLI-agent rich input not open (`CLIAgentSessionsModel::is_input_open(terminal_view_id)`). Gates the Enter path and the hint only.
   These are the single source of truth consumed by (3) and (4), so the hint can never advertise an Enter that wouldn't fire, and the buttons can never disagree with Enter on pane-level availability (PRODUCT §5, §7).
3. Enter hook: add `fn maybe_send_top_queued_row_on_enter(&mut self, ctx) -> bool` and insert it at the front of the else-if chain in `input_enter` (before `maybe_launch_cloud_handoff_request`, input.rs:12984). Conditions:
   - `queued_prompts_panel` is `Some` and `panel.should_render(ctx)`;
   - `self.can_enter_send_queued_prompt(ctx)`;
   - head row of `QueuedQueryModel::queue(conv_id)` exists and `!row.is_locked()`, where `conv_id = BlocklistAIHistoryModel::active_conversation_id(self.terminal_view_id)` (same lookup the panel uses).
   When all hold, call `send_queued_row_now(...)` with the head row and return `true`.
4. Panel state + UI (`app/src/terminal/view/queued_prompts_panel.rs`):
   - Add `pane_can_send: bool` and `enter_can_send: bool` fields to `QueuedPromptsPanelView` with a `pub fn set_send_availability(&mut self, pane_can_send, enter_can_send, ctx)` that `ctx.notify()`s and re-runs `update_send_now_availability` on change. `Input` pushes both predicate values at panel construction, from the existing empty-transition detection (input.rs:9756), and from the sites where the other predicate inputs change (CLI-agent rich input open/close, shared-session role changes); a small `refresh_queued_panel_send_availability(ctx)` helper on `Input` keeps the push sites uniform. Exact subscription points to be confirmed during implementation.
   - `update_send_now_availability` (L285-324) additionally disables every row's Send-now button when `!pane_can_send`, with a tooltip explaining sending is unavailable (e.g. "Read-only viewers cannot send prompts."). Edit/delete buttons are unaffected.
   - `render_header` gains the hint: when `enter_can_send`, no row is in inline edit mode, and the head row is sendable (`!is_locked()`), append a "⏎ to send" `Text` after the "N queued" label, same `sub_text_color` styling.
5. Telemetry (`app/src/server/telemetry/events.rs`): new event `QueuedPromptSentNow { origin: TelemetryQueuedQueryOrigin, trigger: QueuedPromptSendNowTrigger }` with `QueuedPromptSendNowTrigger { SendNowButton, EnterOnEmptyInput }`, payload + descriptions following the adjacent `QueuedPrompt*` events. Emitted from the shared helper in (1). (Send-now currently has no telemetry; this adds it for both triggers.)

No new feature flag: the behavior ships under the existing `QueueSlashCommand` gate the panel already requires.

## Testing and validation

- Unit tests in `app/src/terminal/input_tests.rs` next to the existing queued-panel host tests (L1277+), driving `input_enter`:
  - empty buffer + queued prompt row → head row dispatched, removed from queue, buffer untouched (PRODUCT §1, §11); a second Enter sends the next row (§3).
  - empty buffer + queued command row → command executed (§1).
  - non-empty / whitespace buffer → no queue send (§6).
  - shell-mode input with empty buffer → queue send instead of empty command (§2).
  - locked initial cloud-mode head row → no send (§5).
  - read-only shared-session viewer → no send (§5).
  - panel hidden (inline menu open / no queue) → no send (§8).
- Panel tests in `app/src/terminal/view/queued_prompts_tests.rs`: hint shown only when `enter_can_send` and head row sendable; hidden during inline edit and when the pushed flag is false (§7, §9); Send-now buttons disabled when `pane_can_send` is false (via `send_now_button_disabled_for_test`) but not merely because `enter_can_send` is false (§5).
- `cargo check` + `./script/format`; manual smoke: queue two prompts during a running conversation, hit Enter twice with an empty input.

## Parallelization

Not beneficial: the change is small and tightly coupled (one host file + one panel file share the dispatch helper and the empty-state plumbing). A single agent implements it on this branch (`harry/app-4717-change-it-so-hitting-enter-w-an-empty-buffer-and-queued`).
