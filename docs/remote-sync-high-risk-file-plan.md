# Remote Sync High-Risk File Plan

## Scope

This document expands the high-risk section of [remote-sync-risk-checklist.md](/Users/later0day/Desktop/warp/docs/remote-sync-risk-checklist.md) into per-file merge guidance.

The intent is not to merge now. The intent is to prepare a safe replay plan for the current i18n and language-setting work once the branch is rebased or merged onto the latest `origin/master`.

Prep completed before writing this plan:

- Working branch: `codex/i18n-sync-prep-20260502-043641`
- Repository history is no longer shallow
- `git merge-base HEAD origin/master` resolves again

## app/src/terminal/input.rs

**Upstream behavior changes**

- Upstream added cloud-mode-v2 slash command UI plumbing, including `CloudModeV2SlashCommandView`.
- Upstream added host, harness, and environment selector support tied to cloud mode v2.
- Upstream added selector visibility subscriptions so focus returns to the input editor after menu dismissal.
- Upstream added default-host propagation from workspace metadata and `UserWorkspacesEvent`.
- Upstream split slash-command data source creation into regular and cloud-mode-v2 paths.
- Upstream added explicit helper methods:
  `open_v2_host_selector`, `open_v2_harness_selector`, and `open_v2_environment_selector`.
- Upstream added `InputAction::DismissCloudModeV2SlashCommandsMenu`.
- Upstream changed key-handling paths so slash-command navigation behaves differently while cloud-mode-v2 composition is active.

**Local i18n changes**

- Local change in this file is narrow: the `No active conversation to export` toast in `export_conversation_to_file` now goes through `tr_static`.

**Recommended source of truth**

- Keep upstream behavior wholesale.
- Replay the local toast localization onto the upstream version after the cloud-mode-v2 behavior is in place.

**Manual merge checkpoints**

- `Input::new`
- Slash-command data source construction around the inline slash menu and v2 slash menu creation
- Host and harness selector subscriptions
- `open_v2_host_selector`
- `open_v2_harness_selector`
- `open_v2_environment_selector`
- Arrow-key and selection handling paths gated by `is_cloud_mode_input_v2_composing`
- `export_conversation_to_file`

## app/src/terminal/input/slash_commands/mod.rs

**Upstream behavior changes**

- Upstream introduced `cloud_mode_v2_view` and exports `CloudModeV2SlashCommandView`.
- Upstream expanded slash-command execution to support `/host`, `/harness`, `/environment`, and `/continue-locally`.
- Upstream pulled in agent-conversation and harness types needed by those new command flows.
- Upstream changed `handle_slash_command_model_event`, `handle_slash_commands_menu_event`, and `execute_slash_command` to support the new cloud-mode-v2 command paths.

**Local i18n changes**

- Local change is narrow: the success toast in the `export_to_clipboard` branch is localized via `tr_static`.

**Recommended source of truth**

- Keep upstream slash-command behavior entirely.
- Re-apply the localized export toast after merge.

**Manual merge checkpoints**

- `handle_slash_command_model_event`
- `handle_slash_commands_menu_event`
- `execute_slash_command`
- The `export_to_clipboard` branch
- Any command-specific branches for `/host`, `/harness`, `/environment`, and `/continue-locally`

## app/src/terminal/view.rs

**Upstream behavior changes**

- Upstream added orchestration-pills support through `OrchestrationPillBar`.
- Upstream added nested cloud-agent exit logic through `can_exit_agent_view_for_terminal_view` and `can_pop_nested_cloud_agent_view`.
- Upstream added `has_pending_ssh_command` for workspace-side pending remote-session detection.
- Upstream added `SwitchAgentViewToConversation` handling.
- Upstream changed remote-server setup state handling, including an `Updating...` state.
- Upstream widened some platform conditionals to include FreeBSD.
- Upstream moved some setup and model initialization ordering around the AI context model and pill bar construction.

**Local i18n changes**

