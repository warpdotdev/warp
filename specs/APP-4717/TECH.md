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

1. Shared dispatch helper on `Input` (`app/src/terminal/input.rs`): extract the body of the `QueuedPromptsPanelEvent::SendNow` arm into `fn send_queued_row_immediately(&mut self, conversation_id, query_id, text, is_command, trigger, ctx)`. Both the panel-event arm and the Enter path call it. It emits the new telemetry event (below) on dispatch.
2. Panel send state (`app/src/terminal/view/queued_prompts_panel.rs`):
   - `can_send_prompt: bool` (host-pushed via the change-detecting `set_can_send_prompt` setter) — whether the terminal can send prompts at all (false for read-only shared-session viewers, via the existing `SharedSessionStatus::is_reader()` helper). Gates the Send-now buttons (re-runs `update_send_now_availability`, with a "Read-only viewers cannot send prompts." tooltip), empty-Enter sends, and the hint. Edit/delete buttons are unaffected.
   - Host-input emptiness is *not* pushed: the panel holds the host editor's `ViewHandle` (passed at construction) and reads `is_empty` live at decision time, so the Enter decision cannot trail same-update buffer changes. A subscription to the host editor's `Edited`/`BufferReplaced` events re-renders the panel on empty↔non-empty transitions (a cached `host_editor_was_empty` flag only damps these notifications), mirroring the `CLIAgentSessionsModel` pattern below.
   The panel observes the CLI-agent rich input itself (a `CLIAgentSessionsModel` subscription plus a live `is_input_open` read — Enter submits to the CLI agent while it is open) and exposes `enter_sends_queued_prompt(ctx)` = `should_render` + `can_send_prompt` + live editor emptiness + rich input closed. The hint shows when that holds, no row is in inline edit mode, and the head row is sendable (`!is_locked()`), so the hint can never advertise an Enter that wouldn't fire.
3. Enter path: inlined in `input_enter`'s dispatch chain (before `maybe_launch_cloud_handoff_request`): when `panel.enter_sends_queued_prompt(ctx)` holds, look up the head row of the active conversation's queue (`BlocklistAIHistoryModel::active_conversation_id` + `QueuedQueryModel::queue(...).first()`, skipping a locked head) and dispatch it via `send_queued_row_immediately`; a locked head means Enter does nothing.
4. Push sites: `Input` seeds `can_send_prompt` at panel construction (`is_reader`) and passes the editor handle in; `TerminalView::on_self_role_updated` pushes `set_can_send_prompt(role.can_execute())` when the shared-session role changes (panel reached via the `Input::queued_prompts_panel()` accessor).
5. Header hint rendering: `render_header` appends an enter keycap chip (`render_keystroke_with_color_overrides`, the same component the "? for help" message-bar hints use) followed by "to send" text. The text uses the header's `sub_text_color`; the keycap glyph uses `internal_colors::text_disabled` so it is dimmer. Spacing follows the message-bar hint rules (`render_message_bar_items`): 8px label→keycap, 4px keycap→text.
6. Telemetry (`app/src/server/telemetry/events.rs`): new event `QueuedPromptSentNow { origin: TelemetryQueuedQueryOrigin, trigger: QueuedPromptSendNowTrigger }` with `QueuedPromptSendNowTrigger { SendNowButton, EnterOnEmptyInput }`, payload + descriptions following the adjacent `QueuedPrompt*` events. Emitted from the shared helper in (1). (Send-now currently has no telemetry; this adds it for both triggers.)

No new feature flag: the behavior ships under the existing `QueueSlashCommand` gate the panel already requires.

## Testing and validation

- Unit tests in `app/src/terminal/input_tests.rs` next to the existing queued-panel host tests (L1277+), driving `input_enter`:
  - empty buffer + queued prompt row → head row dispatched, removed from queue, buffer untouched (PRODUCT §1, §11); a second Enter sends the next row (§3).
  - empty buffer + queued command row, default shell mode → command executed instead of an empty shell submission (§1, §2).
  - non-empty buffer → no queue send (§6).
  - locked initial cloud-mode head row → no send (§5) — the `!is_locked` filter in the Enter path is the only guard on this path.
  Send-permission gating (read-only viewer) and the flag-off case are intentionally not host-tested: the former is a pushed flag whose effects are covered by the panel tests below, and with the flag off the panel (and hook target) doesn't exist.
- Panel tests in `app/src/terminal/view/queued_prompts_tests.rs`: hint hidden during inline edit and for a locked head (§7, §9); Send-now buttons disabled when `can_send_prompt` is false (via `send_now_button_disabled_for_test`) but not merely because the input is non-empty (§5). Panel tests reuse the host input's own panel when the flag is on — a second panel on the same terminal view would fight over edit-editor focus and commit edits on blur.
- `cargo check` + `./script/format`; manual smoke: queue two prompts during a running conversation, hit Enter twice with an empty input.

## Parallelization

Not beneficial: the change is small and tightly coupled (one host file + one panel file share the dispatch helper and the empty-state plumbing). A single agent implements it on this branch (`harry/app-4717-change-it-so-hitting-enter-w-an-empty-buffer-and-queued`).
