# /rename-tab Slash Command
Linear: [APP-4005](https://linear.app/warpdotdev/issue/APP-4005/add-rename-tab-slash-command)
Figma: none provided
## Summary
Add a `/rename-tab` slash command that lets users rename the active Warp tab from the input by providing the desired name inline: `/rename-tab <name>`.
The command should reuse existing tab custom-title state so names set by the slash command behave like names set through the existing tab rename UI.
## Problem
Renaming a tab currently requires interacting with the tab UI directly: double-clicking a horizontal tab, double-clicking the relevant vertical-tabs row/header, or using the tab context menu. Users who are already typing in Warp's input must switch interaction modes to rename the current tab.
An argumentless `/rename-tab` flow that opens the tab rename editor is too subtle: the editor can be focused in the tab strip while the user's attention remains in terminal input. Requiring the name in the command keeps the interaction explicit and avoids relying on a small visual focus change outside the input.
## Goals
- Add a discoverable `/rename-tab` static slash command.
- Require a tab name argument.
- Set the active tab's custom name directly by running `/rename-tab <name>`.
- Keep behavior consistent between horizontal tabs and vertical tabs.
- Make the command available in Warp-owned slash-command inputs where renaming the active tab is meaningful, including standard terminal input and Warp Agent / Cloud Agent input surfaces backed by a tab.
- Preserve existing rename-tab semantics for persistence, display, telemetry, and reset behavior.
## Non-goals
- Opening the existing inline rename editor from `/rename-tab` with no argument.
- Renaming an arbitrary non-active tab by index, title, pane id, or selector.
- Renaming individual split panes or code editor tabs.
- Adding a new tab naming UI.
- Changing how automatic terminal titles, conversation titles, directory labels, or vertical-tabs metadata are computed.
- Changing the existing double-click or context-menu rename flows.
- Adding tab-name templating, interpolation, or shell-variable expansion.
- Making CLI-agent rich input support general Warp workspace commands unless that input surface already supports them by the time this feature is implemented.
## Figma / design references
Figma: none provided.
This feature should use the existing slash-command menu item styling. No new visual design is required.
## User experience
### Slash command discovery
When the user types `/` in an eligible Warp input, `/rename-tab` appears in the slash-command menu.
The command should use copy similar to:
- Name: `/rename-tab`
- Description: `Rename the current tab`
- Argument hint: `<tab name>`
The command takes a required argument. Selecting the command from the slash-command menu inserts `/rename-tab ` into the input so the user can type the new tab name before executing.
### Availability
The command is available when all of the following are true:
- The current input can execute Warp static slash commands.
- The input belongs to a workspace tab that can be renamed through the existing tab rename action.
- There is an active tab in the current workspace window.
Expected available contexts:
- Standard terminal panes when slash commands in terminal input are enabled.
- Warp Agent input associated with a terminal-backed tab.
- Cloud Agent input associated with a terminal-backed tab.
Expected unavailable contexts:
- Inputs that do not expose Warp static slash commands.
- CLI-agent rich input if that surface is currently restricted to native CLI-agent passthrough commands and the explicit existing allowlist.
- Non-input surfaces such as the command palette, settings search, code editor text fields, and modal text fields.
If future work broadens CLI-agent rich input to include general Warp workspace commands, `/rename-tab` should become eligible there as long as executing it is handled by Warp rather than written through to the CLI agent's PTY.
### Executing with an argument
When the user executes `/rename-tab <name>`, Warp sets the active tab's custom name directly without opening the inline rename editor.
The direct-set behavior:
- Applies to the active tab in the current workspace window.
- Treats the full argument after the command and separating space as the desired name.
- Preserves internal spaces in the argument.
- Trims leading and trailing whitespace before deciding the final name.
- Requires the trimmed name to be non-empty.
- Saves the custom name using the same underlying custom tab title state as the existing rename flow.
- Updates the tab label immediately in both horizontal and vertical tab UI.
- Persists the custom name anywhere existing custom tab names persist.
- Clears the invoking input after successful execution.
- Returns focus to the same terminal or agent input surface that invoked the command.
Examples:
- `/rename-tab deploy` sets the active tab name to `deploy`.
- `/rename-tab API server` sets the active tab name to `API server`.
- `/rename-tab release candidate 1` sets the active tab name to `release candidate 1`.
### Missing and empty arguments
The command should not rename or clear a tab when no non-empty name is provided.
- `/rename-tab` should not execute the rename action because the required argument is missing.
- `/rename-tab ` or `/rename-tab    ` should show a concise error instead of opening the rename editor or clearing the custom tab name.
- Selecting `/rename-tab` from the slash-command menu should insert the command and a trailing space so the user can type the required name.
### Relationship to existing tab names
The command sets the same custom tab name that users set through the existing rename flow.
After a custom name is set:
- The tab displays the custom name instead of the automatic focused-pane title where existing custom-name behavior already applies.
- The tab context menu exposes the existing reset option when appropriate.
- Resetting the tab name through existing UI clears the name set by `/rename-tab`.
- Automatic title updates from the shell, active pane, or conversation metadata do not overwrite the custom name.
### Horizontal tabs
With horizontal tabs enabled, `/rename-tab <name>` affects the active horizontal tab only and updates the visible horizontal tab label immediately without opening the editor.
### Vertical tabs
With vertical tabs enabled, `/rename-tab <name>` affects the active tab only and updates all visible vertical-tabs representations of the tab immediately, including group headers and representative rows that display the custom tab title.
The command should not require the vertical tabs panel to already be focused.
### Split panes
If the active tab contains split panes, `/rename-tab <name>` renames the tab that contains the focused pane. It does not rename the focused pane.
Changing focus between split panes may change the tab's automatic title, but once `/rename-tab <name>` sets a custom tab name, pane focus changes should not replace that custom name.
### Agent and cloud-agent tabs
For Warp Agent and Cloud Agent conversations that live inside a tab with a Warp input, `/rename-tab <name>` should rename that tab, not the conversation itself.
The command should not change:
- AI conversation title.
- Cloud agent run title.
- Prompt text.
- Conversation history.
- Agent management records.
If the current tab's visible label is derived from an agent conversation title, running `/rename-tab <name>` should set a custom tab name that takes precedence in tab UI according to the existing custom-title rules.
### Errors and unavailable states
If the command is somehow executed when no active tab exists, the active tab cannot be renamed, or the provided name is empty after trimming, Warp should fail gracefully:
- Do not send `/rename-tab` to the shell or agent.
- Do not mutate any tab state.
- Show a concise error toast such as `Please provide a tab name after /rename-tab` or `Cannot rename the current tab`.
This should be rare because normal command availability and required-argument behavior should prevent most invalid execution paths.
### Telemetry
Executing `/rename-tab <name>` should emit the same slash-command acceptance telemetry as other handled static slash commands.
The existing tab rename telemetry should remain meaningful: direct-set execution should count as setting a custom tab name when the resulting name differs from the current display/custom title.
## Success criteria
1. `/rename-tab` appears in the slash-command menu in standard terminal input when static slash commands are available.
2. `/rename-tab` appears in Warp Agent and Cloud Agent input surfaces where static slash commands can rename the enclosing active tab.
3. `/rename-tab` does not appear in inputs that cannot execute Warp static slash commands.
4. Selecting `/rename-tab` from the slash-command menu inserts `/rename-tab ` instead of executing immediately.
5. Executing `/rename-tab` with no argument does not open the inline rename editor.
6. Executing `/rename-tab deploy` immediately sets the active tab's custom name to `deploy` without opening the editor.
7. Executing `/rename-tab API server` preserves the internal space and sets the active tab's custom name to `API server`.
8. Executing `/rename-tab    ` does not clear the active tab's custom name.
9. Direct-set execution updates the visible tab label immediately in horizontal tabs.
10. Direct-set execution updates the visible tab label immediately in vertical tabs.
11. In a split-pane tab, the command renames the containing tab, not an individual pane.
12. In an agent or cloud-agent tab, the command renames the tab, not the underlying conversation or run.
13. Names set through `/rename-tab` can be reset through the existing reset-tab-name UI.
14. Custom names set through `/rename-tab` persist anywhere existing custom tab names persist.
15. The invoking input is not sent to the shell, terminal PTY, or agent as literal `/rename-tab` text when handled as a slash command.
16. Slash-command acceptance telemetry is emitted for successful command execution.
17. Existing double-click and context-menu rename flows continue to work unchanged.
## Validation
- Unit test slash-command registration to ensure `/rename-tab` is unique, has a required argument, and does not execute on selection.
- Unit test slash-command parsing for `/rename-tab`, `/rename-tab name`, `/rename-tab multi word name`, and empty argument input.
- Unit test direct-set behavior against the workspace or pane-group title state: setting a non-empty name, preserving internal spaces, trimming surrounding whitespace, and not mutating other tabs.
- Manual validation with horizontal tabs: select `/rename-tab` from the menu, confirm it inserts the command and waits for a name.
- Manual validation with horizontal tabs: execute `/rename-tab My Tab` and confirm the active tab label updates without opening the editor.
- Manual validation with vertical tabs: repeat the direct-set flow and confirm the visible vertical-tabs label updates in the active vertical-tabs mode.
- Manual validation with split panes: focus different panes in the same tab and confirm `/rename-tab <name>` renames the tab only.
- Manual validation with a Warp Agent tab: execute `/rename-tab Agent Work` and confirm the tab label changes while the conversation title/history does not.
- Manual validation with a Cloud Agent tab if available: execute `/rename-tab Cloud Work` and confirm the tab label changes while cloud-agent metadata does not.
- Regression validation: double-click rename, context-menu rename, and reset tab name still work after using the slash command.
## Open questions
- Should `/rename-tab` be added to CLI-agent rich input even while that input is otherwise restricted to CLI-native passthrough commands plus a small explicit allowlist? This spec assumes no for the first implementation unless the broader CLI-agent input model changes.
- Should direct-set telemetry distinguish slash-command-driven renames from editor-driven renames, or is existing slash-command acceptance telemetry plus existing tab-rename telemetry sufficient?
