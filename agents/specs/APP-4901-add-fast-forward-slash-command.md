# Spec: Add `/auto-approve` slash command to TUI

Task: [APP-4901](https://linear.app/warpdotdev/issue/APP-4901/add-fast-forward-slash-command-to-gui-and-tui-toggle-cmdshifti-dynamic)
Target repository: `warpdotdev/warp`
Base commit researched: `f0e3db9cd798663eadb2295e95dd411d65c1e3aa`
Requester: `<@U092DE4RP1A>`
Originating thread: [Slack](https://warpdotdev.slack.com/archives/C0BDQDW8V5E/p1784595948410339)
Estimate: M (3)

## PRODUCT

### Summary

Add a static `/auto-approve` slash command to the TUI. It performs the same per-conversation toggle modeled by the existing shared autoexecute override, without adding or renaming a GUI slash command, shortcut, control, or state.

This PR also removes the TUI's current force-enabled default for newly created conversations. New TUI conversations must start with the normal `RespectUserSettings`/off override; `/auto-approve` is then the explicit way to enable `RunToCompletion`.

### Key design choices

- Reuse `TerminalAction::ToggleAutoexecuteMode` and `ConversationSelection` state rather than introducing a second toggle or a new persisted setting.
- Register the command only in TUI settings mode and do not mutate global AI settings.
- Include the TUI force-enable removal in this same PR. PR #13886 is unrelated execution-profile work and is not a dependency.

### Behavior

1. **Command registration and availability**
   - `/auto-approve` appears in the static slash-command registry with a stable description such as `Toggle auto approve`, no argument, and no prompt text.
   - In the TUI it is available wherever the existing TUI slash-command data source exposes non-cloud Agent conversations. The command takes no argument; `/auto-approve anything` is not treated as this command.
   - It is not registered in GUI settings mode.
   - The TUI slash-command row appends `(currently on)` or `(currently off)` in green based on the selected conversation's current override.
   - Accepting the row or submitting the exact command executes immediately and records the normal static-slash-command acceptance telemetry.

2. **Toggle semantics**
   - If the selected conversation's `autoexecute_override` is `RespectUserSettings` (off), `/auto-approve` changes it to `RunToCompletion` (on).
   - If it is `RunToCompletion` (on), `/auto-approve` changes it to `RespectUserSettings` (off).
   - The slash command, TUI chip, and `ctrl+shift+i` action all read and mutate the same selected-conversation state.
   - The command must not add a new global preference or call settings save APIs. Existing per-conversation override persistence performed by the shared history model remains the single state path; no additional persistence mechanism is introduced.

3. **TUI defaults**
   - The initial TUI conversation, every `/new`/`/agent` conversation, and replacement conversations created after removal/clear start with `RespectUserSettings` (off), not `RunToCompletion`.
   - Selecting/restoring an existing conversation preserves that conversation's existing override.
   - Toggling a newly created conversation on and then off returns it to `RespectUserSettings`.

4. **TUI warping indicator**
   - While a response is in progress, the warping row includes a right-aligned `▶▶ Auto approve on` or `▶▶ Auto approve off` indicator followed by the existing `Ctrl + C to stop` hint.
   - The indicator reflects the selected conversation's current override.
   - The indicator is clickable and toggles the same selected-conversation state without transient success-color feedback.
   - `ctrl+shift+i` toggles auto approve in the TUI.
   - After toggling through `/auto-approve` or `ctrl+shift+i`, the indicator is green for three seconds and then returns to the normal muted color.

5. **Safety and edge behavior**
   - Existing GUI shortcut, footer, and command behavior remains unchanged.
   - `/auto-approve` does not submit an AI prompt, create a conversation, change the active model, or alter unrelated queued-prompt/long-running-command state.
   - Empty/new-conversation state and conversation removal/replacement remain usable after the toggle; no stale selection may cause the hint or command to target another terminal surface.

## TECH

### Context

- Static command definitions, registry population, and availability semantics are in `app/src/search/slash_command_menu/static_commands/commands.rs:411-439,581-643` and `app/src/search/slash_command_menu/static_commands/mod.rs:18-122` (`Availability` requires all listed context bits).
- TUI availability and filtering are in `app/src/terminal/input/slash_commands/data_source/tui.rs:20-104`; shared parsing/matching is in `app/src/terminal/input/slash_commands/data_source/core.rs`.
- Shared slash-command classification and execution entry points are `app/src/terminal/input/slash_commands/mod.rs:132-174,305-489`; TUI classification is the `TuiSlashCommand` enum and `from_static_command` mapping in that module.
- The existing GUI keyboard binding and action remain unchanged; the TUI registers its own `ctrl+shift+i` action against the shared conversation-selection state.
- The shared selection contract is `app/src/ai/blocklist/conversation_selection.rs:60-105,160-179`. GUI selection toggles the selected history conversation in `app/src/ai/blocklist/agent_view/conversation_selection.rs:145-189`; TUI selection toggles either pending new state or the selected history conversation in `crates/warp_tui/src/conversation_selection.rs:229-296`.
- Per-conversation state and its existing write/event path are `app/src/ai/agent/conversation.rs:3639-3653` and `app/src/ai/blocklist/history_model.rs:1275-1286,2444-2456`. `BlocklistAIContextModel` forwards the toggle at `app/src/ai/blocklist/context_model.rs:738-749`.
- The current TUI force-enable behavior is in `crates/warp_tui/src/conversation_selection.rs:43-70,84-101,229-245,258-268`: new conversations pass `true` to `start_new_conversation`, and deferred replacements seed `RunToCompletion`. The existing regression test is `crates/warp_tui/src/conversation_selection_tests.rs:325-354`.
- TUI slash-command dispatch is `crates/warp_tui/src/terminal_session_view.rs:2407-2574`; the command-to-executor mapping must be exhaustive. Existing classifier tests are `app/src/terminal/input/slash_commands/mod_tests.rs`, and TUI menu/render tests are `crates/warp_tui/src/slash_commands_tests.rs`.

### Design alternatives

- **Add a second TUI state path:** rejected. The TUI slash command, chip, and shortcut must use the shared `ConversationSelection` state.
- **Add a new settings-backed `auto_approve` preference:** rejected. The request is a per-session/per-conversation control, and the existing `AIConversationAutoexecuteMode` plus history event already models the required state. Do not add an `AISettings` field or settings UI.
- **Make `/auto-approve` a prompt prefix:** rejected. It is an immediate local action, like the existing keybinding/button, and must never be queued or sent to the model.
- **Depend on PR #13886 for the TUI default change:** rejected after verification. #13886 is file-backed execution-profile work. The requester explicitly resolved this by requiring the force-enable removal in this same PR.
- **Store display state separately from the conversation override:** rejected. Both the TUI slash-row suffix and warping indicator derive on/off from `ConversationSelection`; only the warping indicator's three-second color feedback is transient view state.

### Proposed changes

1. Add a TUI-only `/auto-approve` `StaticCommand` with no argument, stable description, AI/active-Agent/non-cloud availability, and no new feature flag. Register it only for `SettingsMode::Tui` and add registry/availability tests.
2. Add an `AutoApprove` variant to `TuiSlashCommand`, map it in `from_static_command`, and execute it in `TuiTerminalSessionView` by calling the existing context-model toggle, clearing the input, and recording acceptance telemetry. Add classifier and execution coverage.
3. Remove TUI force enablement in all new-conversation paths:
   - pass `false`/`RespectUserSettings` when creating the initial conversation, `/new`/`/agent` replacement, and deferred replacement;
   - preserve existing override state for selected existing conversations;
   - update the current force-enable regression test and add tests for initial, replacement, and on→off transitions.
4. Extend TUI inline-menu rows with an optional green trailing status and derive `/auto-approve`'s `(currently on/off)` suffix from `ConversationSelection`.
5. Extend the TUI warping row with a right-aligned, clickable auto-approve status and register `ctrl+shift+i` for the same toggle. Slash-command and keyboard toggles start or restart a cancelable three-second success-color timer; click toggles clear any active feedback and retain the normal muted/hover style.
6. Preserve the existing conversation-history write/event path and do not write settings. Do not modify GUI shortcuts, controls, or slash-command execution.

### Open questions resolved

- **Does “per-session” mean no persistence at all?** No new settings persistence is allowed; the existing selected-conversation override/history write path is retained because it is the current source of truth for the button and keyboard action. The feature must not introduce a global preference.
- **What is the prerequisite represented by the request's PR reference?** The cited #13886 is unrelated. Per requester clarification, this PR itself includes the minimal TUI force-enable removal; no stacking or external prerequisite is required.
- **What happens in the GUI?** Nothing changes. `/auto-approve` is absent from the GUI registry, and existing GUI autoexecute controls retain their current copy and behavior.
- **What command arguments are accepted?** None. An exact `/auto-approve` executes immediately; text after a space does not match the no-argument static command and must not be silently discarded.

## Validation & verification criteria

All criteria must pass before the implementation PR is marked ready.

1. **Registry contract:** `commands_tests` verifies exactly one `/auto-approve` entry in TUI mode, no entry in GUI mode, stable description/icon, no argument, no new feature flag, and availability requiring AI Agent View + active conversation while excluding cloud context.
2. **Shared parsing:** `app/src/terminal/input/slash_commands/mod_tests.rs` verifies exact `/auto-approve` TUI classification, immediate selection behavior, and rejection of `/auto-approve trailing text` as the no-argument command.
3. **TUI command mapping/execution:** `app/src/terminal/input/slash_commands/mod_tests.rs` covers `TuiSlashCommand::AutoApprove`; a TUI session test invokes the command twice and asserts the selected conversation changes off→on→off through `pending_query_autoexecute_override`.
4. **TUI defaults regression:** `crates/warp_tui/src/conversation_selection_tests.rs` proves the initial TUI conversation, `/new`/`/agent` replacement, and deferred replacement all start with `RespectUserSettings` rather than `RunToCompletion`, while selecting an existing conversation preserves its override. The old `tui_new_conversation_preserves_pending_autoexecute_override` assertion must be replaced with the off-by-default expectation.
5. **No GUI change:** `/auto-approve` is absent from GUI-mode command registration and the GUI shortcut overlay, footer, and input history subscriptions remain unchanged.
6. **No collateral behavior:** existing slash-command, conversation-selection, history toggle/persistence, and TUI session tests pass; no global preference or settings serialization changes are introduced.
7. **TUI visual proof:** run the built TUI, open the slash menu, capture `/auto-approve` with its green current-state suffix, execute it, and capture the warping row's on/off indicator in both its transient green and normal muted states. Verify `ctrl+shift+i` toggles with the same transient feedback and clicking the indicator toggles without it. Attach the screenshot evidence to the task/PR; do not commit media.
8. **Formatting, lint, and presubmit:** `./script/format` passes, the repository's required clippy checks pass, and `./script/presubmit` completes successfully on the final implementation branch.
