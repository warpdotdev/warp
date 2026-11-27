# Compact horizontal tabs: technical proposal

## Context

- `app/src/workspace/tab_settings.rs:459` defines the existing `TabSettings`
  model and public setting storage paths.
- `app/src/tab.rs:1103` stores the rendering choice on each `TabComponent`;
  `app/src/tab.rs:1273` reads it from the current settings when constructing tabs.
  The same component supplies horizontal tab drag previews and group members.
- `app/src/settings_view/appearance_page.rs:4862` contains the dedicated settings
  widget. `app/src/settings_view/appearance_page.rs:299` registers its Command
  Palette action pair.
- `app/src/workspace/view.rs:3844` handles the setting-change notification;
  `app/src/workspace/view.rs:23496` maintains its command context flag.

## Proposed changes

Add a default-false, GUI-only public boolean setting at
`appearance.tabs.compact_horizontal_tabs`. Use the existing cloud-sync policy
that respects the user's settings-sync choice. The existing settings framework
also registers the setting in the generated schema.

Read the choice during `TabComponent` construction and preserve the existing
rendering branches when false. When true, constrain the visible tab to 20 logical
pixels, cap its normal width at 170 pixels, and use the rounded theme-derived
highlight without side dividers. Adjust internal padding for grouped members so
their contents can fit. Retain the existing 34-pixel title-bar slot and shared
panel-header dimensions; center the smaller visible tab in that slot.

Provide a separate `SettingsWidget` with an owned `SwitchStateHandle` and focused
search terms. Dispatch the same action from the Settings switch and existing
`ToggleSettingActionPair` machinery. Handle the new settings event by invalidating
the workspace and update its command context flag, so the appropriate enable or
disable command is presented. No process environment variable or restart is used.

## Testing and validation

- Invariants 1 and 4: `compact_horizontal_tabs_persists_changes_across_restarts`
  uses a real temporary TOML settings store and fresh app instances to verify
  the initial default, enabled persistence, disabled persistence and public
  storage hierarchy.
- Invariants 1 and 4: `compact_horizontal_tabs_search_restores_rows_after_clearing`
  filters actual settings widgets, checks the selected widget's identity, and
  clears the query to restore the full list.
- Invariants 2, 3 and 5: manually inspect normal/compact tabs in a built macOS app;
  toggle in Settings, switch tabs, create and close a tab, and inspect colors and
  title clipping. Exercise the Command Palette action pair.
- Invariant 6: enable vertical tabs temporarily, confirm the stored compact
  setting does not alter their density, then return to horizontal tabs.
- Run targeted Rust tests and Clippy; format the changed Rust files. Linux and
  Windows rendering remain for CI and maintainer verification.

## Risks

Twenty-pixel tabs are intentionally dense. Check title and close-button alignment
under zoom, grouped/pinned tabs and drag previews. This opt-in design leaves the
existing default path intact. The feature proposal and specifications are not yet
approved by maintainers.
