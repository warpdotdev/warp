# CODE-1827 PR 1: Unified TUI Blocking State
## Context
The TUI resolves the front-of-queue blocking view from the action queue and separately resolves the session's semantic interaction state. Focus, rendering, input ownership, and shortcuts must consume one authoritative snapshot without losing the concrete view handle.
## Changes
Define `BlockingInputSource` in `crates/warp_tui/src/terminal_session_view.rs` with `LongRunningCommand`, `AskQuestion(ViewHandle<TuiAskQuestionView>)`, `Permission(ViewHandle<TuiPermissionPrompt>)`, and `Orchestration(ViewHandle<TuiOrchestrationBlock>)`.

Store the resolved enum directly in `TuiInteractionState::Blocking` in `crates/warp_tui/src/terminal_session_view/state.rs`. View-backed variants map to disabled composer input and carry the exact focus/render target. `LongRunningCommand` stores no handle and maps to PTY ownership. This removes the parallel unit `Blocked` state and `PlainUserCommand` PTY variant while retaining the existing composer, startup, process, and user-controlled terminal-use states.
## Testing
Cover enum resolution, input-target mapping, focus transfer, long-running command presentation, and view-backed blocker suppression. Retain the ask-question, permission, orchestration, transcript, and focus suites.
