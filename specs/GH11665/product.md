# GH11665: Agent pane tab config commands should start agent prompts
GitHub issue: https://github.com/warpdotdev/warp/issues/11665
Figma: none provided
## Summary
Tab configs should give authors a clear way to open an Agent Mode pane and automatically start the agent with an initial prompt. For `type = "agent"` panes, the `commands` array should be treated as agent prompts submitted after the pane is ready, not as shell commands that briefly run before Agent Mode opens.
To preserve setup workflows, agent panes should also support a separate `setup_commands` array for shell commands that must run before the agent prompt starts. This makes the two phases explicit: terminal setup first, then agent prompts.
## Problem
The current tab config schema accepts `commands` on `type = "agent"` panes, but the launch path treats those entries like terminal setup commands. When a user writes a natural-language prompt such as `Explain the project structure.`, Warp opens the configured directory, may briefly show the text in terminal mode, then enters Agent Mode with an empty input and no conversation. From the user's perspective the command was silently ignored.
This is especially confusing because `commands` works for `type = "terminal"` panes and the tab config reference describes `commands` as applying to `terminal` and `agent` panes. Users reasonably expect the same field to feed the active pane type: shell commands for terminal panes and agent prompts for agent panes.
At the same time, Warp-generated Oz worktree tab configs and some manually-authored configs need shell setup before Agent Mode starts. Reusing one array for both shell setup and agent prompts makes that impossible to express unambiguously.
## Goals
- Make `commands` on `type = "agent"` panes start an agent conversation automatically.
- Preserve terminal-pane behavior: `commands` on `type = "terminal"` panes continue to execute as shell commands in order.
- Add an explicit setup phase for agent panes so shell commands can run before entering/submitting to the agent.
- Avoid silently dropping or hiding configured agent prompts.
- Keep directory, parameter templating, focus, tab title, tab color, split layout, and shell selection behavior consistent with existing tab configs.
- Make generated Oz worktree tab configs continue to work by moving their shell setup into the explicit setup phase.
- Update tab config guidance so authors can tell the difference between shell setup commands and agent prompts.
## Non-goals
- Changing launch configuration YAML semantics outside tab configs.
- Changing terminal-pane command execution.
- Adding a general migration UI for every existing hand-authored agent-pane config.
- Supporting cloud-mode (`type = "cloud"`) prompt auto-submission in this change.
- Adding a new visual editor for tab config prompt phases.
- Changing Agent Mode prompt execution semantics, permissions, slash-command handling, or telemetry beyond the new tab-config entrypoint metadata needed for observability.
- Running terminal setup commands after Agent Mode has already started.
## Figma / design references
Figma: none provided. This is a behavioral/schema clarification and should reuse existing tab config menus, error toasts, Agent Mode UI, and command/prompt rendering.
## User experience
### Agent pane `commands`
When a tab config leaf pane has `type = "agent"` and a non-empty `commands` array, Warp treats each array entry as an agent prompt.
Example:
```toml
name = "Test Agent"

[[panes]]
id = "main"
type = "agent"
directory = "~/some-project"
commands = ["Explain the project structure."]
```
Opening this tab should:
1. Create a terminal-backed Agent Mode pane at `~/some-project`.
2. Enter Agent Mode for that pane.
3. Submit `Explain the project structure.` as the first user prompt in a new conversation.
4. Show the prompt and the agent response in the conversation history.
5. Leave the input empty unless another prompt is queued or the agent requires follow-up.
The prompt must not be sent to the shell as a terminal command.
### Multiple agent prompts
If an agent pane contains multiple `commands` entries, Warp submits the first entry as the initial prompt and queues the remaining entries for the same conversation in array order.
Example:
```toml
commands = [
  "Summarize this repository.",
  "Then identify the test command.",
]
```
Expected behavior:
- The first prompt starts the conversation.
- Later prompts are queued for the same conversation and run only after the previous agent turn completes, following existing queued-prompt behavior.
- If the first prompt fails to submit, later prompts remain unsent rather than being submitted to the shell.
### Empty and missing commands
If an agent pane has no `commands` field or `commands = []`, Warp opens Agent Mode with no initial prompt, matching today's empty-Agent-Mode behavior.
### Agent setup commands
Agent panes may include `setup_commands` for shell commands that should run before Agent Mode starts.
Example:
```toml
name = "Agent in worktree"

[[panes]]
id = "main"
type = "agent"
directory = "~/repo"
setup_commands = [
  "git worktree add -b {{branch}} ../{{branch}}",
  "cd ../{{branch}}",
]
commands = ["Review the diff and suggest next steps."]

[params.branch]
type = "text"
description = "Worktree branch name"
default = "my-feature"
```
Expected behavior:
1. Warp opens a terminal session at the rendered `directory`.
2. Warp runs each `setup_commands` entry as a terminal command in order.
3. If every setup command succeeds, Warp enters Agent Mode and submits/queues the `commands` prompts.
4. If any setup command fails, Warp stops running remaining setup commands, does not submit agent prompts, and leaves the pane in terminal mode with the failing command visible so the user can diagnose it.
5. Parameter values in `setup_commands` are shell-quoted the same way values in terminal `commands` are quoted today.
### Setup commands without prompts
If an agent pane has `setup_commands` but no `commands`, Warp runs setup commands and then enters an empty Agent Mode conversation only after setup succeeds. This preserves workflows that create a prepared shell context before handing control to the user in Agent Mode.
### Terminal panes
For `type = "terminal"` panes, `commands` behavior is unchanged:
- Entries run as shell commands in order.
- `setup_commands` is not needed and should be rejected or ignored with a clear parse/config error if present on terminal panes, depending on the existing tab config validation approach chosen by implementation.
### Cloud panes
For `type = "cloud"` panes, `commands` prompt auto-submission remains out of scope for this issue. Cloud panes should continue to reject or ignore unsupported terminal-only fields according to existing behavior until a dedicated cloud tab config spec defines prompt and environment semantics.
### Directory, shell, layout, focus, and params
- `directory` continues to set the working directory for terminal-backed panes before setup commands or agent prompts run.
- `shell` continues to apply only to the terminal session used by `terminal` and `agent` panes. For agent panes, it affects setup command execution, not agent prompt text.
- `title`, `color`, split layout, and `is_focused` continue to behave as they do today.
- Template parameters continue to render in titles, directories, setup commands, and prompts.
- Values inserted into shell setup commands must be shell-quoted. Values inserted into agent prompts should be rendered as user text, not shell-quoted.
### Errors and feedback
Warp should avoid silent failures:
- Invalid tab config schema should continue to surface parse errors through the existing tab config error path.
- A failing setup command should leave visible terminal output and not auto-submit prompts afterward.
- If Agent Mode cannot start because AI is unavailable, disabled, or blocked by account/team state, Warp should show the same user-facing error it shows for manually entering Agent Mode and should not submit prompts to the shell.
- If multiple prompts are queued, the queue UI should make the queued prompts visible using existing queued-prompt surfaces.
### Generated tab configs
Warp-generated Oz tab configs that need worktree setup should write setup shell commands to `setup_commands` and agent prompts, if any, to `commands`. Generated terminal and CLI-agent configs should keep their existing terminal-command behavior.
Existing Warp-generated worktree configs that used `commands` for setup should continue to work through a narrowly-scoped compatibility path when they match Warp's generated worktree command pattern.
## Success criteria
1. Opening a tab config with `type = "agent"` and `commands = ["Explain the project structure."]` starts a new Agent Mode conversation with that text as the first user prompt.
2. The same agent-pane prompt is not executed in the shell and does not produce a transient `Explain: command not found`-style terminal block.
3. Opening an agent pane with multiple `commands` submits the first prompt and queues the remaining prompts for the same conversation in order.
4. Opening an agent pane with no `commands` still opens empty Agent Mode.
5. Opening an agent pane with only `setup_commands` runs setup commands in terminal mode and then enters empty Agent Mode after success.
6. Opening an agent pane with both `setup_commands` and `commands` runs setup commands first, then submits the first prompt only after setup succeeds.
7. If an agent-pane setup command fails, no agent prompt is submitted and the failing terminal block remains visible.
8. Terminal pane `commands` behavior is unchanged.
9. Generated Oz worktree tab configs continue to create/switch into worktrees rather than sending `git worktree ...` text as agent prompts.
10. Template parameters render correctly in agent prompts without shell quoting artifacts such as added single quotes around spaces.
11. Template parameters render correctly in setup commands with shell quoting preserved.
12. Split-pane configs can contain terminal panes and agent panes with independent commands/prompts without cross-pane interference.
13. Focus still lands on the configured focused pane or the first leaf when no pane is explicitly focused.
14. Agent prompt submission surfaces the same errors as manual Agent Mode entry when AI is disabled or unavailable.
15. Tab config documentation and bundled tab-config guidance describe `commands` versus `setup_commands` for agent panes.
## Validation
- Unit test tab config parsing/rendering for `type = "agent"` with `commands`, with `setup_commands`, with both, and with parameter templating in both fields.
- Unit test that terminal-pane `commands` rendering remains unchanged.
- Unit test generated Oz worktree tab config output so setup shell commands are stored as `setup_commands`.
- Unit test the compatibility path for Warp-generated legacy worktree agent configs if that path is implemented.
- Unit test or view-model test the pane template launch path so agent prompts are passed to Agent Mode rather than to `set_pending_command_queue`.
- Unit test multiple prompt queueing by asserting later prompts are appended to the existing queued-prompt model for the active conversation.
- Integration test opening a single-pane agent tab config with one prompt and assert a conversation starts with that initial user query.
- Integration test opening an agent tab config with setup commands and a prompt; assert setup blocks complete before the prompt is submitted.
- Integration/manual validation for setup failure: make `setup_commands = ["false"]` and confirm the prompt is not sent.
- Manual validation from the `+` menu on macOS using the reporter's reproduction config.
## Open questions
- Should `setup_commands` be accepted on `terminal` panes as an alias for `commands`, or should it be rejected to keep the schema simple? This spec prefers rejection/clear feedback for the first implementation.
- Should cloud panes eventually treat `commands` as initial cloud-agent prompts? This issue does not define cloud behavior because cloud panes have different environment and attachment semantics.
- Should there be a one-time migration for all existing hand-authored agent-pane configs that currently use `commands` for shell setup, or is compatibility for Warp-generated worktree configs sufficient?
