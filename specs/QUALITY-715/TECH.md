# QUALITY-715: Do not auto-open details panel for orchestration child shared sessions
# Context
Linear issue: https://linear.app/warpdotdev/issue/QUALITY-715/dont-open-agent-info-side-panel-by-default. The issue has no additional description or comments; the required behavior is that opening a shared session child agent from the parent's orchestration UI should not show the conversation details side panel by default. Regular shared session viewers, including direct links to a child shared session, should keep the current default and open the panel.
The relevant implementation is in the Warp client worktree at `/Users/matthew/src/dont-open-agent-info-sidepane/warp` on branch `matthew/dont-open-agent-info-sidepane`. No `warp-server` or `warp-proto-apis` changes are expected for the preferred client-side fix.
The side panel is `ConversationDetailsPanel`, owned by `TerminalView`. `TerminalView` tracks `is_conversation_details_panel_open` and `has_auto_opened_conversation_details_panel` in `app/src/terminal/view.rs:2830`. The panel renders only when `is_conversation_details_panel_open` is true and `can_show_conversation_details_ui_from_model` says details are available (`app/src/terminal/view.rs:26677`). The toggle action updates the same boolean and fetches panel data (`app/src/terminal/view.rs:26095`), so the feature should only change initial auto-open behavior, not remove the ability to open the panel manually.
Shared ambient agent session viewers currently auto-open the panel from `TerminalView::on_session_share_joined` after the viewer joins an ambient-agent shared session (`app/src/terminal/view/shared_session/view_impl.rs:687`). That method calls `maybe_auto_open_conversation_details_panel` for every `SessionSourceType::AmbientAgent` when `FeatureFlag::CloudMode` is enabled (`app/src/terminal/view/shared_session/view_impl.rs:708`). `maybe_auto_open_conversation_details_panel` unconditionally sets `is_conversation_details_panel_open = true` the first time it runs (`app/src/terminal/view/ambient_agent/view_impl.rs:960`).
One shared-session child path is driven by `OrchestrationViewerModel` and `PaneGroup`. `OrchestrationViewerModel::apply_children_fetch` polls `GET /agent/runs?ancestor_run_id={parent_task_id}`, creates a local child `AIConversation`, links it to the parent, marks it as `is_viewing_shared_session`, and emits `EnsureSharedSessionViewerChildPane` when a child `session_id` becomes available (`app/src/terminal/shared_session/viewer/orchestration_viewer_model.rs:228`). `PaneGroup::ensure_shared_session_viewer_child_pane` creates a hidden shared-session viewer for that child session, restores the child conversation, and enters agent view with `AgentViewEntryOrigin::SharedSessionSelection` (`app/src/pane_group/mod.rs:3428`). The child viewer then reaches the same `on_session_share_joined` auto-open path as any other ambient shared session (`app/src/terminal/shared_session/viewer/terminal_manager.rs:799`), which is why the panel currently opens by default for child views.
Not all child agent views originate from `OrchestrationViewerModel`. Local parent agents create child conversations through `StartAgentExecutor`, which emits `Event::StartAgentConversation` (`app/src/ai/blocklist/action_model/execute/start_agent.rs:303`), handled by `dispatch_start_agent_conversation` in `app/src/pane_group/pane/terminal_pane.rs:1470`. The local child path uses `create_hidden_child_agent_conversation` in `app/src/pane_group/child_agent.rs:131` for fresh StartAgent children, and `PaneGroup::create_hidden_child_agent_pane` for restored local, remote, and viewer-side placeholder children (`app/src/pane_group/mod.rs:3216`). Navigation to these children still happens through `RevealChildAgent`, `SwapPaneToConversation`, `OpenChildAgentInNewPane`, and `OpenChildAgentInNewTab` after `ensure_hidden_child_agent_pane_for_conversation` materializes the child pane if needed (`app/src/pane_group/pane/terminal_pane.rs:1425`, `app/src/pane_group/mod.rs:3076`).
The important distinction is opening context, not task shape. A child task can be opened directly from its own shared-session link, in which case it should behave like a standalone ambient shared-session viewer and keep the details panel open by default. The panel should only be suppressed when the child view is entered as an auxiliary child pane owned by the parent's orchestration UI, such as a pill-bar click, child status-card reveal, or split-off from the parent orchestration viewer.
# Requirements
1. When the user opens an orchestration child agent from the parent's orchestration UI in a shared-session viewer, the child viewer should start with the conversation details panel closed, regardless of whether that child was materialized by `OrchestrationViewerModel` or by the local parent-agent child pane path.
2. The panel toggle must remain available for that child viewer whenever details are available, so the user can still open the panel manually.
3. Direct links to a child agent's shared session should keep the current default and open the conversation details panel.
4. Existing default behavior must remain unchanged for regular non-child ambient shared session viewers.
5. Existing default behavior must remain unchanged for local cloud-mode runs and non-shared ambient agent views.
6. The fix should be client-only because the behavior depends on client-side navigation context.
# Design options
## Option A: Explicit per-view auto-open policy set by parent orchestration UI paths
Add a `ConversationDetailsPanelAutoOpenPolicy` enum to `TerminalView`, defaulting to the current behavior. Parent-owned child pane creation/reveal paths set the policy to suppress the initial details-panel auto-open before the child viewer joins or enters its ambient session. Direct shared-session links never set the policy, even if the target task is a child.
Tradeoffs:
- Correctly distinguishes direct child links from parent-context child navigation.
- Does not rely on `parent_conversation_id`, `parent_run_id`, or `is_viewing_shared_session`, which describe what the conversation is rather than how the user opened it.
- Slightly more stateful than deriving from metadata.
- Requires setting the policy at every parent-owned child pane materialization path that can reach the ambient auto-open code.
## Option B: Derive suppression from shared-session child metadata at auto-open time
Add a helper on `TerminalView` that checks the active conversation in `BlocklistAIHistoryModel` and the terminal model's shared-session state. Return false from the auto-open path when the active conversation has `parent_conversation_id().is_some()` and the view is a shared-session viewer.
Tradeoffs:
- Minimal state and no persistence/schema changes.
- Covers multiple child creation paths because it does not depend on the creation site.
- Incorrectly suppresses direct child shared-session links, because those links also represent child tasks in shared-session viewers.
- Not recommended unless product decides every child shared-session surface should suppress the panel, including direct links.
## Option C: Change `on_session_share_joined` to inspect server task parent metadata
Use the `AmbientAgentTask.parent_run_id` field (`app/src/ai/ambient_agents/task.rs:234`) to suppress auto-open when the joined task is known to be a child.
Tradeoffs:
- Could cover direct links to child shared sessions if product later decides direct links should suppress the panel.
- More asynchronous and likely requires adding a fetch-before-auto-open path, because `on_session_share_joined` only receives `SessionSourceType::AmbientAgent { task_id }`.
- Risks delaying or flickering the panel for regular viewers while task metadata loads.
- Not recommended for QUALITY-715 because direct child links should keep opening the panel.
# Proposed changes
Implement Option A. Suppression should be an explicit property of the `TerminalView`'s opening context.
1. In `app/src/terminal/view.rs`, add `ConversationDetailsPanelAutoOpenPolicy` with variants `DefaultOpen` and `DefaultClosed`. Store it as a private `conversation_details_panel_auto_open_policy` field on `TerminalView`, initialized to `DefaultOpen` in `TerminalView::new`.
2. Add a method on `TerminalView`, for example `suppress_initial_conversation_details_panel_auto_open(&mut self)`, that sets the policy to `DefaultClosed` before the join-time auto-open can run. This method should not close the panel if the user has already opened it manually; it only affects future calls to `maybe_auto_open_conversation_details_panel`.
3. Update `maybe_auto_open_conversation_details_panel` in `app/src/terminal/view/ambient_agent/view_impl.rs` to preserve existing one-shot behavior and consult the policy:
   - if `has_auto_opened_conversation_details_panel` is already true, return;
   - always set `has_auto_opened_conversation_details_panel = true` before applying the policy;
   - if the policy is `DefaultClosed`, return without setting `is_conversation_details_panel_open` and without calling `fetch_and_update_conversation_details_panel`;
   - otherwise keep the current behavior: set `is_conversation_details_panel_open = true`, set `has_auto_opened_conversation_details_panel = true`, fetch details, and notify.
