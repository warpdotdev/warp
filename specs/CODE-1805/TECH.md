# TUI Shell Command Execution — Tech Spec

Implements the behavior in [`PRODUCT.md`](./PRODUCT.md). References are pinned to commit `51145bb70dc2e461d1152880e8f173dce28ac165`.

Stacked on the `InputModePolicy` refactor ([`specs/input-mode-policy/TECH.md`](../input-mode-policy/TECH.md)), which makes `BlocklistAIInputModel` view-agnostic so the TUI can reuse the GUI's input-mode state machine deterministically (`{AI, locked}` default via `TuiInputModePolicy`; no reactive GUI transitions).

## Context

The TUI prompt input ([`crates/warp_tui/src/input/view.rs @ 51145bb7`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warp_tui/src/input/view.rs)) is a `TuiInputView` backed by a char-cell `CodeEditorModel`. Keystrokes map to `TuiInputAction`s in `TuiInputElement::dispatch_event` ([view.rs (940-1022)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warp_tui/src/input/view.rs#L940-L1022)); Enter emits `TuiInputViewEvent::Submitted` and clears the buffer ([view.rs (492-497)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warp_tui/src/input/view.rs#L492-L497)).

`TuiTerminalSessionView` ([`crates/warp_tui/src/terminal_session_view.rs @ 51145bb7`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warp_tui/src/terminal_session_view.rs)) routes every `Submitted` to `send_prompt` ([terminal_session_view.rs (129-137)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warp_tui/src/terminal_session_view.rs#L129-L137)) — there is no shell path. It already:
- constructs a `BlocklistAIInputModel` ([terminal_session_view.rs (90-98)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warp_tui/src/terminal_session_view.rs#L90-L98)) whose `InputConfig` is unused by the TUI today;
- bridges agent-requested commands to PTY execution via `TuiTerminalSessionEvent::ExecuteCommand` with `CommandExecutionSource::AI` ([terminal_session_view.rs (196-246)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warp_tui/src/terminal_session_view.rs#L196-L246)), and the transcript renders the resulting command blocks;
- draws the input border in cyan ([terminal_session_view.rs (262-281)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warp_tui/src/terminal_session_view.rs#L262-L281)). There is currently no hint line below the input.

GUI prior art (the semantics we mirror):
- Shell mode state = `InputConfig { input_type: InputType::Shell, is_locked: true }` on `BlocklistAIInputModel` ([`app/src/ai/blocklist/input_model.rs:112 @ 51145bb7`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/ai/blocklist/input_model.rs#L112)).
- Typed-only `!` detection strips the prefix immediately; the buffer never contains `!` ([`app/src/terminal/input.rs (10206-10247) @ 51145bb7`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/terminal/input.rs#L10206-L10247), `TERMINAL_INPUT_PREFIX` at [input.rs:489](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/terminal/input.rs#L489)).
- Shell submissions execute with `CommandExecutionSource::User` via `try_execute_command_from_source` ([input.rs:7150](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/terminal/input.rs#L7150)), gated by `can_execute_command` ([input.rs (6945-6960)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/terminal/input.rs#L6945-L6960)) which blocks when the active block is long-running and not in-band.
- A successful execution cancels any in-progress conversation with `CancellationReason::UserCommandExecuted` ([input.rs (13392-13406)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/terminal/input.rs#L13392-L13406), reason defined at [`app/src/ai/agent/mod.rs:94`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/ai/agent/mod.rs#L94), `cancel_conversation_progress` at [`app/src/ai/blocklist/controller.rs:2596`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/ai/blocklist/controller.rs#L2596)).
- Visuals use `theme.ansi_fg_blue()` for the prefix glyph and border ([`app/src/terminal/input/agent.rs (206-220)`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/terminal/input/agent.rs#L206-L220), prefix indicator at [input.rs:16243](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/terminal/input.rs#L16243)).

## Proposed changes

### 0. TUI framework dependency: `TuiFlex` cross-axis sizing

The shell-mode gutter is composed with existing `warpui_core::elements::tui` primitives on top of the stacked `TuiFlex` change (branch `harry/tui-flex-alignment`):
- **Styled text**: `TuiText::with_style(TuiStyle)` applies one style to a whole run ([`crates/warpui_core/src/elements/tui/text.rs:45 @ 51145bb7`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warpui_core/src/elements/tui/text.rs#L45)). The blue `!` glyph and the single-color hint line each render as their own `TuiText`; no rich-text/multi-span element is required.
- **Horizontal composition**: the gutter is a `TuiFlex::row()` — a fixed `!` affordance child followed by the editor element as a flex child filling the leftover width. The row sizes to the editor's height because `TuiFlex` (per the stacked branch) sizes its cross axis to its largest child, clamped to the constraint, matching the GUI `Flex`'s content-sized cross-axis policy; cross-axis positioning/fill uses `TuiFlex::with_cross_axis_alignment(CrossAxisAlignment)` (shared with the GUI), with `Stretch` used by the transcript's full-width input banner in `crates/warp_tui/src/agent_block_sections.rs`.
- **Border color**: the input border already uses `TuiContainer::with_border_style` ([container.rs:120](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warpui_core/src/elements/tui/container.rs#L120)); shell mode just swaps the style's fg. Theme-token → `Color` conversion (`CoreFill::from(ThemeFill::from(...)).into()`) already exists in the session view's render.
- **Esc key**: the runtime already converts `KeyCode::Esc` to keystroke key `"escape"` ([`crates/warpui_core/src/runtime/event_conversion.rs:146`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warpui_core/src/runtime/event_conversion.rs#L146)); the input key table just gains a binding for it, gated on a shell-mode keymap-context flag. No new `TuiEvent` variants.
- **Clickable gutter**: the `!` affordance is a `TuiHoverable` (`crates/warpui_core/src/elements/tui/hoverable.rs`), whose clicks use the GUI's press-then-release pairing — an unconsumed mouse-down inside the area arms the click on the shared `MouseState`, and the mouse-up fires the handler only when released inside the area.
- **New surface area is crate-local to `warp_tui`**: two `TuiInputAction` variants (`ExitShellMode`, `SetCursor`), the footer hint slot, and the reusable `TransientHint` notice (`crates/warp_tui/src/transient_hint.rs`) driven by an abortable delayed task via the existing view-context spawn APIs.

### 1. Exports (`app/src/tui_export.rs`)

The input-mode types (`InputConfig`, `InputType`, `InputTypeAutoDetectionSource`, `InputModePolicy`) are exported by the stacked policy PR; `CancellationReason` and `CommandExecutionSource` are already exported. This PR additionally exports `BlockSpacing` and `BlockPadding` so the TUI can define its own transcript block spacing (§5).

### 2. Shell-mode state and editing (`crates/warp_tui/src/input/view.rs`)

Shell mode lives on the shared `BlocklistAIInputModel` (per GUI): `TuiInputView` gains an `input_mode: ModelHandle<BlocklistAIInputModel>`, wired by the session view at construction (unit tests construct one via `BlocklistAIInputModel::mock`, a `test-util`-gated constructor that skips production subscriptions). The single definition of "in shell mode" is `input_mode_policy::is_shell_mode` — `input_config() == SHELL_LOCKED_CONFIG` — shared by the input view and the session view. The model's `TuiInputModePolicy` (from the stacked PR) makes the default `{AI, locked}` and the state deterministic.

Action handling changes:
- `InsertChar('!')` with the cursor at the buffer start (no active selection) and not already in shell mode: set `InputConfig { Shell, locked: true }` (source `InputTypeAutoDetectionSource::ShellPrefix`) instead of inserting the char. Anywhere else — or when typing over a selection anchored at the start — `!` inserts literally. Detection at the `InsertChar` level is inherently typed-only — `TuiEvent` has no paste variant ([`crates/warpui_core/src/elements/tui/event.rs:24`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warpui_core/src/elements/tui/event.rs#L24)), so PRODUCT.md #3 falls out for free; if bracketed paste is later routed through `InsertChar`, a pasted leading `!` entering shell mode is acceptable per product decision.
- `Backspace` with the cursor at the very start while in shell mode: exit shell mode (reset config to AI-locked) instead of deleting; text preserved (PRODUCT.md #10).
- New `TuiInputAction::ExitShellMode` registered as the `tui:input:exit_shell_mode` binding on plain `esc`, with a context predicate requiring a shell-mode keymap-context flag that `TuiInputView::keymap_context` inserts while in shell mode (the same conditional-flag pattern the GUI `Input` uses), so esc stays available to ancestors otherwise (PRODUCT.md #11).
- `Submit` no longer clears the buffer; it only emits `Submitted`. A new `pub fn clear(&mut self, ctx)` clears buffer + scroll, and the session view calls it on every accepted submission (agent prompt or executed shell command). This is what lets a blocked shell submission retain its text (PRODUCT.md #20).

Rendering changes:
- `TuiInputView::render` branches on shell mode: outside it, the plain editor element; in it, a `TuiFlex::row()` composed by `TuiInputView::shell_element` — a fixed gutter child (the `!` glyph styled with the shell-mode accent `theme.ansi_fg_blue()`, wrapped in a one-column right-padding `TuiContainer` and a `TuiHoverable` whose click state lives on the view) followed by the editor as a flex child. The flex hands the editor a slot two columns narrower and offset right, so the width pushed onto the char-cell render state, cursor placement, and mouse mapping need no shell-mode special-casing inside `TuiInputElement` at all. Because the affordance is outside the buffer, selection/select-all/cursor math need no special-casing either (PRODUCT.md #2, #8). A click on the gutter dispatches `SetCursor` at the buffer start — a cursor placement that never starts a drag selection, so it cannot leave the view's `is_selecting` armed.

### 3. Submit routing and execution (`crates/warp_tui/src/terminal_session_view.rs`)

The `Submitted` handler branches on `is_shell_mode`:
- Agent path: unchanged (`send_prompt`), then `input_view.clear()`.
- Shell path (`fn execute_user_command`):
  1. Whitespace-only text → no-op, stay in shell mode (PRODUCT.md #16-17).
  2. PTY-availability check mirroring `can_execute_command`: lock the `TerminalModel` and reject when the session isn't bootstrapped or the active block `is_active_and_long_running()` and not in-band. On reject: keep the input text, show the transient hint `cannot run — command already running` (PRODUCT.md #20). Keep the lock scope minimal per the terminal-model locking guidance.
  3. Cancel any in-progress conversation: `ai_controller.cancel_conversation_progress(id, CancellationReason::UserCommandExecuted, ctx)` for the selected conversation when `status().is_in_progress()` (PRODUCT.md #19), mirroring [input.rs (13392-13406)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/terminal/input.rs#L13392-L13406).
  4. Emit `TuiTerminalSessionEvent::ExecuteCommand(ExecuteCommandEvent { command, session_id, source: CommandExecutionSource::User, should_add_command_to_history: true, .. })` — the same PTY-intent bridge agent commands use, so the transcript terminal-block rendering comes for free (PRODUCT.md #13-14).
  5. `input_view.clear()` then `input_view.exit_shell_mode()` — the input view owns both mode transitions (PRODUCT.md #15).

Border color: `render` picks `ansi_fg_blue()` when `is_shell_mode`, else the current cyan (PRODUCT.md #5).

### 4. Footer hint slot with transient messages (`crates/warp_tui/src/transient_hint.rs`)

The session view's status footer owns a single left hint slot (the Figma `← for conversations` slot), highest priority first: the ctrl-c exit confirmation while armed, else the transient notice, else the shell-mode callout `shell mode · esc to exit` in the accent color (PRODUCT.md #7). Persistent content is computed from the input mode each render, so exiting shell mode reverts automatically (PRODUCT.md #12).

Transient notices are a reusable view-owned `TransientHint`: `show(text, ctx, projection)` displays the notice and spawns its 3s `Timer` expiry, aborting the superseded notice's timer — at most one live expiry exists and it always belongs to the current notice, so no generation guard is needed. This is the extensible transient-notice pattern (PRODUCT.md #21); future callers just invoke `show_transient_hint`.

### 5. Transcript block spacing and terminal-block background

Shell commands render as terminal blocks in the TUI transcript (PRODUCT.md #13); two rendering fixes make those blocks look right:

- **`BlockSpacing`** (`app/src/terminal/terminal_manager.rs`): the spacing baked into `TerminalModel` block heights — per-block padding, reserved Warp-prompt height, and the memory-stats footer row — was previously derived from GUI settings unconditionally inside `compute_block_size` / `create_terminal_model`. It is now a `BlockSpacing` struct passed by the frontend at model creation; settings-driven frontends (local/remote/mock/shared-session-viewer managers) use `BlockSpacing::for_gui`, and `create_tui_model` takes it as a parameter. The TUI passes `TRANSCRIPT_BLOCK_SPACING` (`crates/warp_tui/src/transcript_view.rs`): exactly `BLOCK_TOP_PADDING_ROWS` (1) blank row above each block, no Warp-prompt reserve, and no memory-stats row — the transcript renders whole rows, so the GUI's fractional pixel-derived padding would otherwise ceil into several blank rows per block.
- Agent blocks apply the same one-row top padding (`crates/warp_tui/src/agent_block.rs`) and no longer pad after their last section, so every adjacent block pair — terminal or agent — is separated by exactly one row.
- Terminal-block cells whose background is the theme's default are rendered with the background unset (`crates/warp_tui/src/terminal_block.rs`), so blocks inherit the TUI's own background instead of painting the theme's background color; explicitly-set cell backgrounds still paint.

## Testing and validation

Unit tests follow the repo convention (separate `_tests.rs` files included via `#[cfg(test)] #[path = ...]`), run with `cargo nextest run -p warp_tui`.

Mode-transition semantics (the `{AI, locked}` default sticking, `{Shell, locked}` writes applying, reactive events not rewriting the config) are covered by the app crate's `input_model` tests added in the stacked policy PR. The TUI-side tests construct the shared model via `BlocklistAIInputModel::mock` (`app/src/ai/blocklist/input_model.rs`, gated on `test-util` like `Appearance::mock`; it skips the production subscriptions, whose reactivity the TUI policy disables anyway) and cover the view and element behavior:

`crates/warp_tui/src/input/view_tests.rs` (extends the existing `App::test`-based suite):
- `!` at the buffer start enters shell mode without inserting; `!` elsewhere inserts literally; `ExitShellMode` is a no-op outside shell mode.
- Enter emits `Submitted` without clearing; `clear()` empties buffer and resets scroll (#20's retain-on-reject depends on this split).
- Esc is never consumed by the element (the shell-mode exit is the keymap binding gated on the shell-mode context flag), and the flag is present in the keymap context exactly while in shell mode (#11, #23).
- Shell-mode gutter geometry (via the composed `shell_element` row): the rendered cursor shifts right by 2 (#2), mouse mapping measures from the editor's slot after the gutter with gutter presses/releases consumed by the affordance's click handler (#8), wrapping happens two columns earlier (#8), and a gutter click places the cursor without arming a drag selection.

`crates/warp_tui/src/transient_hint_tests.rs`: show/supersede display semantics; expiry timing and superseded-timer aborts ride on the framework's `SpawnedFutureHandle::abort`.

`crates/warp_tui/src/agent_block_tests.rs`: updated to pin the one-row top padding block layout (§5).

Session-view routing (shell submit → `ExecuteCommand` with `CommandExecutionSource::User`, conversation cancellation, blocked-PTY transient hint) is not unit-testable for the same constructibility reason (`TuiTerminalSessionView` needs a full `TerminalSurfaceInit`); it is validated manually.

Manual validation (run the TUI binary from `crates/warp_tui/src/bin` against a real session):
- Visuals against PRODUCT.md #5-7 and the GUI: blue `!`, blue border, `shell mode · esc to exit` callout; colors change with theme (#24).
- Entering/exiting: `!` on empty and non-empty input (#1-3), backspace/esc preserving text (#10-12), `!` mid-text literal (#4).
- End-to-end: `!ls` renders a live-streaming terminal block in the transcript and the input resets to agent mode (#13-15); run a shell command mid-agent-stream and verify the conversation cancels (#19); run one while an agent tool command occupies the PTY and verify the transient message (#20-21).
- `./script/presubmit` before the PR.

## Parallelization

Not proposed: the work is a single tightly-coupled change centered on two files in one crate (`warp_tui`) plus a two-line export change in `app`. The input-view editing changes, session-view routing, and hint line all depend on the same shell-mode state and submit-flow refactor, so splitting them across agents would serialize on merge conflicts rather than save wall-clock time.

## Risks and mitigations

- **Submit-flow refactor**: moving buffer-clearing from `TuiInputView::submit` to the session view changes the agent path too. Covered by the input-view test suite (submit keeps the buffer; `clear()` resets it).
- **TerminalModel locking**: the PTY-availability check adds a `model.lock()` call site in the submit path; keep the lock scope to the availability read only (no nested calls), per the repo's locking guidance.
- **`BlocklistAIInputModel` default config**: handled by the stacked policy PR — `TuiInputModePolicy::initial_config` is `{AI, locked}` and reactive GUI transitions are disabled, so `is_shell_mode` cannot be true at startup or flip spontaneously.
