# Terminal manager view abstraction — TECH

## Context

The local TTY terminal manager now has a reusable construction path for terminal frontends. The GUI remains the only frontend in this change, but the manager no longer needs to treat `TerminalView` as part of the object-safe manager contract.

The pre-change local manager was tightly coupled to `TerminalView`:

- [`app/src/terminal/writeable_pty/terminal_manager_util.rs:23 @ 0b2273da`](https://github.com/warpdotdev/warp/blob/0b2273da3443eacc8d78b748f37962427e968fe5/app/src/terminal/writeable_pty/terminal_manager_util.rs#L23) wired `PtyController` directly to `ViewHandle<TerminalView>` and matched `terminal::view::Event` variants for PTY writes, resize, command execution, and native completions.
- [`app/src/terminal/local_tty/terminal_manager.rs:139 @ 0b2273da`](https://github.com/warpdotdev/warp/blob/0b2273da3443eacc8d78b748f37962427e968fe5/app/src/terminal/local_tty/terminal_manager.rs#L139) stored `ViewHandle<TerminalView>` directly and called GUI-specific lifecycle hooks for shell startup, spawn failures, shell launch-data updates, and Unix terminal-attributes/password-prompt handling.
- [`app/src/terminal/terminal_manager.rs:24 @ 0b2273da`](https://github.com/warpdotdev/warp/blob/0b2273da3443eacc8d78b748f37962427e968fe5/app/src/terminal/terminal_manager.rs#L24) exposed `view() -> ViewHandle<TerminalView>` from the object-safe `TerminalManager` trait, so callers recovered the GUI view from the boxed manager.

The current design splits the terminal manager responsibilities into three layers:

1. `TerminalSurface` and `PtyIntent` define the narrow frontend-to-PTY boundary.
2. `TerminalManager<S>` owns local terminal model/session/controller/event-loop state for any terminal surface.
3. `TerminalManager<S>::create_model` is the generic model-producing construction path. The current GUI pane system calls it with `S = TerminalView` and a GUI surface setup callback.

## Proposed changes

### Terminal surface API

`app/src/terminal/writeable_pty/terminal_surface.rs` defines `PtyIntent`, `PtyIntentEvent`, and `TerminalSurface`.

`PtyIntent` is the only event vocabulary the generic PTY wiring understands. It is intentionally limited to PTY/session-driving actions: process control, byte writes, resize, command execution, and native shell completions. UI events, pane orchestration, remote-server choice UI, and shared-session protocol events remain on concrete surface event types.

`PtyIntentEvent` preserves the `From<&SurfaceEvent> for Option<PtyIntent>` projection pattern while hiding the higher-ranked bound behind a simple method:

```rust
pub(crate) trait PtyIntentEvent {
    fn pty_intent(&self) -> Option<PtyIntent>;
}

impl<T> PtyIntentEvent for T
where
    for<'a> Option<PtyIntent>: From<&'a T>,
{
    fn pty_intent(&self) -> Option<PtyIntent> {
        Option::<PtyIntent>::from(self)
    }
}
```

`TerminalSurface` is the frontend contract consumed by `TerminalManager<S>`. It requires a surface event type that implements `PtyIntentEvent` and lifecycle hooks for shell startup, PTY spawn failure, launch-data updates, and Unix password-prompt polling.

`TerminalView` implements this contract and maps only the existing PTY-driving `terminal::view::Event` variants to `PtyIntent`:

- `CtrlD`
- `ShutdownPty`
- `WriteBytesToPty`
- `WriteAgentInputToPty`
- `Resize`
- `ExecuteCommand`
- `RunNativeShellCompletions`

Every other `TerminalView` event maps to `None`.

### View-agnostic terminal manager trait

`crate::terminal::TerminalManager` no longer exposes `view() -> ViewHandle<TerminalView>`. The object-safe trait now describes terminal model/session lifecycle:

- `model()`
- `on_view_detached(...)`
- `as_any()`
- `as_any_mut()`

Constructors that create a `TerminalView` return the view separately alongside the boxed manager. This keeps the boxed manager view-agnostic while preserving existing pane code that needs the `TerminalView` handle.

Current constructors that return `(manager, surface)`:

- `local_tty::TerminalManager<S>::create_model`, called by the GUI path as `TerminalManager::<TerminalView>::create_model`
- `remote_tty::TerminalManager::create_model`
- `MockTerminalManager::create_model`
- `shared_session::viewer::TerminalManager::new`
- `shared_session::viewer::TerminalManager::new_deferred`

### Generic local manager construction

`app/src/terminal/local_tty/terminal_manager.rs` defines the local manager as:

```rust
pub struct TerminalManager<S> {
    view: ViewHandle<S>,
    // model, controllers, event-loop state, and retained handles
}
```

`TerminalManager<S>::create_model(...)` is the canonical local terminal construction API. It owns the local terminal session construction order, creates the manager-owned channels/models/controllers internally, calls one surface setup callback that returns `(surface, post_wire)`, wires the surface to the `PtyController` through `wire_up_pty_controller_with_surface`, runs `post_wire`, boxes the manager as a WarpUI model, schedules shell determination, and returns `(manager_model, surface)`.

The surface factory callback receives only the components a surface needs:

- `TerminalModel`
- `Sessions`
- `ModelEventDispatcher`
- wakeup receiver
- inactive PTY reads receiver
- terminal colors
- current terminal size

The manager stores the remaining startup and lifetime components itself, including the event-loop sender/receiver, channel event proxy, `PtyController`, `RemoteServerController`, and shell-starter source. Shell determination later consumes that stored startup state from the manager after it has been registered as a WarpUI model.

The surface setup callback can return a deferred `post_wire` closure for surface-specific wiring. The constructor runs that closure after the PTY controller is wired and the manager has been assembled. This keeps ordering-sensitive post-wiring inside the manager-owned construction flow without requiring the generic manager core to import `TerminalView`.

For the GUI caller, `TerminalManager::<TerminalView>::create_model(...)` now:

1. Resolves GUI-specific restored blocks from explicit restored blocks and conversation restoration.
2. Uses a surface setup callback that creates `CurrentPrompt`, `PromptType`, and `TerminalView` from the manager-created surface components, then returns `(view, post_wire)`.
3. The returned `post_wire` closure appends the GUI restoration separator when needed, wires remote-server choice UI, and wires `TerminalView`-specific session sharing.
4. The generic constructor boxes the manager, schedules shell determination, and returns `(manager_model, terminal_view)`.

This keeps the end-to-end local terminal construction protocol in `TerminalManager<S>::create_model` while keeping GUI-specific work in the `TerminalView` surface setup callback. A future TUI surface can call the same function with a different surface setup callback.

### Generic PTY wiring

`wire_up_pty_controller_with_surface<T, S>(...)` replaces the direct `TerminalView` wiring. It subscribes to the surface event stream, calls `event.pty_intent()`, and handles only `PtyIntent` values.

The behavior of each intent matches the old `TerminalView` event match:

- raw bytes and agent bytes write to the PTY controller
- resize resizes the PTY
- command execution resolves shell type from `Sessions`, sets workflow state, writes the command, and updates command history when requested
- native completions call through to `PtyController::run_native_shell_completions`
- PTY disconnection exits the `TerminalModel`

`wire_up_remote_server_controller_with_view` remains GUI-specific because the remote-server install/skip choice is rendered as `TerminalView` rich content.

### TerminalView-specific session sharing boundary

Local session sharing remains GUI-specific in this change. The reusable lower layers stay unchanged:

- `shared_session::sharer::Network`
- ordered terminal event flow from `TerminalModel`
- existing shared-session handler helpers

The local sharer wiring is grouped behind `wire_up_terminal_view_session_sharing(...)`. That helper owns the current `TerminalView` adapter responsibilities:

- prompt updates
- presence selection
- LLM/input-mode/conversation broadcasts
- AgentView and active-agent registration
- CLI-agent session broadcasts
- local sharer `Network` setup
- network status UI reactions

`shared_session::manager::Manager` remains GUI-shaped and continues storing `TerminalView` handles. The sharing boundary is now easier to locate and refactor later without making this change TUI-sharing-ready.

### Unix password-prompt polling

The local manager still owns the Unix `termios` poller, but the surface decides whether polling is useful and how to react.

The poller now subscribes to `ModelEventDispatcher` rather than `TerminalView` events:

- `ModelEvent::AfterBlockStarted { is_for_in_band_command: false, .. }` records the active block index and starts polling only if `surface.should_poll_for_password_prompt(ctx)` returns true.
- `ModelEvent::BlockCompleted(completed)` stops polling and calls `surface.on_polled_block_completed(completed, ctx)`.
- `TerminalAttributesPollerEvent::TermiosQueryFinished` checks for ECHO off and ICANON on, calls `surface.on_possible_password_prompt(block_index, ctx)`, and stops polling after the first detected prompt.

For `TerminalView`, these hooks preserve existing password notification and SSH upload behavior.

## Testing and validation

Run formatting and compile checks:

- `./script/format`
- `cargo check -p warp`
- `cargo clippy -p warp --all-targets --tests -- -D warnings`

Run focused tests covering:

- terminal manager/view constructor paths
- PTY write and command execution events
- pane creation paths that now receive the `TerminalView` separately
- shared-session start/stop behavior

Validation performed during implementation:

- `./script/format`
- `cargo check -p warp`

## Parallelization

Do not parallelize implementation across child agents. The core changes touch the same local manager, object-safe trait, PTY wiring, and pane construction call sites; parallel work would create overlapping edits.

Validation can be performed separately after the code compiles.

## Risks and mitigations

- **Surface boundary grows too broad.** Keep `PtyIntent` limited to PTY/session-driving actions and leave pane, UI, remote-server choice, and shared-session protocol events on concrete surface event types.
- **Constructor churn breaks pane creation.** Return `TerminalView` explicitly from GUI constructors and update pane helpers at the same call sites that previously used `manager.view()`.
- **Shared-session behavior regresses.** Keep protocol/model layers unchanged and move the GUI-specific sharer setup as a group rather than rewriting it.
- **Password-prompt behavior changes.** Route the same termios detection through explicit `TerminalSurface` hooks and preserve one-notification-per-command behavior.

## Out of scope

- No TUI backend or `LaunchMode::Tui`.
- No terminal-history rendering, virtual list, or key-passthrough work.
- No generic shared-session frontend abstraction.
- No blanket object-safe trait implementation for every `TerminalManager<S>`; current detach behavior is still specific to the `TerminalView` manager.
