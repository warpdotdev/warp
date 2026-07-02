# TUI Shell Command Execution — Tech Spec

Implements the behavior in [`PRODUCT.md`](./PRODUCT.md). References are pinned to commit `51145bb70dc2e461d1152880e8f173dce28ac165`.

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

### 0. TUI framework impact: no new elements or events needed

Everything ships with existing `warpui_core::elements::tui` primitives; the framework itself does not change:
- **Styled text**: `TuiText::with_style(TuiStyle)` applies one style to a whole run ([`crates/warpui_core/src/elements/tui/text.rs:45 @ 51145bb7`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warpui_core/src/elements/tui/text.rs#L45)). The blue `!` glyph and the single-color hint line each render as their own `TuiText`; no rich-text/multi-span element is required.
- **No `TuiRow` exists** for horizontal composition, but none is needed: `TuiInputElement` is already a hand-rolled element that lays out rows, paints selection via `TuiBuffer::set_style`, and reports the cursor cell, so the 2-column gutter is an area offset inside its existing `layout`/`render`, with the affordance rendered as a `TuiText` into a 2×1 sub-rect.
- **Border color**: the input border already uses `TuiContainer::with_border_style` ([container.rs:120](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warpui_core/src/elements/tui/container.rs#L120)); shell mode just swaps the style's fg. Theme-token → `Color` conversion (`CoreFill::from(ThemeFill::from(...)).into()`) already exists in the session view's render.
- **Esc key**: the runtime already converts `KeyCode::Esc` to keystroke key `"escape"` ([`crates/warpui_core/src/runtime/event_conversion.rs:146`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warpui_core/src/runtime/event_conversion.rs#L146)); the input key table just gains an arm for it. No new `TuiEvent` variants.
- **New surface area is crate-local to `warp_tui`**: one `TuiInputAction` variant (`ExitShellMode`), the hint-line view, and the transient-hint timer (a spawned delayed task via the existing view-context spawn APIs, as `ctx.spawn_stream_local` is already used in the session view).

### 1. Export shared input-mode types (`app/src/tui_export.rs`)

Add `InputConfig`, `InputType`, and `InputTypeAutoDetectionSource` (from `app/src/ai/blocklist/input_model.rs`) plus `CancellationReason` (from `app/src/ai/agent/mod.rs`) to [`app/src/tui_export.rs`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/tui_export.rs#L27-L30). `CommandExecutionSource` is already exported (line 37).

### 2. Shell-mode state and editing (`crates/warp_tui/src/input/view.rs`)

Shell mode lives on the shared `BlocklistAIInputModel` (per GUI): `TuiInputView` gains a `ModelHandle<BlocklistAIInputModel>` (passed in from `TuiTerminalSessionView`) and a `fn is_shell_mode(&self, ctx) -> bool` reading `is_input_type_locked() && !input_type().is_ai()`. The session view sets the model's config to `InputConfig { input_type: InputType::AI, is_locked: true }` at construction so the TUI default is explicit (the model's GUI-oriented default is shell/autodetect).

Action handling changes:
- `InsertChar('!')` with the cursor at offset start and not already in shell mode: set `InputConfig { Shell, locked: true }` (source `InputTypeAutoDetectionSource::ShellPrefix`) instead of inserting the char. Anywhere else, `!` inserts literally. Detection at the `InsertChar` level is inherently typed-only — `TuiEvent` has no paste variant ([`crates/warpui_core/src/elements/tui/event.rs:24`](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warpui_core/src/elements/tui/event.rs#L24)), so PRODUCT.md #3 falls out for free; if bracketed paste is later routed through `InsertChar`, a pasted leading `!` entering shell mode is acceptable per product decision.
- `Backspace` with the cursor at the very start while in shell mode: exit shell mode (reset config to AI-locked) instead of deleting; text preserved (PRODUCT.md #10).
- New `TuiInputAction::ExitShellMode` bound to plain `esc` in the key table; no-op when not in shell mode so esc remains free for future use (PRODUCT.md #11).
- `Submit` no longer clears the buffer; it only emits `Submitted`. A new `pub fn clear(&mut self, ctx)` clears buffer + scroll, and the session view calls it on every accepted submission (agent prompt or executed shell command). This is what lets a blocked shell submission retain its text (PRODUCT.md #20).

Rendering changes (`TuiInputElement`):
- The element captures `shell_mode: bool` at render time. When set, `layout` lays the editor out at `terminal_width - 2` (the width pushed onto the char-cell render state), and `render`/`cursor_position`/`offset_at` inset all rows by 2 columns; the `! ` affordance is drawn at the origin of the first visible row styled with `theme.ansi_fg_blue()`. Because the affordance is outside the buffer, selection/select-all/cursor math need no special-casing (PRODUCT.md #2, #8) — only the 2-column x-offset in `offset_at` ([view.rs (791-817)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/crates/warp_tui/src/input/view.rs#L791-L817)) and `cursor_position` must account for it.

### 3. Submit routing and execution (`crates/warp_tui/src/terminal_session_view.rs`)

The `Submitted` handler branches on `is_shell_mode`:
- Agent path: unchanged (`send_prompt`), then `input_view.clear()`.
- Shell path (`fn execute_user_command`):
  1. Whitespace-only text → no-op, stay in shell mode (PRODUCT.md #16-17).
  2. PTY-availability check mirroring `can_execute_command`: lock the `TerminalModel` and reject when the active block `is_active_and_long_running()` and not in-band. On reject: keep the input text, show the transient hint `cannot run — command already running` (PRODUCT.md #20). Keep the lock scope minimal per the terminal-model locking guidance.
  3. Otherwise emit `TuiTerminalSessionEvent::ExecuteCommand(ExecuteCommandEvent { command, session_id, source: CommandExecutionSource::User, should_add_command_to_history: true, .. })` — the same PTY-intent bridge agent commands use, so the transcript terminal-block rendering comes for free (PRODUCT.md #13-14).
  4. Cancel any in-progress conversation: `ai_controller.cancel_conversation_progress(id, CancellationReason::UserCommandExecuted, ctx)` for the selected conversation when `status().is_in_progress()` (PRODUCT.md #19), mirroring [input.rs (13392-13406)](https://github.com/warpdotdev/warp/blob/51145bb70dc2e461d1152880e8f173dce28ac165/app/src/terminal/input.rs#L13392-L13406).
  5. `input_view.clear()` and reset `InputConfig` to AI-locked (PRODUCT.md #15).

Border color: `render` picks `ansi_fg_blue()` when `is_shell_mode`, else the current cyan (PRODUCT.md #5).

### 4. Hint line with transient messages (new `crates/warp_tui/src/input_hint_line.rs`)

A small view/model owned by `TuiTerminalSessionView`, rendered as a one-row `TuiText` below the input box in the session view's column (the Figma `← for conversations` slot):
- `enum HintContent { None, ShellMode }` for persistent content — `ShellMode` renders `shell mode · esc to exit` in `ansi_fg_blue()` (PRODUCT.md #7). Persistent content is computed from current state each render, so exiting shell mode reverts automatically (PRODUCT.md #12).
- `fn show_transient(&mut self, text: String, ctx)` displays `text` for 3s, then reverts and notifies, via a spawned timer task that clears the message (guarded by a generation counter so overlapping transients don't clear each other early). This is the extensible transient-notice pattern (PRODUCT.md #21); future callers just invoke `show_transient`.

## Testing and validation

Unit tests follow the repo convention (separate `_tests.rs` files included via `#[cfg(test)] #[path = ...]`), run with `cargo nextest run -p warp_tui`.

`crates/warp_tui/src/input/view_tests.rs` (extends the existing `App::test`-based suite):
- `!` on empty input and before existing text enters shell mode without inserting the char (PRODUCT.md #1-2); `!` mid-text inserts literally (#4).
- Backspace at text start exits shell mode preserving text (#10); esc exits preserving text (#11); backspace/esc when not in shell mode behave as today (#23).
- Enter emits `Submitted` without clearing; `clear()` empties buffer and resets scroll.
- In shell mode: wrapping/cursor/mouse `offset_at` account for the 2-column inset; select-all covers only command text (#2, #8).

`crates/warp_tui/src/terminal_session_view` tests (new `terminal_session_view_tests.rs`, patterned on `conversation_selection_tests.rs`):
- Shell-mode submit emits `ExecuteCommand` with `CommandExecutionSource::User` and clears + resets to agent mode (#13-15); agent-mode submit still routes to the conversation (#23).
- Whitespace-only shell submit is a no-op (#16-17).
- Submit with an in-progress conversation cancels it with `CancellationReason::UserCommandExecuted` (#19).
- Submit while the active block is long-running does not emit `ExecuteCommand`, retains input text, and sets the transient hint; the hint clears after the timeout (#20-21).

Manual validation (run the TUI binary from `crates/warp_tui/src/bin` against a real session):
- Visuals against PRODUCT.md #5-7 and the GUI: blue `!`, blue border, `shell mode · esc to exit` callout; colors change with theme (#24).
- End-to-end: `!ls` renders a live-streaming terminal block in the transcript (#13); run a shell command mid-agent-stream and verify the conversation cancels (#19); run one while an agent tool command occupies the PTY and verify the transient message (#20).
- `./script/presubmit` before the PR.

## Parallelization

Not proposed: the work is a single tightly-coupled change centered on two files in one crate (`warp_tui`) plus a two-line export change in `app`. The input-view editing changes, session-view routing, and hint line all depend on the same shell-mode state and submit-flow refactor, so splitting them across agents would serialize on merge conflicts rather than save wall-clock time.

## Risks and mitigations

- **Submit-flow refactor**: moving buffer-clearing from `TuiInputView::submit` to the session view changes the agent path too. Covered by the routing unit tests and the existing input-view test suite.
- **TerminalModel locking**: the PTY-availability check adds a `model.lock()` call site in the submit path; keep the lock scope to the availability read only (no nested calls), per the repo's locking guidance.
- **`BlocklistAIInputModel` default config**: the model defaults to GUI semantics (shell/autodetect). The session view must set AI-locked at construction or `is_shell_mode` would be true on startup; the startup-state unit test covers this.
