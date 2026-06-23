# TECH: Route the warp-tui front-end through a real shared TerminalModel (PR1)

## Status and audience
This is an implementation spec for a single PR on branch `harry/share-terminal-model` (parent `kevin/prototype-tui`). It is written to be executed exactly as written by an implementing agent. Where a decision is locked, do not deviate. Where something is genuinely ambiguous or the code does not match this spec, STOP and ask the orchestrator before guessing (see "Execution protocol").

## Goal
Replace the TUI prototype's raw `$SHELL -c` subprocess command path with a real, persistent, Warp-bootstrapped `TerminalModel` session, and render that model in the TUI. After this PR, a `!`-prefixed submission runs as a real terminal block in a shared `TerminalModel`, output renders from the model's grid, the user can interact with running/interactive programs (keystroke passthrough), and full-screen programs (alt-screen, e.g. `vim`/`top`) render. No agent, conversation, streaming, or tool calling is added in this PR.

## Locked decisions
1. **Command trigger:** a leading `!` runs the rest as a command through the `TerminalModel`. Plain (non-`!`) text stays a local transcript append exactly as today — it is intentionally reserved for the agent prompt in a later PR. Do not change plain-submit behavior.
2. **One persistent session:** the TUI owns exactly one bootstrapped shell session (one `TerminalModel` + one PTY) for the app's lifetime. cwd and env persist across commands. No per-command session, no multi-session.
3. **Rendering is block-list-based (GUI-shaped):** render the `TerminalModel`'s `BlockList` in order (concrete structure and owning model in Phase 4). This PR produces only command blocks; render by iterating the model's blocks so other block kinds (e.g. AI blocks) can interleave later, rather than keeping a parallel list of command entries.
4. **Alt-screen rendering:** when the model reports alt-screen active, render the alt-screen grid as a full-pane terminal instead of the block list.
5. **Keystroke passthrough:** route input by the model's `TerminalInputState`. When idle (`InputEditor`), the bottom input view composes the next `!command`. When a command is running (`LongRunningCommand`) or `AltScreen` is active, forward keystrokes to the PTY (the TUI behaves like a real terminal). When `NotBootstrapped`, drop input.
6. **GUI behavior-preserving:** the GUI terminal path must be unchanged in behavior. The session-core construction is factored into a shared sub-helper that the GUI orchestrator calls internally; the GUI keeps a single entry point.
7. **Server-backed PTY:** the TUI uses the same server-backed `PtySpawner` as the GUI (`app/src/lib.rs`). The `warp-tui` entry point dispatches worker subcommands before launching the TUI (`run_tui` → `dispatch_cli_command`), so the terminal server's re-exec of this binary runs the server rather than recursively launching TUIs.

## Out of scope (do NOT implement in this PR)
- Any agent cluster: `BlocklistAIController`, `BlocklistAIActionModel`, `BlocklistAIContextModel`, `BlocklistAIInputModel`, `ActiveSession`, `ShellCommandExecutor`, `AgentViewController`/`AgentViewHost`.
- Conversation streaming, tool calling, file edits, the diff-decision seam.
- The `build_agent_cluster` sub-helper (only `build_session_core` is built now).
- Multiple sessions/panes, split panes, shared sessions, SSH/remote.
- Removing or rewriting `command_output.rs`'s byte→grid path beyond what rendering requires.

## Current state (verified on this branch)
TUI base:
- `app/src/lib.rs:1117-1122`: Tui short-circuits before `initialize_app`, running only `crate::tui::init(ctx)`, so no app singletons are registered. `pty_spawner` is `None` (`:1009-1016`) and `callbacks` empty (`:1021-1028`) for Tui; the non-Tui closure (`:1124-1175`) registers singletons, then calls `initialize_app` and `launch`.
- `app/src/tui.rs`: `RootTuiView` (`TuiTranscriptView` + `TuiInputView`); `!`-submit → `transcript.run_command`, plain → `append`, Cancel → `cancel_running`.
- `app/src/tui/transcript_view.rs`: `run_command` runs `$SHELL -c` as a piped subprocess (the path being replaced). `app/src/tui/command_output.rs`: `render_output_to_buffer(bytes, width) -> TuiBuffer`.
- `app/src/tui/input_view.rs`: `TuiInputView` on `TuiEventHandler`; unconsumed/chorded keys propagate to ancestors.

