*Proposed change: Improve TUI orchestration tab-bar keybindings*

*Summary:* When the orchestration tab bar owns keyboard focus, make the plain Down arrow return focus to the session input (while retaining Shift+Down as an alias), and make Escape return to the main/orchestrator agent and focus its input.
*Key design choices:* Keep input-focus and Escape bindings on the terminal-session surface, where input ownership and cross-session focus are available; reuse the existing semantic orchestration-session switch path instead of adding raw key handling to the tab-bar view; keep Escape out of the footer hint so the footer only advertises the requested Down affordance.
*Design alternatives:*
- Register the new bindings in `register_orchestration_surface_bindings` — rejected because that helper is shared by terminal sessions and cloud-run surfaces, while cloud-run views do not own a prompt input or the terminal-session focus transition.
- Handle Escape/Down inside `TuiTabBarView` — rejected because the retained tab bar only owns presentation and selection events; session switching and input focus belong to `TuiTerminalSessionView`.
- Remove Shift+Down and replace it with Down — rejected because the ticket explicitly requires Shift+Down to remain a working alias for existing users.

*Root cause / approach:* At `d02e147360b9183444928c6037e11a462c0f98b1`, `TuiTerminalSessionView::init` registers `tui:orchestration_tabs:focus_input` only for `shift-down`, and `render_orchestration_tab_footer` advertises `Shift + ↓`. The terminal-session action `FocusDefaultInteractionTarget` already clears tab focus and focuses the default input owner via `set_orchestration_tab_focus(false, ctx)`. Add a second `down` binding to that action while retaining `shift-down`. Add a terminal-session-scoped editable Escape binding/action that obtains the tab bar's configured `main_tab` key, calls the existing `switch_to_orchestration_tab`/`switch_to_orchestration_conversation` path with `keep_tab_focus = false`, and therefore switches to the root session when a child is selected or simply returns focus to the input when the root is already selected. Expose the main-tab key through the retained tab-bar API rather than duplicating tab-bar configuration in the session view. Update the terminal orchestration footer from `Shift + ↓` to `↓`; do not add an Escape hint.

*Affected files:*
- `crates/warp_tui/src/terminal_session_view.rs` — terminal-session bindings, Escape action, and main-session focus transition.
- `crates/warp_tui/src/tab_bar.rs` — read-only accessor for the configured main-tab key.
- `crates/warp_tui/src/orchestration_tab_bar.rs` — Down footer copy; existing shared navigation bindings remain unchanged.
- `crates/warp_tui/src/terminal_session_view_tests.rs` (and/or focused TUI keybinding tests) — binding, focus, session-switch, and footer regression coverage.

*Open questions resolved:*
- Down focus behavior is limited to the focused orchestration-tab context and must not change ordinary input-editor Down behavior.
- Shift+Down remains an equivalent focus-input binding.
- Escape always targets the configured root/main tab; if that tab is already selected, no session switch is required, but input focus must still be restored.
- Escape has no footer hint.

*Risks / blast radius:* Keymap precedence could cause Down to be shadowed by an input-editor binding if the orchestration context is not scoped correctly; assert binding context and exercise the live key path. Escape could accidentally keep child-tab focus or switch the wrong retained session; test both a selected child and an already-selected root. Shared left/right/Tab/Shift+left/Shift+right navigation and cloud-run tab behavior must remain unchanged.

*Validation & verification criteria* (must ALL pass before merge):
1. Add a regression test that initializes the TUI bindings and, in a context containing `TuiTerminalSessionView` plus `ORCHESTRATION_TAB_BAR_FOCUSED_FLAG`, finds the `tui:orchestration_tabs:focus_input` bindings for both `down` and `shift-down`; verify neither binding matches a normal input context without the tab-focus flag.
2. Add a regression test for the Escape binding/action in a fixture with a root session and at least one child session: with a child selected and the tab bar focused, dispatch Escape and verify the focused session becomes the root/main session, `orchestration_tabs_focused` is false, and the root input view owns focus.
3. Cover the root-selected edge case: dispatch Escape while the main/root tab is already selected and verify the selected session remains unchanged while tab focus is cleared and the input view receives focus.
4. Render the terminal orchestration footer in a focused-tab state and assert it advertises `↓` for sending a message, does not contain `Shift + ↓`, and contains no Escape/`esc` hint. Existing left/right/Tab and Shift+left/Shift+right navigation bindings continue to be present and scoped to the tab-focused context.
5. Run the focused unit suite and ensure it passes, including the new regression tests: `cargo test -p warp_tui`.
6. Run the repository presubmit without errors: `./script/presubmit`.
7. Because this changes a user-facing TUI surface, exercise the live orchestration tab flow with computer-use verification: capture visual proof while the tab bar is focused showing the `↓` footer hint, press Down and confirm the input receives focus without changing the selected agent, press Escape from a child tab and confirm the main agent/input is focused, and repeat Escape with the main tab selected to confirm it only focuses input. Attach the resulting screenshot proof to the task/PR.
