# TECH: `TuiSessions` full-view multi-session container

First PR in the TUI local-orchestration stack above the orchestration-card
branch. The follow-up adds `TuiOrchestrationModel` and materializes native
local children as background sessions.

## Context

The TUI currently retains one terminal manager in a singleton and one
`TuiTerminalSessionView` in `RootTuiView`. The shared AI layer already supports
multiple surfaces keyed by `terminal_surface_id`, but the TUI has no container
for multiple live terminal surfaces or a model-level notion of focus.

Every TUI session needs a full view. `LocalTtyTerminalManager` requires a
terminal surface synchronously, and background orchestration children use the
same view-backed terminal machinery as the focused session. This follows the
GUI's hidden-pane model: background sessions retain complete views, while only
the focused view participates in rendering and input routing.

## Proposed changes

### New: `crates/warp_tui/src/session_registry.rs`

- Add `TuiSessionId(EntityId)`, using the eagerly-created view's entity id as
  both session identity and shared-model `terminal_surface_id`.
- Add `TuiSessions`, a singleton retaining each session's
  `TuiTerminalSessionView` and terminal manager (the manager kept only to tie
  the PTY and event loop to the session's lifetime), plus the window and exit
  summary context needed to construct additional session views.
- Track `focused_session_id` and emit `SessionAdded` and `FocusChanged`
  events. All session creation paths register here so orchestration can
  subscribe to every session, including future nested children.
- Session removal (with a `SessionRemoved` event and focus fallback) lands
  with the orchestration PR that first needs it.

### Changed: `crates/warp_tui/src/root_view.rs`

- Replace the single authenticated child with projection of
  `TuiSessions::focused_session()`.
- Subscribe to session events for redraws.
- Session creation does not flow through the root; it only projects sessions.
- Return only the focused view from `child_view_ids()`, keeping background
  views out of rendering and the responder chain.

### Changed: `crates/warp_tui/src/session.rs`

- Replace the single-session singleton with `TuiSessions`.
- Create the full `TuiTerminalSessionView` synchronously inside the terminal
  manager's surface callback, then register the view and its returned manager
  with `TuiSessions` so the container owns the session lifetime.
- The login bootstrap registers the first session focused.

### Changed: `crates/warp_tui/src/terminal_session_view.rs`
- Keep construction focus-neutral. When `TuiSessions` activates a session, the
  view focuses its current input owner and refreshes the exit summary.
- Route later blocker, process, CLI-subagent, and conversation-restoration
  focus requests through focused-session guards so background views cannot
  steal focus or replace the focused session's resume token.

## Non-goals

- Session navigation UI or keybindings.
- Session persistence; `TuiSessionId` is process-local.
- Remote or CLI-harness child materialization.

## Testing and validation

- Unit-test add/focus behavior and event emission on `TuiSessions`.
- Construct two full session views and verify the root projects only the
  focused view, background registration does not steal focus, and focus
  changes reuse retained views.
- Run `cargo nextest run -p warp_tui --no-fail-fast` and `./script/format`.