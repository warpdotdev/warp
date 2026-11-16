# Prevent CLI Agent Frame Redraws from Accumulating on Resize
## Context
Claude Code and similar CLI agents run pseudo-TUI interfaces in the primary screen. When the terminal is resized narrower, the process receives a PTY resize event, redraws its frame, and commonly clears the screen before writing the new frame. In Warp, those old frames can become block output instead of being overwritten, so repeated resizes append multiple historical frames to the active block.
This does not appear to be caused by the CLI-agent trailing-blank-line trimming work in `specs/APP-4004/TECH.md` or `specs/APP-4006/TECH.md`. That work changes displayed height and cursor visibility for active CLI-agent blocks. It does not send resize events, change alt-screen state, or append PTY output. The accumulation mechanism is in primary-screen operations that preserve visible rows into scrollback, specifically full-screen clears and primary-screen resize reflow.
Relevant code:
- `app/src/terminal/view.rs:14654` — `TerminalView::resize_internal` updates the terminal model before the resize event is emitted to the PTY controller.
- `app/src/terminal/writeable_pty/terminal_manager_util.rs:69` — view resize events are forwarded to `PtyController::resize_pty`.
- `app/src/terminal/writeable_pty/pty_controller.rs:604` — `resize_pty` sends `Message::Resize(size_update.new_size)` when rows, columns, pane size, or refresh changes.
- `app/src/terminal/local_tty/event_loop.rs:173` and `app/src/terminal/local_tty/unix.rs:642` — the local PTY event loop handles `Message::Resize` by calling `ioctl(TIOCSWINSZ)`.
- `app/src/terminal/model/terminal_model.rs:1957` and `app/src/terminal/model/blocks.rs:1988` — the model and active block are reflowed on size changes.
- `app/src/terminal/model/grid/resize.rs:61` — primary-screen resize pushes visible rows into `flat_storage`, resizes `flat_storage`, then pulls rows back into visible grid storage.
- `app/src/terminal/model/grid/resize.rs:71` — alt-screen resize skips that reflow path and resizes the visible grid in place.
- `app/src/terminal/model/terminal_model.rs:2620` and `app/src/terminal/model/blocks.rs:3563` — ANSI `clear_screen` is delegated from the terminal model through the active block list.
- `app/src/terminal/model/grid/ansi_handler.rs:798` — `GridHandler::clear_screen` is where `ansi::ClearMode` is applied to the active grid.
- `app/src/terminal/model/grid/ansi_handler.rs:848` — `ClearMode::All` clears alt-screen grids in place, but primary-screen grids call `clear_viewport()`.
- `app/src/terminal/model/grid/ansi_handler.rs:1645` and `app/src/terminal/model/grid/ansi_handler.rs:1674` — `clear_viewport()` computes the visible rows to clear, then calls `scroll_region_up`, which pushes those rows into `flat_storage` before resetting the visible grid.
- `app/src/terminal/model/grid/grid_handler.rs:310` and `app/src/terminal/model/grid/grid_handler.rs:393` — `GridHandler` already owns display-only state toggled by higher-level block features.
- `app/src/terminal/model/blockgrid.rs:297`, `app/src/terminal/model/block.rs:1101`, and `app/src/terminal/view.rs:11778` — active CLI-agent state already propagates from `CLIAgentSessionsModelEvent::Started/Ended` into active-block output-grid behavior for trailing blank row trimming.
The target model is that CLI-agent frame redraws should treat primary-screen frame replacement operations as mutations of the live visible grid, not as scrollback-producing history. That includes primary-screen full erases and primary-screen resize reflow. Warp should not globally adopt this for all primary-screen blocks because Warp’s block model intentionally preserves `clear`-style shell output and ordinary command output in scrollback. The fix should be scoped to active CLI-agent frame redraws.
## Proposed changes
Add an explicit “primary-screen frame redraws happen in place” mode to the active CLI-agent output grid.
Implementation shape:
- Add a `FullGridClearBehavior` field to `GridHandler`, defaulting to `FullGridClearBehavior::Scroll` in `GridHandler::new`. `FullGridClearBehavior::Clear` gates both primary-screen full clears and primary-screen resize reflow for active CLI-agent grids.
- Add a one-way enable method on `GridHandler`, then thread it through `BlockGrid` and `Block`:
  - `GridHandler::enable_full_grid_clear_behavior()`
  - `BlockGrid::enable_full_grid_clear_behavior()`
  - `Block::enable_full_grid_clear_behavior()`
