# TECH.md - Add optional confirmation before closing tabs

Issue: https://github.com/warpdotdev/warp/issues/10995
Product spec: `specs/GH10995/product.md`

## Context

Tab close entry points already converge on workspace actions:

- `app/src/tab.rs:518-553` builds horizontal tab context-menu close actions for
  "Close tab", "Close other tabs", and "Close Tabs to the Right".
- `app/src/tab.rs:1297-1302` dispatches `WorkspaceAction::CloseTab(tab_index)`
  from the horizontal tab close button.
- `app/src/tab.rs:1944-1947` dispatches `WorkspaceAction::CloseTab(tab_index)`
  from horizontal tab middle-click.
- `app/src/workspace/view/vertical_tabs.rs:2420-2425` mirrors middle-click close
  for vertical tabs.
- `app/src/workspace/view/vertical_tabs.rs:2548-2550` dispatches
  `WorkspaceAction::CloseTab(tab_index)` from the vertical tab close button.
- `app/src/workspace/view/vertical_tabs.rs:2742-2749` dispatches
  `WorkspaceAction::CloseTabGroup(group_id)` from the vertical tab-group close
  button.
- `app/src/workspace/view.rs:9539-9574` builds tab-group context-menu close
  actions for closing all tabs in a group, other tabs, tabs above/left, and tabs
  below/right.
- `app/src/workspace/view.rs:23023-23059` handles `CloseTab`,
  `CloseActiveTab`, `CloseOtherTabs`, `CloseNonActiveTabs`, `CloseTabsRight`,
  `CloseTabsRightActiveTab`, `CloseTabGroup`, `CloseTabsOutsideGroup`,
  `CloseTabsAboveGroup`, and `CloseTabsBelowGroup`.

The actual close logic is centralized in `Workspace::close_tabs`:

- `app/src/workspace/view.rs:11608-11706` collects the tab indices, checks
  existing confirmations, cancels tab rename state, and removes tabs in reverse
  order.
- `app/src/workspace/view.rs:11618-11627` shows the existing shared-session
  confirmation when a shared tab is being closed.
- `app/src/workspace/view.rs:11630-11695` builds and shows the existing
  running-process / unsaved-state quit warning.
- `app/src/workspace/view.rs:11723-11729` treats last-tab close as a
  window-close case by passing `skip_confirmation || is_last_tab`.
- `app/src/workspace/view.rs:11745-11795` routes regular bulk close actions
  through the same `close_tabs` helper.
- `app/src/workspace/view.rs:6863-6885` routes "close tab group" through
  `close_tabs` and removes the group after close succeeds.
- `app/src/workspace/view.rs:7233-7270` routes tab-group bulk close actions
  outside, above, and below a group through the same close helpers.
- `app/src/workspace/view.rs:11495-11557` prunes empty tab groups as individual
  tabs are removed.
- `app/src/workspace/view.rs:11548-11562` adds removed tabs to the undo stack
  only after the tab is actually removed.

One implementation caveat: `close_tab_group` has caller-specific completion
behavior after `close_tabs` returns. Any asynchronous general confirmation path
must preserve that behavior instead of only calling `close_tabs` from the dialog
callback.

Relevant existing UI/settings references:

- `app/src/workspace/tab_settings.rs:449-558` defines `TabSettings`, including
  tab appearance settings under `appearance.tabs.*`.
- `app/src/settings_view/appearance_page.rs:127-463` registers settings toggle
  actions that can be exposed through keybindings / command-palette settings
  search.
- `app/src/settings_view/appearance_page.rs:1448-1488` renders the Appearance >
  Tabs settings category.
- `app/src/settings_view/appearance_page.rs:4488-4509` renders the existing
  "Tab close button position" settings row, a nearby placement reference.
- `app/src/workspace/close_session_confirmation_dialog.rs:21-39` already has an
  `OpenDialogSource` enum that identifies the close source.
