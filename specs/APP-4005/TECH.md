# /rename-tab Slash Command Technical Spec
Linear: [APP-4005](https://linear.app/warpdotdev/issue/APP-4005/add-rename-tab-slash-command)
Product spec: `specs/APP-4005/PRODUCT.md`
## Problem
Users can rename tabs today through tab UI interactions, but there is no keyboard-first slash-command path from the terminal or agent input. Technically, this requires connecting two existing subsystems:
- Static slash-command registration, parsing, availability, and execution from terminal input.
- Workspace-level tab custom-title state.
The command should require the new tab name inline. An argumentless flow that opens the existing rename editor is intentionally excluded because the focused editor in the tab strip is too subtle relative to the user's attention in terminal input.
## Relevant code
- `app/src/search/slash_command_menu/static_commands/commands.rs:10` — static command declarations such as `/agent`, `/cloud-agent`, and `/open-file`.
- `app/src/search/slash_command_menu/static_commands/commands.rs:445` — `Registry` and `all_commands()` registration path for static slash commands.
- `app/src/search/slash_command_menu/static_commands/mod.rs:9` — `Availability`, `Argument`, and `StaticCommand` definitions.
- `app/src/terminal/input/slash_commands/data_source/mod.rs:139` — active slash-command recomputation from session context, agent view state, local/repository state, and CLI-agent input state.
- `app/src/terminal/input/slash_commands/data_source/mod.rs:151` — `CLI_AGENT_INPUT_ALLOWED_COMMANDS`, which keeps CLI-agent rich input restricted to passthrough-compatible commands.
- `app/src/terminal/input/slash_command_model.rs:369` — parsing static slash commands and arguments from input text.
- `app/src/terminal/input/slash_commands/mod.rs:280` — `Input::execute_slash_command`, the handler for static command execution.
- `app/src/workspace/action.rs:82` — existing tab-related `WorkspaceAction` variants including `RenameTab`, `ResetTabName`, and `RenameActiveTab`.
- `app/src/workspace/action.rs:615` — `WorkspaceAction::should_save_app_state_on_action`, where tab name mutations are marked as requiring app-state save.
- `app/src/workspace/view.rs:4590` — `rename_tab`, which opens the editor and emits `TabRenameEvent::OpenedEditor` for existing UI flows.
- `app/src/workspace/view.rs:4664` — `clear_tab_name`, which clears a custom title and emits `TabRenameEvent::CustomNameCleared` for existing UI flows.
- `app/src/workspace/view.rs:18264` — `Workspace::handle_action`, which maps workspace actions to rename handlers.
- `app/src/pane_group/mod.rs:4636` — `PaneGroup::display_title`, which resolves custom title before focused-pane title.
- `app/src/pane_group/mod.rs:4656` — `PaneGroup::set_title`, which stores tab-level custom title and treats empty strings as clearing the custom title.
- `app/src/tab.rs:269` — tab context-menu rename/reset actions using the same `WorkspaceAction` variants.
- `app/src/workspace/view/vertical_tabs.rs:1410` — vertical-tabs rows receive the shared rename editor and tab rename state.
- `app/src/terminal/input/slash_command_model_tests.rs:7` — slash-command argument parsing test pattern.
- `app/src/workspace/view_test.rs:642` — existing tab rename editor selection/reset tests.
- `app/src/server/telemetry/events.rs:414` — existing `TabRenameEvent` values.
## Current state
Static slash commands are represented as `StaticCommand` values in `commands.rs`. Each command declares a name, description, icon, availability flags, AI-mode behavior, and optional/required argument metadata. The registry is rebuilt at startup from `all_commands()`.
Each terminal input owns a `SlashCommandDataSource` that filters registered commands against the current session context. `Availability::ALWAYS` makes a command available in both terminal view and agent view, subject to global AI enablement and terminal slash-command settings. CLI-agent rich input is a special case: when it is open, the data source filters static commands to `CLI_AGENT_INPUT_ALLOWED_COMMANDS` only, currently `"/prompts"` and `"/skills"`.
`SlashCommandModel::parse_slash_command` splits the input at the first space. For a command with a required argument:
- `/command` does not parse as a completed slash command.
- `/command value` parses with `argument = Some("value")`.
- `/command ` parses with `argument = Some("")`, so execution handlers should still reject empty required arguments when needed.
`Input::execute_slash_command` matches on command name and performs side effects. After a handled command, it clears the input editor and emits slash-command accepted telemetry.
Tab rename state is owned by `Workspace` and each tab's `PaneGroup`. Custom tab names are stored on `PaneGroup::custom_title`; `PaneGroup::display_title` resolves custom title first and falls back to the focused pane's automatic title. The slash command should set that same custom-title state directly.
## Proposed changes
### Add the static command
Add a new `RENAME_TAB` static command in `app/src/search/slash_command_menu/static_commands/commands.rs`.
Recommended shape:
- `name: "/rename-tab"`
- `description: "Rename the current tab"`
- `icon_path`: reuse an existing bundled icon that reads as edit/rename, such as `bundled/svg/pencil-line.svg`.
- `availability: Availability::ALWAYS`
- `requires_ai_mode: false`
- `argument: Some(Argument::required().with_hint_text("<tab name>"))`
Do not call `with_execute_on_selection()`. Selecting the command from the slash-command menu should insert `/rename-tab ` and wait for the user to provide the required name.
Add `RENAME_TAB` to `all_commands()` unconditionally. No new feature flag is needed because this is an additive command that uses existing tab title behavior and is easy to remove if needed.
### Add a workspace action for direct setting
Add a new workspace action for direct active-tab title updates:
- `WorkspaceAction::SetActiveTabName(String)`
This avoids making terminal input know the active tab index. It also keeps all tab state mutation inside `Workspace`, matching existing ownership.
Add the new action to:
- `WorkspaceAction::should_save_app_state_on_action` as `true`.
- `Workspace::handle_action`.
Implement a workspace helper along these lines:
- `fn set_active_tab_name(&mut self, title: &str, ctx: &mut ViewContext<Self>)`
Behavior:
1. If there is no active tab, log a warning and return.
2. If a tab rename editor is already open, cancel or clear the transient editor state before direct-setting. The slash command should be authoritative and should not leave stale rename UI.
3. Trim the incoming title.
4. If the trimmed title is non-empty:
   - Compare it with the active pane group's current `display_title`.
   - If different, set it through `PaneGroup::set_title`.
   - Emit `TabRenameEvent::CustomNameSet` only when state changes.
5. Notify the UI.
The slash-command execution handler should reject empty or whitespace-only names before dispatching this action.
### Handle command execution
Add a branch to `Input::execute_slash_command` in `app/src/terminal/input/slash_commands/mod.rs`:
- If `command.name == commands::RENAME_TAB.name`:
  - If `argument` is `None` or trims to an empty string, show a concise error toast such as `Please provide a tab name after /rename-tab` and return `true`.
  - Otherwise, dispatch `WorkspaceAction::SetActiveTabName(trimmed_name.to_owned())`.
The existing post-match code should then clear the invoking input and emit static slash-command accepted telemetry for successful execution.
This should not fall through to the shell or agent as literal `/rename-tab` text when handled as a slash command.
### Preserve CLI-agent rich input restrictions
Do not add `/rename-tab` to `CLI_AGENT_INPUT_ALLOWED_COMMANDS` in the initial implementation. That input currently intentionally exposes only passthrough-compatible commands (`/prompts`, `/skills`) while composing text for a running CLI agent.
If the CLI-agent input model later supports Warp-handled workspace commands, this command can be added there as a follow-up by widening the allowlist and ensuring execution is intercepted by Warp rather than written to the PTY.
### Tests
Use existing unit-test patterns rather than adding integration infrastructure.
Recommended tests:
- In `app/src/terminal/input/slash_command_model_tests.rs`, add focused assertions for `/rename-tab` parsing:
  - `/rename-tab` does not parse as a completed slash command because the required argument is missing.
  - `/rename-tab Backend` parses with `Some("Backend")`.
  - `/rename-tab Backend API` preserves the rest of the line as one argument.
  - `/rename-tab ` parses with `Some("")`, and execution rejects the empty name.
- In `app/src/search/slash_command_menu/static_commands/commands.rs` tests, rely on the existing uniqueness test and add a direct assertion that `COMMAND_REGISTRY.get_command_with_name("/rename-tab")` exists with required argument metadata and `should_execute_on_selection = false`.
- In `app/src/workspace/view_test.rs`, add workspace-level tests for direct active-tab setting:
  - Setting a non-empty name updates the active tab's `display_title`.
  - Internal spaces are preserved after trimming surrounding whitespace.
  - Direct-setting one tab does not mutate another tab.
  - If rename editor state was active, direct-set leaves no tab in `is_tab_being_renamed` state.
Because this change affects visible UI, perform manual validation in both horizontal and vertical tabs after unit tests pass.
## End-to-end flow
1. User selects `/rename-tab` from the slash-command menu.
2. Because the command has a required argument and does not execute on selection, Warp inserts `/rename-tab ` into the input.
3. User types the desired tab name and submits the slash command.
4. `SlashCommandModel` parses the command and argument.
5. `Input::execute_slash_command` trims and validates the argument.
6. `Input::execute_slash_command` dispatches `WorkspaceAction::SetActiveTabName`.
7. `Workspace` mutates the active tab's `PaneGroup` custom title.
8. Horizontal and vertical tabs re-render from `display_title`.
9. Input clears the slash command buffer and emits slash-command telemetry.
No code path should dispatch `WorkspaceAction::RenameActiveTab` for `/rename-tab`.
## Risks and mitigations
- **Accidentally sending `/rename-tab` to the shell or agent**: Make the command a handled branch in `execute_slash_command` and return `true` for detected invalid empty-argument execution. Cover parsing and execution-adjacent behavior in tests.
- **CLI-agent input confusion**: Keep `/rename-tab` out of `CLI_AGENT_INPUT_ALLOWED_COMMANDS` for now so the command is not offered in passthrough-only input.
- **Stale rename editor state**: If direct-set runs while a rename editor is active, clear `current_workspace_state.tab_being_renamed` and the shared editor buffer before mutating the active tab name.
- **Focus surprises after direct-set**: `PaneGroup::set_title` refocuses the focused pane. Verify this does not steal focus from the intended terminal or agent input in manual testing.
- **Whitespace semantics**: Required-argument parsing can still produce `Some("")` for `/rename-tab `; reject empty trimmed names in `execute_slash_command`.
- **Active tab index assumptions**: Do not pass a tab index from `Input`. The workspace should use its own `active_tab_index` at execution time.
## Testing and validation
Run targeted tests first:
- `cargo test -p warp --lib rename_tab`
- `cargo test -p warp --lib set_active_tab_name`
- `cargo test -p warp --lib terminal::input::slash_command_model::tests`
Then run formatting/lint checks as appropriate:
- `cargo fmt --check`
- `cargo clippy -p warp --lib --tests -- -D warnings`
Manual validation:
1. In horizontal tabs, select `/rename-tab` from the slash-command menu and verify it inserts `/rename-tab ` rather than executing immediately.
2. In horizontal tabs, type `/rename-tab API server`, press Enter, verify the active tab label changes immediately and the shell does not receive the command.
3. Type `/rename-tab    ` and verify no tab name is cleared.
4. Enable vertical tabs and repeat the direct-set flow.
5. In a split-pane tab, focus a non-primary pane and verify direct-set renames the containing tab only.
6. In a Warp Agent tab, run `/rename-tab Agent Work` and verify only the tab label changes.
7. Verify tab context-menu **Reset tab name** still clears names set by the slash command.
Because this changes UI behavior, after implementation invoke the `verify-ui-change-in-cloud` skill in an eligible local non-sandboxed environment.
## Follow-ups
- Decide whether CLI-agent rich input should support Warp-handled workspace commands like `/rename-tab`.
- Consider adding a command-palette entry or keybinding for direct active-tab rename if users want a non-slash-command keyboard path.
- Consider adding a dedicated slash-command telemetry source to `TabRenameEvent` only if product analytics need to distinguish slash-command-driven tab renames beyond existing slash-command accepted telemetry.