- In `GridHandler::clear_screen(ClearMode::All)`, clear in place when either the grid is alt-screen or `full_grid_clear_behavior` is `FullGridClearBehavior::Clear`. Otherwise keep the existing `clear_viewport()` path. This preserves existing primary-screen/block semantics outside the scoped CLI-agent case.
- Factor the in-place branch into a small helper so alt-screen and CLI-agent primary-screen clears share the same behavior. The helper should reset the active grid cells using the current cursor background template, mark the affected region dirty through the existing dirty-cell machinery, and evict visible image placements/secrets that no longer have backing cells if the current scroll-clear path does that indirectly.
- In `GridHandler::resize_storage`, treat unfinished primary-screen grids with `FullGridClearBehavior::Clear` like alt-screen grids for resize purposes: resize visible `GridStorage` in place instead of pushing visible rows into `flat_storage`, resizing `flat_storage`, and popping rows back. This prevents the current frame from becoming block scrollback before the CLI agent receives SIGWINCH and redraws.
- Enable `FullGridClearBehavior::Clear` in `TerminalView::handle_cli_agent_sessions_event` when a matching `CLIAgentSessionsModelEvent::Started` arrives. The behavior is one-way for that block; once the block is finished, resize ignores the clear behavior because the output is immutable.
- Do not reuse `FeatureFlag::TrimTrailingBlankLines` for this behavior. If rollback is desired, add a distinct feature flag; otherwise rely on the CLI-agent-only session scope. The two behaviors address related symptoms but have different terminal semantics.
Expected data flow:
1. Pane resize updates Warp’s model and sends a PTY resize to the running process.
2. Because the active block is marked as a CLI-agent frame-redraw grid, primary-screen resize updates visible grid storage in place instead of first pushing the old visible frame into `flat_storage`.
3. Claude Code receives the resize and emits a primary-screen full clear plus a new frame.
4. Because the same mode is enabled, `ClearMode::All` clears the current visible frame in place instead of pushing it into `flat_storage`.
5. Claude’s new frame is written into the same active grid area, so the block shows the latest frame rather than an accumulated stack of old frames.
The fix intentionally does not change:
- `ClearMode::Above`, `Below`, `Saved`, `ResetAndClear`, or `ActiveBlock`.
- Alt-screen behavior, which already clears full-screen frames in place.
- Non-agent primary-screen `ESC[2J` behavior, including shell prompt `clear`/Ctrl-L-style history preservation.
- Non-agent primary-screen resize reflow and history preservation.
- The trailing blank row trimming predicate or cursor-hiding behavior from APP-4004/APP-4006.
The important invariant is that visible rows should enter `flat_storage` only when they represent historical block output. During active CLI-agent frame redraws, the visible rows are the mutable frame surface; preserving them into `flat_storage` turns ephemeral frames into accumulated output.
## Testing and validation
Add targeted model tests first, then manually verify against Claude Code.
Recommended unit tests in `app/src/terminal/model/grid/grid_handler_test.rs`:
- Primary-screen `ClearMode::All` with the new flag disabled keeps existing behavior: visible rows are moved into `flat_storage` by `clear_viewport()`.
- Primary-screen `ClearMode::All` with the new flag enabled clears active rows in place: `history_size()` does not grow, old frame text is gone, and text written after the clear appears as the only current frame.
- Alt-screen `ClearMode::All` remains in-place and unchanged.
- Primary-screen resize with the new flag disabled keeps existing behavior: resize reflow may move visible rows into `flat_storage`.
- Primary-screen resize with the new flag enabled resizes visible storage in place: `history_size()` does not grow during resize.
- `ClearMode::Saved`, `ResetAndClear`, and `ActiveBlock` still clear history/state through their existing paths.
Recommended wiring tests:
- A started CLI-agent session marks the active block output grid for in-place frame redraw clears.
- A finished block no longer relies on reverting this behavior; finished-grid resize follows the normal scrollback path even if the behavior remains set.
Manual validation:
- Capture raw PTY output while resizing Claude Code, if possible, and confirm the redraw contains a full-screen clear such as `ESC[H ESC[2J`.
- Run Claude Code in Warp, resize the pane narrower several times, and verify the active block overwrites the current frame instead of accumulating prior frames.
- Repeat a normal shell `clear`/Ctrl-L flow outside a CLI-agent session and verify Warp still preserves the prior primary-screen contents according to existing block semantics.
- Resize ordinary primary-screen command output outside a CLI-agent session and verify existing reflow/scrollback behavior is unchanged.
- Smoke-test Codex/OpenCode CLI-agent sessions to ensure trailing blank trimming and cursor behavior from APP-4004/APP-4006 are unchanged.
Suggested commands:
- `cargo test -p app terminal::model::grid::grid_handler_test -- --nocapture` or the narrowest package/test invocation that matches this repo’s current test target names.
- If implementation touches broader terminal-model behavior, run the relevant terminal model tests before manual verification.
## Risks and mitigations
- Primary-screen full clear and resize reflow have user-visible history semantics in Warp. Mitigation: scope the in-place behavior only to active CLI-agent sessions, and only for `ClearMode::All` plus primary-screen resize while the frame-redraw mode is active.
- Some CLI agents might intentionally use primary-screen clears to preserve previous output. Mitigation: the behavior applies only while Warp has detected an active CLI-agent session, where full-screen clears are overwhelmingly frame redraws; completed blocks no longer use the in-place resize path.
- Images and secret metadata may outlive in-place-cleared cells if only grid cells are reset. Mitigation: reuse or extend existing clear/eviction helpers so ancillary grid state matches the cleared visible region.
- Delayed lifecycle events can target the wrong active block. Mitigation: only enable the behavior on session start and do not rely on a later disable event for correctness.
- Resize ordering may still produce edge cases if a process writes during the gap between model resize and PTY resize delivery. Mitigation: the scoped resize path prevents the main old-frame preservation mechanism; treat backend-before-model resizing as a follow-up only if manual verification still shows accumulation.
