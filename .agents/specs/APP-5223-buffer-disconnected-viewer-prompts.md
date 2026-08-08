# APP-5223: Preserve disconnected viewer prompts across reconnect and cloud follow-up

## Ticket provenance

- Linear issue: [APP-5223](https://linear.app/warpdotdev/issue/APP-5223/buffer-shared-session-agent-prompts-and-convert-to-cloud-follow-up-on)
- Ticket ID: `cdcdca24-d40e-465e-b1d5-ea587424e322`
- Ticket source: `adhoc`
- Relevant repository: `warpdotdev/warp`
- Estimate: L (5)

## Product specification

### Summary

When an executor submits an Agent Mode prompt as a shared-session viewer, the client currently
routes from `SharedSessionStatus`. That status remains `ActiveViewer` while the viewer websocket is
reconnecting. The prompt is frozen in the input and sent toward the viewer network, but the network
silently drops it whenever its stage is not `JoinedSuccessfully`. The prompt is then neither
delivered nor retained, and the input can remain frozen indefinitely.

Preserve every viewer prompt that the local network rejects or the session-sharing server does not
acknowledge within a bounded window by moving it into the existing conversation-keyed
`QueuedQueryModel`. A transient reconnect retries the head prompt into the same live shared session.
A fatal ambient-session disconnect converts the head prompt into a cloud follow-up when the viewer
owns an eligible task; the remaining rows continue in FIFO order after the replacement execution
session has joined. The existing queued-prompts panel remains the single durable and editable user
surface; no second pending-prompt queue is introduced.

### User-visible behavior

1. Submitting a prompt while the viewer websocket is joined behaves as it does today. The prompt
   travels to the sharer, the viewer input remains frozen only until the existing correlated
   `AgentPromptRequestInFlight` acknowledgement, and no visible queued row remains after that
   acknowledgement.
2. If the viewer network rejects the prompt because the network is reconnecting, has closed, or
   cannot enqueue the server message, or if the server does not return the correlated in-flight
   acknowledgement within five seconds, the exact prompt and its pending attachments appear once
   in the existing queued-prompts panel for that conversation.
3. After fallback queueing, the input is unfrozen immediately. The queue panel is the in-flight
   affordance. A draft the user typed after the submission began is never overwritten. If the
   editor was still showing only the failed submission, it is cleared through the same
   shared-input/ephemeral semantics used by successful viewer submissions.
   A queued prompt by itself is not treated as an active agent turn: when no independently active
   response stream or agent-controlled command remains, `conversation.status().is_in_progress()`
   and the corresponding controller state are false, so the stale `Warping...` indicator is not
   shown. If a prior turn is genuinely still running, its indicator remains valid and is not
   suppressed by the queued follow-up.
4. A transient reconnect automatically retries only the FIFO head into the same shared session
   after `RejoinedSuccessfully`, provided all of the following still match:
   - the reconnect event belongs to the current `Network`;
   - the session ID equals the row's original target session;
   - the row's conversation still resolves to its stored server conversation token;
   - the viewer is still an executor and the target conversation can accept a follow-up.
5. A reconnect does not blindly refire when the sharer has moved to a different conversation, the
   conversation was cancelled or removed, the viewer lost execution permission, or the session or
   server token no longer matches. The row stays queued and visible rather than being redirected to
   unrelated work.
6. When the ambient execution ends fatally and the current user owns a resumable task, the FIFO
   head becomes the cloud-to-cloud follow-up that starts the replacement execution. This handoff
   reuses the existing follow-up eligibility checks and never falls back to a local Agent Mode
   conversation.
7. After the replacement execution's shared session has actually joined, one next FIFO row is
   submitted into that live session. Later rows continue through the existing one-prompt-per-ready
   drain lifecycle. `ExecutionSessionReady` may arm the drain, but it must not send while the new
   network is still `BeforeJoined` or `Joining`.
8. A normal shared local session has the same transient-reconnect retry behavior. If it ends
   permanently, there is no cloud fallback: the row stays queued and visible.
9. Read-only viewers, non-owners, blocked task sources, missing task or conversation metadata,
   disabled handoff, and other ineligible fatal paths never lose or locally execute the prompt.
   `QueuedQueryModel` remains the canonical copy and the existing failure toast is shown. When the
   input is empty, any restoration affordance must remain linked to that same queue row; manually
   sending it atomically claims the row instead of creating an independent duplicate.
10. Queued cloud follow-ups preserve image and file attachments. They use
    `upload_pending_attachments_to_task` before follow-up dispatch, and a replacement VM can
    download the task attachments at startup. Upload or follow-up eligibility failure restores the
    same row at its original FIFO position and shows the existing toast.
11. Rows created by this failure path are unlocked while idle: users can edit, delete, reorder,
    manually push, or allow them to auto-fire using the existing panel behavior. A row may be
    transiently claimed by one dispatch attempt so concurrent lifecycle events cannot mutate or
    submit it twice.
12. The queue remains app-wide and conversation-keyed, so rows survive
    `TerminalManager::attach_execution_session` replacing the viewer `Network`.
13. No new item-count bound or age cap is added. This matches `QueuedQueryModel` today and retains
    the panel's direct edit/delete/reorder controls. Adding a special limit only for disconnected
    viewer rows would make otherwise identical queued prompts behave inconsistently and could
    reintroduce silent loss during a long outage.
14. If the queued-prompts compile/runtime surface is ever disabled, the client must not append a
    hidden row. It restores the failed submission to the input when safe and shows a failure toast.
    Stable builds require no separate implementation because `queue_slash_command`,
    `queued_prompts_v2`, `handoff_cloud_cloud`, and `cloud_mode_image_context` are default
    features at the pinned revision.
15. Diagnostic logging reports every server message rejected locally by
    `send_message_to_server`, including the network stage, shared-session ID, and a content-free
    message kind/discriminant. It also reports acknowledgement timeout using only request/session
    identifiers and static reason labels. It never logs prompt text, attachment names or contents,
    input buffer contents, or other user content.

### Out of scope

- A second queue owned by `Network`, `TerminalManager`, or `Input`.
- Persisting queued prompts across application restarts beyond `QueuedQueryModel`'s current
  lifecycle.
- A new server protocol message or durable server-side prompt inbox. The selected contract reuses
  the existing `AgentPromptRequestInFlight(AgentPromptRequestId)` acknowledgement as its
  server-round-trip acceptance signal.
- Automatically redirecting a row to a different conversation or unrelated shared session.
- A new feature flag, new queue panel, new toast copy, or a stable-channel fallback queue.
- Changing task ownership, cloud-follow-up eligibility, or `blocks_cloud_followups()` policy.

## Technical specification

### Current state and verified root cause

All references are pinned to `9c0a0fbc1e923e3bcfddf75ae6db9a454201a67d`.

- `QueuedQuery`, `QueuedQueryKind::Prompt`, attachments, origin locking, and the app-wide
  conversation-keyed `QueuedQueryModel` live in
  `app/src/ai/blocklist/queued_query.rs:43-287`. The history model removes queues when
  conversations are removed; queues do not belong to a viewer `Network`.
- `Input::submit_queued_prompt_for_active_pane` already chooses cloud follow-up before the viewer
  route and carries queued attachments into the viewer upload path
  (`app/src/terminal/input.rs:14021-14123`).
- `TerminalView::drain_queued_prompts` owns one-row autofire and the existing
  peek/submit/remove sequence (`app/src/terminal/view.rs:5469-5591`). Its current sequence is not
  failure-atomic: the row is removed after submission regardless of a later viewer-network
  rejection.
- `upload_files_then_submit_cloud_followup` already uploads pending attachments with
  `upload_pending_attachments_to_task` and restores the typed prompt on upload failure
  (`app/src/terminal/input.rs:14580-14649`). By contrast, the queued cloud path still logs that
  attachments are unsupported and drops them (`app/src/terminal/input.rs:14048-14061`); that
  comment and behavior are stale.
- `resolve_ai_query_routing` uses `SharedSessionStatus::ActiveViewer` to select
  `AIQueryRouting::LiveRemoteVm` (`app/src/terminal/view/shared_session/cloud_conversation_continuation.rs:133-216`).
  The terminal status remains active through a transient network reconnect.
- `Network::reconnect_websocket` changes only the network stage and emits `Reconnecting`
  (`app/src/terminal/shared_session/viewer/network.rs:443-465`). `Network::is_connected` is an
  available fast-path signal but cannot close the asynchronous upload race
  (`app/src/terminal/shared_session/viewer/network.rs:1028-1030`).
- `Network::send_message_to_server` silently returns when `stage != JoinedSuccessfully`, and also
  discards `try_send` failure after an error log (`app/src/terminal/shared_session/viewer/network.rs:829-840`).
  `send_agent_prompt_request` cannot report either outcome to `TerminalManager`
  (`app/src/terminal/shared_session/viewer/network.rs:981-995`).
- `try_send` only enqueues into the intermediate `ws_proxy_tx`; serialization and the real
  `sink.send(...).await` occur later in the websocket proxy task and may still fail
  (`app/src/terminal/shared_session/viewer/network.rs:320-350`). Therefore local channel
  acceptance is not sufficient proof that the prompt left the client or reached the server.
- `send_agent_prompt_request` already mints an `AgentPromptRequestId`, and
  `DownstreamMessage::AgentPromptRequestInFlight(id)` round-trips that ID as
  `NetworkEvent::AgentPromptRequestInFlight(id)`
  (`app/src/terminal/shared_session/viewer/network.rs:734-735`, `:951-957`). The terminal manager
  currently ignores the ID and uses the event only to unfreeze the input
  (`app/src/terminal/shared_session/viewer/terminal_manager.rs:1227-1245`).
- `DownstreamMessage::RejoinedSuccessfully` already flushes buffered shared-input updates before
  emitting `NetworkEvent::ReconnectedSuccessfully`
  (`app/src/terminal/shared_session/viewer/network.rs:613-629`). This ordering is the correct
  boundary for retrying a prompt whose original input clear may itself have been buffered.
- `TerminalManager` observes `ReconnectedSuccessfully` but currently only refreshes view state
  (`app/src/terminal/shared_session/viewer/terminal_manager.rs:838-850`).
- `TerminalManager::attach_execution_session` tears down the old network, installs the new one,
  marks the terminal `ViewPending`, and calls `connect`
  (`app/src/terminal/shared_session/viewer/terminal_manager.rs:431-493`).
  `AmbientAgentViewModelEvent::ExecutionSessionReady` initiates that swap
  (`app/src/terminal/view.rs:4788-4798`), before the new network has joined.
- Fatal reconnect handling calls `end_current_ambient_session`, records the ended execution, and
  enables follow-up UI, but it does not drain `QueuedQueryModel`
  (`app/src/terminal/shared_session/viewer/terminal_manager.rs:1692-1901` and
  `app/src/terminal/view/shared_session/view_impl.rs:891-943`). Therefore an additional fatal-end
  drain is required to dispatch the first cloud follow-up; merely draining after a new execution
  joins is circular because no new execution would have been requested.
- Existing queue drains are triggered by conversation completion, queued-command completion, and
  promptless setup (`app/src/terminal/view.rs:4926-4969`, `:5337`, `:5643-5657`). None is tied to
  viewer reconnect or the successful join of a replacement execution.

Two investigation corrections are material to this design:

1. Reusing `QueuedQueryModel` alone does not preserve the original transient reconnect behavior.
   It needs a same-session rejoin trigger in addition to fatal-handoff triggers.
2. The current `FinishReason::Error` branch does not leave the row queued when the input is empty;
   it pops the row and moves its text and attachments into the input
   (`app/src/terminal/view.rs:5534-5583`). APP-5223 instead keeps the queue row canonical. An
   independent editable input copy is prohibited because it could later submit the same prompt
   twice.
3. The reported stale `Warping...` symptom reproduces through controller state. A focused
   pre-fix test using the real viewer submission path confirmed that an undelivered prompt leaves
   the input in its loading state and the selected conversation at `ConversationStatus::InProgress`.
   `BlocklistAIStatusBar::render_warping_indicator_for_latest_exchange` renders `Warping...` when
   `conversation.status().is_in_progress()` (or the active block remains under agent control), so
   queueing must clear only stale submission state while preserving any independently active turn
   (`app/src/ai/blocklist/block/status_bar.rs:772-790`).

### Proposed changes

#### 1. Make viewer prompt acceptance observable, correlated, and content-safe

- Mint `AgentPromptRequestId` before the terminal event crosses into `Network`, and carry that ID
  with the queue-row claim and `SendAgentPrompt` event. `Network::send_agent_prompt_request` must
  accept the caller-provided ID rather than minting an unobservable ID internally.
- Add a small result type for local network submission, for example
  `ServerMessageSendOutcome::{LocallyQueued, Undeliverable}` or an equivalent `Result`.
- Make `Network::send_message_to_server` return that local outcome:
  - return `Undeliverable` when the stage is not `JoinedSuccessfully`;
  - return `Undeliverable` when `ws_proxy_tx.try_send` fails;
  - return `LocallyQueued` only after `try_send` accepts the message.
- Log every `Undeliverable` result at `warn!`. Include `self.stage`, `self.session_id`, and an
  exhaustive content-free kind string or `std::mem::discriminant` for `ServerMessage`.
  Do not format `ServerMessage` with `Debug`, because prompt and attachment data are nested in the
  message.
- Propagate the result through `send_agent_prompt_request`. Other callers may explicitly ignore
  the result after retaining the new warning, but the prompt path must consume it.
- Treat correlated `AgentPromptRequestInFlight(id)`, not `try_send`, as accepted by the
  session-sharing server. The pending claim remains canonical until that event arrives.
- Start a five-second acknowledgement timer only after `LocallyQueued`. Five seconds matches the
  existing shared-session startup timeout, is long relative to the expected local/server
  acknowledgement round trip, and bounds the current indefinite frozen-input failure. The timer
  is cancelled by the matching acknowledgement, immediate local rejection, network
  `Reconnecting`/terminal end, or row deletion.
- If the timer expires while the same request is still pending, atomically restore/unlock its row,
  unfreeze the input, clear stale viewer-submission-in-flight controller state, and warn without
  content. Do not immediately resend on the same nominally joined network; the next explicit
  reconnect/ready/manual trigger owns retry.
- Every logical row/revision has one stable request ID, reused for a transport retry. A matching
  late acknowledgement finalizes that same row and cancels any still-pending retry. Editing after
  timeout creates a new row revision/request ID, so an acknowledgement for the retired revision
  cannot delete or submit the user's edited intent. Duplicate acknowledgements are idempotent.

#### 2. Represent undelivered prompts in the existing queue

- Add an unlocked sibling `QueuedQueryOrigin`, named for the disconnected-viewer case (for example
  `DisconnectedViewer`). `QueuedQuery::is_locked` must return false for it.
- Add optional retry-target/delivery metadata to a prompt row rather than putting a second queue
  on `Network`:
  - original `SessionId`;
  - original `ServerConversationToken`;
  - stable `AgentPromptRequestId`, row revision, pending-attempt generation, and acknowledgement
    timer/claim identity;
  - enough terminal/task identity to reject a stale replacement-network event without storing
    user content in logs.
- Add constructors/update helpers that:
  - create a disconnected-viewer prompt from an immediate typed submission with its
    `PendingAttachment`s;
  - convert an already queued prompt to the disconnected origin and target while retaining its ID,
    text, attachments, and FIFO position.
- The immediate viewer event must carry retry-safe data until local network acceptance is known.
  Pending attachments are cloned/retained across asynchronous upload so a post-upload network
  rejection can still append the row. The event's uploaded `AgentAttachment`s remain transport
  data and are not the queue's only copy of file intent.
- Stage the canonical row under an exclusive pending-ack claim before the send. It is not exposed
  as an idle/editable panel row while the normal acknowledgement window is active. On correlated
  acknowledgement, finalize/remove it. On `Undeliverable` or timeout, restore/unlock exactly one
  visible row and immediately exit the viewer loading state. Clear only the failed submission's
  editor presentation; never overwrite a newer draft.
- If queue surfaces are disabled, do not create an invisible row. Restore the input when safe and
  toast.

#### 3. Make one-row drain transactional

- Replace the current peek-then-unconditional-remove flow with one operation that atomically
  claims/removes the eligible FIFO head and returns a retry token containing:
  - conversation ID;
  - original index;
  - complete `QueuedQuery`.
- Pass that token through viewer upload/send and queued cloud-follow-up upload/dispatch.
- Complete the claim only on:
  - correlated `AgentPromptRequestInFlight` from the same network/session and request ID; or
  - successful validation and start of `submit_cloud_followup`.
- A `LocallyQueued` viewer send retains the claim in an awaiting-ack state; it is not completion.
  On upload failure, network `Undeliverable`, acknowledgement timeout, stale target, ineligible
  cloud follow-up, or a disabled required feature, restore the exact query at its original relative
  position. If user edits/reorders/deletes while no claim is active, subsequent dispatch uses the
  current row revision and request ID.
- Only one lifecycle callback can own a claimed row. A reconnect, fatal-end callback, replacement
  join, and ordinary conversation-finished callback that arrive close together therefore observe
  no claimable head and cannot double-submit it.
- Extend the same claim/restore contract to LRC multi-row submission if it calls the shared queued
  viewer path; do not retain an unconditional `remove_fired_row` caller that can erase a restored
  row.

#### 4. Route drains by explicit lifecycle trigger and target identity

Introduce one central eligibility/router boundary for a claimed prompt, with an explicit trigger
instead of allowing each caller to infer routing independently:

- **Ordinary conversation-ready trigger:** retain existing local/viewer/cloud routing, but apply
  disconnected-row target validation before a remote send.
- **Transient rejoin trigger:**
  - invoke from `TerminalManager` after the current `Network` emits
    `ReconnectedSuccessfully`;
  - ignore an event from an old/replaced network;
  - require the original session ID, current session ID, and current network session ID to match;
  - require the local conversation to resolve to the stored server token and remain eligible for a
    follow-up;
  - claim and send only the FIFO head. Keep the row queued on any mismatch.
- **Fatal ambient-execution-end trigger:**
  - invoke after `record_ambient_execution_ended` and follow-up UI state have been updated, but
    before discarding all routing context for the ended network;
  - require the claimed row to target the ended session and resolve to the same conversation/task;
  - route the head through `try_submit_pending_cloud_followup`, never the old viewer network;
  - if the task is not eligible, restore the row and show the existing toast.
- **Replacement-execution trigger:**
  - `ExecutionSessionReady`/`attach_execution_session` records that this replacement network should
    drain the relevant conversation after join;
  - do not submit at `ExecutionSessionReady`, because `connect` has not completed;
  - consume the marker only when that exact new network emits `JoinedSuccessfully`;
  - claim and send one next FIFO row through the viewer path. A stale join from the ended network
    or a later unrelated network is ignored.

Every trigger acts on the FIFO head only. A nonmatching disconnected head blocks later rows rather
than allowing them to leapfrog into a different session. Existing completion events continue the
queue after the first row is accepted.

#### 5. Reuse the real cloud attachment upload path

- Remove the stale "cloud follow-up does not support attachments" comment and warning.
- Refactor the queued cloud branch to collect the claimed row's `PendingAttachment`s and call
  `upload_pending_attachments_to_task` using the same task resolution and feature gating as
  `upload_files_then_submit_cloud_followup`.
- Dispatch the follow-up only after upload succeeds. The task definition remains the source from
  which the new execution downloads the attachments at startup.
- Upload failure restores the claimed row at its prior FIFO position and shows the existing upload
  failure toast. It must not clear a newer editor draft.
- Avoid duplicate uploads within one attempt. Retrying after a failed or unknown upload may invoke
  the existing idempotent task-attachment behavior; no attachment is silently dropped.

#### 6. Keep queue and editor state coherent on failure

- The queue row is the canonical durable copy after an undeliverable viewer send.
- Immediately unfreeze the viewer input after queue insertion/restoration. If the editor contains a
  newer draft, leave it untouched.
- If an ineligible fatal drain occurs with an empty editor, use the existing queue panel edit/send
  surface or a row-linked editor representation. The representation must retain the
  `(conversation_id, query_id)` identity, and a manual send must atomically claim/remove that same
  row. Do not place an untracked second copy in the editor.
- Deleting the queue row cancels the pending retry. Reordering changes later FIFO order. Editing
  creates a new row revision/request ID and changes the exact text/attachments used by the next
  attempt.
- Track viewer prompt delivery separately from agent execution. While awaiting the correlated
  acknowledgement, retain today's loading/in-flight treatment. When the row becomes visibly
  queued, clear that delivery-pending state. If no response stream is active and the terminal block
  is not under agent control, ensure the conversation/controller no longer reports an in-progress
  turn; the queue panel is then the only pending affordance and `Warping...` is absent. Do not
  suppress `Warping...` for a prior response stream or agent-controlled command that is genuinely
  active.

### Ordering and no-duplication invariants

1. `QueuedQueryModel` holds the only durable copy while a prompt is pending.
2. A prompt is either idle in the queue, exclusively claimed by one local/awaiting-ack dispatch
   attempt, or acknowledged/accepted by one submission path. It is never in two of these states
   simultaneously.
3. The queue head is removed before an asynchronous upload/send begins and restored on every known
   pre-acceptance failure. This prevents a second trigger from observing and dispatching it.
4. A correlated acknowledgement from the old session and cloud-follow-up dispatch are mutually
   exclusive terminal outcomes for one row revision. Fatal teardown cannot claim an already
   acknowledged/removed row; an undeliverable or timed-out old send restores before the fatal
   drain can claim it.
5. Reconnect input updates are flushed before the prompt retry. This preserves clear/update order
   for the shared editor.
6. An old network event is ignored after `current_network` changes. A replacement join marker is
   keyed to the exact new session and consumed once.
7. FIFO is strict across all origins and triggers. An ineligible or mismatched head remains a
   barrier; later prompts do not bypass it.
8. At most one row is accepted per reconnect, fatal-end, replacement-join, or ordinary
   conversation-ready event. Later conversation-ready events advance the queue.
9. A stable request ID and attempt generation make acknowledgement handling idempotent. A late or
   duplicate acknowledgement can finalize only its unchanged logical row revision; it cannot
   remove a newer user edit or cause another drain.

### Design alternatives

- **Selected: reuse `QueuedQueryModel` with retry metadata and transactional claims.** It already
  owns attachments, UI controls, FIFO order, and conversation lifecycle, and it survives network
  replacement. A claim/restore operation extends its existing responsibility without creating a
  second source of truth.
- **Rejected: a queue on `Network`.** The fatal path destroys and replaces `Network`; preserving
  rows would require a transfer protocol and duplicate queue UI/model state.
- **Rejected: a queue on `TerminalManager` or `Input`.** It would duplicate
  `QueuedQueryModel`'s edit/delete/reorder/attachment behavior and complicate cleanup keyed to
  `BlocklistAIHistoryModel`.
- **Rejected: submission-time `Network::is_connected()` as the only signal.** It is useful as an
  optional fast path, but the socket can change stage while asynchronous attachments upload. The
  actual send boundary must report known local rejection.
- **Rejected: `try_send` acceptance as delivery success.** It only transfers the message into
  `ws_proxy_tx`; serialization or the later websocket write can fail, so it preserves the silent
  loss window.
- **Rejected: websocket write-completion oneshot as the authoritative success signal.** A oneshot
  from the proxy task would detect serialization/write failures and is stronger than `try_send`,
  but a successful socket write is not proof that the server accepted or routed the request. The
  existing ID-correlated server acknowledgement closes more of the failure window without a new
  protocol message. Local rejection remains the fast failure path.
- **Selected: correlated server acknowledgement with a five-second timeout.** This adds a bounded
  pending state and late-ack bookkeeping, but it prevents indefinite frozen input when the proxy
  accepts a message that is never acknowledged. Stable per-revision request IDs and transactional
  claims preserve client-side de-duplication across timeout and reconnect.
- **Rejected: wait only for replacement execution readiness.** It loses transient same-session
  retry and is circular for the first fatal prompt: without dispatching the head as a follow-up,
  no replacement execution will become ready.
- **Rejected: restore an independent editor copy while retaining the row.** Two editable copies
  can both submit. A row-linked representation or the existing queue editor preserves a single
  identity.
- **Rejected: add a special count/age cap.** Existing queued prompts are intentionally user
  managed and unbounded. A disconnected-only cap would require lossy overflow behavior and provide
  little protection beyond controls already present in the panel.

### Risks and mitigations

- **False retry into changed work:** validate network, session, conversation token, role, and
  conversation readiness before same-session refire; otherwise retain the head.
- **Double-submit from racing events:** atomically take the head before async work, pass a typed
  retry token end to end, correlate acknowledgement by stable request ID/revision, and restore only
  that token.
- **Proxy accepts but server never acknowledges:** retain the claim for five seconds, then make the
  canonical row visible and editable and unfreeze the input. Do not retry on the same connection
  without an explicit trigger.
- **Late/duplicate acknowledgement:** use stable per-revision IDs and compare-and-swap the pending
  attempt. Acknowledgement of a retired revision cannot remove a newer edit; duplicate events are
  no-ops.
- **Stale `Warping...`:** clear viewer-delivery pending state when a row becomes queued and assert
  that an otherwise idle conversation/controller is not in progress. Preserve the indicator for
  independently active output or agent-controlled commands.
- **FIFO corruption after failure:** restore at the original relative index and test failures with
  rows before/after the claimed row.
- **Attachment duplication or loss:** retain pending attachment intent until acceptance, use the
  existing task upload helper, and cover upload success/failure plus post-upload network rejection.
- **Editor clobbering:** never restore over a nonempty/newer draft; queue UI remains canonical.
- **Old-network event after swap:** compare event network/session identity to `current_network` and
  key the replacement-join marker to the expected session.
- **Logging user content:** use an exhaustive static message kind/discriminant and test that no
  `Debug` rendering of the message is introduced.
- **Deadlock/UI freeze:** keep `TerminalModel` lock scopes short and do not call queue, input, or
  event-dispatch helpers while retaining a model lock.

### Expected affected files

- `app/src/ai/blocklist/queued_query.rs`
- `app/src/ai/blocklist/queued_query_tests.rs`
- `app/src/ai/blocklist/block/status_bar.rs`
- `app/src/terminal/input.rs`
- `app/src/terminal/input_tests.rs`
- `app/src/terminal/view.rs`
- `app/src/terminal/view/queued_prompts_tests.rs`
- `app/src/terminal/shared_session/viewer/network.rs`
- `app/src/terminal/shared_session/viewer/network_tests.rs`
- `app/src/terminal/shared_session/viewer/terminal_manager.rs`
- `app/src/terminal/shared_session/viewer/terminal_manager_tests.rs`
- `app/src/terminal/view/shared_session/view_impl.rs`
- `app/src/terminal/view/shared_session/view_impl_tests.rs`

## Validation and verification criteria

All criteria are required before merge.

1. **Pre-fix bug reproduction:** Add a deterministic test that places a viewer `Network` in
   `Stage::Reconnecting`, submits an Agent prompt through the real terminal/input event path, and
   demonstrates the verified current behavior: the network does not accept the message, no queue
   row exists, the input remains frozen, and the selected conversation/controller remains
   `InProgress`, satisfying the `Warping...` gate. The post-fix assertions are one unlocked queued
   row, editable input, cleared viewer-delivery pending state, and—when no independent response
   stream or agent-controlled command exists—`conversation.status().is_in_progress() == false` so
   `render_warping_indicator_for_latest_exchange` has no stale reason to render. A genuinely active
   prior turn remains in progress.
2. **Network outcome and acknowledgement contract:** In `viewer/network_tests.rs` and
   `terminal_manager_tests.rs`, assert that `send_message_to_server`/`send_agent_prompt_request`
   returns `Undeliverable` for every non-joined stage and a closed/full send channel, and
   `LocallyQueued` only when the local channel accepts the message. Assert that only matching
   `AgentPromptRequestInFlight(request_id)` completes/removes the pending row and unfreezes normal
   input; a different or duplicate ID is a no-op.
3. **Safe logging:** Exercise a dropped `SendAgentPrompt` containing sentinel prompt text,
   attachment names, and attachment content. Capture the warning and assert it includes stage,
   session ID, and message kind/discriminant but none of the sentinel user content. No
   `report_error!`/Sentry event is added.
4. **Immediate fallback queueing:** In `input_tests.rs` or `terminal_manager_tests.rs`, submit one
   plain prompt during reconnect and assert exactly one
   `QueuedQueryOrigin::DisconnectedViewer` row with the original conversation, session target, and
   token; assert `is_locked() == false`, input unfreezes immediately, and the failed text is not
   independently resubmittable outside the row.
5. **Async race with attachments:** Begin a viewer prompt with image and file attachments while
   joined, hold the upload future, transition to reconnecting, then complete upload. Assert the
   actual send returns `Undeliverable`, exactly one row retains both pending attachments, the
   editor is usable, and neither text nor attachments are lost or duplicated.
6. **Happy-path no queue:** Submit the same plain and attachment prompts while joined. Assert the
   local channel receives one `AgentPromptRequest` with the staged row's request ID, a matching
   in-flight acknowledgement arrives before timeout, no visible fallback row remains, and the
   input unfreezes/clears as before.
7. **Acknowledgement timeout and late acknowledgement:** Drive the five-second timer
   deterministically. Assert local `try_send` without a matching acknowledgement restores one
   unlocked row, unfreezes input, clears stale delivery/`Warping...` state for an otherwise idle
   conversation, and does not immediately resend on the same connection. Cover acknowledgement at
   the timeout boundary, after timeout but before another claim, after edit to a new revision,
   during reconnect retry, and duplicate acknowledgement. Each unchanged logical revision is
   accepted at most once; a retired ID never removes a newer edit.
8. **Transient rejoin:** In `terminal_manager_tests.rs`, queue at least two rows for the same
   disconnected viewer target, emit `RejoinedSuccessfully`, and assert buffered input updates flush
   first, exactly the FIFO head is accepted by the same session, and the second row remains queued
   until the next ready event.
9. **Transient mismatch matrix:** Parameterize rejoin tests over old-network event, different
   session ID, different/missing server token, cancelled/removed conversation, lost executor role,
   and conversation that moved to incompatible in-progress work. Assert zero prompt sends, the
   exact queue remains, no later row leapfrogs, and no unrelated local/cloud submission occurs.
10. **Fatal first-row handoff:** Simulate reconnect exhaustion for an owned, resumable ambient task
   with queued rows. Assert `record_ambient_execution_ended` runs before routing, exactly the FIFO
   head becomes `pending_followup_prompt`, the old network receives nothing, and remaining rows
   survive the network teardown.
11. **Ineligible fatal matrix:** Parameterize non-owner/read-only, blocked source,
    `HandoffCloudCloud` disabled, missing task/model/token, and failed eligibility resolution.
    Assert the head is restored at the same position, the whole queue remains visible and ordered,
    the existing error toast appears, a newer input draft is untouched, and any empty-input
    restoration remains linked to the same query ID.
12. **Replacement execution join:** Emit `ExecutionSessionReady` and assert no queued prompt is
    sent before network join. Then join the exact replacement session and assert one next row is
    sent once. Re-emit readiness/join and stale old-network events and assert no duplicate send.
13. **Ordering across all triggers:** Create three rows, race reconnect/fatal-end/join and an
    ordinary conversation-finished event around delayed upload futures, and assert accepted prompt
    IDs are strictly FIFO with each ID accepted at most once. Include a failed attempt restored
    between two rows.
14. **Queued cloud attachments:** In `queued_prompts_tests.rs`/`input_tests.rs`, fire a queued cloud
    follow-up with an image and file. Assert both are passed to
    `upload_pending_attachments_to_task` before `submit_cloud_followup`, then assert the replacement
    startup observes the task attachments using existing attachment-download coverage.
15. **Attachment failure restoration:** Fail image upload, file upload, and task upload completion
    separately. Assert no follow-up starts, the full row is restored at its original position, the
    existing toast appears, and a nonempty editor draft is unchanged.
16. **Queue controls and lifecycle:** Assert disconnected rows can be edited, deleted, reordered,
    manually pushed, and auto-fired; deletion prevents retry; history conversation removal still
    cleans the queue; `attach_execution_session` leaves rows intact.
17. **Feature-off behavior:** Override queued-prompts, handoff, and image-context flags separately.
    Assert no hidden queue entry or attachment drop: either the visible input is safely restored or
    the existing visible row remains, and an error toast identifies that submission did not
    proceed.
18. **Existing regressions:** Run focused tests for
    `queued_query`, `queued_prompts`, `viewer::network`, `viewer::terminal_manager`, shared-session
    cloud continuation, and terminal input routing. Existing local queue, LRC auto-queue,
    queued-command, cloud handoff, viewer CRDT input, and reconnect tests must remain green.
19. **Repository gates:** From the repository root run the documented L/cross-cutting checks:
    - `./script/format --check`
    - `./script/check_no_inline_test_modules`
    - `cargo clippy --workspace --exclude warp_completer --all-targets --tests -- -D warnings`
    - `cargo clippy -p warp --all-targets --tests -- -D warnings`
    - `cargo clippy -p warp_completer --all-targets --tests -- -D warnings`
    - `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`
    - `cargo nextest run -p warp_completer --features v2`
    - `cargo test --doc`
    `./script/presubmit` may be used for the combined gate.
20. **Running UI verification:** After a successful build, exercise the real Warp GUI with computer
    use against a test shared ambient session. Capture and attach a video to the implementation PR
    showing:
    - prompt submission during transient disconnect;
    - immediate input unfreeze and one editable queued-panel row;
    - same-session refire after reconnect without duplication;
    - fatal disconnect converting the first row to a cloud follow-up;
    - remaining FIFO rows surviving the network swap and continuing after the replacement session
      joins;
    - an ineligible fatal path retaining the row and preserving a separately typed draft.
    Repeat at least one flow with an image/file attachment and confirm the new execution receives
    it. Visual proof is mandatory because the queue panel, input loading state, toast, and new
    execution transition are user-visible.
