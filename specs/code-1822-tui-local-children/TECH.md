# TECH: `TuiOrchestrationModel` + background local child agents

Second PR of the two-PR stack, building on the full-view `TuiSessions`
container. Accepting a local `run_agents` request in the TUI spawns native
child agents in background sessions and makes the run observable in the
parent transcript.

## Context

The shared orchestration engine is frontend-neutral and already partially wired into the TUI:

- The TUI's `TuiRunAgentsCardView` drives the shared `RunAgentsExecutor` accept path
  ([crates/warp_tui/src/run_agents_card_view.rs:200-236 @ 41d0b004](https://github.com/warpdotdev/warp/blob/41d0b004219adff1624cbaf942f52b2d64244d75/crates/warp_tui/src/run_agents_card_view.rs#L200-L236)).
- `RunAgentsExecutor` fans out per child to `StartAgentExecutor::dispatch`, which emits
  `StartAgentExecutorEvent::CreateAgent(Box<StartAgentRequest>)` and awaits materialization
  ([app/src/ai/blocklist/action_model/execute/start_agent.rs:499,532-566 @ 41d0b004](https://github.com/warpdotdev/warp/blob/41d0b004219adff1624cbaf942f52b2d64244d75/app/src/ai/blocklist/action_model/execute/start_agent.rs#L532-L566)).
  Nothing in `crates/warp_tui` subscribes to that event today, and `StartAgentExecutor` is not yet
  exported via `app/src/tui_export.rs`.
- In the GUI, materialization is `TerminalView::handle_start_agent_executor_event`
  ([app/src/terminal/view.rs:7630-7650 @ 41d0b004](https://github.com/warpdotdev/warp/blob/41d0b004219adff1624cbaf942f52b2d64244d75/app/src/terminal/view.rs#L7630-L7650))
  → `PaneGroup::create_hidden_child_agent_conversation`
  ([app/src/pane_group/child_agent/mod.rs:130-179 @ 41d0b004](https://github.com/warpdotdev/warp/blob/41d0b004219adff1624cbaf942f52b2d64244d75/app/src/pane_group/child_agent/mod.rs#L130-L179)) —
  pane-tree machinery the TUI cannot reuse. The frontend-neutral pieces it calls are reusable:
  `BlocklistAIHistoryModel::start_new_child_conversation`
  ([history_model.rs:508-544 @ 41d0b004](https://github.com/warpdotdev/warp/blob/41d0b004219adff1624cbaf942f52b2d64244d75/app/src/ai/blocklist/history_model.rs#L508-L544)),
  the `children_by_parent` lineage index, `ai_client.create_agent_task`, and
  `StartAgentExecutor`'s self-completion off `BlocklistAIHistoryEvent`s
  ([start_agent.rs:144-310 @ 41d0b004](https://github.com/warpdotdev/warp/blob/41d0b004219adff1624cbaf942f52b2d64244d75/app/src/ai/blocklist/action_model/execute/start_agent.rs#L144-L310)).
- Messaging/lifecycle needs almost no TUI work: `OrchestrationEventStreamer`,
  `OrchestrationEventService`, `LocalAgentTaskSyncModel`, and `MessageHydrator` are
  frontend-neutral singletons already registered by the shared bootstrap the TUI binary runs
  ([app/src/lib.rs:2051-2057 @ 41d0b004](https://github.com/warpdotdev/warp/blob/41d0b004219adff1624cbaf942f52b2d64244d75/app/src/lib.rs#L2051-L2057)).
  Transport is server-mediated (SSE + RPC) even between local agents in one process. The one
  gap (found in live verification): the streamer only opens a conversation's SSE stream once a
  *consumer* registers for it (`register_agent_event_consumer`), which the GUI drives from the
  agent view's `ActiveAgentViewsModel` bridge — a surface the TUI lacks. Without it, children
  sent messages but the parent never received them. `TuiOrchestrationModel` therefore registers
  the parent as a consumer on dispatch and each child on materialization (and unregisters on
  failed-launch cleanup).
- The TUI transcript currently drops orchestration traffic on the floor:
  `MessagesReceivedFromAgents`/`EventsFromAgents` are explicit no-ops
  ([crates/warp_tui/src/agent_block.rs:670-671 @ 41d0b004](https://github.com/warpdotdev/warp/blob/41d0b004219adff1624cbaf942f52b2d64244d75/crates/warp_tui/src/agent_block.rs#L670-L671)).

Scope decisions (from the architecture discussion): native Oz children only; CLI-harness children
(Claude/Codex/OpenCode) are a follow-up, and remote children (whose GUI spawn path is coupled to
ambient-agent panes) also resolve as explicit per-child failures for now. Children stay invisible
— no navigation, no status bar; `TuiSessions` focus never moves off session 0 in this PR.
Following the GUI's hidden-pane prior art, each child session gets a full (backgrounded)
`TuiTerminalSessionView` retained by `TuiSessions`; the view doubles as the
terminal manager's PTY surface.

## Proposed changes

### Changed: `app/src/tui_export.rs`

- Re-export `StartAgentExecutor`, `StartAgentExecutorEvent`, and `StartAgentRequest` (plus any
  associated outcome types needed to fail a child), mirroring the existing
  `RunAgentsExecutor` exports.

### New: `crates/warp_tui/src/orchestration_model.rs` — `TuiOrchestrationModel`

A `SingletonEntity` owning all TUI orchestration coordination:

- Subscribes to `TuiSessions::SessionAdded` and, for each registered session, subscribes to that
  session's `StartAgentExecutor`. Because all session creation flows through `TuiSessions`
  (PR 2 invariant), every session — including children, enabling future nesting — is wired.
- On `StartAgentExecutorEvent::CreateAgent`, dispatches on the request mirroring the GUI's
  per-mode dispatch:
  - **Native (no harness)**: `ai_client.create_agent_task` (server task row → run_id, which is
    what activates messaging/lifecycle for the child) → create a background session via the
    shared `create_local_terminal_session` helper (PTY manager + full backgrounded view,
    registered unfocused with `TuiSessions`) → inherit the parent's execution
    profile/base model and apply the run-wide model override →
    `BlocklistAIHistoryModel::start_new_child_conversation` + `set_task_id` →
    `record_new_conversation_request_complete` (echoes the child `AIConversationId` so
    `StartAgentExecutor` resolves its pending slot) → send the child's prompt via the child
    session's `BlocklistAIController::send_agent_query_in_conversation`.
  - **CLI-harness (Claude/Codex/OpenCode) and Remote**: resolve the pending slot with a clear
    per-child failure outcome — a clean `failed` entry in the `launched` result rather than a
    spawn-timeout hang. The failure path creates the child conversation on a synthetic surface,
    marks it `Error`, and echoes it to the executor (which then also emits
    `CleanupFailedChildLaunch`; the model tears down any mapped child session on that event).
    TODO(code-1822): implement CLI-harness children by reusing the frontend-neutral
    `prepare_local_harness_child_launch`
    ([app/src/pane_group/pane/local_harness_launch.rs:158 @ 41d0b004](https://github.com/warpdotdev/warp/blob/41d0b004219adff1624cbaf942f52b2d64244d75/app/src/pane_group/pane/local_harness_launch.rs#L158)),
    and remote children with a TUI-native spawn path.
- `crates/warp_tui/src/session.rs` extracts `create_local_terminal_session`, the single
  session-materialization helper shared by the login bootstrap (focused) and child creation
  (background). Callers provide the window from their existing view context, while the helper
  obtains process-level exit-summary context from `TuiSessions`; `TuiOrchestrationModel` derives
  the window from the requesting parent session rather than storing view-layer state.
- Tracking state is thin and session-dimensional only:
  - `child_session_by_conversation: HashMap<AIConversationId, TuiSessionId>`
  - `parent_sessions: HashSet<TuiSessionId>`
  Conversation lineage is always read from `BlocklistAIHistoryModel` (`children_by_parent`,
  `parent_conversation_id`) — never mirrored here. This model adds only the conversation↔session
  mapping that the shared layer doesn't know about, and is the future home/data source for session
  navigation and child-status UI.
- `TuiTerminalSessionView` remains orchestration-ignorant; the coordinator
  uses narrow accessors for its action model and controller.

### Changed: `crates/warp_tui/src/agent_block.rs` — minimal orchestration transcript rendering

- Replace the no-op arms for `MessagesReceivedFromAgents` and `EventsFromAgents` with simple
  rendered lines: sender + subject for messages; sender + lifecycle transition for events.
- `TODO: add full status rendering based on MOCs.` marks the intentionally
  minimal message/lifecycle lines.
- Suppress the `WaitForEvents` tool-call row ("Waiting for agent events…") entirely: the GUI
  renders nothing for this action (its output match falls through to a no-op), so the TUI skips
  emitting a transcript section for it rather than using the generic fallback label.

### Non-goals

- Remote and CLI-harness local children (explicit per-child failures, above), session
  navigation/reveal, child cleanup UX on completion (children idle like GUI hidden panes;
  lifecycle events surface state).

## Testing and validation

- Unit tests on `TuiOrchestrationModel` (per `rust-unit-tests`/`tui-testing` conventions):
  - `CreateAgent` with a CLI harness or Remote mode resolves the executor's pending slot with the
    per-child failure message and materializes no session.
  - Sessions added after model init get their executors subscribed (the nesting invariant),
    proven by a late-registered session's dispatch resolving.
  - The native path's session materialization spawns a real PTY, so it is validated end-to-end
    (below) rather than unit-tested against a mocked server client.
- Render-to-lines test: transcript renders message/lifecycle lines for
  `MessagesReceivedFromAgents`/`EventsFromAgents` outputs.
- End-to-end manual validation per `tui-verify-change` (the key checkpoint that run_id
  registration + SSE lifecycle work for TUI-spawned children): in `./script/run-tui`, prompt an
  orchestration (`run_agents` with 2 local children) → accept → card shows launched; child
  lifecycle (`in_progress`/`succeeded`) and completion messages appear in the parent transcript;
  parent's tool result contains per-child `launched` entries with agent run ids.
- `./script/presubmit` before submit.

## Parallelization

Mostly none — the orchestration model, tui_export additions, and materializer are one coupled unit.
The transcript-rendering change (`agent_block.rs`) is independent and could be split to a parallel
local agent in a separate worktree, but it is ~50 LOC; not worth the coordination overhead. A
single agent implements this PR sequentially.

## Follow-ups

- CLI-harness local children (see TODO above).
- Richer transcript rendering for agent messages/events (see TODO above).
- Session navigation + child-status surface (next milestone; builds on `TuiSessions` focus and
  `TuiOrchestrationModel` tracking).