Reusable APIs:
- `TerminalManager::create_model` (`app/src/terminal/local_tty/terminal_manager.rs`) builds the session core (`create_terminal_model` + PTY event loop + `init_pty_controller_model`) then the view + wiring. `terminal_manager_util.rs` holds `init_pty_controller_model` and `wire_up_pty_controller_with_view` (forwards to `PtyController::{write_bytes, write_command, resize_pty}`; `PtyDisconnected` → `model.exit`).
- `TerminalModel` (`app/src/terminal/model/terminal_model.rs`): `terminal_input_state()` `:1550`, `is_alt_screen_active()` `:1847`, `alt_screen()` `:1839`, `block_list()` `:1619`, `exit()` `:1455`, `resize()` `:1908`. Cell type `app/src/terminal/model/cell::{Cell, Flags}` (the GPU renderer `grid_renderer.rs` uses it — do NOT reuse it).

## Implementation

Work in four phases; each phase must compile (`cargo check -p warp --features tui`). Commit at the end of each phase (see Execution protocol).

### Phase 1 — Route the TUI through `initialize_app` (gated)
Goal: the TUI runs the same bootstrap as other modes so the session core's singletons exist, but skips the heavyweight terminal/GUI pieces.
- Remove the short-circuit at `app/src/lib.rs:1117-1122`. Let `LaunchMode::Tui` fall through into the main `app_builder.run(...)` closure (`lib.rs:1124-1175`).
- Keep `pty_spawner = None` for Tui (`lib.rs:1009-1016`) and empty `callbacks` for Tui (`lib.rs:1021-1028`) unchanged.
- In that closure, the `pty_spawner` singleton registration (`lib.rs:1143-1146`) currently `.expect(...)`s a `Some`. For Tui this is `None`; guard it so the singleton is registered only when `Some` (do not register a terminal-server pty spawner for Tui).
- In `initialize_app` (`lib.rs:1183+`), gate the heavyweight terminal-specific work to non-Tui: the default terminal / default `ActiveSession` creation and any GUI-workspace-only setup. Start by skipping only the default terminal/`ActiveSession` and rely on compilation + run to reveal anything else the session core genuinely needs; pull back only the specific manager required. Preserve the existing registration ORDER for everything else.
- After `initialize_app` returns, branch on Tui: for Tui call `crate::tui::init(ctx)` and return; for non-Tui call `launch(ctx, app_state, launch_mode)` as today. The existing `unreachable!("LaunchMode::Tui is handled before launch()")` in `launch` must be updated/removed consistently (Tui now returns before `launch`).
- Acceptance: `cargo run -p warp --features tui --bin warp-tui` starts, shows the existing TUI, and does not panic; the app singletons needed by the session core are present.

### Phase 2 — Extract the `build_session_core` sub-helper
Goal: one shared, view-free constructor for the session core, called by the GUI orchestrator internally and by the TUI directly.
- Add a function (location: a new module under `app/src/terminal/`, e.g. `app/src/terminal/session_core.rs`, or inside `local_tty/terminal_manager.rs` if cleaner) named `build_session_core` that performs the existing steps 1-4 of `TerminalManager::create_model`: create the channels, `Sessions`, `ModelEventDispatcher`, the `TerminalModel` (via the existing `create_terminal_model`), start the PTY + event loop, and `init_pty_controller_model`. It returns a struct, e.g.:
  ```rust
  pub(crate) struct SessionCore {
      pub model: Arc<FairMutex<TerminalModel>>,
      pub sessions: ModelHandle<Sessions>,
      pub model_events: ModelHandle<ModelEventDispatcher>,
      pub pty_controller: ModelHandle<PtyController>,
      // include any handles the existing create_model holds that the GUI still needs
      // (e.g. event loop join handle / inactive pty reads receiver) so create_model can keep them.
  }
  ```
- Refactor `TerminalManager::create_model` to call `build_session_core` for those steps, then continue to build the view, agent cluster, and `wire_up_pty_controller_with_view` exactly as before. The GUI's single entry point and behavior are unchanged.
- Do NOT build the agent cluster in `build_session_core`. That is a later PR.
- Acceptance: GUI builds and existing terminal tests pass; `build_session_core` is callable without a view.

### Phase 3 — TUI session ownership, command routing, input routing
Goal: the TUI owns one session core and drives it.
- Add a TUI-owned singleton (e.g. `TuiTerminalSession`) registered for Tui (under `#[cfg(feature = "tui")]`, in `initialize_app` or `tui::init`) that calls `build_session_core` once with a startup directory of the process cwd and the user's shell, and holds the resulting `SessionCore`. It exposes:
  - `model(&self) -> Arc<FairMutex<TerminalModel>>`
  - `run_command(&self, command: String, ctx)` → resolve the session's `ShellType`, then `pty_controller.write_command(&command, shell_type, source, ctx)` (mirror the GUI's `view::Event::ExecuteCommand` handler in `terminal_manager_util.rs`).
  - `write_input_bytes(&self, bytes: Vec<u8>, ctx)` → `pty_controller.write_bytes(bytes, ctx)`.
  - `resize(&self, cols, rows, ctx)` when the TUI area changes → `pty_controller.resize_pty(...)` and `model.lock().resize(...)`.
