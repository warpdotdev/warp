# Tech Spec: Attachments on Queued Prompts

See `specs/APP-4617/PRODUCT.md` for user-visible behavior.

## Context
Queued prompts (V2) store per-conversation rows in `QueuedQueryModel` (`app/src/ai/blocklist/queued_query.rs`). Before this change a `QueuedQuery` held only `text` + `origin`; staged attachments lived solely in the live input staging on `BlocklistAIContextModel.pending_attachments`. Two things coupled attachments to the live input rather than the queued row:

- At enqueue time the input attachments were left in place or cleared, so a row never carried them.
- The send path always sourced pending attachments from the context model: `input_for_query` built image context from `vec![]` and `input_context_for_request` (`app/src/ai/blocklist/controller/input_context.rs`) appended `context_model.pending_files()` as `FilePathReference`s. A fired queued row therefore picked up whatever was currently staged, and a direct send after queuing re-sent the previous attachments.

Relevant code (current branch):
- `app/src/ai/blocklist/queued_query.rs` — `QueuedQuery` (text/origin), `AutofireAction`, `pop_for_autofire` (removed the head row and returned its action)
- `app/src/ai/blocklist/context_model.rs` — `pending_attachments` staging, `clear_pending_attachments`
- `app/src/ai/blocklist/controller.rs:3132` — `input_for_query`; send path in `send_query` (~758)
- `app/src/ai/blocklist/controller/input_context.rs (230-)` — pending-file → `FilePathReference` conversion
- `app/src/ai/blocklist/controller/slash_command.rs:68` — `SlashCommandRequest::send_request`
- `app/src/terminal/input.rs:13179` `submit_queued_prompt`, `:13277` `submit_queued_prompt_for_active_pane`, `:5242` `execute_skill_command`, viewer send (~13767)
- `app/src/terminal/input/slash_commands/mod.rs (1069-)` — `/queue` handling
- `app/src/terminal/view.rs:5199` enqueue, `:5227` `drain_queued_prompts`
- `app/src/terminal/view/pending_user_query.rs` — legacy pending-query submission

## Proposed changes
### 1. Rows own their attachments (`queued_query.rs`)
`QueuedQuery` gains `attachments: Vec<PendingAttachment>`, a `new_with_attachments(text, origin, attachments)` constructor (`:59`; `new` delegates to it), and an `attachments()` accessor (`:100`). `attachments_for(conversation_id, query_id)` (`:374`) returns a row's attachments by id without removing it; it returns `&[]` when the row is absent.

### 2. Capture-and-clear at every enqueue site
Each enqueue site drains the live input via a new `BlocklistAIContextModel::take_pending_attachments(ctx)` (`context_model.rs:987`) and stores the drained set on the row with `new_with_attachments`. `take_pending_attachments` emits the same `UpdatedPendingContext` event as `clear_pending_attachments` so the input's attachment chips disappear. Sites: `terminal/view.rs:5199`, the two auto-queue-toggle paths in `terminal/input.rs` (~13433, ~13490), and the `/queue` in-progress branch in `slash_commands/mod.rs`.

### 3. Auto-fire becomes peek + remove (`queued_query.rs`, `view.rs`)
`pop_for_autofire` (which mutated the queue) is replaced by:
- `peek_autofire(conversation_id) -> Option<AutofireAction>` (`:326`) — read-only; returns the head row's action while leaving the row in the queue so the send path can resolve its attachments by id.
- `remove_fired_row(conversation_id, query_id, ctx)` (`:349`) — removes the row after dispatch/restore and clears edit state if it pointed at that row.

Both `AutofireAction` variants now carry `query_id`; `PopFromEditMode` additionally carries `attachments`. `drain_queued_prompts` (`view.rs:5227`) peeks, dispatches or restores, then calls `remove_fired_row`. This peek-then-remove ordering is required: the row must stay addressable during the synchronous send so attachments can be read by id.

### 4. Send path resolves attachments by source (`controller.rs`, `input_context.rs`)
`InputQuery` gains `queued_query_id: Option<QueuedQueryId>`. In `send_query`, the attachment set for a `UserSubmittedQueryFromInput` is resolved once:
- `Some(id)` → `QueuedQueryModel::attachments_for(conversation_id, id)` (the fired row)
- `None` → `context_model.pending_attachments()` (live staging)

`input_for_query` (`:3132`) now takes `prompt_attachments: Vec<PendingAttachment>`, splits them into image context (sent inline) and file references, and no longer relies on `input_context_for_request` for pending files. The pending-file → `FilePathReference` conversion (with duplicate-basename suffixing) moves out of `input_context.rs` into a shared `add_pending_file_attachments` (`controller.rs:3190`); `input_context.rs` no longer sources pending files.