- Local changes are broad but presentation-focused:
  agent header label, detected-file context menu labels, block context menu labels, prompt context menu labels, pane split labels, session-sharing menu labels, and several low-frequency tooltips.
- Local changes also added `ctx`/`app` threading into some helper calls so menu labels can be localized.

**Recommended source of truth**

- Keep upstream behavior and control flow.
- Re-apply local i18n substitutions on top of the upstream menu and header structures.
- Do not preserve local helper signatures if upstream moved or replaced the corresponding call path; instead, re-thread `ctx` only where still necessary.

**Manual merge checkpoints**

- Agent header / exit-agent-view UI around the `"for terminal"` button
- Orchestration-pill-bar creation and subscription
- `can_exit_agent_view_for_terminal_view`
- `has_pending_ssh_command`
- Detected-file context menu block
- Block selection context menu construction
- Prompt context menu construction
- `session_sharing_context_menu_items` call sites
- `SwitchAgentViewToConversation`

## app/src/workspace/view.rs

**Upstream behavior changes**

- Upstream added `AppExecutionMode::can_show_onboarding()` guards to suppress onboarding in headless contexts.
- Upstream changed remote-server event handling to refresh active session state when setup fails.
- Upstream changed conversation-fork handling with `has_initial_query` to suppress misleading hints during immediate follow-ups.
- Upstream changed `update_active_session` to recognize pending SSH remote-session transitions through `has_pending_ssh_command`.
- Upstream adjusted terminal panel background rendering to fix the horizontal-tabs darkening issue.
- Upstream now updates active session state on `TerminalViewStateChanged`.

**Local i18n changes**

- Local changes in this file are broad and menu-heavy:
  search placeholders, toolbar and overflow menu labels, new-session menu items, app menu items mirrored in workspace menus, and many toast messages.
- Local changes also localized string-based dispatch comparisons for entries like `New worktree config` and `New tab config`.

**Recommended source of truth**

- Keep upstream state-management and rendering behavior.
- Re-apply local text substitutions after that.
- Treat string-based dispatch comparisons as fragile; re-check whether upstream replaced those branches with enum- or action-based routing before replaying localized comparisons.

**Manual merge checkpoints**

- Onboarding guards around `should_show_agent_onboarding`
- Remote-server subscription near `update_active_session`
- `update_active_session`
- Fork flow and `show_fork_toast`
- Terminal background and main-panel rendering
- New-session menu construction
- Toolbar / overflow menu construction
- String comparison branches for `New worktree config` and `New tab config`
- Toast call sites for AI credits, conversation restore, conversation deletion, sampling, sync disable, and changelog/update

## app/src/settings_view/appearance_page.rs

**Upstream behavior changes**

- Upstream only changed platform gating so app-icon related UI skips both Linux and FreeBSD, not just Linux.

**Local i18n changes**

- Local changes here are functional, not cosmetic-only:
  display-language setting UI, new `SetDisplayLanguage` action, a new `language_dropdown`, localized category titles, localized dropdown items, localized default-font labels, and language-change subscriptions that refresh the page controls.
- Local code adds `build_language_dropdown`, `set_display_language`, `update_language_dropdown`, and `update_localized_dropdowns`.

**Recommended source of truth**

- Keep local language-setting feature.
- Preserve upstream FreeBSD gating everywhere app-icon conditionals are checked.
- This file is high risk mainly because the local feature is large, not because upstream changed much.

**Manual merge checkpoints**

- `default_font_label`
- `AppearanceSettingsPageView::new`
- `build_language_dropdown`
- `set_display_language`
- `update_language_dropdown`
- `update_localized_dropdowns`
- Category construction in `ordered_categories`
- All app-icon visibility checks that now exclude FreeBSD

## app/src/settings_view/ai_page.rs

**Upstream behavior changes**

