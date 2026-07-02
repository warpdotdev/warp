# TUI Ctrl-C — Tech Spec

See `specs/tui-ctrl-c/PRODUCT.md` for the product behavior.

## Context

Ctrl-c never reached the TUI view layer: the headless input driver intercepted it before any dispatch (`is_ctrl_c` in `crates/warpui_core/src/runtime/mod.rs`) and called `terminate_app(ForceTerminate)`. Everything needed for contextual handling already existed in the view layer:

- `TuiTerminalSessionView` (`crates/warp_tui/src/terminal_session_view.rs`) owns the transcript, the input view, and `ModelHandle<BlocklistAIController>`, and its view id is the `terminal_surface_id` keying conversation state.
- Streaming detection: `BlocklistAIHistoryModel::active_conversation(surface_id)` + `ConversationStatus::is_in_progress()` — the same pattern the GUI's `cancel_active_conversation` uses (`app/src/terminal/input.rs`).
- Cancellation: `BlocklistAIController::cancel_conversation_progress(conversation_id, CancellationReason::ManuallyCancelled, ctx)` cancels both in-flight response streams and pending tool actions.
- Prior art: the GUI agent view implements the identical press-again-to-exit UX (`ENTER_OR_EXIT_CONFIRMATION_WINDOW` = 1s, `PendingConfirmation::Exit { expires_at }` in `app/src/ai/blocklist/agent_view/controller.rs`).

## Proposed Changes

### 1. Route ctrl-c through normal dispatch (`crates/warpui_core/src/runtime/mod.rs`)

The `is_ctrl_c` intercept in `spawn_tui_driver`'s dispatch task is removed; ctrl-c now flows through the standard keymap + element-tree dispatch like any other keystroke. Quitting becomes the responsibility of the app's views.

### 2. `TuiEventHandler::new` takes `Box<dyn TuiElement>` (`crates/warpui_core/src/elements/tui/event_handler.rs`)

The constructor now accepts the child as an already-boxed trait object (callers use `.finish()`), so wrappers compose uniformly with helpers that return `Box<dyn TuiElement>` (e.g. `RootTuiView`'s placeholder helpers).

### 3. `ExitConfirmation` state machine (`crates/warp_tui/src/exit_confirmation.rs`)

A small, context-free module so the timing logic is directly unit-testable:

- `CTRL_C_EXIT_WINDOW: Duration = 1s` (GUI parity).
- `ExitConfirmation { expires_at: Option<Instant> }` with `is_armed()`, `should_exit(now)`, `arm(now) -> expires_at`, `disarm()`, and `expire(expires_at)` (stale-timer-safe: a re-arm supersedes an earlier timer, whose `expire` becomes a no-op).

### 4. Session-level ctrl-c handling (`crates/warp_tui/src/terminal_session_view.rs`)

- Ctrl-c dispatches through the TUI **keymap pass**: `TuiTerminalSessionView::init` registers a fixed (non-remappable) binding `ctrl-c → TuiTerminalSessionAction::Interrupt` with predicate `id!("TuiTerminalSessionView")` (see `crates/warp_tui/src/keybindings.rs`, which aggregates per-view `init` fns at TUI startup). The session view focuses the input view, so the responder chain is `[root, session, input]`; the input context binds no ctrl-c, so the keystroke falls through to the session binding — preserving "ctrl-c fires only when the input doesn't consume it". The view's `TypedActionView::Action` changed from `()` to `TuiTerminalSessionAction`.
- `handle_interrupt`:
  1. If `exit_confirmation.should_exit(now)` → `ctx.terminate_app(ForceTerminate, None)`. The `TuiDriverHandle`'s guards restore raw mode / the alternate screen on teardown.
  2. Else `cancel_active_conversation(ctx)` — cancels the surface's active conversation when it is non-empty (a fresh conversation defaults to `InProgress` before any exchange exists, mirroring the GUI's `is_empty()` guard) and `is_in_progress() || is_blocked()`; falls back to clearing the input via the new `TuiInputView::clear`.
  3. Always re-arms the confirmation and spawns a `Timer::after(CTRL_C_EXIT_WINDOW)` that calls `expire(expires_at)` + `notify()` to hide the hint when the window lapses.
