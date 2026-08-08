# GH11665: Tech Spec — Agent pane tab config commands should start agent prompts
Product spec: `specs/GH11665/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/11665
## Problem
Tab configs currently parse `commands` on all leaf pane types, render those entries into `PaneTemplateType::PaneTemplate.commands`, and `PaneGroup` feeds that vector into `TerminalView::set_pending_command_queue` for both terminal and agent panes. For `PaneMode::Agent`, `PaneGroup` then defers entering Agent Mode until those pending shell commands finish.
That means `type = "agent"` with `commands = ["Explain the project structure."]` runs the natural-language text as a shell command, then opens Agent Mode with no initial prompt. The product behavior should instead treat agent-pane `commands` as agent prompts while retaining a separate setup command path for shell preparation.
## Relevant code
- `app/src/tab_configs/tab_config.rs:96` — `TabConfigPaneType` defines `Terminal`, `Agent`, and `Cloud`.
- `app/src/tab_configs/tab_config.rs:114` — `TabConfigPaneNode` is the TOML schema for flat `[[panes]]` entries.
- `app/src/tab_configs/tab_config.rs:129` — `commands: Option<Vec<String>>` is currently shared by all leaf pane types.
- `app/src/tab_configs/tab_config.rs:219` — `render_tab_config` converts a `TabConfig` into a `PaneTemplateType`.
- `app/src/tab_configs/tab_config.rs:365` — `resolve_pane_node` maps `TabConfigPaneType::Agent` to `PaneMode::Agent`.
- `app/src/tab_configs/tab_config.rs:387` — `commands` are rendered with the shell-quoted template context today.
- `app/src/launch_configs/launch_config.rs:94` — `PaneMode` is the lower-level mode used by launch/tab templates.
- `app/src/launch_configs/launch_config.rs:105` — `PaneTemplateType::PaneTemplate` stores `commands`, `pane_mode`, and `shell`.
- `app/src/pane_group/mod.rs:1298` — template panes create a terminal session for `PaneMode::Terminal` and `PaneMode::Agent`.
- `app/src/pane_group/mod.rs:1320` — non-cloud `commands` are passed to `terminal.set_pending_command_queue`.
- `app/src/pane_group/mod.rs:1331` — agent panes without commands enter Agent Mode immediately.
- `app/src/pane_group/mod.rs:1341` — agent panes with commands currently set `enter_agent_view_after_pending_commands`.
- `app/src/terminal/view.rs:8605` — `set_pending_command_queue` stores shell commands and seeds the next pending command.
- `app/src/terminal/view.rs:11785` — pending command completion advances the queue or emits `PendingCommandCompleted`.
- `app/src/terminal/view.rs:11801` — deferred agent entry currently enters Agent Mode with `None` after setup commands finish.
- `app/src/terminal/view/agent_view.rs:44` — `enter_agent_view` accepts an optional `initial_prompt`.
- `app/src/terminal/view/agent_view.rs:48` — `enter_agent_view_for_new_conversation` can start a new conversation with `initial_prompt`.
- `app/src/terminal/input.rs:13150` — `submit_queued_prompt` shows how existing queued prompts are resubmitted into the active conversation.
- `app/src/ai/blocklist/queued_query.rs:23` — `QueuedQueryOrigin` enumerates telemetry/informational origins for queued prompts.
- `app/src/tab_configs/session_config.rs:99` — `build_tab_config` builds generated tab configs for Terminal, Oz, and CLI agents.
- `app/src/tab_configs/session_config.rs:112` — generated worktree setup commands are currently appended to `commands`.
- `app/src/tab_configs/session_config.rs:149` — `SessionType::Oz` maps to `TabConfigPaneType::Agent`.
- `app/src/tab_configs/session_config_tests.rs:133` — tests currently assert Oz worktree commands are stored in `commands`.
- `resources/bundled/skills/tab-configs/SKILL.md:48` — bundled tab config guidance says `commands` applies to `terminal` and `agent`.
## Current state
The current data flow has only one command vector:
1. TOML `commands` parse into `TabConfigPaneNode.commands`.
2. `render_tab_config` renders every command through the shell-quoted template context.
3. The rendered vector is stored in `PaneTemplateType::PaneTemplate.commands` regardless of pane mode.
4. `PaneGroup::pane_tree_from_template_recursive` starts a terminal session for agent panes.
5. If the vector is non-empty, it queues those strings through `TerminalView::set_pending_command_queue`.
6. For `PaneMode::Agent`, it enters Agent Mode immediately only when the command vector is empty; otherwise it sets a boolean to enter Agent Mode after pending commands finish.
7. When the queued shell commands complete, `TerminalView` enters Agent Mode with no initial prompt.
This behavior is useful for setup commands, including generated Oz worktree configs, but it cannot represent initial agent prompts. It also applies shell quoting to text that should be a user-authored natural-language prompt.
## Proposed changes
### 1. Extend the tab config pane schema with `setup_commands`
Add an optional field to `TabConfigPaneNode`:
```rust
pub setup_commands: Option<Vec<String>>,
```
Semantics:
- `commands` keeps its existing meaning for `type = "terminal"` panes.
- `commands` becomes agent prompt text for `type = "agent"` panes.
- `setup_commands` is shell setup text for `type = "agent"` panes.
- `setup_commands` is rendered through the shell-quoted template context.
- Agent-pane `commands` are rendered through the unquoted template context because they are prompt text, not shell commands.
Update the tab config parser/serializer tests to cover the new field.
### 2. Add agent prompt storage to pane templates
Extend `PaneTemplateType::PaneTemplate` with a default/skipped field such as:
```rust
#[serde(skip_serializing_if = "Vec::is_empty", default)]
pub initial_agent_prompts: Vec<CommandTemplate>,
```
`CommandTemplate` can be reused because it already wraps a rendered string, but the lower-level field name should make the semantic difference clear. `commands` should continue to mean shell command queue at the pane-template layer so existing launch config YAML behavior stays unchanged.
Rendering rules in `resolve_pane_node`:
- Terminal pane:
  - `commands` = rendered `node.commands` with quoted context.
  - `initial_agent_prompts` = empty.
  - `setup_commands` should be rejected during validation or ignored with a logged warning; prefer validation if there is an existing parse-error surface available.
- Agent pane:
  - `commands` = rendered `node.setup_commands` with quoted context.
  - `initial_agent_prompts` = rendered `node.commands` with unquoted context.
  - `pane_mode = PaneMode::Agent`.
- Cloud pane:
  - Keep existing behavior. Do not add cloud prompt handling in this issue.
This keeps `PaneGroup`'s existing shell command queue mostly intact while adding an explicit prompt vector.
### 3. Launch agent prompts after optional setup commands
Update `PaneGroup::pane_tree_from_template_recursive` to destructure `initial_agent_prompts`.
For `PaneMode::Agent`:
- If shell setup `commands` is empty:
  - Enter Agent Mode immediately with the first prompt from `initial_agent_prompts`, if any.
  - Append remaining prompts to the active conversation queue.
- If shell setup `commands` is non-empty:
  - Queue setup commands exactly as today.
  - Store a deferred agent-entry payload containing the rendered prompt vector.
  - After setup commands complete successfully, enter Agent Mode with the first prompt and queue the rest.
  - If setup fails, clear remaining setup commands, clear the deferred agent-entry payload, do not enter Agent Mode, and surface the failure through the visible terminal block plus any existing pending-command failure feedback.
Replace the current boolean `enter_agent_view_after_pending_commands` in `TerminalView` with a payload-capable state, for example:
```rust
enum DeferredAgentViewEntry {
    AfterPendingCommands { initial_prompts: Vec<String> },
}
```
or a simpler `Option<Vec<String>>` if no other variants are expected. The old empty-Agent-Mode case can be represented by an empty vector.
### 4. Queue additional agent prompts
After entering Agent Mode with the first prompt, determine the active conversation id the same way `init_project` does after calling `enter_agent_view_for_new_conversation`: read `agent_view_controller.as_ref(ctx).agent_view_state().active_conversation_id()`.
For each remaining prompt:
- Append a `QueuedQuery` for that conversation in order.
- Add `QueuedQueryOrigin::TabConfig` to `app/src/ai/blocklist/queued_query.rs` so telemetry/debugging can distinguish prompts created by tab config launch.
- Reuse existing queued-prompt UI and auto-fire drain behavior rather than adding a separate queue.
If there is no active conversation after entry, log an error and leave the remaining prompts unsent.
### 5. Preserve generated Oz worktree behavior
Update `app/src/tab_configs/session_config.rs`:
- For `SessionType::Oz` with `enable_worktree = true`, put worktree setup shell commands in `setup_commands`, not `commands`.
- For `SessionType::Oz` with no worktree and no prompt, continue emitting no commands.
- For `SessionType::Terminal` and `SessionType::CliAgent(_)`, keep existing `commands` behavior unchanged because those are terminal commands.
Update `app/src/tab_configs/session_config_tests.rs` assertions:
- `oz_with_worktree_has_worktree_commands_but_no_agent_command` should assert `setup_commands` contains the generated worktree commands and `commands` is `None`.
- Terminal and CLI-agent worktree tests should keep asserting `commands`.
### 6. Add narrowly-scoped legacy compatibility for generated worktree configs
Existing users may already have generated Oz worktree tab configs on disk where `type = "agent"` and `commands` contains the generated `git worktree add ...` plus `cd ...` setup sequence. Without compatibility, those configs would start an agent prompt saying `git worktree add ...`.
Add a helper in `tab_config.rs`, used only for agent panes with `setup_commands.is_none()`:
- Detect the legacy generated worktree shape:
  - `commands` length is at least 2.
  - first command contains `git worktree add`.
  - one of the first two commands contains the Warp-generated worktree path pattern from `generated_worktree_repo_dir` or the `{{autogenerated_branch_name}}` / `{{worktree_branch_name}}` placeholders used by `build_tab_config`.
- If detected, render those `commands` as setup shell commands and leave `initial_agent_prompts` empty.
- Log a warning encouraging the generated config to be rewritten with `setup_commands`.
Keep the compatibility narrow. Hand-authored agent-pane `commands` should follow the new documented prompt semantics.
### 7. Update documentation and bundled tab config guidance
Update `resources/bundled/skills/tab-configs/SKILL.md`:
- Leaf `commands`: terminal shell commands for `terminal`; agent prompts for `agent`.
- New `setup_commands`: shell commands for `agent` before the agent starts.
- `shell`: affects terminal panes and agent setup commands.
- Examples for:
  - an agent pane with one initial prompt.
  - an agent pane with setup commands and then a prompt.
If public docs in the docs repository also define the tab config schema, update them in the implementation PR or open a paired docs follow-up if that repo is not part of the implementation branch.
### 8. Telemetry and error handling
No new user-visible telemetry event is required for this spec, but implementation should preserve existing Agent Mode entry and queued-prompt telemetry. Add `QueuedQueryOrigin::TabConfig` so queue events can identify their source internally.
For setup failure:
- Existing terminal block output should remain visible.
- Do not submit prompts.
- Clear the deferred agent-entry payload to avoid accidental later submission.
For Agent Mode entry failure:
- Use the same error surface as `enter_agent_view_for_new_conversation`.
- Do not submit or queue remaining prompts.
## End-to-end flow
### Agent pane with only prompts
1. User selects a tab config from the `+` menu.
2. `Workspace::open_tab_config_with_params` renders the config.
3. `resolve_pane_node` maps agent `commands` to `initial_agent_prompts` and leaves shell `commands` empty.
4. `PaneGroup` creates the terminal-backed pane.
5. `PaneGroup` enters Agent Mode with the first prompt.
6. `TerminalView` starts a new conversation and submits the initial prompt.
7. Remaining prompts are appended to `QueuedQueryModel`.
### Agent pane with setup and prompts
1. User selects a tab config from the `+` menu.
2. `resolve_pane_node` maps `setup_commands` to shell `commands` and maps agent `commands` to `initial_agent_prompts`.
3. `PaneGroup` creates the terminal-backed pane and queues setup shell commands.
4. `TerminalView` runs setup commands as separate shell blocks.
5. On successful completion of the final setup command, `TerminalView` enters Agent Mode with the first prompt and queues the rest.
6. On failed setup, `TerminalView` stays in terminal mode and discards the deferred prompt payload.
## Risks and mitigations
### Risk: existing hand-authored setup configs change behavior
Some users may have manually used `commands` on agent panes as setup commands. The new semantics treat them as prompts.
Mitigation: add `setup_commands`, preserve generated worktree configs through a narrow compatibility detector, and update docs clearly. Avoid broad heuristics that would make natural-language prompt configs unpredictable.
### Risk: shell quoting leaks into prompts
Current `commands` rendering shell-quotes parameter values. That is correct for setup commands but wrong for prompt text.
Mitigation: render agent prompts with the unquoted context and add tests with spaces/special characters in params.
### Risk: prompt submission happens before cwd/setup is ready
If setup commands are present, prompt submission must wait until the final successful setup command completes.
Mitigation: replace the boolean deferred-entry flag with a payload stored on `TerminalView` and consume it only from the existing pending-command completion path.
### Risk: multiple prompts create race conditions
Submitting several prompts immediately could conflict with in-progress conversation state.
Mitigation: submit only the first prompt directly and append remaining prompts to the existing queued-prompt model, which already handles auto-fire after conversation completion.
### Risk: launch config serialization compatibility
`PaneTemplateType` is used by older launch config YAML in addition to tab configs.
Mitigation: add the new field with `default` and `skip_serializing_if = "Vec::is_empty"`, and leave existing `commands` semantics unchanged at the pane-template layer.
## Testing and validation
### Unit tests
- `app/src/tab_configs/tab_config_tests.rs`
  - Parse an agent pane with `commands = ["Explain the project."]` and assert render output has empty shell `commands` and `initial_agent_prompts = ["Explain the project."]`.
  - Parse an agent pane with `setup_commands` and `commands` and assert both vectors map to the correct pane-template fields.
  - Verify setup commands use shell-quoted parameter substitution.
  - Verify agent prompts use unquoted parameter substitution.
  - Verify terminal-pane `commands` still map to shell `commands`.
  - Verify legacy generated worktree agent configs map `commands` to setup commands if the compatibility path is implemented.
- `app/src/tab_configs/session_config_tests.rs`
  - Update Oz worktree tests to assert generated setup commands are written to `setup_commands`.
  - Keep terminal and CLI-agent tests unchanged except for struct field updates.
- `app/src/launch_configs/launch_config_tests.rs`
  - Add serde round-trip coverage for `initial_agent_prompts` default/skip behavior if needed.
- `app/src/ai/blocklist/queued_query_tests.rs`
  - Assert `QueuedQueryOrigin::TabConfig` can be appended and drained like other origins.
### Integration or view tests
- Add or extend a tab config integration test to open a single agent pane with one prompt and assert the active conversation's initial user query matches the configured prompt.
- Add a setup-command integration test using a harmless command such as `pwd` or `true` followed by a prompt, asserting setup runs first and the prompt starts afterward.
- Add a failure test with `setup_commands = ["false"]` if the integration harness can assert no conversation was started.
### Manual validation
- Reproduce the issue's config on macOS:
  - Open the tab from the `+` menu.
  - Confirm Agent Mode opens in the configured directory.
  - Confirm `Explain the project structure.` appears as the initial user prompt and the agent starts responding.
- Validate an agent worktree config generated from the session config modal still creates the worktree and enters Agent Mode after setup.
- Validate a multi-pane tab config with one terminal pane and one agent pane runs terminal commands and agent prompts independently.
### Commands
Run targeted tests first:
- `cargo test -p warp --lib tab_configs::tab_config::tests`
- `cargo test -p warp --lib tab_configs::session_config::tests`
- `cargo test -p warp --lib queued_query`
Then run formatting and the smallest available compile check for touched crates:
- `cargo fmt --check`
- `cargo check -p warp --lib`
Before the implementation PR is considered ready, follow the repository's normal presubmit guidance for Rust linting and integration coverage.
## Follow-ups
- Define cloud-pane prompt auto-submission in a separate spec if product wants `type = "cloud"` to support initial prompts.
- Consider adding an explicit tab config schema version if future behavior changes need less heuristic compatibility handling.
- Consider a docs-only migration guide for users who intentionally used agent-pane `commands` as shell setup commands.