4. Set the suppress policy in parent-owned child pane creation paths:
   - `PaneGroup::ensure_shared_session_viewer_child_pane`, before or in the same `new_terminal_view.update` closure that restores the child conversation and enters agent view (`app/src/pane_group/mod.rs:3428`).
   - Do not rely on `PaneGroup::create_hidden_child_agent_pane`'s `child_conversation.is_viewing_shared_session()` placeholder branch as the primary fix. That branch creates a loading placeholder when the user clicks before `OrchestrationViewerModel` has a `session_id`; the placeholder is discarded and replaced by `ensure_shared_session_viewer_child_pane` when the real child shared-session viewer becomes joinable.
   - Set the policy at the hidden ambient-agent child pane creation helper `PaneGroup::insert_ambient_agent_pane_hidden_for_child_agent`. This single suppression site covers both restored remote child panes (`PaneGroup::create_hidden_child_agent_pane` remote-child branch, which delegates to this helper) and freshly-spawned remote children created by the local-orchestrator `StartAgentExecutionMode::Remote` path (`launch_remote_child` in `app/src/pane_group/pane/terminal_pane.rs`). The view is suppressed before the environment setup loading screen and any subsequent `AmbientAgentViewModelEvent::SessionReady`/`FollowupSessionReady` event can run.
   - Do not set the policy in ordinary fresh local child creation (`create_hidden_child_agent_conversation`) unless implementation identifies a concrete local-parent shared-session viewer or Cloud Agent child path that reaches `maybe_auto_open_conversation_details_panel`. The current fresh local child pane path creates a local child agent pane, not a shared-session viewer or remote Cloud Agent child, so setting the policy there would be unnecessary and risks changing non-shared behavior.