- Upstream merged org and user execute-command denylists into one view with per-row editability.
- Upstream added `command_denylist_tooltip_mouse_state_handles`.
- Upstream changed denylist gating from an all-or-nothing editability check to per-row disabled state.
- Upstream updated allowlist, directory allowlist, and MCP list rendering to propagate disabled state at the item level.
- Upstream now uses `BlocklistAIPermissions::get_org_execute_commands_denylist(...)` to distinguish organization-owned denylist entries.

**Local i18n changes**

- Local changes are large and page-wide:
  localized permission labels, localized permission descriptions, localized voice-input key labels, localized placeholders, localized dropdown items, and `LanguageSettings` subscriptions via `refresh_localized_controls`.
- Local code introduced helpers such as `action_permission_label`, `write_to_pty_dropdown_items`, and `voice_input_toggle_key_label`.

**Recommended source of truth**

- Keep upstream denylist and per-row editability behavior.
- Re-apply local localization helpers and subscriptions on top of the upstream rendering structure.
- Do not revert upstream item-level disabled and tooltip handling back to older global editability checks.

**Manual merge checkpoints**

- `AISettingsPageView::new`
- `refresh_localized_controls`
- Permission dropdown item builders and voice-input dropdown builders
- Placeholder initialization for command and path inputs
- `AgentsWidget` sections around command denylist, command allowlist, directory allowlist, and MCP lists
- Any direct use of `.display_name()` or hardcoded labels in dropdowns

## app/src/ai/execution_profiles/editor/mod.rs

**Upstream behavior changes**

- Upstream formats context-window numeric values using `thousands::Separable`.
- Upstream replaced the old single denylist tooltip handle model with per-row `command_denylist_tooltip_mouse_state_handles`.
- Upstream updates mouse-state handles when profile permissions change.
- Upstream loosened gating around the command-denylist section so row-level behavior can decide editability instead of the old all-or-nothing page-level gate.

**Local i18n changes**

- Local changes are substantial:
  localized pane title, localized upgrade footer, localized permission labels and descriptions, localized dropdown item builders, and `LanguageSettings`-driven `refresh_localized_controls`.

**Recommended source of truth**

- Keep upstream numeric formatting and denylist row-state behavior.
- Replay local localization on top of the upstream control structure.
- Do not restore the older single-tooltip-handle model.

**Manual merge checkpoints**

- `render_upgrade_footer`
- `ExecutionProfileEditorView::new`
- `refresh_localized_controls`
- `update_mouse_state_handles`
- Context-window parse and format paths
- Permission dropdown setup for apply-diffs, read-files, execute-commands, write-to-pty, computer-use, and ask-user-question

## app/src/ai/execution_profiles/editor/ui_helpers.rs

**Upstream behavior changes**

- Upstream formats context-window min/max labels with thousands separators.
- Upstream changed list-section plumbing so disabled state and tooltip state live on each `InputListItem`.
- Upstream rebuilt `render_command_denylist_section` around organization-owned rows with per-row disable and tooltip behavior.
- Upstream changed `render_input_list(...)` call patterns to align with the new item-level state model.

**Local i18n changes**

- Local changes localized nearly every visible label and description in the execution-profile editor helper layer:
  header, models section, permissions section, directory allowlist, command allowlist, command denylist, MCP allowlist, MCP denylist, web-search toggle, and plan-auto-sync toggle.

**Recommended source of truth**

- Keep upstream per-row list-state behavior and new `render_input_list` contract.
- Re-apply local translations against the new helper structure.
- Do not blindly restore older helper signatures or old workspace-override tooltip behavior.

**Manual merge checkpoints**

- `render_header_section`
- `render_models_section`
- `render_permissions_section`
- `render_command_denylist_section`
- `render_plan_auto_sync_toggle`
- `render_web_search_toggle`
- `render_list_section`

## Practical Use

- Resolve these files in the order listed above.
- In each file, apply upstream behavior first.
- Re-run only the local i18n patch on top of the upstream result.
- After each high-risk file batch, run:
  - `cargo fmt --all`
  - `cargo check -p warp`
  - `cargo test -p warp i18n::tests::all_i18n_keys_have_non_empty_catalog_entries`
