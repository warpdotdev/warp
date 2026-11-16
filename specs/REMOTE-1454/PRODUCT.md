# REMOTE-1454: Cloud Mode setup UI for non-oz harnesses
## Summary
`CloudModeSetupV2` currently works end-to-end only for the Oz harness. For non-oz harnesses (claude, gemini, any future third-party harness) the same UI surfaces, but because the viewer has no Oz `AppendedExchange` to transition out of the setup phase, the harness command itself (e.g. `claude --session-id … < /tmp/oz_prompt`) is permanently classified as an environment setup command.
This spec defines the Cloud Mode setup UX for non-oz harnesses so the experience feels consistent with Oz: the user's prompt is clearly preserved as a queued user query, real environment startup commands are grouped under a collapsible setup summary, and the harness CLI itself renders as a normal long-running CLI-agent session.
## Problem
For non-oz cloud runs today (with `CloudModeSetupV2` enabled), the run is dispatched from Warp and the remote sandbox runs `oz agent run --harness=claude …`, which in turn launches `claude …` as a shell command in the shared session. The viewer sees:
- The user's prompt shown at the top of the agent conversation as `CloudModeInitialUserQuery` (like an Oz query).
- Environment setup commands hidden behind the setup-commands summary ("Running setup commands…").
- The `claude …` block also hidden behind the setup-commands summary, because `is_executing_oz_environment_startup_commands` only flips off when an Oz exchange is appended and no such exchange arrives for claude-code runs.
- The CLI agent detection (Warp's `CLIAgent::Claude`) fires but the block remains flagged as a setup command, so the familiar CLI-agent UI (native TUI, CLI-agent footer, rich input) is not surfaced correctly.
The net effect: the run looks visually like Oz setup forever and the harness's real work is buried under a "setup" row.
## Goals
- Preserve the user's submitted prompt while the run is still before the harness command starts, but communicate that it is waiting to be picked up rather than already being answered by an agent.
- Group real environment startup commands (before the harness runs) under the collapsible setup-commands summary, consistent with Oz.
- Let the harness command block (the one that runs `claude …` / `gemini …` / etc.) render as a normal long-running CLI-agent session, not as a setup command.
- Keep failure, cancel, and GitHub-auth UX understandable for runs that fail before the harness even starts.
- Keep the Oz harness flow from REMOTE-172 unchanged.
## Non-goals
- Changing how the remote sandbox launches the harness command, or how the external conversation transcript is uploaded.
- Changing the shared-ambient-session UX for late joiners or historical replay beyond "don't show a pending query block."
- Exposing a new surface to edit, approve, reject, or re-run the harness command.
- Redesigning the CLI-agent session rendering itself; we rely on the existing CLI-agent UX once the harness block is running.
## Figma / design references
Figma: none provided.
Reference issue: https://linear.app/warpdotdev/issue/REMOTE-1454/fix-cloud-mode-setup-v2-ui-for-other-harnesses
## Gating
- Applies when all of the following are true:
  - `CloudModeSetupV2` is enabled.
  - `AgentHarness` is enabled (this is the multi-harness feature flag that gates third-party harness selection in the cloud-mode harness selector, the harness flag on `oz agent run`, and the CLI-agent conversation restoration paths).
  - The run's selected harness is not Oz (today: `claude`, `gemini`; applies to any future third-party harness).
- For Oz runs the behavior from REMOTE-172 is unchanged. When `AgentHarness` is disabled, third-party harnesses aren't selectable in the first place, so this path cannot be reached.
- The viewer resolves the run's harness from `AmbientAgentTask.agent_config_snapshot.harness` (see `app/src/ai/ambient_agents/task.rs`), and/or from the locally selected harness in `AmbientAgentViewModel` for the spawner.
## User experience
### Entry and initial prompt
- When the user starts a non-oz Cloud Mode run with an initial prompt, the cloud agent view does NOT insert a top-of-conversation `CloudModeInitialUserQuery` block.
- Instead, the view inserts a pending/queued user-query indicator in the conversation, reusing the existing queued prompt visual treatment (`PendingUserQueryBlock` style), showing:
  - The user's avatar / display name.
  - The prompt text.
  - A "Queued" status.
- The queued indicator does NOT offer a "Send now" button (there is no active conversation to interrupt, and the prompt is already being carried to the harness via the sandbox, not re-submitted as an Oz prompt) and does NOT offer a close / dismiss ("Remove queued prompt") button: the queued state is owned by the cloud run's lifecycle, not by the user, and dismissing it locally would orphan the prompt from the pending-run context.
- The queued indicator is removed only by the run itself transitioning out of the pre-harness phase (harness start, failure, cancel, or auth required).
- The run's selected harness can be surfaced in the queued copy (e.g. "Queued — waiting for Claude Code to start") but copy is not critical for this spec.
### Startup progress and setup commands
- Startup progress messages ("Connecting to Host", "Creating Environment", "Starting Environment", "Setting up environment") behave the same as in Oz setup-v2.
- Environment startup commands executed before the harness command is launched appear grouped under the collapsible setup-commands summary, with the same running/success/failure affordances and expand/collapse behavior described in REMOTE-172.
- The summary row copy stays "Running setup commands…" while still before the harness command runs.
- For non-oz runs, the setup-commands summary row and per-command rows use the same horizontal padding as a regular terminal command block, so that when the harness CLI block takes over there is no horizontal shift between the setup UI and the harness's own terminal content. Oz runs continue to use the agent-output indent from REMOTE-172 (unchanged).
### Harness-start transition
- The viewer considers the harness command "started" when a block begins executing whose command is detected as the harness's CLI, i.e. `detect_cli_agent_from_model` returns a `CLIAgent` variant matching the run's harness (e.g. `CLIAgent::Claude` for the `claude` harness, `CLIAgent::Gemini` for `gemini`, and analogous for future harnesses).
- On that transition:
  - The pending user-query indicator is removed from the conversation.
  - New blocks are no longer classified as Oz environment setup commands.
  - The setup-commands summary auto-collapses to "Ran setup commands", remains expandable by the user, and its previously-inserted per-command rows remain accessible.
  - The viewer forces a fresh terminal-size report to the sharer so the harness CLI (e.g. claude's native TUI) lays out using the viewer's current dimensions rather than whatever size the sandbox PTY happened to be during environment setup. Without this, the harness TUI can start at a stale size and misrender until the user manually nudges the pane.
- The harness command block itself is NOT classified as a setup command. It appears as a normal long-running CLI-agent session: native CLI TUI visible in the block, `CLIAgentSession` set for the view, and the CLI-agent footer / rich input behave as they do when a user runs `claude` in any other Warp terminal.
- Subsequent blocks (anything the harness runs, any follow-ups, etc.) also render normally — not under the setup summary.
### Failure / cancellation / GitHub auth (pre-harness)
- If the run fails, is cancelled, or requires GitHub auth before the harness command starts:
  - The pending user-query indicator is removed.
  - The existing non-setup-v2 error, cancelled, or auth-required UI is shown (same copy and affordances used today for pre-first-exchange failures in Oz runs).
- If any of these states occur after the harness command has already started, behavior is whatever the existing CLI-agent / ambient-agent flow does today (out of scope for this spec).
### Input handling during setup
- Just like Oz setup-v2, remote setup-command input from the shared session must not overwrite the visible local input, and remote input-mode changes caused by environment setup must not flip the visible input mode (see existing handling in `app/src/terminal/shared_session/viewer/terminal_manager.rs`).
- Submitting input in the local Warp prompt while the run is still pre-harness is disallowed (same as `should_block_cloud_mode_setup_submission` logic today); once the harness command is running, input routes to the harness TUI as usual.
### Shared ambient-session viewers and historical replay
- Late joiners who were not the original spawner, and users replaying a completed non-oz cloud run, do NOT see a pending user-query indicator; there is no live prompt to queue.
- Setup commands that already ran before the join still render under a collapsed setup-commands summary.
- The harness command block renders as a normal CLI-agent session block.
- Conversation-ended tombstones and the parent-terminal entry block behavior are unchanged from REMOTE-172.
### Parent terminal entry block
- Same as REMOTE-172. Copy and iconography for running/failed/auth/cancelled states remain meaningful for non-oz runs (e.g. "Agent is working on task" applies while the harness CLI is running).
## Edge cases
1. **No environment startup commands before harness**: the pending user-query indicator still appears, no setup-commands summary is inserted, and the indicator is removed when the harness block starts.
2. **Setup command fails before harness starts**: the failed command's row shows a failure icon and remains inspectable; the pending user-query indicator stays visible until the run transitions to a terminal state, at which point it is removed along with the fallback error UI.
3. **User cancels before the harness block starts**: pending indicator is removed, existing cancelled UI is shown.
4. **GitHub auth required before the harness block starts**: pending indicator is removed, existing auth-required UI is shown.
5. **Harness command is detected by `CLIAgent::detect` but is not the run's configured harness** (rare / misconfiguration): for safety, the transition should still fire on the first detected third-party CLI-agent block so we never get stuck in "queued" state forever.
6. **Run is spawned with a non-oz harness but no harness metadata is visible to the client yet** (e.g. `AgentConfigSnapshot.harness` hasn't been fetched): the spawner knows its own selected harness via `AmbientAgentViewModel`; the initial queued indicator uses that. For viewers that don't have harness metadata, they should behave like an Oz viewer until the harness becomes known — acceptable because viewers don't render the pending indicator anyway.
7. **Historical replay of a non-oz run**: no pending indicator, setup commands already collapsed, harness block shows as a CLI-agent session.
8. **Nested Cloud Mode sessions / empty composing sessions / sibling sessions**: same behavior as REMOTE-172 (this spec layers on top of those flows without changing them).
## Success criteria
- Starting a non-oz cloud run immediately renders a queued user-query indicator styled like the existing pending prompt UI — not a top-of-conversation user query block.
- Environment setup commands appear under the collapsible setup-commands summary, consistent with Oz.
- When the harness command starts, the queued indicator is removed, the setup-commands summary auto-collapses, and the harness command renders as a normal CLI-agent session (native TUI visible, CLI-agent footer / input available).
- Pre-harness failure, cancel, and auth states remove the queued indicator and fall back to the existing error / cancel / auth UI.
- Replay and late-joining viewers never show a queued indicator.
- Oz cloud runs are unchanged.
- Disabling `CloudModeSetupV2` restores the legacy loading behavior for all harnesses.
## Validation
- Spawn a claude-code cloud run against an environment with multiple successful startup commands: confirm the queued prompt indicator appears, the setup-commands summary shows "Running setup commands…" expanded by default, and on harness start the indicator is removed, the summary collapses to "Ran setup commands", and the claude TUI is visible as the active block.
- Spawn a gemini cloud run and verify the same behavior using the Gemini CLI.
- Spawn a claude-code cloud run with no environment startup commands and confirm the queued indicator still works and is removed on harness start.
- Spawn a claude-code cloud run whose environment setup fails: confirm the queued indicator is removed on failure and the existing failed-run UI is shown.
- Cancel a claude-code cloud run before the harness block starts: confirm the queued indicator is removed and the cancelled UI is shown.
- Trigger GitHub-auth-required for a claude-code cloud run: confirm the queued indicator is removed and the auth UI is shown.
- Join an existing claude-code shared session while the harness is running: confirm no queued indicator, setup summary renders collapsed with prior command rows, and the claude TUI is the active CLI-agent block.
- Replay a completed claude-code conversation: confirm no queued indicator, setup summary renders from persisted data, and the conversation displays the claude transcript.
- Re-run the first scenario with `CloudModeSetupV2` disabled and confirm legacy behavior is restored for non-oz harnesses.
- Re-run all Oz-harness validation from REMOTE-172 and confirm no regression.