5. Do not set the policy when opening a shared session directly from a child session link. Direct links enter through the normal shared-session viewer creation path (`create_shared_session_viewer` and `on_session_share_joined`) without a parent orchestration owner, so they should keep the default auto-open behavior.
6. Do not suppress based only on `parent_conversation_id`, `AmbientAgentTask.parent_run_id`, or `conversation.is_viewing_shared_session()`. Those are task/conversation properties and cannot distinguish direct child links from parent orchestration UI navigation.
7. Do not change `TerminalAction::ToggleConversationDetailsPanel`. Manual toggles should continue to set `is_conversation_details_panel_open`, fetch data, and render the side panel for child viewers.
8. Do not change `on_session_share_joined`'s ambient-agent check for regular viewers. The default auto-open behavior for direct child links and non-child ambient shared sessions remains driven by the existing `FeatureFlag::CloudMode` and `SessionSourceType::AmbientAgent` condition.
9. Do not add a server or proto field for this fix.
# Testing and validation
Add unit coverage in the Warp client.
1. In `app/src/terminal/view/shared_session/view_impl_tests.rs` or a nearby `TerminalView` test module, add a test that sets the new suppress policy, calls `maybe_auto_open_conversation_details_panel`, and asserts:
   - `is_conversation_details_panel_open` remains false,
   - `has_auto_opened_conversation_details_panel` becomes true,
   - calling `TerminalAction::ToggleConversationDetailsPanel` still opens the panel when details are available.