- Add minimal PTY lifecycle wiring for the TUI equivalent to the GUI's `wire_up_pty_controller_with_view`, but with no view: subscribe to `PtyControllerEvent::PtyDisconnected` and call `model.lock().exit(ExitReason::PtyDisconnected)`. Do NOT route `ShellCommandExecutor` events (no agent this PR).
- Update `app/src/tui.rs`: on `InputEvent::Submitted`, a `!`-prefixed text calls `TuiTerminalSession::run_command`; plain text keeps `transcript.append`. `InputEvent::Cancel` maps to sending the cancel/interrupt to the running command (write the interrupt byte, e.g. Ctrl-C `0x03`, via `write_input_bytes`) rather than killing a subprocess.
- Input routing by state: in `RootTuiView`, read `model.lock().terminal_input_state()` to decide where keys go:
  - `InputEditor` / `NotBootstrapped`: keep focusing `TuiInputView`; it composes the next `!command`.
  - `LongRunningCommand` / `AltScreen`: forward keystrokes to the PTY via `write_input_bytes`. Implement key→bytes encoding (see "Key encoding").
  - Implementation approach: add a root-level key interception (a `TuiEventHandler` wrapping the column, or a key fallback on the root) that, when the state is `LongRunningCommand`/`AltScreen`, consumes `Event::KeyDown` and forwards encoded bytes, returning `true` to stop propagation to the input view. When `InputEditor`, return `false` so the input view handles keys as today.
- Acceptance: `!ls` runs in the persistent shell and cwd persists across `!cd ...` then `!pwd`; typing into a running interactive program (e.g. `!python3` then expressions) reaches the program.

### Phase 4 — Rendering the model
Goal: render the model instead of the old captured-bytes entries. Mirror the GUI's prior art — `BlockListElement` (`app/src/terminal/block_list_element.rs`) renders the block list and `AltScreenElement` (`app/src/terminal/alt_screen/alt_screen_element.rs`) renders the alt-screen, both reading the model and its `Cell`/`Flags` grid via `grid_renderer.rs` — but paint with `TuiElement`s into a `TuiBuffer`. Do NOT reuse the GPU `grid_renderer.rs` painting; use it only as a read-loop reference for pulling cells from a `GridHandler`.

Owning model / data source: the single source of truth is the `TerminalModel` owned by `TuiTerminalSession` as `Arc<FairMutex<TerminalModel>>` (Phase 3). The render view reads it each frame; there is no separate render-state model. Read block data via `model.block_list().blocks() -> &Vec<Block>` (ordered; `block_at(BlockIndex)` also available). Per `Block`, the grids are `Block::prompt_and_command_grid() -> &BlockGrid` (prompt + command line) and `Block::output_grid() -> &BlockGrid` (output); each `BlockGrid` exposes `grid_handler() -> &GridHandler` (the styled cell grid) and `len_displayed() -> usize` (displayed row count). The cell type is `app/src/terminal/model/cell::{Cell, Flags}` (the same type `grid_renderer.rs` reads). Alt-screen grid: `model.alt_screen().grid_handler()`.

Build these, all under `app/src/tui/`:
- `grid_render.rs`: `fn cell_to_style(cell: &Cell) -> TuiStyle` (fg/bg color + bold/italic/underline/reverse from `Flags`); and `fn render_grid(grid: &GridHandler, area: TuiRect, buffer: &mut TuiBuffer)` iterating rows/cols, reading each `Cell`, writing a styled glyph into the buffer.
- `TuiBlockElement`: a `TuiElement` rendering one `Block` — its `prompt_and_command_grid` then its `output_grid` over `len_displayed()` rows — via `render_grid`.
- `TuiBlockListElement`: a `TuiElement` iterating `model.block_list().blocks()` in order, emitting one `TuiBlockElement` per block, laid out bottom-anchored (reuse `BottomAnchoredColumn`). This replaces the per-entry command list. Plain-text `append` prompts remain as their own entries interleaved in order — that ordering is the seam that lets AI blocks interleave later.
- Alt-screen: when `model.is_alt_screen_active()`, render `model.alt_screen().grid_handler()` full-pane via `render_grid` instead of the block list.
- Repaint: subscribe the TUI root (or transcript view) to the session's `ModelEventDispatcher` so streamed PTY output / block updates trigger `ctx.notify()`.
- Replace the raw-subprocess `run_command` in `transcript_view.rs`. Command output now comes from the model grid, so `command_output.rs`'s byte→grid path is no longer used for command rendering (leave it only if still referenced elsewhere).
- Acceptance: command output renders with ANSI styling from the model; `!vim`/`!top` renders full-screen and returns to the block list on exit.