- `app/src/workspace/close_session_confirmation_dialog.rs:79-114` is specific to
  shared sessions and should not be reused for ordinary tab-close copy.
- `app/src/quit_warning/mod.rs:55-64` models the existing warning dialog for
  running processes, shared sessions, and unsaved state.

## Proposed Changes

### 1. Add a tab setting

Add a boolean setting to `TabSettings`:

- Rust field: `confirm_before_closing_tabs`
- Generated setting type: `ConfirmBeforeClosingTabs`
- Default: `false`
- Supported platforms: `SupportedPlatforms::ALL`
- Sync behavior: `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`
- TOML path: `appearance.tabs.confirm_before_closing_tabs`
- Description: "Whether to ask for confirmation before closing tabs."

This keeps the setting next to existing tab appearance preferences and preserves
the default behavior for all existing users.

Add unit coverage in `app/src/workspace/tab_settings_tests.rs` for:

- default value is false
- TOML path is `appearance.tabs.confirm_before_closing_tabs`
- hierarchy is `appearance.tabs`
- TOML key is `confirm_before_closing_tabs`

### 2. Add the Settings UI row

Update `app/src/settings_view/appearance_page.rs`:

- Add `AppearancePageAction::ToggleConfirmBeforeClosingTabs`.
- Add `toggle_confirm_before_closing_tabs`.
- Add `ConfirmBeforeClosingTabsWidget`.
- Place the widget in the existing "Tabs" category near the close-button related
  settings, after "Show tab indicators" or near "Tab close button position".
- Label: "Confirm before closing tabs".
- Optional description: "Ask before closing tabs from buttons, shortcuts, and
  tab menus."
- Search terms: "confirm close tab closing tabs accidental".

Add a command-palette toggle through `init_actions_from_parent_view`, matching
other Appearance > Tabs toggles. Suggested visible action text: "confirm before
closing tabs".

Telemetry is optional. If a new telemetry event is not added, make sure existing
`TabOperations` telemetry continues to emit only after tabs actually close, not
when the confirmation dialog is shown.

### 3. Add a general tab-close confirmation dialog

Do not reuse `CloseSessionConfirmationDialog`, because its title, body, primary
button, and "Don't show again" checkbox are shared-session-specific.

Preferred implementation:

- Add a small general tab-close confirmation helper in workspace close logic,
  using the same modal/callback infrastructure as existing warnings.
- The helper receives:
  - `OpenDialogSource`
  - the tab indices that would close
  - `add_to_undo_stack`
  - whether this is a single-tab or bulk close
  - enough continuation context to finish caller-specific close behavior, such
    as removing a tab group after a confirmed `CloseTabGroup`
- Single-tab copy:
  - title: "Close tab?"
  - body: "This tab will be closed."
  - confirm: "Close tab"
  - cancel: "Cancel"
- Bulk copy:
  - title: `Close {count} tabs?`
  - body: "These tabs will be closed."
  - confirm: "Close tabs"
  - cancel: "Cancel"

The dialog should not include a "Don't show again" checkbox in the initial
version. The setting is already the explicit opt-in/out control.

### 4. Preserve confirmation precedence

Update `Workspace::close_tabs` so the order is:

1. Empty close requests return without showing the general confirmation.
2. Last-tab/window-close handling remains owned by `close_tab`.
3. Existing shared-session confirmation.
4. Existing running-process / unsaved-state warning.
5. New general tab-close confirmation, only when
   `TabSettings::as_ref(ctx).confirm_before_closing_tabs` is true.
6. Rename cancellation and tab removal.

The general confirmation must only appear after higher-severity warnings have
decided they do not need to show. This avoids double dialogs and preserves
existing safety semantics.

### 5. Avoid weakening warning checks after the dialog opens

The existing `skip_confirmation: bool` means "skip all confirmation checks". Do
not use that flag blindly when confirming the new general dialog, because risk
state can change while the dialog is open.

