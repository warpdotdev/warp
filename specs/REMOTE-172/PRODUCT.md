# REMOTE-172: Requested-command-style setup UI for Cloud Mode environments
## Summary
Cloud Mode should show environment setup as part of the agent conversation instead of making startup commands feel like unrelated terminal activity. When a cloud agent run starts, the user should immediately see their submitted prompt, clear environment startup progress, and collapsible rows for any setup commands that run before the first agent response.
The desired outcome is a Cloud Mode startup experience that is readable, debuggable, and consistent with the requested-command UX used elsewhere in Agent Mode.
## Problem
Cloud Mode environments can execute startup commands before the agent sends its first response. Without a dedicated UI, those commands can obscure the user's original request, leak command text into the prompt/input area, generate unrelated passive suggestions, or look like ordinary terminal blocks rather than setup work performed for the cloud agent.
Users need to understand that Warp is preparing the environment, what commands ran, whether those commands succeeded, and how to inspect details when setup fails or looks slow.
## Goals
- Preserve the initial user request in the conversation while the cloud agent environment starts.
- Show startup progress and setup commands in a visually cohesive Agent Mode surface.
- Make each environment startup command inspectable without overwhelming the default view.
- Clearly communicate running, succeeded, failed, cancelled, and authentication-required states.
- Avoid treating environment setup commands as normal user-entered terminal commands.
- Avoid leaking remote setup-command input state into the local visible prompt.
- Keep the old Cloud Mode setup UI available when the rollout flag is disabled.
## Non-goals
- Changing how cloud environments are created or which startup commands run.
- Allowing users to edit, approve, reject, or re-run environment startup commands from this UI.
- Replacing the existing requested-command UX for agent-requested commands.
- Changing cloud agent scheduling, task dispatch, capacity limits, or billing behavior.
- Providing a Figma-perfect redesign of all Cloud Mode loading and details-panel surfaces.
## Figma / design references
Figma: none provided.
Reference issue: https://linear.app/warpdotdev/issue/REMOTE-172/use-requested-command-ux-for-create-environment-like-init
## User experience
### Entry and initial prompt
- When the user starts a Cloud Mode run with an initial prompt, the cloud agent view should show that prompt as a user query block as soon as the run is dispatched.
- The initial prompt block should use the same user-query visual treatment as Agent Mode: user avatar/display name, bold prompt text, and normal Agent Mode spacing.
- If the cloud agent is cancelled before producing an exchange, the prompt block should show a simple `Cancelled` state.
- If the cloud agent requires GitHub authentication before producing an exchange, the prompt block should show a simple `Auth required` state.
- If the cloud agent fails before producing an exchange, the prompt block should show a simple `Failed` state.
- The first real AI exchange should not duplicate the same initial prompt/header when an optimistic initial prompt block was already inserted for the live startup flow.
- Historical replay should remain faithful to persisted conversation data and must not suppress the first AI block query solely because the new live startup UI exists.
### Startup progress
- While Warp is waiting for the cloud agent session to become ready, the status surface should show a shimmering progress message.
- Progress text should map to the known startup phase:
  - `Connecting to Host (Step 1/3)` before the task is claimed.
  - `Creating Environment (Step 2/3)` after the task is claimed but before the harness starts.
  - `Starting Environment (Step 3/3)` after the harness starts but before the shared session is ready.
