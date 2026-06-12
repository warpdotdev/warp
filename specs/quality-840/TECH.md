# TECH: Route code review actions to the focused conversation's terminal

Ticket: QUALITY-840 · PR: #12524 · Branch: `harry/quality-840-add-to-context-diff-hunk-targets-bound-terminal-view-not`

Code references are pinned to commit `cc73265`.

## Context

The code review panel's "attach as context" actions write content into a `TerminalView` — a selection prompt, a `<change:...>` diff-hunk attachment, or a diff-set attachment. Before this change, `CodeReviewView` stored a `WeakViewHandle<TerminalView>` captured when the view was created and used it for every action. With agent orchestration, parent/child conversations swap panes in and out of the layout tree (`RevealChildAgent` / `SwapPaneToConversation` in `app/src/pane_group/pane/terminal_pane.rs`), so the captured handle could point at a sub-agent's (possibly hidden) terminal while the user was focused on the parent agent. Attach actions then landed in the wrong conversation's input.

Review *comment submission* already solved this: it resolves its target at send time via `RightPanelView::find_review_terminal` (focused terminal > repo-preferred terminal > any available terminal). This spec brings the attach actions (and every other terminal use in the panel) onto that same late-bound resolution, and removes the stored `TerminalView` reference from `CodeReviewView` entirely.

Affected surfaces, all in `app/src/code_review/code_review_view.rs`:

- Selection highlight → "Add as context" tooltip / Cmd-L: `insert_selection_as_context` (`code_review_view.rs:5725`). The tooltip itself needs a terminal *synchronously during render* — `LocalCodeEditorView` calls a `terminal_target_fn` closure both to decide tooltip visibility and to compute the file path relative to the terminal's cwd (`app/src/code/local_code_editor.rs:1786`).
- Gutter paperclip → attach diff hunk: `insert_diff_hunk_as_context` (`code_review_view.rs:6025`).
- Header / per-file "Add diff set as context": `insert_diff_as_context` (`code_review_view.rs:5822`).
- Non-attach uses of the old handle: `preferred_review_session` (session hint for remote `GetDiffState`), `session_env` (remote/WSL zero-state enablement), the "Initialize codebase" pwd check, and the `OpenRepository` / `InitProjectForCurrentDirectory` actions.

Prior art, all in `app/src/workspace/view/right_panel.rs`:

- `route_review_comments` (`right_panel.rs:1314`) — comment submission's routing entry point.
- `find_review_terminal` (`right_panel.rs:1639`) → `find_available_terminal_for_review` (`right_panel.rs:1604`) — focused > repo-preferred > any available, gated by `review_terminal_status` (`right_panel.rs:1399`) (cwd inside repo; for non-CLI-agent terminals also AI enabled, not executing, input box visible).
- `PaneGroup::visible_terminal_views` excludes hidden child-agent panes and `focused_session_view` reflects swapped-in conversations, so focused-first resolution always lands on the conversation the user is actually viewing.

## Proposed changes

### `ReviewActionTargetProvider` trait

A provider trait defined next to the consumer, `code_review_view.rs (591-611)`:

- `attach_terminal(repo_path, app)` — the terminal that should receive attach-as-context payloads.
- `focused_terminal(app)` — the focused (or active) terminal of the hosting pane group, regardless of repo, for environment checks and terminal-targeted actions.

`CodeReviewView` stores `Option<Box<dyn ReviewActionTargetProvider>>` (replacing the `WeakViewHandle<TerminalView>` field) and exposes two private helpers, `attach_target_terminal` / `focused_terminal` (`code_review_view.rs (2877-2890)`). All three attach paths, both selection-tooltip closures (`code_review_view.rs:3099`, `:3192`), and the non-attach uses resolve through these helpers at action/render time. `set_terminal_view` and the old getter are deleted, and the plumbing that threaded a terminal handle through `open_code_review`, `create_code_review_view`, and `CodeReviewPaneContext` is removed.

### `RightPanelView` implementation

`RightPanelReviewActionTargetProvider` (`right_panel.rs (121-177)`) holds a `WeakViewHandle<RightPanelView>` and is injected in `create_code_review_view`:

