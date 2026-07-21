# Spec: Add `/fast-forward` slash command to GUI and TUI

Task: [APP-4901](https://linear.app/warpdotdev/issue/APP-4901/add-fast-forward-slash-command-to-gui-and-tui-toggle-cmdshifti-dynamic)
Target repository: `warpdotdev/warp`
Base commit researched: `f0e3db9cd798663eadb2295e95dd411d65c1e3aa`
Requester: `<@U092DE4RP1A>`
Originating thread: [Slack](https://warpdotdev.slack.com/archives/C0BDQDW8V5E/p1784595948410339)
Estimate: M (3)

## PRODUCT

### Summary

Add a static `/fast-forward` slash command to the local GUI Agent View and the TUI. It must perform the same per-conversation toggle as the existing `cmd+shift+i` / `ctrl+shift+i` `terminal:toggle_autoexecute_mode` action. The GUI Agent Shortcuts overlay must describe the action dynamically as `toggle on` when fast-forward is currently off and `toggle off` when it is currently on.

This PR also removes the TUI's current force-enabled default for newly created conversations. New TUI conversations must start with the normal `RespectUserSettings`/off override; `/fast-forward` is then the explicit way to enable `RunToCompletion`.

### Key design choices

- Reuse `TerminalAction::ToggleAutoexecuteMode` and `ConversationSelection` state rather than introducing a second toggle or a new persisted setting.
- Make the command non-cloud/per-conversation: hide it from cloud/ambient GUI conversations, keep the existing cloud action no-op/locked behavior, and do not mutate global AI settings.
- Include the TUI force-enable removal in this same PR. PR #13886 is unrelated execution-profile work and is not a dependency.

### Behavior

1. **Command registration and availability**
   - `/fast-forward` appears in the static slash-command registry with a stable description such as `Toggle fast forward`, no argument, and no prompt text.
   - In the GUI it is available only when AI is enabled, Agent View is active, a conversation is active, and the conversation is not a cloud/ambient agent. It is not shown in cloud-mode slash menus.
   - In the TUI it is available wherever the existing TUI slash-command data source exposes non-cloud Agent conversations. The command takes no argument; `/fast-forward anything` is not treated as this command.
   - The TUI slash-command row appends `(currently on)` or `(currently off)` in green based on the selected conversation's current override.
   - Accepting the row or submitting the exact command executes immediately and records the normal static-slash-command acceptance telemetry.

2. **Toggle semantics**
   - If the selected conversation's `autoexecute_override` is `RespectUserSettings` (off), `/fast-forward` changes it to `RunToCompletion` (on).
   - If it is `RunToCompletion` (on), `/fast-forward` changes it to `RespectUserSettings` (off).
   - The slash command, GUI fast-forward button, and `cmd+shift+i`/`ctrl+shift+i` action all read and mutate the same selected-conversation state.
   - The command must not add a new global preference or call settings save APIs. Existing per-conversation override persistence performed by the shared history model remains the single state path; no additional persistence mechanism is introduced.

3. **TUI defaults**
   - The initial TUI conversation, every `/new`/`/agent` conversation, and replacement conversations created after removal/clear start with `RespectUserSettings` (off), not `RunToCompletion`.
   - Selecting/restoring an existing conversation preserves that conversation's existing override.
   - Toggling a newly created conversation on and then off returns it to `RespectUserSettings`.

4. **Dynamic GUI hint**
   - The Agent Shortcuts overlay row for the existing fast-forward keybinding changes its action text to exactly `toggle on` while the selected conversation is off and exactly `toggle off` while it is on.
   - The row updates after toggling via slash command, keyboard shortcut, footer button, selecting a different conversation, creating a new conversation, or receiving the corresponding history/selection event; it must not display stale text.
   - The row remains hidden in cloud zero-state contexts under the current shortcut-overlay rules.

5. **TUI warping indicator**
   - While a response is in progress, the warping row includes a right-aligned `▶▶ Fast forward on` or `▶▶ Fast forward off` indicator followed by the existing `Ctrl + C to stop` hint.
   - The indicator reflects the selected conversation's current override.
   - After the override changes, the fast-forward indicator is green for three seconds and then returns to the normal muted color.

6. **Safety and edge behavior**
   - Direct dispatch of `TerminalAction::ToggleAutoexecuteMode` retains the existing cloud no-op/locked behavior and pending-blocked-action handling.
   - `/fast-forward` does not submit an AI prompt, create a conversation, change the active model, or alter unrelated queued-prompt/long-running-command state.
   - Empty/new-conversation state and conversation removal/replacement remain usable after the toggle; no stale selection may cause the hint or command to target another terminal surface.

## TECH

### Context

- Static command definitions, registry population, and availability semantics are in `app/src/search/slash_command_menu/static_commands/commands.rs:411-439,581-643` and `app/src/search/slash_command_menu/static_commands/mod.rs:18-122` (`Availability` requires all listed context bits).
- GUI command availability is computed by `app/src/terminal/input/slash_commands/data_source/gui.rs:38-218`; TUI availability and filtering are in `app/src/terminal/input/slash_commands/data_source/tui.rs:20-104`. Both surfaces share parsing/matching in `app/src/terminal/input/slash_commands/data_source/core.rs`.
- Shared slash-command classification and execution entry points are `app/src/terminal/input/slash_commands/mod.rs:132-174,305-489`; TUI classification is the `TuiSlashCommand` enum and `from_static_command` mapping in that module.
- The existing keyboard binding is registered as `terminal:toggle_autoexecute_mode` with `cmdorctrl-shift-I` in `app/src/terminal/view/init.rs:973-980`; the action is `TerminalAction::ToggleAutoexecuteMode` in `app/src/terminal/view/action.rs:355-685`.
- The existing action implementation in `app/src/terminal/view.rs:26951-26970` guards cloud context, accepts a pending blocked action, and delegates to `BlocklistAIContextModel::toggle_pending_query_autoexecute`.
- The shared selection contract is `app/src/ai/blocklist/conversation_selection.rs:60-105,160-179`. GUI selection toggles the selected history conversation in `app/src/ai/blocklist/agent_view/conversation_selection.rs:145-189`; TUI selection toggles either pending new state or the selected history conversation in `crates/warp_tui/src/conversation_selection.rs:229-296`.
- Per-conversation state and its existing write/event path are `app/src/ai/agent/conversation.rs:3639-3653` and `app/src/ai/blocklist/history_model.rs:1275-1286,2444-2456`. `BlocklistAIContextModel` forwards the toggle at `app/src/ai/blocklist/context_model.rs:738-749`.
- The current TUI force-enable behavior is in `crates/warp_tui/src/conversation_selection.rs:43-70,84-101,229-245,258-268`: new conversations pass `true` to `start_new_conversation`, and deferred replacements seed `RunToCompletion`. The existing regression test is `crates/warp_tui/src/conversation_selection_tests.rs:325-354`.
- The GUI dynamic shortcut overlay is `app/src/ai/blocklist/agent_view/shortcuts/mod.rs:108-223`; it currently renders `toggle auto-accept` at line 216. The overlay is constructed by `app/src/terminal/input/agent.rs:280-350`, and `Input` history/context subscriptions that trigger redraws are in `app/src/terminal/input.rs:2731-2860`.
- TUI slash-command dispatch is `crates/warp_tui/src/terminal_session_view.rs:2407-2574`; the command-to-executor mapping must be exhaustive. Existing classifier tests are `app/src/terminal/input/slash_commands/mod_tests.rs`, and TUI menu/render tests are `crates/warp_tui/src/slash_commands_tests.rs`.

### Design alternatives

- **Duplicate toggle logic in GUI and TUI:** rejected. Separate mutations would diverge on new-conversation state, persistence, cloud guards, or event redraws. Extend the shared `ConversationSelection`/`BlocklistAIContextModel` path and only add surface-specific dispatch arms.
- **Add a new settings-backed `fast_forward` preference:** rejected. The request is a per-session/per-conversation control, and the existing `AIConversationAutoexecuteMode` plus history event already models the required state. Do not add an `AISettings` field or settings UI.
- **Make `/fast-forward` a prompt prefix:** rejected. It is an immediate local action, like the existing keybinding/button, and must never be queued or sent to the model.
- **Depend on PR #13886 for the TUI default change:** rejected after verification. #13886 is file-backed execution-profile work. The requester explicitly resolved this by requiring the force-enable removal in this same PR.
- **Store display state separately from the conversation override:** rejected. Both the TUI slash-row suffix and warping indicator derive on/off from `ConversationSelection`; only the warping indicator's three-second color feedback is transient view state.

### Proposed changes

1. Add a `/fast-forward` `StaticCommand` to the registry with no argument, stable description, AI/active-Agent/non-cloud availability, and no new feature flag. Add registry/availability tests.
2. Add a `FastForward` variant to `TuiSlashCommand`, map it in `from_static_command`, and execute it in `TuiTerminalSessionView` by calling the existing context-model toggle, clearing the input, and recording acceptance telemetry. Add classifier and execution coverage.
3. Add a GUI execution arm in `Input::execute_slash_command` that dispatches `TerminalAction::ToggleAutoexecuteMode`. Keep command availability filtering responsible for hiding cloud commands; retain the action's existing cloud no-op as the direct-dispatch safety net.
4. Remove TUI force enablement in all new-conversation paths:
   - pass `false`/`RespectUserSettings` when creating the initial conversation, `/new`/`/agent` replacement, and deferred replacement;
   - preserve existing override state for selected existing conversations;
   - update the current force-enable regression test and add tests for initial, replacement, and on→off transitions.
5. Extend `AgentShortcutsViewContext`/`render_agent_shortcuts_view` to receive the current `pending_query_autoexecute_override` state and render exactly `toggle on` or `toggle off`. Add redraw triggers for `UpdatedAutoexecuteOverride` and relevant selection/new-conversation events so the label tracks the selected state.
6. Extend TUI inline-menu rows with an optional green trailing status and derive `/fast-forward`'s `(currently on/off)` suffix from `ConversationSelection`.
7. Extend the TUI warping row with a right-aligned fast-forward status. Use `UpdatedAutoexecuteOverride` to start or restart a cancelable three-second success-color timer, then return to the normal muted style.
8. Preserve the existing conversation-history write/event path and do not write settings. Keep cloud behavior unchanged: cloud commands are filtered out and direct `ToggleAutoexecuteMode` remains a safe no-op.

### Open questions resolved

- **Does “per-session” mean no persistence at all?** No new settings persistence is allowed; the existing selected-conversation override/history write path is retained because it is the current source of truth for the button and keyboard action. The feature must not introduce a global preference.
- **What is the prerequisite represented by the request's PR reference?** The cited #13886 is unrelated. Per requester clarification, this PR itself includes the minimal TUI force-enable removal; no stacking or external prerequisite is required.
- **Where does dynamic hint text apply?** The triaged code path identifies the GUI Agent Shortcuts overlay. The TUI gets the command row and toggle behavior, but no new TUI shortcut overlay is added.
- **What happens in cloud/ambient conversations?** Fast-forward is conceptually locked on there. The GUI slash command is hidden via `NOT_CLOUD_AGENT`; direct action dispatch remains the existing no-op/locked path.
- **What command arguments are accepted?** None. An exact `/fast-forward` executes immediately; text after a space does not match the no-argument static command and must not be silently discarded.

## Validation & verification criteria

All criteria must pass before the implementation PR is marked ready.

1. **Registry contract:** `commands_tests` verifies exactly one `/fast-forward` entry, stable description/icon, no argument, no new feature flag, and availability requiring AI Agent View + active conversation while excluding cloud context.
2. **Shared parsing:** `app/src/terminal/input/slash_commands/mod_tests.rs` (or equivalent) verifies exact `/fast-forward` classification on both GUI/TUI data sources, immediate selection behavior, and rejection of `/fast-forward trailing text` as the no-argument command.
3. **GUI execution equivalence:** a regression test exercises GUI slash-command execution and asserts it dispatches the same `TerminalAction::ToggleAutoexecuteMode` path as the existing keybinding, clears only the command input, records static-command acceptance, and does not submit an AI prompt or mutate `AISettings`.
4. **GUI toggle transition:** a model/action test starts with `RespectUserSettings`, executes `/fast-forward`, asserts `RunToCompletion`, executes it again, and asserts `RespectUserSettings`; the selected conversation ID and unrelated settings remain unchanged.
5. **TUI command mapping/execution:** `app/src/terminal/input/slash_commands/mod_tests.rs` covers `TuiSlashCommand::FastForward`; a TUI session test invokes the command twice and asserts the selected conversation changes off→on→off through `pending_query_autoexecute_override`.
6. **TUI defaults regression:** `crates/warp_tui/src/conversation_selection_tests.rs` proves the initial TUI conversation, `/new`/`/agent` replacement, and deferred replacement all start with `RespectUserSettings` rather than `RunToCompletion`, while selecting an existing conversation preserves its override. The old `tui_new_conversation_preserves_pending_autoexecute_override` assertion must be replaced with the off-by-default expectation.
7. **Dynamic shortcut copy:** a deterministic shortcut-rendering/helper test asserts `toggle on` for off and `toggle off` for on. Event-driven coverage proves the GUI overlay refreshes after `UpdatedAutoexecuteOverride`, selection changes, new conversation creation, and slash/keyboard/button toggles without stale text.
8. **Cloud and safety behavior:** availability tests prove `/fast-forward` is absent from GUI cloud/ambient command results; an action test proves direct `ToggleAutoexecuteMode` remains a no-op/locked path in cloud context and still accepts pending blocked actions in local context.
9. **No collateral behavior:** existing slash-command, conversation-selection, history toggle/persistence, GUI footer fast-forward, and TUI session tests pass; no global preference or settings serialization changes are introduced.
10. **TUI visual proof:** run the built TUI, open the slash menu, capture `/fast-forward` with its green current-state suffix, execute it, and capture the warping row's on/off indicator in both its transient green and normal muted states. Attach the screenshot evidence to the task/PR; do not commit media.
11. **GUI visual proof:** run the built GUI in a local Agent conversation, capture `/fast-forward` in the slash menu, execute it, and capture the Agent Shortcuts overlay first showing `toggle on` and then `toggle off` after the second toggle (also verify the reverse transition via the existing shortcut or footer button). Attach screenshots through the UI verification workflow; do not commit media.
12. **Formatting, lint, and presubmit:** `./script/format` passes, the repository's required clippy checks pass, and `./script/presubmit` completes successfully on the final implementation branch.
