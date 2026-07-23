*Proposed change: Prefix the TUI agent input bar with `>`*

*Summary:* The live `TuiInputView` always renders a two-cell prompt gutter followed by the editable `TuiEditorElement`. Agent input uses a plain cyan `>`; shell input uses its existing accent-styled `!` (`crates/warp_tui/src/input/view.rs:447-477`). The prompt is outside the editor buffer and does not affect submitted text, selection offsets, or shell-mode detection.

*Rendering contract:*
- `TuiInputView::render` owns one shared row composition. It conditionally selects only the prompt glyph and style: `>` with `accent_text_style()` for agent input, or `!` with `shell_mode_accent_style()` for shell input (`crates/warp_tui/src/input/view.rs:452-458`).
- The prompt occupies one cell followed by one right-padding cell. The editor remains a flex child and receives the remaining width (`crates/warp_tui/src/input/view.rs:459-477`).
- The gutter uses the persistent `prefix_mouse_state`. Clicking it dispatches `SetCursor { offset: 1 }`, placing the cursor at the start without starting a drag selection (`crates/warp_tui/src/input/view.rs:459-470`).
- Agent input always shows `>`, including for an empty buffer. Shell input shows only `!`; the two prompts are never composed together.
- Wrapped and continuation rows remain aligned with the editor at the two-column offset. The prompt appears only on the first visual row.
- The live input keeps its existing background, border, spacing, and editor styling. The agent prompt uses `accent_text_style()` without the bold or tinted submitted-input treatment.

*Affected files:* `crates/warp_tui/src/input/view.rs` contains the shared render path. `crates/warp_tui/src/input/view_tests.rs` adds focused full-view coverage while existing editor behavior tests continue rendering the editor directly. No submitted-input renderer, GUI code, server code, persistence, style token, or feature flag changes.

*Risks:* The gutter narrows the editor by two columns and shifts full-view cursor and mouse coordinates. Editor-only behavior remains owned by `TuiEditorElement`; full-view tests cover only the composition boundary.

*Validation:*
1. `agent_mode_render_has_prompt_gutter` renders the full agent input row and verifies the `> ` gutter, plain accent styling, cursor offset, and gutter-narrowed wrapping (`crates/warp_tui/src/input/view_tests.rs:313-344`).
2. Existing shell tests render through the same `TuiInputView::render` path and verify the `!` prompt, cursor offset, mouse mapping, gutter click consumption, and wrapping.
3. `cargo nextest run -p warp_tui --lib input` passes.
4. A live `./script/run-tui` check confirms agent and shell prompts render in the shared gutter without other input-bar changes.
