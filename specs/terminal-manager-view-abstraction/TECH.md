# Terminal manager view abstraction — TECH

## Context

This PR productizes the first slice of the `harry/share-terminal-model` prototype: make the local terminal manager able to drive a non-GUI terminal frontend later, without adding the TUI backend in this PR.

Today the local terminal manager is tightly coupled to `TerminalView` in two places:

- [`app/src/terminal/writeable_pty/terminal_manager_util.rs:23 @ 0b2273da`](https://github.com/warpdotdev/warp/blob/0b2273da3443eacc8d78b748f37962427e968fe5/app/src/terminal/writeable_pty/terminal_manager_util.rs#L23) wires `PtyController` directly to `ViewHandle<TerminalView>` and matches `terminal::view::Event` variants for PTY writes, resize, command execution, and native completions.
- [`app/src/terminal/local_tty/terminal_manager.rs:139 @ 0b2273da`](https://github.com/warpdotdev/warp/blob/0b2273da3443eacc8d78b748f37962427e968fe5/app/src/terminal/local_tty/terminal_manager.rs#L139) stores `ViewHandle<TerminalView>` directly and calls GUI-specific lifecycle hooks for shell startup, spawn failures, shell launch-data updates, and Unix terminal-attributes/password-prompt handling.

The object-safe terminal manager trait is still GUI-shaped:

- [`app/src/terminal/terminal_manager.rs:24 @ 0b2273da`](https://github.com/warpdotdev/warp/blob/0b2273da3443eacc8d78b748f37962427e968fe5/app/src/terminal/terminal_manager.rs#L24) exposes `view() -> ViewHandle<TerminalView>`.

Many pane, ambient, and cloud-mode call sites still rely on that `view()` method. Removing it now would create broad UI churn unrelated to this first slice, so this PR keeps the object-safe trait unchanged and makes only the local terminal manager internals generic over a new surface abstraction.

The current local constructor is also monolithic:

- [`app/src/terminal/local_tty/terminal_manager.rs:218 @ 0b2273da`](https://github.com/warpdotdev/warp/blob/0b2273da3443eacc8d78b748f37962427e968fe5/app/src/terminal/local_tty/terminal_manager.rs#L218) creates channels, `TerminalModel`, `Sessions`, `ModelEventDispatcher`, `PtyController`, `TerminalView`, sharing models, and the manager itself in one GUI-specific flow.

This PR should keep the public GUI constructor stable while extracting reusable construction phases that a later TUI constructor can reuse.

## Proposed changes

### Add the terminal surface API

Add `app/src/terminal/writeable_pty/terminal_surface.rs` and export it from `app/src/terminal/writeable_pty/mod.rs`.

The API should keep the original prototype’s event projection pattern: each concrete surface defines how its own event type collapses into a PTY/session intent using `From<&Self::Event> for Option<PtyIntent>`.

```rust
/// A normalized request from a terminal UI surface to the PTY controller.
///
/// This is the intentionally narrow vocabulary that `TerminalManager` uses to
/// drive the PTY without knowing the concrete UI implementation. It should only
/// contain actions that are meaningful to the PTY/session boundary: process
/// control, byte writes, resizing, command execution, and native shell
/// completions. UI events, pane orchestration, remote-server choice UI, and
/// shared-session protocol events stay on concrete surface event types instead
/// of leaking into this enum.
pub(crate) enum PtyIntent {
    CtrlD,
    ShutdownPty,
    WriteBytes(Cow<'static, [u8]>),
    WriteAgentInput {
        bytes: Cow<'static, [u8]>,
        mode: AIAgentPtyWriteMode,
    },
    Resize(SizeUpdate),
    ExecuteCommand(ExecuteCommandEvent),
    RunNativeShellCompletions {
        buffer_text: String,
        results_tx: Sender<Vec<ShellCompletion>>,
    },
}

/// A UI surface driven by `TerminalManager` for a terminal frontend.
///
/// `TerminalView` is the only implementation in this PR. A future TUI root can
/// implement the same contract without making `TerminalManager` depend on the
/// GUI view type.
pub(crate) trait TerminalSurface: Entity + Sized + 'static
where
    for<'a> Option<PtyIntent>: From<&'a Self::Event>,
{
    fn on_shell_determined(&mut self, ctx: &mut ViewContext<Self>);

    fn on_active_shell_launch_data_updated(
        &mut self,
        shell_launch_data: Option<ShellLaunchData>,
        ctx: &mut ViewContext<Self>,
    );

    fn on_pty_spawn_failed(&mut self, error: anyhow::Error, ctx: &mut ViewContext<Self>);

    #[cfg(unix)]
    fn should_poll_for_password_prompt(&self, ctx: &AppContext) -> bool;

    #[cfg(unix)]
    fn on_possible_password_prompt(
        &mut self,
        block_index: Option<BlockIndex>,
        ctx: &mut ViewContext<Self>,
    );

    #[cfg(unix)]
    fn on_polled_block_completed(
        &mut self,
        completed: &BlockCompletedEvent,
        ctx: &mut ViewContext<Self>,
    );
}
```

All trait methods should be required rather than default no-ops. Future implementations should intentionally opt in or explicitly no-op for each lifecycle hook.

`PtyIntent` should own or cheaply clone payloads because the PTY wiring receives borrowed surface events from `ctx.subscribe_to_view`. `ExecuteCommandEvent` is already cloneable, `Cow<'static, [u8]>` preserves the current `TerminalView` byte-event shape, and native completion senders can be cloned as they are today.

### Implement the API for TerminalView

Implement `From<&TerminalView::Event> for Option<PtyIntent>` in `app/src/terminal/view.rs` near the existing `Event` enum and terminal lifecycle methods.

The conversion maps only these existing `TerminalView::Event` variants:

- `CtrlD`
- `ShutdownPty`
- `WriteBytesToPty`
- `WriteAgentInputToPty`
- `Resize`
- `ExecuteCommand`
- `RunNativeShellCompletions`

Every other `TerminalView` event returns `None`.

Implement `TerminalSurface` for `TerminalView` next to that conversion. Lifecycle hooks delegate to the existing `TerminalView` behavior:

- [`TerminalView::on_shell_determined` at `app/src/terminal/view.rs:15923 @ 0b2273da`](https://github.com/warpdotdev/warp/blob/0b2273da3443eacc8d78b748f37962427e968fe5/app/src/terminal/view.rs#L15923)
- [`TerminalView::on_pty_spawn_failed` at `app/src/terminal/view.rs:15955 @ 0b2273da`](https://github.com/warpdotdev/warp/blob/0b2273da3443eacc8d78b748f37962427e968fe5/app/src/terminal/view.rs#L15955)
- [`TerminalView::on_active_shell_launch_data_updated` at `app/src/terminal/view.rs:25716 @ 0b2273da`](https://github.com/warpdotdev/warp/blob/0b2273da3443eacc8d78b748f37962427e968fe5/app/src/terminal/view.rs#L25716)

Unix hooks preserve today’s password notification and SSH upload behavior:

- `should_poll_for_password_prompt` returns true when needs-attention notifications are enabled/unset or when SSH drag-and-drop upload handling needs password detection.
- `on_possible_password_prompt` triggers upload password state plus optional notification.
- `on_polled_block_completed` emits upload completion only when the view is an SSH uploader.

### Refactor PTY wiring

Rename `wire_up_pty_controller_with_view` in `app/src/terminal/writeable_pty/terminal_manager_util.rs:23` to `wire_up_pty_controller_with_surface<T: EventLoopSender, S: TerminalSurface>`.

The surface subscription converts the borrowed event with:

```rust
let Some(intent) = Option::<PtyIntent>::from(event) else {
    return;
};
```

The match on `PtyIntent` should preserve current behavior exactly:

- `CtrlD` calls `PtyController::write_end_of_transmission_char`.
- `ShutdownPty` calls `PtyController::shutdown_pty`.
- `WriteBytes` calls `PtyController::write_bytes`.
- `WriteAgentInput` calls `PtyController::write_agent_bytes`.
- `Resize` calls `PtyController::resize_pty`.
- `ExecuteCommand` resolves shell type from `Sessions`, sets the active block workflow state, calls `PtyController::write_command`, and updates command history when requested.
- `RunNativeShellCompletions` calls `PtyController::run_native_shell_completions`.

The `PtyDisconnected` subscription should keep the current weak-handle pattern to avoid cycles. It only needs the surface to still exist before calling `model.lock().exit(ExitReason::PtyDisconnected)`.

Keep `wire_up_remote_server_controller_with_view` GUI-specific. Remote-server install/skip events are emitted by `TerminalView` rich content blocks and are not part of the PTY/session surface contract.

### Make local TerminalManager generic

Change the local manager to:

```rust
pub struct TerminalManager<S: TerminalSurface = TerminalView> {
    view: ViewHandle<S>,
    // existing manager-owned fields
}
```

Keep the default type parameter exactly so existing GUI code can continue using `TerminalManager` as shorthand for `TerminalManager<TerminalView>`. Future non-GUI frontends can opt in explicitly with `TerminalManager<MySurface>` without forcing current call sites, tests, or downcasts to change unless they need the non-default surface.

Default to making manager code generic whenever it depends only on the shared surface contract or on manager-owned session/PTY state. The generic impl should own:

- `Drop`
- event-loop shutdown
- model access helpers
- shell determination and spawn handling
- init-script enqueueing
- PTY creation and event-loop startup
- PTY controller wiring
- PS1/Warp prompt bindkey forwarding
- throughput recording
- integration-test PID access when possible
- Unix password-poller wiring

Keep GUI-specific code in `impl TerminalManager<TerminalView>` only when it touches `TerminalView` internals or GUI-only models. That includes:

- the existing public `create_model(...)` constructor
- shared-session sharer setup
- agent-view and active-agent registration
- prompt, presence, LLM, and input-mode broadcast wiring
- remote-server choice-block wiring
- `on_view_detached` sharing teardown

`TerminalManager<TerminalView>` remains the only local manager instantiation that implements the object-safe `crate::terminal::TerminalManager` trait in this PR.

### Factor reusable constructor phases

Keep `TerminalManager<TerminalView>::create_model(...)` behavior and return type unchanged, but do not leave it as one monolithic GUI-only constructor. Split reusable construction phases into private helpers that a future TUI constructor can call without constructing a `TerminalView`.

Reusable constructor helpers should cover:

- channel creation
- inactive PTY read broadcast creation
- `ChannelEventListener` creation
- `Sessions` and `ModelEventDispatcher` model registration
- `TerminalModel` creation
- restored-block/conversation restoration preprocessing where it is not GUI-specific
- `PtyController` registration
- remote-server controller registration
- manager assembly
- surface-to-PTY wiring
- async shell-starter resolution and spawn kick-off

The GUI constructor remains responsible for GUI-only setup around those helpers:

- resolving restored AI conversations into `TerminalView` inputs
- creating `TerminalView`
- appending restoration separators only when needed for the GUI block list
- registering active-agent and agent-view models
- shared-session sharer wiring
- prompt, presence, LLM, and input-mode broadcasts
- remote-server choice-block wiring
- pane/window UI integration

Do not add a public TUI constructor in this PR. It is okay to introduce a private `SessionCore`-like helper struct if that is the cleanest way to return shared construction products, but keep it narrowly scoped to this slice and avoid pulling in broader TUI rendering/session logic from the prototype branch.

### Move Unix password-poller wiring to the surface boundary

The password-poller hook exists because the local manager owns the Unix `termios` polling machinery, but only the surface knows whether password-prompt detection is useful and how to present the result. In the GUI today it powers needs-attention password notifications and SSH drag-and-drop upload password/completion events.

Move the local manager’s terminal-attributes poller off `TerminalViewEvent::BlockStarted` and `TerminalViewEvent::BlockCompleted`.

Subscribe to `ModelEventDispatcher` instead:

- On `ModelEvent::AfterBlockStarted { is_for_in_band_command: false, .. }`, record the active block index and call `surface.read(ctx, |surface, ctx| surface.should_poll_for_password_prompt(ctx))` before starting the poller.
- On `ModelEvent::BlockCompleted(completed)`, stop polling and call `surface.on_polled_block_completed(completed, ctx)`.
- On `TerminalAttributesPollerEvent::TermiosQueryFinished`, if ECHO is off and ICANON is on, call `surface.on_possible_password_prompt(block_index, ctx)` and stop the poller after the first detected prompt, matching today’s one-notification-per-command behavior.

A future TUI surface can return false or no-op these hooks.

### Other manager types

Use the generic PTY wiring for `app/src/terminal/remote_tty/terminal_manager.rs` because it still drives a `TerminalView` and validates the new abstraction without genericizing the remote manager itself.

Do not genericize `MockTerminalManager` or `shared_session::viewer::TerminalManager` in this PR unless required by compilation.

Leave `crate::terminal::TerminalManager::view() -> ViewHandle<TerminalView>` intact for this slice. Because that object-safe trait is still GUI-shaped, only `TerminalManager<TerminalView>` should implement it. The future TUI branch can either remove `view()` from the object-safe trait or avoid boxing the TUI manager behind that trait.

## Testing and validation

Run formatting and focused compile checks:

- `./script/format`
- `cargo check -p warp`

Run targeted tests that cover terminal manager/view wiring and existing terminal behavior where available:

- terminal view tests that cover command execution events and PTY write events
- pane creation tests that rely on `TerminalManager::view()`
- SSH file-upload/password-prompt tests, if present

The key acceptance criterion is that the GUI path compiles and behaves identically while local `TerminalManager` no longer needs to know that its surface is specifically `TerminalView` for PTY/session lifecycle wiring.

## Parallelization

Do not parallelize implementation across child agents. The changes are tightly coupled across `TerminalSurface`, PTY wiring, local manager generics, and constructor factoring; parallel edits would likely create overlapping changes in the same files.

Validation can be separated after the implementation compiles: one pass can run targeted tests while another reviews the generic/GU-specific boundary, but a single implementer should own the code changes.

## Risks and mitigations

- **Accidentally widening the surface boundary.** Keep `PtyIntent` limited to PTY/session-driving actions and leave shared-session, pane, and remote-server UI events on concrete surface types.
- **Regressing password notifications or SSH upload handling.** Preserve the current termios detection behavior and route results through explicit Unix surface hooks.
- **Over-genericizing GUI-only code.** Keep code in `impl TerminalManager<TerminalView>` when it touches `TerminalView` internals, agent-view models, presence UI, or shared-session GUI state.
- **Constructor refactor churn.** Keep the public GUI constructor behavior and return type unchanged while extracting private helpers underneath it.

## Out of scope

- No TUI backend or `LaunchMode::Tui`.
- No terminal-history rendering, virtual list, or key-passthrough work.
- No removal of `TerminalManager::view()` from the object-safe trait.
- No behavior changes to GUI terminal startup, command execution, remote-server setup, session sharing, password notifications, or SSH file upload.
