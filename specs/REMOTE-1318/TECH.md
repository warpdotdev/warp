# REMOTE-1318: Tech spec

See `PRODUCT.md` for user-facing behavior.

## Context

The command denylist controls which commands the agent must ask permission to execute. Two sources can contribute entries: the organization (workspace) and the user's execution profile.

**Current data flow:**
- `AiAutonomySettings.execute_commands_denylist: Option<Vec<AgentModeCommandExecutionPredicate>>` — org override, from `workspace.rs (663-670)`.
- `AIExecutionProfile.command_denylist: Vec<AgentModeCommandExecutionPredicate>` — per-profile user list, from `execution_profiles/mod.rs (233)`.
- `get_execute_commands_denylist_for_profile()` in `permissions.rs (395-413)` — resolves the effective list. Currently `unwrap_or_else`: org **replaces** user.
- `is_command_denylist_editable()` in `ai.rs (1730-1736)` — returns `false` when org override exists, disabling both the editor and all row remove buttons.

**Current UI rendering:**
- `render_list_section()` in `ui_helpers.rs (640-691)` — generic list renderer for the profile editor. Passes a single `is_editable` bool; when false, wraps the **entire** section in a workspace tooltip via `wrap_disabled_with_workspace_override_tooltip`.
- `render_command_denylist()` in `ai_page.rs (4483-4513)` — legacy settings page denylist. Same pattern: passes `!is_command_denylist_editable(app)` as a single `disabled` flag.
- `render_input_list()` in `settings_page.rs (1013-1056)` — renders the editor + item rows. Takes a single `disabled: bool` for all rows.
- `InputListItem` in `settings_page.rs (1003-1007)` — `{ item, mouse_state_handle, on_remove_action }`. No per-item disabled state.
- Mouse state handles are stored as `Vec<MouseStateHandle>` per list (one per row, for close buttons). Recreated when the list changes.

**Other callers of `render_input_list`** (must stay backward-compatible): `ai_page.rs` (allowlist, directory allowlist, MCP lists), `ui_helpers.rs` (all list types via `render_list_section`), `update_environment_form.rs` (environment setup commands).

## Proposed changes

### 1. Merge denylist in `permissions.rs`

Change `get_execute_commands_denylist_for_profile()`:
```
When org override is Some(org_list):
  merged = org_list
  for each item in user_profile.command_denylist:
    if merged does not contain item:
      merged.push(item)
  return merged
When org override is None:
  return user_profile.command_denylist (unchanged)
```

Add `pub fn get_org_execute_commands_denylist(ctx: &AppContext) -> Vec<AgentModeCommandExecutionPredicate>` — returns just the org entries (or empty vec). Used by rendering code to determine which rows are org-owned.

### 2. Per-item disabled state in `InputListItem` (`settings_page.rs`)

Add one field:
- `is_disabled: bool` — whether the remove button is disabled and text uses disabled color.

Remove the `disabled: bool` parameter from `render_input_list`. Each row uses `item.is_disabled` instead.

Keep `wrap_disabled_with_workspace_override_tooltip` in `ui_helpers.rs` — it is not needed by `render_input_list`. Tooltip wrapping is handled at the call site (see sections 4 and 5 below).

Update all 6 callers of `render_input_list`:
- Non-denylist callers: set `is_disabled` uniformly based on the existing global disabled logic.
- Denylist callers: set per-item `is_disabled` based on org membership.

### 3. Always enable denylist editor

`is_command_denylist_editable()` in `ai.rs`: remove the `has_override_for_execute_commands_denylist()` check. Return `self.is_any_ai_enabled(app)` only.

In `editor/mod.rs`, `update_all_editor_interaction_states()` (line 1269): change the denylist editor line to enable whenever AI is on (drop the `&& !ai_autonomy_settings.has_override_for_execute_commands_denylist()` condition).

In `ai_page.rs`, two handlers update the denylist editor state:
- `TeamsChanged` handler (line 474): same change.
- `IsAnyAIEnabled` handler (line 891): same change.

### 4. Profile editor denylist rendering (`ui_helpers.rs` + `editor/mod.rs`)

Add `command_denylist_tooltip_mouse_state_handles: Vec<MouseStateHandle>` to `ExecutionProfileEditorView`. Create alongside existing `command_denylist_mouse_state_handles` in `update_mouse_state_handles()`, one per merged-list item (used only for org rows).

Update `render_command_denylist_section()` in `ui_helpers.rs`:
- Get org denylist via `BlocklistAIPermissions::get_org_execute_commands_denylist(app)`
- Build rows directly using `render_alternating_color_list_item` (not via `render_list_section`) so each disabled org row can be individually wrapped with `wrap_disabled_with_workspace_override_tooltip`, which stays in `ui_helpers.rs`
- Always show the editor (regardless of org override)

`render_list_section()` is unchanged — non-denylist lists still use its existing `is_editable` + whole-list tooltip pattern.

### 5. Legacy settings page denylist rendering (`ai_page.rs`)

Add `command_denylist_tooltip_mouse_state_handles: Vec<MouseStateHandle>` to `AISettingsPageView`.

Update `render_command_denylist()`:
- Get org denylist
- Build `InputListItem`s with per-item `is_disabled` based on org membership
- Wrap each disabled row in `wrap_disabled_with_workspace_override_tooltip` (imported from `crate::ai::execution_profiles::editor::ui_helpers`)
- Always pass the editor

Update mouse state handle creation in `new()` and `AISettingsChangedEvent::AgentModeCommandExecutionDenylist` handler to size both handle Vecs to the merged list.

### 6. Profile data event handling

In `ExecutionProfileEditorView`, subscribe to `UserWorkspacesEvent::TeamsChanged` to refresh mouse state handles and denylist rendering when the org override changes at runtime.

## Testing and validation

**Unit tests** — extend `permissions_test.rs`:
- Test that `get_execute_commands_denylist_for_profile` returns the union when org override is set (PRODUCT.md invariant 1).
- Test deduplication: user entry matching org entry appears once (invariant 2).
- Test merged list is used for execution checks (invariant 3).
- Test cross-profile: org denylist applies to all profiles (invariant 4).
- Test no-override path is unchanged (invariant 5).
- Test empty org override (`Some([])`) still allows user entries (invariant 15).
- Test `get_org_execute_commands_denylist` returns the right entries.

**Manual verification:**
- With an org denylist override active: verify the editor is enabled, org rows show disabled × and tooltip, user rows are removable.
- Without an org override: verify unchanged behavior.
- Add a user entry that duplicates an org entry: verify it doesn't appear twice.
- Remove a user entry: verify it disappears and remaining rows render correctly.

**Presubmit:** `cargo fmt` + `cargo clippy` must pass.
