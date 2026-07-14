# Edit sent agent messages and regenerate
## Context
Product behavior is defined in `specs/GH10933/product.md`. The feature affects the sent user prompt UI, AI request submission, conversation history mutation, persisted task state, and the existing rewind safety path.
Inspected commit: `ac4225c1805811a46bfa9df7531e6a4f0058ab12`.
- [`app/src/ai/blocklist/block/view_impl.rs (861-1019) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/ai/blocklist/block/view_impl.rs#L861-L1019) renders the user query portion of an `AIBlock` and wires the prompt header/overflow controls.
- [`app/src/ai/blocklist/block/view_impl/query.rs (24-111) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/ai/blocklist/block/view_impl/query.rs#L24-L111) renders sent prompt text and attachments.
- [`app/src/ai/blocklist/block/view_impl/header.rs (24-126) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/ai/blocklist/block/view_impl/header.rs#L24-L126) receives `conversation_id`, `exchange_id`, and overflow menu handles, which are enough to target an editable prompt exchange.
- [`app/src/terminal/view/context_menu.rs (394-475) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/terminal/view/context_menu.rs#L394-L475) builds the AI block overflow menu with copy, fork, and rewind items.
- [`app/src/terminal/view.rs (24287-24393) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/terminal/view.rs#L24287-L24393) implements rewind: cancel active progress, revert diffs, fork a pre-rewind backup, truncate history, remove AI blocks, and clear stale action results.
- [`app/src/terminal/view.rs (24301-24418) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/terminal/view.rs#L24301-L24418) splits the rewind flow into two phases: `show_rewind_confirmation_dialog` dispatches `WorkspaceAction::ShowRewindConfirmationDialog`, and `rewind_ai_conversation` (invoked only after confirmation) does the destructive work (cancel with `CancellationReason::Reverted`, stop any running command, revert diffs backward through blocks, fork a `PRE_REWIND_PREFIX` backup, `truncate_conversation_from_exchange`, `remove_ai_blocks_for_exchanges`, `clear_finished_action_results`). The edit flow reuses this same two-phase confirm-then-execute pattern.
- [`app/src/terminal/view.rs (25884-25904) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/terminal/view.rs#L25884-L25904) wires `TerminalAction::RewindAIConversation` → `show_rewind_confirmation_dialog` and `TerminalAction::ExecuteRewindAIConversation` → `rewind_ai_conversation`; the edit actions mirror this action pair.
- [`app/src/workspace/rewind_confirmation_dialog.rs (1-269) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/workspace/rewind_confirmation_dialog.rs#L1-L269) is the `RewindConfirmationDialog` view to reuse/adapt for the edit warning modal: fixed `enter`/`escape` bindings mapped to `RewindConfirmationAction::Confirm`/`Cancel`, a `RewindDialogSource { ai_block_view_id, exchange_id, conversation_id }` payload, a `Dialog` with destructive-warning copy plus Cancel/Rewind buttons, and `RewindConfirmationEvent::Confirm { rewind_source }` / `Cancel` events.
- [`app/src/ai/blocklist/block/view_impl/header.rs (107-135) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/ai/blocklist/block/view_impl/header.rs#L107-L135) builds the prompt header's `right_row`, which renders the hover-visible Rewind button (`ChildView::new(props.rewind_button)`, gated by `FeatureFlag::RevertToCheckpoints && !is_restored`) and the overflow menu button. This is where the new on-hover Edit button is added.
- [`app/src/ai/blocklist/block/view_impl.rs (924-951) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/ai/blocklist/block/view_impl.rs#L924-L951) is where `AIBlock::render` constructs `header::Props` with `rewind_button: &self.rewind_button`; the Edit button is a sibling `ViewHandle<ActionButton>` owned by `AIBlock` and passed through the same props.
- [`crates/warp_features/src/lib.rs @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/crates/warp_features/src/lib.rs) defines the `FeatureFlag` enum (re-exported via `warp_core::features::FeatureFlag`) where `FeatureFlag::EditSentAgentMessages` is added.
- [`app/src/ai/blocklist/controller.rs (196-273) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/ai/blocklist/controller.rs#L196-L273) defines `RequestInput`, including task-scoped `AIAgentInput`s and common request fields.
- [`app/src/ai/blocklist/controller.rs (2168-2366) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/ai/blocklist/controller.rs#L2168-L2366) sends a request after checking in-flight streams, building `ConversationData`, and subscribing to the response stream.
- [`app/src/ai/blocklist/controller.rs (3001-3062) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/ai/blocklist/controller.rs#L3001-L3062) builds a normal `AIAgentInput::UserQuery` from prompt text, current context, static query type, mode, and attachments.
- [`app/src/ai/blocklist/controller/input_context.rs (47-98) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/ai/blocklist/controller/input_context.rs#L47-L98) constructs request context for user queries.
- [`app/src/ai/blocklist/history_model.rs (776-806) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/ai/blocklist/history_model.rs#L776-L806) appends new request input to a conversation via `AIConversation::update_for_new_request_input`.
- [`app/src/ai/blocklist/history_model.rs (2036-2073) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/ai/blocklist/history_model.rs#L2036-L2073) exposes `truncate_conversation_from_exchange`.
- [`app/src/ai/agent/conversation.rs (1659-1750) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/ai/agent/conversation.rs#L1659-L1750) creates a new exchange for request input and emits `AppendedExchange`.
- [`app/src/ai/agent/conversation.rs (3758-3828) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/ai/agent/conversation.rs#L3758-L3828) truncates exchanges/messages from a selected exchange onward, resets the root task when empty, persists state, and cleans stale hidden/reverted exchange state.
- [`app/src/ai/blocklist/block/compact_agent_input.rs (16-97) @ ac4225c1805811a46bfa9df7531e6a4f0058ab12`](https://github.com/warpdotdev/warp/blob/ac4225c1805811a46bfa9df7531e6a4f0058ab12/app/src/ai/blocklist/block/compact_agent_input.rs#L16-L97) provides a reusable inline editor that submits on Enter and cancels on Escape.
## Proposed changes
1. Gate the feature with a new feature flag, tentatively `FeatureFlag::EditSentAgentMessages`, until dogfood confirms the destructive-regeneration UX.
2. Add edit entry points — a primary on-hover button plus the overflow menu item (both dispatch the same action):
   - Add a `TerminalAction` (e.g. `EditAIPrompt { ai_block_view_id, exchange_id, conversation_id }`) that opens inline edit mode for a prompt.
   - **On-hover button (primary).** Add an Edit `ActionButton` to the prompt header's `right_row` in `view_impl/header.rs`, mirroring the existing hover-visible `rewind_button`: a sibling `ViewHandle<ActionButton>` owned by `AIBlock`, constructed the same way `self.rewind_button` is and passed through `header::Props`. Gate it on `FeatureFlag::EditSentAgentMessages` and the same editability/`!is_restored` checks; it inherits the adjacent controls' hover-visibility behavior so it shows on prompt hover.
   - **Overflow menu (secondary).** Add “Edit message” to `open_ai_block_overflow_context_menu` and the rich-content block context menu near the existing Copy/Fork/Rewind items, dispatching the same `EditAIPrompt` action.
   - Expose both entry points only when a new helper confirms the block is mutable, non-restored, user-authored, active-request-like, and backed by a loaded local `AIConversation`.
3. Add AIBlock editing state:
   - Add an `AIBlock` edit state that stores the target input index, original prompt text, and a child editor view.
   - Reuse `CompactAgentInput` or a thin sibling view based on `EditorView` for multiline prompt editing. If `CompactAgentInput` is reused, extend it with configurable submit behavior so Enter/Shift+Enter match the final design.
   - Render the editor in place of `query::maybe_render` when the targeted prompt is in editing mode, with “Save and regenerate” and “Cancel” affordances using shared button themes. The “Save and regenerate” primary action opens the confirmation modal (item 5) rather than mutating state directly.
   - Keep attachments visible/read-only under the editor for the initial release.
4. Add prompt edit extraction helpers:
   - Add `AIConversation::editable_user_query_for_exchange(exchange_id)` or an equivalent helper returning the original `AIAgentInput::UserQuery` fields, the task ID, prompt text, and mode.
   - Keep this helper narrow: it should reject passive requests, resume-only exchanges, action-result-only exchanges, read-only transcript cases, and exchanges without a user query.
   - Add an `AIAgentInput` helper to clone a user query with replacement text while preserving original context, static query type, referenced attachments, user query mode, running-command metadata, and intended agent.
5. Gate the destructive send behind a reused confirmation modal, then edit-and-regenerate on confirm (mirroring rewind's two-phase confirm-then-execute):
   - When the user activates “Save and regenerate” (or presses Enter) with non-empty changed text, do not mutate state yet — show a warning confirmation modal first.
   - Reuse/adapt `RewindConfirmationDialog` (`app/src/workspace/rewind_confirmation_dialog.rs`) for this modal: either parameterize its title/body/confirm-label so it can present edit-specific copy (e.g. “Save and regenerate” confirm), or add a thin sibling dialog that shares its structure and its `enter`/`escape` → Confirm/Cancel fixed bindings. Carry an edit payload analogous to `RewindDialogSource` (block/exchange/conversation ids plus the edited text).
   - Add a `TerminalAction` pair mirroring `RewindAIConversation`/`ExecuteRewindAIConversation`: one action opens the modal (like `show_rewind_confirmation_dialog`), and one executes edit-and-regenerate on the modal's `Confirm` event. Cancel/Escape returns to inline edit with the edited text intact and no mutation.
   - On confirm, add `BlocklistAIController::edit_user_query_and_regenerate(conversation_id, exchange_id, edited_query, ctx)` that: validates non-empty changed text and resolves the editable query helper before mutating anything; cancels active work with `CancellationReason::Reverted`; reverts diffs from the edited block through the branch end (reuse the `revert_all_diffs` loop); forks a pre-edit backup via `fork_conversation` (new `PRE_EDIT_PREFIX`, or intentionally reuse the pre-rewind backup bucket); calls `truncate_conversation_from_exchange`; removes AI blocks via `remove_ai_blocks_for_exchanges`; and clears stale results via `clear_finished_action_results` — i.e. the same body as `rewind_ai_conversation`.
   - Send a new `RequestInput::for_task(vec![edited_user_query_input], resolved_task_id, ...)` through the existing `send_request_input` path so streaming, telemetry, status, persistence, usage refresh, and error handling stay consistent.
6. Preserve correct request context:
   - For the first version, preserve the original prompt’s captured `context` and `referenced_attachments` and replace only the query text. This makes regeneration deterministic with respect to the original turn and avoids silently attaching unrelated current terminal state.
   - If product later wants newly typed `<block:...>` or file references to resolve during edits, add explicit attachment-editing UX and parse those references separately instead of relying on current pending context.
7. Update metadata and navigation surfaces:
   - If the edited exchange was the initial query, ensure `AIConversation::initial_query`, `AIConversationMetadata.initial_query`, conversation title fallback, and `AgentConversationsModel` rows update to the edited text after the new request is appended.
   - Emit or reuse a history event that makes conversation lists/search rows refresh after an edit, especially when the title/initial query changes before streaming finishes.
8. Add telemetry and accessibility:
   - Add telemetry for edit opened, edit cancelled, edit submitted, edit submit no-op, and edit regeneration failed/succeeded, with existing conversation/exchange identifiers and entrypoint.
   - Add accessibility content for the menu action and inline editor controls.
   - Add the final keyboard shortcut only after product/design confirms the binding; wire it to the latest editable user prompt when the input buffer is empty.
9. Cloud/shared-session constraints:
   - Do not expose editing in shared-session viewer mode or transcript-only viewer mode.
   - For sharer-owned shared sessions, either defer editing or send a synthetic shared-session event so viewers see the same truncation/regeneration. The safer first release is local mutable conversations only, with shared-session author editing as a follow-up.
   - For ambient/cloud runs, verify whether the server uses supplied `ConversationData.tasks` as the authoritative context for follow-ups after truncation. If the server instead rehydrates from `server_conversation_token` history, add a backend/API change before enabling non-initial middle-message edits for cloud-backed conversations.
## End-to-end flow
1. User hovers a sent prompt and clicks the on-hover Edit button (or selects “Edit message” from the overflow menu).
2. `TerminalView` dispatches the edit action to the matching `AIBlock`.
3. `AIBlock` enters inline edit mode with the prompt text prefilled.
4. User edits the text and activates “Save and regenerate” (or presses Enter).
5. Because the send is destructive, `TerminalView` shows the reused Rewind-style confirmation modal instead of mutating state.
6. If the user cancels (Escape/Cancel), the block returns to inline edit with the edited text intact and nothing changes. If the user confirms (Enter/Confirm), the flow proceeds.
7. `AIBlock`/modal emits an event to `TerminalView` with `conversation_id`, `exchange_id`, and edited text; `TerminalView` calls the controller edit-and-regenerate entry point and performs UI-side diff reversion/block removal alongside history truncation.
8. `BlocklistAIHistoryModel` truncates the conversation from the old exchange onward.
9. `BlocklistAIController` sends the edited `AIAgentInput::UserQuery` through `send_request_input`.
10. Existing response stream handling appends the new exchange, streams output, persists conversation state, updates usage, and renders normal completion/error UI.
## Risks and mitigations
1. Server-side conversation context may not match local truncation for cloud-backed middle-message edits. Mitigate by verifying request semantics before enabling cloud-backed edits; if needed, fork/new-token server support should land before release.
2. Destructive truncation can discard useful downstream work. Mitigate with clear “Save and regenerate” copy, pre-edit backup, and reuse of existing rewind/fork recovery behavior.
3. Generated file edits can become inconsistent with regenerated history. Mitigate by reusing rewind’s diff reversion path and blocking regeneration if irreversible side effects cannot be handled.
4. Preserving original context may surprise users who edit text expecting to add/remove attachments. Mitigate by making attachment chips visible/read-only and documenting attachment editing as a follow-up.
5. Editing first prompts can leave stale titles/search metadata. Mitigate with explicit metadata refresh tests.
6. A one-click hover affordance for a destructive action risks accidental data loss. Mitigate by always routing the send through the reused Rewind-style confirmation modal (never regenerating directly from the hover button or inline Save) and by keeping the pre-edit backup.
## Testing and validation
1. Unit-test `AIConversation` helpers:
   - Finds editable user query input for a root-task exchange.
   - Rejects passive, resume, action-result, and non-user-query exchanges.
   - Produces an edited `AIAgentInput::UserQuery` that preserves context, attachments, mode, static query type, running-command metadata, and intended agent.
2. Unit-test history truncation plus edited append:
   - Editing the first exchange resets root state/server token as expected before the edited request is appended.
   - Editing a middle exchange preserves earlier exchanges and removes downstream message IDs, hidden exchange IDs, reverted action IDs, todo state, and stale response tracking.
3. Controller tests:
   - Empty edit and unchanged edit do not call truncation or send a request.
   - Changed edit cancels active streams, clears stale action results, truncates, and sends one new request.
   - In-flight downstream response cancellation uses the same cancellation path as rewind.
4. UI tests:
   - Editable blocks expose the on-hover Edit button and the “Edit message” menu item; read-only/restored/transcript blocks expose neither.
   - Edit mode pre-fills prompt text, Cancel restores normal rendering, and validation blocks empty prompt submission.
   - Activating “Save and regenerate” opens the confirmation modal; confirming dispatches edit-and-regenerate, and cancelling/Escape leaves history and the inline edit unchanged.
   - Copy query after save copies edited text.
5. Integration test:
   - Add a Warp integration test that sends an agent prompt, opens the edit action via the on-hover button, changes the prompt, submits, confirms the warning modal, verifies old response/downstream block removal, and verifies a new response block appears.
   - Add a second scenario for editing a middle prompt after a follow-up if the integration harness can make deterministic agent responses practical.
6. Manual validation:
   - Verify with a prompt that causes file edits and ensure regeneration reverts stale edits before sending the edited prompt.
   - Verify first-prompt edit updates conversation list title/search text.
   - Verify shared-session viewer and transcript viewer surfaces are read-only.
7. Video artifact (required acceptance criterion): capture a screen recording of the end-to-end flow — hover over a sent prompt → Edit → warning modal → confirm → regenerate — via computer use, and post it to the originating Slack thread and the PR. This is user-facing, so implementation must exercise the running UI and provide visual proof per `factory-verification`.
## Parallelization
Sub-agents are not proposed for the initial implementation. The UI action, destructive truncation, request resend, block removal, metadata refresh, and validation are tightly coupled around the same `AIBlock`/`TerminalView`/`BlocklistAIController`/`BlocklistAIHistoryModel` seams, so parallel edits would create conflict risk greater than the likely wall-clock savings. A single implementer should land the core flow first; a separate follow-up validation pass can be delegated after the implementation stabilizes.
## Follow-ups
1. Add attachment editing in the inline editor.
2. Add edit-without-regenerate if product decides preserving existing responses is useful.
3. Enable shared-session author editing once truncation/regeneration can be synchronized to viewers.
4. Enable cloud-backed middle-message editing after confirming server context semantics or adding a dedicated backend regeneration endpoint.