- After the shared session is ready but before the first agent exchange arrives, the status surface should show `Setting up environment`.
- Cloud Mode tips may appear below these progress messages when available.
- In the new setup flow, progress should be shown in the Agent Mode status surface instead of covering the tab with the legacy full-screen loading screen.
### Startup command grouping
- Startup commands that run before the first cloud-agent exchange should be grouped under a single setup summary row.
- The summary row should say `Running startup commands...` while the cloud agent is still before the first exchange.
- The summary row should say `Ran startup commands` once the cloud agent has moved past the pre-first-exchange setup phase.
- The summary row should include a chevron that communicates whether startup command rows are expanded or collapsed.
- Clicking the summary row should toggle the visibility of startup command rows.
- Startup command rows should default to expanded during setup so users can see what is happening.
- Once the first agent exchange is appended, startup command rows should automatically collapse.
- Users can expand the summary row again after collapse to inspect the commands.
### Individual startup command rows
- Each startup command should render as a compact requested-command-style row before the corresponding terminal command block.
- A running startup command should show a running/progress icon.
- A completed startup command with a successful exit code should show a success/check icon.
- A completed startup command with a non-success exit code should show a failure icon.
- When collapsed, the row should display the command text in a monospace style.
- When expanded, the row should display `Viewing command detail` and reveal the underlying terminal command block/output.
- Clicking a startup command row should toggle only that command's detail visibility.
- Startup command output should be hidden by default behind the row, but must remain available when the user expands the row.
- Startup command rows should update their status when the backing command completes.
### Terminal blocks and input state
- Environment startup command blocks should not appear as ordinary user terminal blocks by default.
- Startup command blocks should be marked as setup commands so they can be hidden, shown, and excluded from unrelated terminal features.
- The visible prompt/input should not be populated by remote setup-command text while the cloud agent is in the pre-first-exchange setup phase.
- Remote input mode changes caused by environment setup should not force the visible local input mode to change while the cloud agent is in the pre-first-exchange setup phase.
- Passive prompt/code suggestions should not be generated for environment startup command completion.
- Cloud Mode sessions should generally suppress passive suggestions for agent conversation output so startup and cloud-agent internals do not create irrelevant suggestions.
### Parent terminal entry block
- When Cloud Mode is started from an existing terminal session, the parent terminal should receive a cloud-agent entry block only after the agent run is dispatched.
- Entering Cloud Mode and exiting without sending a prompt should not leave a persistent `New cloud agent` entry block behind.
- The entry block should show the cloud conversation title when available and fall back to `New cloud agent`.
- The entry block should show a status detail when relevant:
  - `Starting environment...` while waiting for the session.
  - `Agent is working on task` once the agent is running.
  - `Agent failed` on failure.
  - `Authentication required` when GitHub auth is required.
  - `Cancelled` when cancelled.