2. Add a direct-link regression test: create a shared ambient viewer for a child conversation/task without setting the new suppress policy, call `maybe_auto_open_conversation_details_panel`, and assert the panel opens by default. This test is the guard against deriving suppression from child metadata.
3. Add a regression test for the `OrchestrationViewerModel` path: create or restore a child conversation marked `is_viewing_shared_session`, run `ensure_shared_session_viewer_child_pane` or the smallest test-visible equivalent, and assert the resulting child `TerminalView` uses `DefaultClosed` and does not auto-open the panel.
4. Add a regression test or manual validation case where a suppressed parent-owned child pane already exists, then the same child is opened through its own direct shared-session link. The direct-link `TerminalView` should still open the details panel by default, proving the suppression state is per-view and not tied globally to the child conversation/task.
5. If implementation adds suppression to any additional local-parent shared-session viewer path, add targeted coverage for that exact path. Do not add broad tests that assume ordinary local child panes should suppress auto-open.
6. Add or update a regular shared ambient viewer test in `app/src/terminal/view/shared_session/view_impl_tests.rs` that proves a non-child ambient shared session still auto-opens the panel by default.
7. Ensure manual-toggle assertions create a state where conversation details are actually available, otherwise a toggle may set `is_conversation_details_panel_open` without rendering useful panel content.
8. Run focused tests first:
   - `cargo test -p warp-app terminal::view::shared_session::view_impl_tests`
   - `cargo test -p warp-app pane_group::mod_tests`
9. Run the relevant broader client validation required for a Warp client PR after focused tests pass. If local runtime makes a full presubmit impractical, run the repo-standard Rust formatting/check/test commands that cover the touched modules and document any skipped command with the reason.
Manual validation:
1. Start or use an orchestrated cloud-agent shared session that has at least one child agent with a `session_id`.
2. Open the parent shared session in the Warp desktop viewer.
3. Confirm the parent/non-child ambient session still opens with the conversation details panel by default.
4. Select a child from the orchestration pill UI for a server-discovered child and confirm the child view opens with the main transcript visible and the conversation details panel closed.
5. Repeat with a local-parent child path if available, such as a local parent agent that creates a child through StartAgent/local orchestration, and confirm its shared-session child view also opens with the panel closed.
6. Open the same child through its own shared-session link and confirm the conversation details panel opens by default.
7. Click the pane-header conversation details toggle in the parent-context child view and confirm the panel opens and shows the child agent's metadata.
8. Switch back to the parent and another regular shared session to confirm their default behavior was not regressed.
# Parallelization
Parallel child agents are not recommended for implementation. The change is small, tightly coupled to `TerminalView` state, shared-session join behavior, and the child-pane materialization path. Splitting implementation and tests across agents would add merge overhead without meaningful wall-clock savings.
If the work grows to include direct child-session links, then split the work into two sequential phases rather than parallel branches: first land the known orchestration viewer fix, then investigate whether task metadata is available early enough to suppress auto-open for standalone child URLs.
# Risks and mitigations
- Risk: suppressing auto-open also prevents manual access to the panel. Mitigation: only gate `maybe_auto_open_conversation_details_panel`; do not change render availability or `ToggleConversationDetailsPanel`.
- Risk: setting `has_auto_opened_conversation_details_panel = true` when suppressing could block a later desired automatic open. Mitigation: this is intentional only for parent-context child views because the requirement is that they do not default open; direct links and regular viewers keep the `DefaultOpen` policy.
- Risk: missing a parent-owned child creation path would leave the panel open. Mitigation: cover `ensure_shared_session_viewer_child_pane`, and only add broader local-parent coverage if a concrete local-parent shared-session viewer path is found to reach the auto-open code.
- Risk: over-broad suppression would change direct child links. Mitigation: never derive suppression from child metadata alone; direct-link tests must assert the panel still opens by default.
# PR notes
Create the PR from branch `matthew/dont-open-agent-info-sidepane` in the Warp client repo. The PR should reference QUALITY-715, describe the client-only change, and include the focused test results plus manual validation. If no documentation changes are needed, state that the behavior is an internal default-state adjustment with no user-facing docs impact.
