# REMOTE-1478: Follow-up session attach for ambient agent conversations
## Context
Ambient cloud-agent views currently use the session-sharing viewer stack as a one-session transport. The initial cloud-mode view is created as a deferred shared-session viewer, and `create_cloud_mode_view` subscribes to `AmbientAgentViewModelEvent::SessionReady` to call `viewer::TerminalManager::connect_to_session` once in `app/src/terminal/view/ambient_agent/mod.rs (28-72)`.
That one-shot shape is encoded in `viewer::TerminalManager`. `NetworkState::PendingJoin` owns `prompt_type` and `channel_event_proxy`; `connect_session` consumes those values, creates a `Network`, wires subscriptions, and transitions to `NetworkState::Active`. After that, `connect_to_session` returns `false` for `Active`, so the same manager cannot attach to a second session ID in `app/src/terminal/shared_session/viewer/terminal_manager.rs (73-292)`.
When the shared session ends, the viewer path treats the view as permanently finished. `Network::process_websocket_message` handles `DownstreamMessage::SessionEnded` by calling `close_without_reconnection` and emitting `NetworkEvent::SessionEnded` in `app/src/terminal/shared_session/viewer/network.rs (597-600)`. The terminal manager then calls `shared_session_ended`, which cancels in-progress conversations, unregisters from the shared-session manager, calls `TerminalView::on_session_share_ended`, sets `SharedSessionStatus::FinishedViewer`, and clears the write-to-PTY sender in `app/src/terminal/shared_session/viewer/terminal_manager.rs (635-684)` and `app/src/terminal/shared_session/viewer/terminal_manager.rs (1397-1425)`.
`TerminalView::on_session_share_ended` is a UI teardown routine, not a resumable boundary. It may insert the conversation-ended tombstone, clears `shared_session`, unregisters remote peers, flips viewer input to selectable/read-only, and updates pane sharing state in `app/src/terminal/view/shared_session/view_impl.rs (683-735)`.
The viewer network is scoped to a single session ID and websocket endpoint. Same-session reconnect is handled internally by `Network::reconnect_websocket`, but this work does not need new same-session reconnect behavior. Follow-up executions should join a new session with a fresh `Network` and `InitPayload.last_received_event_no: None`, while preserving the existing `TerminalView`, `TerminalModel`, ambient view model, and AI history.
The first join path loads a session snapshot through `EventLoop::new`. It decodes the `scrollback` from `JoinedSuccessfully` and calls `TerminalModel::load_shared_session_scrollback` before processing ordered terminal events in `app/src/terminal/shared_session/viewer/event_loop.rs (68-131)`. That model method restores serialized blocks into the current blocklist but has no explicit follow-up mode or duplicate-block policy in `app/src/terminal/model/terminal_model.rs (1443-1455)` and `app/src/terminal/model/blocks.rs (729-757)`.
The session-sharing server already supports joining arbitrary session IDs as fresh sessions. A fresh viewer gets `JoinedSuccessfully` with that session's scrollback, active prompt, source type, and latest event number from `/Users/zachbai/dev/session-sharing-server/server/src/sessions/manager/join.rs (190-259)` and `/Users/zachbai/dev/session-sharing-server/server/src/sessions/manager/join.rs (546-641)`. The missing client-side contract is how much of a follow-up session's scrollback is new versus rehydrated from the previous VM.
## Implemented changes
The implementation introduces a fresh-session attach path for ambient follow-up executions. This is not a same-session reconnect feature. A follow-up always means:
- the previous session has ended;
- the caller has a new `SessionId`;
- the ambient agent run ID is unchanged;
- the existing ambient terminal view/model should remain the user-visible conversation;
- a new viewer `Network` should join the new session;
- the new session's contents should append to the existing blocklist.
### Rework viewer manager connection ownership
The one-shot `NetworkState` shape was replaced with state that keeps reusable viewer resources outside the per-session network:
- `NetworkResources { prompt_type, channel_event_proxy }`
- `current_network: Arc<FairMutex<Option<ModelHandle<Network>>>>`
- `NetworkState::Idle | Connecting | Active(ModelHandle<Network>)`
`new_internal` now stores `prompt_type` and `channel_event_proxy` on the manager instead of hiding them inside a pending-join state. Initial viewers call the same internal `connect_session` helper used by deferred cloud-mode viewers with `SharedSessionInitialLoadMode::ReplaceFromSessionScrollback`.
The public attach API is intentionally narrow:
- `connect_to_session(session_id, ctx)` for the initial deferred attach;
- `attach_followup_session(session_id, ctx)` for ambient follow-up attaches.
For follow-up attaches, `attach_followup_session`:
- close/drop the old `current_network` if present;
- install a fresh write-to-PTY channel on the terminal model;
- set shared-session status back to `ViewPending`;
- create a new `Network` for the new session ID;
- wire inbound network events for that specific network;
- update `current_network`.
### Avoid per-session outbound subscription leaks
Outbound subscriptions that previously captured a concrete `network` handle now register once and route through `current_network`: view events, LLM preference changes, input mode changes, selected conversation changes, auto-approve changes, CLI agent input changes, and network status changes.
This one-time route matches the stable-view/stable-manager model:
- view/model subscriptions live for the manager lifetime;
- each callback asks for the current active network;
- if no current network is active, it no-ops;
- inbound subscriptions remain per-network because each `Network` emits events independently.
This also makes N follow-up executions behave like a sequence of network replacements rather than a growing chain of listeners.
### Make ambient session end resumable
Permanent viewer teardown is split from ambient execution-ended handling. Normal shared-session viewers still keep today's behavior: ended banner, read-only input, finished viewer state, and no future attach path.
For shared ambient agent sessions, `SessionEnded` is treated as a resumable execution boundary. `ambient_session_ended` unregisters the live shared-session transport from `Manager`, clears the write-to-PTY sender, and clears `current_network` if it still points at the ended session. It does not call `TerminalView::on_session_share_ended`, does not set `SharedSessionStatus::FinishedViewer`, and does not cancel the ambient conversation, so a later `attach_followup_session` can reuse the same view/model.
### Add ambient follow-up attach event
The ambient view model emits `SessionReady { session_id }` for the initial session and `FollowupSessionReady { session_id }` for later fresh session IDs.
The subscription in `create_cloud_mode_view` dispatches initial sessions to `connect_to_session` and follow-up sessions to `attach_followup_session`.
The “continue” trigger and server API for creating the follow-up execution can land separately, but the session-sharing infra should expose a narrow API that only needs a new `SessionId`.
### Add append-aware scrollback loading
The viewer event loop accepts a `SharedSessionInitialLoadMode`:
- `ReplaceFromSessionScrollback` for initial joins;
- `AppendFollowupScrollback` for ambient follow-up joins.
For initial joins, preserve the current call to `load_shared_session_scrollback`.
For follow-up joins, `EventLoop::new` calls the model/blocklist append path:
`append_followup_shared_session_scrollback(scrollback, is_alt_screen_active)`.
That method:
- finish any previous active block that cannot continue receiving output from the old session;
- mark the model bootstrapped/view-pending for the new session;
- skip scrollback blocks already present in the current blocklist;
- append only new blocks from the follow-up session;
- preserve block ID to block index mappings without duplicates;
- restore the new session's active block as the live block for subsequent ordered terminal events;
- send the same wakeup/refresh signals the current initial load path sends.
The dedupe contract should be explicit. The preferred contract is that the follow-up VM preserves `SerializedBlock.id` for rehydrated prior blocks. Then client-side dedupe is deterministic: skip incoming scrollback blocks whose IDs are already present, and append from the first unknown block onward. If the follow-up VM cannot preserve block IDs, the handoff/session producer needs to provide continuation-only scrollback or a join-payload continuation marker. Without one of those contracts, the client cannot reliably distinguish “old output replayed in a new session” from “new output that happens to look identical.”
### Keep conversation identity stable
`SessionSourceType::AmbientAgent { task_id }` currently drives `AmbientAgentViewModel::enter_viewing_existing_session` and `ActiveAgentViewsModel::register_ambient_session` in `app/src/terminal/shared_session/viewer/terminal_manager.rs (624-666)`. Follow-up executions are guaranteed to keep the same run ID, so the ambient view should continue to use the existing task/run identity across session attachments.
The new session ID is only a new transport for the same ambient run, not a new active conversation. `ActiveAgentViewsModel` continues pointing the same terminal view at the same ambient task ID, and follow-up attach updates session transport state without changing the user-visible conversation identity.
## End-to-end flow
1. User starts an ambient cloud conversation.
2. `AmbientAgentViewModel` emits initial `SessionReady`.
3. `viewer::TerminalManager` attaches the first session with initial load mode.
4. The cloud VM finishes; the session-sharing server sends `SessionEnded`.
5. The viewer manager records an ambient execution-ended state and removes the active network without permanently poisoning the view.
6. User clicks “continue” and sends a follow-up prompt.
7. The follow-up orchestration creates a new VM/session and returns a new `SessionId`.
8. `AmbientAgentViewModel` emits a follow-up session-ready event.
9. The existing terminal manager creates a fresh `Network` for the new session ID.
10. The new event loop loads follow-up scrollback in append mode, skipping already-known blocks.
11. New ordered terminal events stream into the same blocklist and ambient view.
12. Steps 4-11 can repeat for N follow-up executions.
## Risks and mitigations
The highest-risk area is scrollback dedupe. Mitigate by making the rehydration contract explicit before implementation: either preserve block IDs, send continuation-only scrollback, or add a continuation marker. Do not rely on byte/string comparisons of terminal output.
Replacing networks without cleaning subscriptions can leak old networks and duplicate outbound messages. Mitigate by routing all outbound subscriptions through one stable current-network handle and testing repeated follow-ups.
Treating ambient `SessionEnded` as resumable may regress normal shared-session viewers if the paths are not cleanly separated. Mitigate with an explicit ambient-only branch keyed off `TerminalModel::is_shared_ambient_agent_session()` or the attach kind, plus regression coverage for non-ambient viewers.
Conversation identity should be stable because follow-up executions reuse the same run ID. The main risk is accidentally treating the new session ID as a new conversation identity; mitigate by keeping `ActiveAgentViewsModel` registration tied to the existing ambient task/run and treating session IDs as transport state only.
## Testing and validation
Targeted event-loop/blocklist coverage was added for:
- follow-up load skips scrollback blocks whose IDs already exist;
- follow-up load appends new blocks in order;
- duplicate block IDs do not corrupt `block_id_to_block_index`.
Additional follow-up work should add direct `viewer::TerminalManager` coverage for:
- initial deferred attach creates one active network;
- ambient `SessionEnded` transitions to resumable ended state;
- follow-up attach replaces the old network with a new session ID;
- repeated follow-up attach does not duplicate outbound sends;
- non-ambient `SessionEnded` still produces finished/read-only viewer behavior.
Additional integration coverage should exercise the ambient flow:
- start cloud mode and join an initial ambient session;
- simulate session ended;
- attach a second session ID into the same terminal view;
- assert old output remains and new output appears after it;
- repeat with a third session ID;
- assert the same ambient view remains visible and active throughout.
Manual validation should exercise a real cloud-mode conversation once the follow-up API exists: start a cloud agent, wait for the VM/session to end, click continue, confirm a new VM starts, and verify the same ambient view appends new output instead of opening a new transcript or replacing prior blocks.
## Parallelization
The work can split across three mostly independent tracks:
- viewer manager lifecycle: reusable resources, current-network replacement, ambient-ended state, and outbound subscription routing;
- blocklist/event-loop append mode: dedupe contract, append loader, and event-loop load mode tests;
- ambient follow-up trigger: server/API integration for “continue,” model event wiring, and stable conversation identity.
The final integration point is the ambient model handing a fresh follow-up `SessionId` to the viewer manager attach API.
