# TECH: `TuiOrchestrationModel` + background local child agents
This change builds on the full-view `TuiSessions` container. Accepting a local
`run_agents` request in the TUI creates native Oz children in background terminal
sessions while the parent remains focused and receives orchestration traffic.
## Architecture
### Shared local Oz launch contract
The GUI and TUI share the frontend-neutral parts of native child launch through
`app/src/ai/blocklist/child_agent_launch.rs (16-93)`:
- `prepare_local_oz_child_launch` normalizes the child name and creates the server task row with
  the prompt and parent run id. The returned `PreparedLocalOzChildLaunch` contains the task id and
  normalized conversation name needed by the frontend-specific materializer.
- `inherit_child_agent_settings` copies the parent's execution profile and effective base model to
  the child surface.
- `apply_child_agent_model_override` installs a non-empty run-wide model override after inheritance.
The GUI hidden-pane path uses the same helpers in
`app/src/pane_group/pane/terminal_pane.rs (1528-1705)`. The TUI therefore does
not export or directly compose `AIClient`, `AgentConfigSnapshot`,
`ServerApiProvider`, or `AIExecutionProfilesModel`.
### Session event bridge
Each `TuiTerminalSessionView` owns its `StartAgentExecutor` subscription and
converts executor events into semantic session events
(`crates/warp_tui/src/terminal_session_view.rs (111-136, 599-614)`):
- `CreateAgent` emits `StartAgentConversation` with the request and a snapshot of the parent's
  current working directory.
- `CleanupFailedChildLaunch` emits the corresponding cleanup event.
`TuiSessions::register_session` owns the `AppContext` subscription to each
session view and forwards orchestration attempts into `TuiOrchestrationModel`.
`TuiSessions::wire_orchestration` subscribes the session owner to child-session
creation and removal intents from the orchestration model. Because every
session, including a background child session, passes through the same
registration boundary, children are also wired to launch descendants
(`crates/warp_tui/src/session_registry.rs (77-181)`).
### Native child launch
`TuiOrchestrationModel` separates task creation from TUI-owned surface creation
(`crates/warp_tui/src/orchestration_model.rs (83-209)`):
1. `begin_local_oz_child_launch` starts shared server-task preparation.
2. The orchestration model emits `CreateLocalOzChildSession` after preparation succeeds.
3. `TuiSessions` creates an unfocused terminal session using the parent's captured working
   directory, then calls `register_local_oz_child_session`.
4. The child inherits the parent's execution profile and effective base model, then receives the
   requested run-wide model override.
5. `BlocklistAIHistoryModel::start_new_child_conversation` establishes lineage on the child
   surface. The task id is stamped before `record_new_conversation_request_complete` resolves the
   pending `StartAgentExecutor` slot.
6. The coordinator registers event consumers for the parent and child conversations.
7. `TuiTerminalSessionView::start_orchestrated_child` attaches the task id to the child controller
   and sends the first prompt (`crates/warp_tui/src/terminal_session_view.rs (1092-1104)`).
`create_local_terminal_session` is the single session factory for both the
focused bootstrap session and background children. Callers pass the window from
their existing view context rather than storing it in `TuiSessions`; child
orchestration derives it from the requesting parent session. The factory's
explicit startup-directory parameter preserves the parent's current directory
for child shells (`crates/warp_tui/src/session.rs (190-243)`).
### Model selection
TUI `agents.model` remains the default model for ordinary TUI surfaces.
Explicit child model selections remain pinned per surface even when they
currently equal the GUI profile or TUI file-backed fallback, so later default
changes cannot alter a running child. Installing or changing that pin persists
the selection, but emits the existing active-model event only when the
surface's effective model changes (`app/src/ai/llms.rs (844-878, 1470-1542)`).
### Streamer and session ownership
The coordinator stores only orchestration runtime bookkeeping
(`crates/warp_tui/src/orchestration_model.rs (31-39)`):
- `child_session_by_conversation` maps a child conversation to its background session.
- `event_consumers_by_session` records which conversation streams each live session consumes.
Conversation lineage remains canonical in `BlocklistAIHistoryModel`. Removing a
conversation also removes its id from `children_by_parent`
(`app/src/ai/blocklist/history_model.rs (2112-2182)`).
### Unsupported modes and failed launch cleanup
Local CLI-harness and remote requests resolve as explicit per-child failures
instead of waiting for the spawn timeout
(`crates/warp_tui/src/orchestration_model.rs (83-156, 211-255)`).
The failure path creates an errored child conversation on a synthetic surface
and echoes its id to `StartAgentExecutor`. The resulting cleanup event:
- deletes the child conversation and persisted state,
- removes it from the parent-child topology,
- emits a removal intent for any mapped background session,
- lets `TuiSessions` remove that owned session, and
- unregisters consumers when `TuiSessions` reports the removal.
This leaves no dead child conversation, session, or streamer registration.
### Transcript rendering
`crates/warp_tui/src/agent_block.rs (775-888)`:
- suppresses the `WaitForEvents` tool-call row, matching the GUI,
- renders sender and subject for each `MessagesReceivedFromAgents` entry, and
- renders the number of received lifecycle events for `EventsFromAgents`.
Lifecycle output currently contains event ids rather than event details, so the
TUI intentionally renders a count rather than a sender/status transition.
## Exports
`app/src/tui_export.rs (52-75)` exposes the shared child-launch functions and
prepared result plus the `StartAgentExecutor` request/event/outcome types needed
by the TUI surface bridge. Server-client and execution-profile implementation
types remain behind the shared launch API.
## Non-goals
- Local CLI-harness children (Claude, Codex, OpenCode, Gemini).
- Remote/cloud child materialization.
- Navigation to or revealing background child sessions.
- Removing completed child sessions; successful children remain retained like GUI hidden panes.
- Rich message bodies and per-event lifecycle status rendering.
## Testing and validation
- `crates/warp_tui/src/orchestration_model_tests.rs (154-221)` verifies that local-harness and
  remote requests resolve with explicit failures while leaving no child topology, extra session,
  or event-consumer state. It also verifies that failed-launch cleanup preserves unrelated
  retained sessions.
- `crates/warp_tui/src/agent_block_tests.rs (290-362)` renders orchestration messages and lifecycle
  counts while asserting that `WaitForEvents` contributes no tool row.
- `app/src/ai/llms_tests.rs (936-1035)` verifies explicit child pins preserve GUI pane behavior,
  suppress redundant model-change events, and precede the TUI file-backed default.
- `app/src/ai/blocklist/history_model_tests.rs (1930-1991)` verifies that removing a conversation
  cleans both its incoming child reference and its outgoing parent index.
- `cargo check -p warp_tui` passes.
- `cargo clippy -p warp_tui --all-targets --all-features --tests -- -D warnings` passes.
- `cargo clippy -p warp --lib --tests --features tui,test-util -- -D warnings` passes.
## Follow-ups
- Add local CLI-harness children by reusing the existing local-harness preparation path.
- Add a TUI-native remote child materializer.
- Add richer received-message and lifecycle-event rendering.
- Add child-session navigation and status UI on top of the retained `TuiSessions` entries.