- Disarm on typing: a `subscribe_to_model` on the input's `CodeEditorModel` disarms the confirmation on `ContentChanged` when the buffer becomes non-empty. The ctrl-c clear itself leaves the buffer empty, so the window it arms survives its own clear.
- Footer: `render_footer()` appends a one-row dim `TuiText` below the input box showing `ctrl-c again to exit` while armed (a single space otherwise, keeping the row height stable). It is a placeholder for the fuller footer design (left contextual hints + right status section).

### 5. Root-level exit fallback (`crates/warp_tui/src/root_view.rs`)

`RootTuiView::init` registers a fixed binding `ctrl-c → RootTuiAction::ExitApp` (→ `terminate_app(ForceTerminate)`) with predicate `id!("RootTuiView")`. While a session exists the deeper session binding wins in the keymap pass; pre-session the responder chain is `[root]`, so this fires — guaranteeing the app is always exitable (the placeholders say "Press Ctrl-C to exit").

### 6. Supporting changes

- `TuiInputView::is_empty(ctx)` / `clear(ctx)` (`crates/warp_tui/src/input/view.rs`); `submit` now reuses `clear`.
- `app/src/tui_export.rs` exports `CancellationReason`.

## Diagram

```
crossterm event ──► spawn_tui_driver dispatch (no ctrl-c intercept)
   └─► TuiScreen::dispatch_event
         ├─ keymap pass (responder chain [root, session, input], deepest-first):
         │     input ctx: tui:input:* editing bindings (no ctrl-c)
         │     session ctx: ctrl-c → Interrupt          ← fixed binding (init)
         │     root ctx:    ctrl-c → ExitApp (pre-session only)
         └─ element tree (only keys the keymap pass left unhandled):
              column: transcript / input box / footer hint
              (input element inserts printable chars; mouse events)

Interrupt ─► handle_interrupt
   ├─ should_exit(now)?        ──► terminate_app(ForceTerminate)
   ├─ conversation running?    ──► cancel_conversation_progress(ManuallyCancelled)
   ├─ else                     ──► input_view.clear()
   └─ arm(now) ─► footer shows "ctrl-c again to exit"
        └─ Timer::after(1s) ─► expire ─► hint hidden
        └─ input becomes non-empty ─► disarm ─► hint hidden
```

## Testing and Validation

- `crates/warp_tui/src/exit_confirmation_tests.rs`: arming, in-window exit, expiry, re-arm superseding a stale timer, disarm.
- `crates/warp_tui/src/input/view_tests.rs::clear_empties_buffer_and_resets_scroll`: `clear` empties the buffer, resets scroll, and restores the cursor to the origin.
- `crates/warpui_core/src/runtime/mod_tests.rs`: `keymap_binding_dispatches_typed_action_to_tui_view` (the keymap pass dispatches a bound action to a TUI view) and `unhandled_keymap_binding_falls_through_to_element_pass` (a matched-but-unhandled binding does not swallow the key); manual validation steps live in the product spec.

## Risks and Mitigations

- **Ctrl-c swallowed by a future subtree element**: element-pass handlers run only when the keymap pass leaves a key unhandled, and ctrl-c is claimed there — so subtree elements cannot starve it. The root-level binding guarantees exit remains possible in every state.
- **GUI binding leakage into the TUI**: GUI bindings never fire in the TUI — predicate-scoped ones don't match TUI contexts, and predicate-less ones dispatch action types no TUI view handles, so the key falls through. Debug-time validators in `crates/warp_tui/src/keybindings.rs` additionally require every keystroke binding matching a TUI context to be TUI-owned (a `tui:` name or the `tui` group), which also prevents permissive multi-keystroke chords from swallowing prefix keys via a pending match.

## Follow-ups / Out of scope

- Full footer per the Figma design: left contextual hints ("↑ to edit • Esc to stop • ← for conversations") and the right model/cwd/branch section; `render_footer` is structured to grow into this.
- `Esc` to stop streaming, reusing `cancel_active_conversation`.
- Wiring `ShellCommandExecutorEvent::CancelExecution` so an agent-requested PTY command can be interrupted (`TODO(tui-agent-cancel)` in `terminal_session_view.rs`).
