# Queued Prompts V2 for `/compact-and` and `/fork-and-compact` — Tech Spec
Builds on the regular Agent Mode queued-prompts panel from `specs/REMOTE-1543/` and the Cloud Mode extension from `specs/APP-4562/`. This spec is the third (and final) step in moving every user-facing queued prompt surface onto the new panel UI behind `QueuedPromptsV2`.
## Context
Today `/compact-and <prompt>` and `/fork-and-compact <prompt>` still file their follow-up prompts through the legacy `PendingUserQueryBlock` path while a summarize (or fork-then-summarize) runs. The follow-up appears as a rich-content placeholder in the blocklist instead of as a row in the new queued-prompts panel.
Both flows funnel into one helper:
- `TerminalView::send_user_query_after_next_conversation_finished` (`app/src/terminal/view/pending_user_query.rs:185`) inserts a `PendingUserQueryKind::QueuedPrompt` block via `insert_pending_user_query_block` and stashes a `queued_prompt_callback` on `TerminalView` (`app/src/terminal/view.rs:4218`).
- `TerminalView::handle_finished_conversation` (`app/src/terminal/view.rs:4788`) drains both the new queue (`drain_queued_prompts`) and the legacy callback. The callback submits via `Input::submit_queued_prompt` on `FinishReason::Complete` and restores the text into the input on the error/cancel reasons.
The two callsites that reach this helper are:
- `/compact-and`: dispatched as `WorkspaceAction::SummarizeAIConversation { initial_prompt, .. }`, which calls `Workspace::summarize_active_ai_conversation` (`app/src/workspace/view.rs:12173-12200`). The active terminal view sends `SlashCommandRequest::Summarize` and, if an `initial_prompt` is present, calls `send_user_query_after_next_conversation_finished(prompt, /*show_close*/ true, /*show_send_now*/ false, ctx)`.
- `/fork-and-compact`: dispatched as `WorkspaceAction::ForkAIConversation { summarize_after_fork: true, initial_prompt, .. }`. After the fork is created and restored into the new pane, `Workspace::handle_forked_conversation_prompts` (`app/src/workspace/view.rs:12071-12117`) sends `SlashCommandRequest::Summarize` on the forked terminal view, then calls the same `send_user_query_after_next_conversation_finished` helper on that new terminal view. The forked conversation has already been restored via `restore_conversation_after_view_creation`, so `selected_conversation_id` on the new terminal view resolves to the forked id by the time the helper runs.
Other `on_next_conversation_finished` callers (`app/src/terminal/view.rs:6744, 13597, 13656`) are internal init/project sequencing — they do not render user-visible queued prompts and stay on the existing path. A grep for `send_user_query_after_next_conversation_finished` confirms `/compact-and` and `/fork-and-compact` are the only remaining queued-prompt surfaces still on the legacy block.
The new queued-prompts panel (`app/src/ai/blocklist/queued_prompts_panel.rs`) and its model `QueuedQueryModel` (`app/src/ai/blocklist/queued_query.rs`) already support exactly the surface this spec needs: append a row per origin, drain on `FinishReason::Complete` via `TerminalView::drain_queued_prompts` (`app/src/terminal/view.rs:5106`), and restore text to the input on error/cancel. For local Agent Mode, `drain_queued_prompts` submits via `Input::submit_queued_prompt_for_active_pane` (`app/src/terminal/input.rs:13145`), which falls back to `Input::submit_queued_prompt` (`app/src/terminal/input.rs:13062`) — the exact path the legacy callback already uses. No new submission plumbing is required.
## Proposed changes
### 1. Two new `QueuedQueryOrigin` variants for telemetry
Extend `QueuedQueryOrigin` (`app/src/ai/blocklist/queued_query.rs:22`) with two variants matching the existing per-origin pattern:
- `CompactAndSlashCommand`
- `ForkAndCompactSlashCommand`
Neither variant is locked (`QueuedQuery::is_locked` keeps `InitialCloudMode` as the only locked origin at `app/src/ai/blocklist/queued_query.rs:63`). These rows are user-managed: interactive drag, edit, and delete, exactly like `QueueSlashCommand` and `AutoQueueToggle` rows. The follow-up has not been sent anywhere yet at queue time; only the summarize is running.
Mirror the new variants into `TelemetryQueuedQueryOrigin` (`app/src/server/telemetry/events.rs:1187`) and its `From<QueuedQueryOrigin>` impl. No new telemetry events are needed — the existing `QueuedPrompt.Edited` / `QueuedPrompt.Deleted` / `QueuedPrompt.Reordered` / `QueuedPrompt.PanelCollapseToggled` events already carry `origin` and gate on `FeatureFlag::QueueSlashCommand`, which is transitively enabled by `QueuedPromptsV2` per `app/Cargo.toml:942`.
### 2. Single `TerminalView` helper hides the V2 gate from callers
Add `TerminalView::enqueue_followup_prompt` (next to `enqueue_prompt` at `app/src/terminal/view.rs:5051`):
```rust path=null start=null
pub fn enqueue_followup_prompt(
    &mut self,
    prompt: String,
    origin: QueuedQueryOrigin,
    conversation_id: AIConversationId,
    ctx: &mut ViewContext<Self>,
) {
    if FeatureFlag::QueuedPromptsV2.is_enabled() {
        self.queued_query_model.update(ctx, |model, ctx| {
            model.append(conversation_id, QueuedQuery::new(prompt, origin), ctx);
        });
    } else {
        self.send_user_query_after_next_conversation_finished(
            prompt,
            /* show_close_button */ true,
            /* show_send_now_button */ false,
            ctx,
        );
    }
}
```
The helper takes an explicit `conversation_id` so the `/fork-and-compact` caller can pass `forked_conversation_id` directly without depending on `selected_conversation_id` post-restoration ordering. The legacy branch ignores it, matching the existing helper which uses the input's selected conversation lazily inside the callback.
The helper is the only place that names `FeatureFlag::QueuedPromptsV2` for this rollout; subsequent cleanup deletes the legacy branch and the helper collapses to a one-liner.
### 3. Route both slash-command paths through the helper
Replace the two `send_user_query_after_next_conversation_finished` callsites in `Workspace` so they delegate to the helper instead:
- `Workspace::summarize_active_ai_conversation` (`app/src/workspace/view.rs:12194`): after sending `SlashCommandRequest::Summarize`, resolve the active conversation id via `terminal.ai_context_model().as_ref(ctx).selected_conversation_id(ctx)` and call `terminal.enqueue_followup_prompt(prompt, QueuedQueryOrigin::CompactAndSlashCommand, conversation_id, ctx)`. If `selected_conversation_id` is `None`, fall through with no follow-up (matches the legacy semantics — the slash-command handler at `app/src/terminal/input/slash_commands/mod.rs:1030-1048` already requires an active conversation to dispatch `/compact-and`).
- `Workspace::handle_forked_conversation_prompts` (`app/src/workspace/view.rs:12097`): pass the already-known `forked_conversation_id` and use `QueuedQueryOrigin::ForkAndCompactSlashCommand`.
Both callsites continue to send `SlashCommandRequest::Summarize` via `ai_controller` before enqueueing the follow-up, so the conversation transitions to `InProgress` before any drain hook can observe it.
### 4. Legacy path is preserved when the flag is off
When `QueuedPromptsV2` is off, the helper's `else` branch is the exact code that runs today: `send_user_query_after_next_conversation_finished` inserts a `PendingUserQueryKind::QueuedPrompt` block, sets `queued_prompt_callback`, and `handle_finished_conversation` drains it on completion. No other code paths change.
`PendingUserQueryIndicator` continues to gate the block's visibility inside `send_user_query_after_next_conversation_finished` (`app/src/terminal/view/pending_user_query.rs:192`); this spec does not modify that gate.
## Testing and validation
Behavior maps to the existing regular-queue panel invariants in `specs/REMOTE-1543/PRODUCT.md` (12-30, 31-37). The new code is the helper plus two callsites; testing focuses on the helper's branching and on the routing changes:
- **Helper branching**: Add unit tests next to the existing terminal view tests in `app/src/terminal/view/queued_prompts_test.rs` proving (a) with V2 on, `enqueue_followup_prompt` appends a row with the supplied origin and conversation id to `QueuedQueryModel`, and (b) with V2 off, it calls `send_user_query_after_next_conversation_finished`, which sets `pending_user_query_view_id` and `queued_prompt_callback`. Use the same `App::test`-style harness already used by sibling tests in that file.
- **`/compact-and` integration**: Dispatch `WorkspaceAction::SummarizeAIConversation { initial_prompt: Some(...) }` on a terminal view with an active conversation. With V2 on, assert the queued-prompts panel contains exactly one row with origin `CompactAndSlashCommand`. With V2 off, assert a `PendingUserQueryKind::QueuedPrompt` rich content was inserted and the legacy callback is set.
- **`/fork-and-compact` integration**: Drive `Workspace::handle_forked_conversation_prompts` with `summarize_after_fork: true` and an `initial_prompt`, then assert the **forked** terminal view's `QueuedQueryModel` contains exactly one row with origin `ForkAndCompactSlashCommand` keyed by the forked conversation id. Verify the source terminal view's queue is untouched.
- **Drain semantics**: With V2 on, simulate `FinishReason::Complete` on the conversation and assert the row drains through `submit_queued_prompt_for_active_pane` → local fallback `submit_queued_prompt`. Simulate `FinishReason::Error` with an empty input and assert the row's text lands in the input editor (matches §35 of `specs/REMOTE-1543/PRODUCT.md`).
- **Telemetry**: Add a serialization assertion for both new `TelemetryQueuedQueryOrigin` variants in the existing telemetry test pattern at `app/src/server/telemetry/events.rs`.
- **Compile gating**: `cargo check -p warp` and `cargo check -p warp --features queued_prompts_v2` both pass; `cargo fmt` and `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings` per the WARP.md PR workflow.
Do not run the app to test.
## Parallelization
Single workstream. The helper, two callsite edits, origin enum changes, and telemetry mirror all sit in a tight ownership boundary (`TerminalView`, `Workspace`, `QueuedQueryModel`, telemetry events). Sub-agents would create merge churn on the same files without reducing wall-clock time.
## Risks and mitigations
- **`selected_conversation_id` not yet resolving to the forked id on `CurrentPane` forks**: the helper takes an explicit `conversation_id` from the caller, and the `/fork-and-compact` path already has `forked_conversation_id` in scope at `handle_forked_conversation_prompts`. The `/compact-and` path uses the selected conversation id resolved at dispatch time on the active terminal view, which is guaranteed to be set because the slash-command handler short-circuits when none exists.
- **Double drain on the legacy path**: `handle_finished_conversation` calls both `drain_queued_prompts` and the legacy `queued_prompt_callback`. With V2 off, only the legacy callback fires (no row was appended). With V2 on, only the V2 row is present (no callback was set). The helper's branching guarantees the two paths are mutually exclusive.
- **Telemetry origin drift between core and telemetry enums**: extending `QueuedQueryOrigin` without mirroring `TelemetryQueuedQueryOrigin` would compile but ship payloads without telemetry for the new origins. The exhaustive `match` in the `From` impl catches that at compile time, per the WARP.md "Exhaustive Matching" guideline.
- **Removing the legacy block UI prematurely**: this spec deliberately keeps `send_user_query_after_next_conversation_finished` and `PendingUserQueryBlock` intact for the V2-off case. Cleanup happens when `QueuedPromptsV2` is removed in a later pass.