- `attach_terminal`: calls the shared `find_review_terminal` (identical to comment routing). Only if that returns `None` does it fall back to the focused terminal, and only when `review_terminal_status` shows no repo-scoping failures (`NoSelectedRepo` / `SessionPathUnavailable` / `SessionOutsideSelectedRepo`) — i.e. the focused terminal is in the repo but merely busy.
- `focused_terminal`: `pane_group.focused_session_view()` falling back to `active_session_view()`.

Per-destination behavior inside each attach method (CLI agent → PTY/rich input, long-running fallback, input-box insert + attachment registration + agent-view entry, telemetry destinations) is unchanged; only the terminal it operates on is resolved differently.

### Parity with comment routing

Verified against the code at `cc73265`: attach actions call the *same* `find_review_terminal` used by `route_review_comments`, so for every case where comment submission finds a terminal, attach actions target the identical terminal. The two paths diverge only when `find_review_terminal` returns `None` (no available terminal anywhere):

- Comments: `route_review_comments` reports `ReviewSubmissionResult::Error` (error toast) — there is no degraded way to deliver a comment batch.
- Attach: falls back to the focused in-repo terminal so the actions' pre-existing degraded modes still work and still target the focused conversation — the "Cannot attach context when terminal is running" toast (`insert_selection_as_context`) and inserting the path into a running command via `handle_file_tree_drop_on_active_command` (`insert_diff_hunk_as_context`).

This is consistency, not divergence: the fallback covers states comment submission treats as outright failure, and preserves attach behavior that predates this change.

### Design decisions

- **Synchronous provider rather than event bubbling.** Comment submission bubbles an event (`SubmitReviewComments`) to `RightPanelView`, which resolves the terminal and performs the send. That transport cannot cover the selection tooltip: `LocalCodeEditorView` resolves the terminal during render (`&AppContext`, no event emission possible) to decide tooltip visibility and compute the cwd-relative path. Since a synchronous resolver is required for that case anyway, all call sites use it — one resolution mechanism, so the tooltip, the relative path, and the eventual insert always agree on the same terminal. Splitting transports (events for actions, resolver for the tooltip) would allow them to drift.
- **Trait rather than boxed closures or a direct `RightPanelView` handle.** A trait defined in `code_review` inverts the dependency (the panel declares the capability; the host implements it), matching the established provider pattern in this exact view (`ShowCommentEditorProvider`, `ShowFindReferencesCardProvider`). A direct `WeakViewHandle<RightPanelView>` would couple the panel upward to its host; a pair of boxed closures (the `TerminalTargetFn` style) loses the shared documented contract and can be wired inconsistently.
- **Resolution against the right panel's `active_pane_group`.** Same scoping as comment routing. A cached `CodeReviewView` is only interactable while its pane group is active, so this cannot resolve against the wrong tab.
- **Behavior deltas on non-attach surfaces.** `preferred_review_session`, `session_env`, the init-project pwd check, and `OpenRepository`/`InitProject` now act on the focused terminal rather than the panel-opening terminal. These are at-interaction-time concerns; focused is the more correct target, and `preferred_review_session` is only a dispatch hint (`None` falls back to any connected session).

## Testing and validation

- `cargo check -p warp` and `cargo check -p warp --tests` pass; `./script/format` and `cargo clippy -p warp --all-targets -- -D warnings` pass.
- `cargo test -p warp --lib code_review`: 115 passed, 0 failed — includes `code_review_view_tests.rs` and `find_model_tests.rs`, which construct `CodeReviewView` with the new provider parameter (`None`).
- The selection-priority logic is shared, unchanged code (`find_available_terminal_for_review`), already exercised by comment-routing usage; no new unit tests were added because the new code is glue requiring a live pane group + focus state.
- Manual verification (the scenario from the bug report): open an orchestrator with a sub-agent, focus the parent conversation, and confirm each action lands in the parent's input — gutter paperclip, highlight + "Add as context", header "Add diff set as context", per-file diff attach. Repeat after swapping to the sub-agent's conversation to confirm it targets the sub-agent. Also confirm the selection tooltip hides when no in-repo terminal can be resolved.

## Parallelization

Not beneficial: the change is a single-PR refactor across three tightly coupled files (trait definition, consumer migration, host implementation) where every step depends on the previous one's types. It was implemented sequentially on one branch.

## Follow-ups

- The code pane (`app/src/code/view.rs`) has its own `with_selection_as_context` target function with window-scoped resolution; it was out of scope here but could adopt the same focused-first selection if similar mistargeting is reported outside the review panel.