- The entry block icon should reflect the run state: cloud icon for normal/running, clock/loading for startup, warning for failure, info for auth, and cancelled icon for cancellation.
- Clicking the entry block should navigate back into the Cloud Mode view.
### Completion, failure, auth, and cancellation
- When the cloud agent session becomes ready, Warp should join the shared session and continue showing setup status until the first exchange arrives.
- When the agent produces the first exchange, the startup-command summary should switch from running to completed language and command details should collapse.
- If startup fails before the session is ready, the UI should preserve the initial prompt and show the failure state.
- If GitHub authentication is required, the UI should preserve the initial prompt and show the auth-required state.
- If the user cancels the run before the first exchange, the UI should preserve the initial prompt and show the cancelled state.
- Cloud-agent details UI should still auto-open when task/session data is available, both for local Cloud Mode and shared ambient sessions.
### Shared ambient sessions and ended conversations
- Joining an existing cloud-agent shared session should mark the terminal as an ambient-agent session and use cloud-agent-specific behavior.
- For shared ambient sessions, ended conversations should show a conversation-ended tombstone once, including under the new setup flow.
- Historical conversation replay must not prematurely insert the ended tombstone.
- Ending a shared ambient session should preserve the shareable object needed for the share dialog when appropriate.
- Session-ended internal-server-error toasts that are specific to normal shared-session recovery should not be shown for Cloud Mode when they do not apply.
### Rollout behavior
- The behavior in this spec is gated by `CloudModeSetupV2`.
- When the flag is disabled, legacy Cloud Mode loading behavior should continue to work.
- When the flag is enabled, the new setup UI should replace the legacy full-screen waiting overlay for startup progress.
## Edge cases
1. **No startup commands run**: the initial prompt and progress/status behavior should still work; no empty startup-command section should appear.
2. **Multiple startup commands run**: all commands before the first exchange should be represented under the same summary row and should maintain independent expanded/collapsed detail state.
3. **Startup command fails**: the failed command row should show a failure icon and remain inspectable; subsequent cloud-agent failure handling should still show the failed state for the run if the run cannot continue.
4. **Command completes while collapsed**: the row status should update from running to success/failure even if details are hidden.
5. **First exchange arrives while commands are expanded**: the setup section should collapse automatically, but the user should be able to re-expand it.
6. **User starts Cloud Mode but sends no prompt**: no parent entry block should be left in the terminal.
7. **User re-enters a cloud agent from the parent terminal**: clicking the entry block should return to the correct cloud agent view.
8. **Nested Cloud Mode session with no dispatched query**: starting another Cloud Mode session should be ignored to avoid empty nested sessions.
9. **Nested Cloud Mode session after a query starts**: starting another Cloud Mode session should create a sibling from the parent terminal context rather than nesting indefinitely.
10. **Historical replay**: replayed conversation data should not hide the first persisted AI block query/header merely because the new live startup UI can render an optimistic prompt.
11. **Shared session replay**: conversation-ended tombstones should not appear during replay before task/conversation liveness is known.
12. **Remote input sync during setup**: setup-command text and input mode changes from the shared session should not overwrite the user's visible local prompt state.
13. **Capacity, quota, or server overload errors**: existing modal/error handling should continue to appear, while the startup UI preserves the initial prompt context.
14. **GitHub auth required**: the run should show an auth-required state rather than looking like an ordinary failure.
15. **Cancelled before task ID arrives**: cancellation should show immediately and should still cancel the server task if the task ID arrives later.
## Success criteria
- Starting a Cloud Mode run immediately renders the submitted prompt in the conversation.
- While the cloud environment starts, users see a clear progress message with the correct phase label.
- Startup commands appear as requested-command-style rows, not as ordinary visible terminal blocks.
- Startup command rows show running, success, and failure states correctly.
- Users can expand a startup command to inspect its terminal details and collapse it again.
- Startup command rows collapse automatically when the first agent exchange arrives.
- The first AI exchange does not duplicate the initial user query in the live startup flow.
- Historical replay still shows the persisted conversation without losing the first query.
- The visible input is not overwritten by environment setup commands.
- Passive suggestions are not generated for environment setup commands.
- Parent terminal entry blocks are created only for dispatched cloud-agent runs and navigate back into the correct cloud-agent view.
- Failure, GitHub auth, and cancellation states remain visible and understandable from the initial prompt context.
- Shared ambient sessions get the appropriate ended-conversation tombstone exactly once and not during replay.
- Disabling `CloudModeSetupV2` restores the legacy loading behavior.
## Validation
- Start a Cloud Mode run against an environment with no startup commands and confirm the initial prompt and progress/status surfaces render without an empty setup-command section.
- Start a Cloud Mode run against an environment with multiple successful startup commands and confirm rows appear expanded during setup, show success after completion, and collapse after the first exchange.
- Start a Cloud Mode run with a failing startup command and confirm the failed row is inspectable and the run failure state is visible if setup cannot proceed.
- While a run is before the first exchange, verify remote setup-command input does not populate the visible prompt and does not flip the visible input mode.
- Verify prompt/passive suggestions are not emitted for startup command blocks.
- Click the setup summary and individual command rows to confirm expand/collapse behavior.
- Start Cloud Mode from a parent terminal, send a prompt, return to the parent, and confirm the entry block shows the correct title/status and navigates back.
- Enter Cloud Mode from a parent terminal and exit without sending a prompt; confirm no parent entry block remains.
- Test failure, cancellation, and GitHub-auth-required startup paths and confirm the initial prompt remains visible with the correct status text.
- Join or replay a shared ambient session and confirm conversation-ended tombstones appear only after the task/conversation is no longer running and not during replay.
- Run with `CloudModeSetupV2` disabled and confirm legacy Cloud Mode setup behavior remains available.