### 5. Conversation routing for fired rows (`controller.rs`, `slash_command.rs`, `input.rs`)
A fired row routes into the conversation it was queued on rather than re-deriving from the current UI selection. `send_queued_slash_command_request` and `send_queued_user_query_in_conversation` thread `queued_query_id` plus a `conversation_id` override; `SlashCommandRequest::send_request` (`slash_command.rs:68`) replaces its `is_queued_prompt: bool` with `queued_query_id: Option<QueuedQueryId>` + `conversation_id_override: Option<AIConversationId>` and derives `is_queued_prompt` from the id. Queued skill invocations resolve `prompt_attachments` from the row (or `vec![]` if the conversation is unknown) and feed them through `add_pending_file_attachments` into `InvokeSkillUserQuery`.

### 6. Preserve next-prompt staging (`controller.rs`, `slash_command.rs`)
The context reset after a send is skipped when `is_queued_prompt` is true (the fired row's attachments came from the row, so the live `pending_attachments` belong to the user's next prompt). The same guard applies to queued skill invocations so they don't clear a new draft's staged attachments; direct skills still reset.

### 7. Split immediate vs. queued submission (`input.rs`, `pending_user_query.rs`, `slash_commands/mod.rs`)
- `submit_queued_prompt` (`input.rs:13179`) now takes `conversation_id` + `query_id` and submits the fired row into that conversation.
- New `submit_user_query_now` (`input.rs:13248`) is the immediate (non-queued) path that resets live staging; used by the `/queue` not-in-progress fallback and the legacy pending-user-query paths in `pending_user_query.rs`.
- `submit_queued_prompt_for_active_pane` (`input.rs:13277`) takes `conversation_id` + `query_id` and branches: cloud follow-up (drop attachments, log a warning), shared-session viewer (upload via the shared path below), local agent (`submit_queued_prompt`).
- `execute_skill_command` (`input.rs:5242`) replaces `is_queued_prompt: bool` with `queued_query_id` + `conversation_id_override`.

### 8. Shared viewer upload path (`input.rs`)
A new `upload_and_send_viewer_prompt` is extracted from the immediate viewer-submit path and shared with the queued viewer drain, so both go through the identical upload-then-send (`Event::SendAgentPrompt`) flow. The queued viewer drain reads the firing row's images/files from `attachments_for` and passes them in.

### 9. Re-stage on restore (`view.rs`)
`drain_queued_prompts`' `PopFromEditMode` branch and the manual edit/restore path call `context_model.append_pending_attachments(row attachments)` after restoring the row's text, so the chips reappear and a manual re-submit keeps them.

## Testing and validation
Unit tests added alongside the changed modules; each maps to PRODUCT.md invariants:
- `context_model_tests.rs` — `take_pending_attachments` drains and returns all staged attachments and clears the input (inv. 1); enqueue moves staged attachments onto the row and leaves the input empty (inv. 1, 7).
- `queued_query_tests.rs` — `peek_autofire` leaves the row until `remove_fired_row` drops it (inv. 3); `PopFromEditMode` carries committed text + attachments and peek is non-mutating (inv. 6).
- `controller_tests.rs` — `input_for_query` builds image/file context purely from the provided attachment set, ignoring live staging, including duplicate-basename suffixing (inv. 3, 5).
- `queued_prompts_tests.rs` — multi-cycle queue keeps each row's attachments independent and draining one leaves the other intact (inv. 2); shared `drain_one` helper mirrors peek + `remove_fired_row`.

Manual verification:
- Stage an image + file, queue while the agent is busy → chips clear from input; on fire the prompt arrives with the image inline and the file referenced (inv. 1, 3).
- Queue two prompts with different attachments → each fires with only its own (inv. 2).
- Stage attachments after queuing → they ride the next manual prompt, not the fired row (inv. 5).
- Edit-mode auto-fire pop and manual restore → attachments re-appear in the input (inv. 6).
- Cloud follow-up fire → text sent, attachments dropped, warning logged (inv. 10).
- Shared-session viewer fire → attachments uploaded and sent (inv. 11).

## Risks and mitigations
- **Double-fire / leaked rows:** peek no longer removes the row, so `remove_fired_row` must run after every dispatch and every restore. `drain_queued_prompts` removes in both `Submit` and `PopFromEditMode` arms immediately after the synchronous dispatch.
- **Attachment lifetime:** attachments are cloned when resolved by id during send and dropped when the row is removed; there is no shared ownership between the row and the live input, which is what keeps inv. 2 and inv. 5 independent.

## Parallelization
Not beneficial. The change is a single tightly-coupled thread through the queued-prompt enqueue, drain, and send paths (`queued_query.rs` → `view.rs`/`input.rs` → `controller.rs`/`slash_command.rs`); the signature changes ripple across these files and must land together. Best done sequentially in one PR.