Recommended refactor:

- Replace or supplement `skip_confirmation: bool` with a small enum, for
  example:

  ```rust
  enum CloseTabsConfirmationMode {
      Normal,
      SkipAll,
      SkipGeneralTabClose,
  }
  ```

- Existing shared-session and quit-warning confirm callbacks can continue to use
  `SkipAll`, preserving today's behavior.
- The new general dialog confirm callback should use `SkipGeneralTabClose`, so
  it re-runs shared-session and running-process / unsaved-state checks before
  closing, while avoiding an infinite loop back into the same general dialog.

If the implementation keeps the boolean API, add a second explicit
`skip_general_tab_close_confirmation` parameter instead. The important invariant
is that confirming the new general dialog must not bypass higher-severity
warnings if those warnings become relevant before the actual close happens.

For grouped-tabs actions, make sure the confirm path either re-enters the
original close action with `SkipGeneralTabClose` or carries an explicit
post-close continuation. In particular, a confirmed `CloseTabGroup` should still
remove the group state the same way the immediate close path does today.

### 6. Bulk close behavior

Keep the current `tab_indices_vec` collection in `close_tabs`. Use its length to
decide whether to show single or bulk copy.

- If `tab_indices_vec.is_empty()`, return `true` without showing the general
  dialog.
- If the length is 1, use single-tab copy.
- If the length is greater than 1, show one bulk dialog for all tabs.
- Treat tab-group close actions that close more than one tab as bulk actions.
- On confirm, close the same set of tab indices using the existing reverse-order
  removal behavior.

### 7. Tests

Update or add workspace tests in `app/src/workspace/view_tests.rs`:

- setting off: `WorkspaceAction::CloseTab` closes immediately, matching current
  behavior
- setting on: `WorkspaceAction::CloseTab` opens the general confirmation and
  does not close immediately
- cancel: leaves tab count, active tab, rename state, and undo stack unchanged
- confirm: closes the intended tab and preserves existing undo behavior
- bulk close: `CloseOtherTabs` shows one dialog and closes all intended tabs only
  after confirm
- grouped tabs: `CloseTabGroup` shows one dialog, closes the intended group
  members only after confirm, and leaves no stale group state behind
- grouped tabs: `CloseTabsOutsideGroup`, `CloseTabsAboveGroup`, and
  `CloseTabsBelowGroup` use one bulk dialog and preserve their current close
  targets
- higher-severity precedence: a shared-session tab shows the existing
  close-session confirmation, not the new general confirmation
- higher-severity precedence: tabs with running processes / unsaved state show
  the existing quit warning, not the new general confirmation
- last-tab close: does not show the new general tab-close confirmation
- confirm callback re-checks higher-severity warnings if relevant state changes
  before final close

Add settings tests in `app/src/workspace/tab_settings_tests.rs` as described in
section 1.

If the modal is difficult to assert directly in `view_tests`, factor the dialog
decision into a small pure helper and unit-test the helper, then keep a focused
integration-style workspace test for the end-to-end close flow.

### 8. Manual validation

Run the app with `./script/run` and verify:

- setting off: close button and keybinding close tabs immediately
- setting on: close button, middle-click, keyboard close, and tab context-menu
  close show the confirmation
- setting on: tab-group close button and tab-group close menu actions show one
  confirmation for the requested group close action
- "Cancel" keeps tabs unchanged
- "Close tab" / "Close tabs" closes the intended tabs
- vertical tabs use the same behavior
- shared-session and running-process warnings still take precedence and are not
  followed by a second generic close confirmation

### 9. Presubmit

Run:

```bash
cargo fmt
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
cargo nextest run -p warp_app --no-fail-fast
```

For a spec-only PR, a markdown/diff check such as `git diff --check` is
sufficient unless maintainers ask for more validation. The implementation PR
should run the full presubmit set.