## Key encoding
Use the GUI's existing encoder — do NOT write a new one. The canonical keystroke→PTY-bytes path is `KeystrokeWithDetails { keystroke, key_without_modifiers, chars }.to_escape_sequence(mode_provider) -> Option<Vec<u8>>`, defined in `crates/warp_terminal/src/model/escape_sequences.rs` and re-exported as `crate::terminal::model::escape_sequences::{KeystrokeWithDetails, ToEscapeSequence}`. This is exactly what `AltScreenElement` and `BlockListElement` call on `Event::KeyDown`. `TerminalModel` implements `ModeProvider` (`is_term_mode_set`), so pass `&*model.lock()` as the `mode_provider`; the encoder already handles APP_CURSOR/SS3 arrows, CSI-u/kitty, C0 control codes, backspace, and meta.
Routing per `Event::KeyDown { keystroke, chars, .. }` when state is `LongRunningCommand`/`AltScreen`:
1. Build `KeystrokeWithDetails { keystroke: &keystroke, key_without_modifiers: <from event details if present, else None>, chars: Some(chars.as_str()) }` and call `.to_escape_sequence(&*model.lock())`.
2. `Some(bytes)` → write `bytes` to the PTY via `write_input_bytes`.
3. `None` (plain printable input) → write `chars` as UTF-8 bytes.
For standalone modifier-key events, mirror the GUI's `maybe_kitty_keyboard_escape_sequence(&*model.lock(), key_code, is_press)` (same module, `kitty_keyboard_protocol`); it only emits under kitty report-all mode and otherwise returns `None` (skip).
One thing to verify while wiring this: the TUI runtime `Event::KeyDown`'s `keystroke` (and details) types must be `warp_terminal`'s `Keystroke` (the shared keymap type `KeystrokeWithDetails` expects). This is expected since the keymap type is shared across `warpui`/`warpui_core`/`warp_terminal`. If there is a genuine type mismatch, STOP and ask the orchestrator rather than writing a parallel encoder.

## Testing and validation
- Must compile: `cargo check -p warp --features tui`. Format: `./script/format` or `cargo fmt`.
- Unit tests (place per repo convention `${file}_tests.rs`):
  - `Cell`/`Flags` → `TuiStyle` mapping (colors + each modifier).
  - Input routing: given each `TerminalInputState`, keys route to the input view vs PTY correctly.
  - Block rendering: a model with known block content rasterizes to the expected `TuiBuffer` cells.
- Existing GUI terminal tests must still pass (Phase 2 is behavior-preserving). Per repo rules for delegated work, run `cargo check` and `cargo fmt`; do NOT run nextest/presubmit. Run only the new unit tests you add (`cargo test -p warp --features tui <test_name>`), since new tests should be verified.
- Manual (orchestrator/user will run): `cargo run -p warp --features tui --bin warp-tui`, then `!ls`, `!cd /tmp` + `!pwd` (cwd persists), `!python3` (interactive), `!vim` (alt-screen), Esc/Ctrl-C to interrupt.

## Risks / watch-items
- The in-process `local_tty` shell must bootstrap headlessly so blocks form (shell-integration DCS hooks). If bootstrap never completes (`terminal_input_state()` stays `NotBootstrapped`), STOP and report — this is the load-bearing assumption.
- `initialize_app` ordering is load-bearing; gating must not reorder the remaining registrations.
- Some GUI helper signatures may differ slightly from this spec's names; match by symbol and adapt, surfacing anything materially different.

## Execution protocol (for the implementing agent)
- Implement this spec exactly. Follow the repository's coding rules (imports at top of file, least possible visibility, concise doc comments on new functions, exhaustive matches, no unused `_`-prefixed params).
- If anything is ambiguous, contradictory with the code, or requires a design choice not covered here, STOP and message the orchestrator with the specific question and options. Do not guess on materially ambiguous points.
- Work directly in this branch and repo (`harry/share-terminal-model`). No separate worktree.
- Commit at logical points (one commit per phase is the expected granularity) using Graphite: `gt modify -a -m "..."` to amend or `gt create -m "..."` for a new commit on this branch, non-interactively. NEVER push (`gt submit`/`git push`) and never create a PR.
- Before handing back, ensure `cargo check -p warp --features tui` passes and `cargo fmt` is clean.
- When done, report back to the orchestrator: what was implemented per phase, any deviations or unresolved questions, and the command results for the final `cargo check`.
