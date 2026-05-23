# Tech Spec: Show tab numbers on tabs

**Issue:** [warpdotdev/warp#4028](https://github.com/warpdotdev/warp/issues/4028)

## Context

Warp renders tabs through two independent subsystems:

- **Horizontal tab bar** — `TabComponent` in `app/src/tab.rs`. The visible title is produced by `TabComponent::render_tab_content` (`app/src/tab.rs:1061`), which is reached from `full_tab_content` (`app/src/tab.rs:1417`). `TabComponent` already carries `tab_index` and is reconstructed each render in `TabComponent::new` (`app/src/tab.rs:778`), which has the `AppContext`.
- **Vertical tabs panel** — `app/src/workspace/view/vertical_tabs.rs`. Each tab group is rendered by `render_tab_group_internal` (which has the real `tab_index`), and each row is built from a `PaneProps` by `render_pane_row` (Expanded), `render_compact_pane_row` (Compact), or `render_summary_tab_item` (Summary). All three assemble a final `Flex::row` of `icon` + text content.

The `Cmd+1..9` shortcuts already exist: `WorkspaceAction::ActivateTabByNumber(n)` → `activate_tab(n - 1)` → `workspace.tabs[n-1]`. So the natural label for `tabs[i]` is `i + 1`.

Tab settings live in `app/src/workspace/tab_settings.rs` via the `define_settings_group!(TabSettings, ...)` macro (sibling example: `show_indicators: ShowIndicatorsButton`). The settings UI for tabs is in `app/src/settings_view/appearance_page.rs` (sibling toggle: `TabIndicatorWidget` / `ToggleTabIndicators`). The macro generates a `TabSettingsChangedEvent` variant per setting; the only exhaustive match on it is `handle_tab_settings_change` in `app/src/workspace/view.rs:3437`.

## Proposed changes

### 1. New setting — `app/src/workspace/tab_settings.rs`

Add to `define_settings_group!(TabSettings, ...)`, mirroring `show_indicators`:

```rust
show_tab_numbers: ShowTabNumbers {
    type: bool,
    default: false,
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "appearance.tabs.show_tab_numbers",
    description: "Whether to show each tab's position number (matching the Cmd+1..9 switch shortcuts) before its title.",
},
```

### 2. Horizontal tabs — `app/src/tab.rs`

- Add a `show_tab_number: bool` field to `TabComponent`.
- In `TabComponent::new`, read `*TabSettings::as_ref(ctx).show_tab_numbers.value()` and store it.
- In `render_tab_content`, when enabled, shadow the title with `format!("{}  {}", self.tab_index + 1, self.title)` for the non-rename branch only. `self.title` and the model are left untouched.

### 3. Settings UI — `app/src/settings_view/appearance_page.rs`

Mirror the `TabIndicatorWidget` path exactly: import `ShowTabNumbers`; add `AppearancePageAction::ToggleShowTabNumbers`; add its dispatch arm; add `toggle_show_tab_numbers` (sets `show_tab_numbers`); add `ShowTabNumbersWidget` and register it in `tab_settings_widgets`.

### 4. Settings-changed handler — `app/src/workspace/view.rs`

The match on `TabSettingsChangedEvent` is exhaustive, so add a notify-only arm for the generated `TabSettingsChangedEvent::ShowTabNumbers { .. } => { ctx.notify(); }` so toggling re-renders live.

### 5. Vertical tabs — `app/src/workspace/view/vertical_tabs.rs`

- Add `tab_number: Option<usize>` to `PaneProps`, initialized to `None` in `PaneProps::new`, and add `tab_number: _` to the exhaustive destructure in `render_pane_row_element`.
- In `render_tab_group_internal`, read `show_tab_number` once and set `pane_props.tab_number = (show_tab_number && row_idx == 0).then_some(tab_index + 1)` on the per-row loop (changed to `enumerate()`), and on the single summary row. This guarantees exactly one number per tab.
- Add `render_tab_number_label(number, appearance)` and prepend it as the first child of the final `Flex::row` in `render_pane_row`, `render_compact_pane_row`, and `render_summary_tab_item` when `props.tab_number` is `Some`. The row's existing `ICON_WITH_STATUS_GAP` spacing separates it from the icon.

## Tradeoffs / alternatives

- **Inline prefix vs. left gutter:** A fixed-width gutter would align titles but distorts widths and complicates the vertical-tab overlay anchoring; the inline element next to the icon is the lowest-risk approach and is consistent across modes. A gutter can be a follow-up.
- **Render-time only vs. stored title:** Numbering at render time keeps copy/search/title byte-faithful (invariant 6). Writing into the stored title was rejected for that reason.

## Testing and validation

- **Invariant 1 (off by default):** `default: false`; existing `tab_settings` tests confirm defaults.
- **Invariants 2–3 (1-based, matches shortcut):** unit-checkable that horizontal and vertical both use `tab_index + 1`; cross-referenced against `ActivateTabByNumber` → `tabs[n-1]`.
- **Invariant 4 (one number per tab, all modes):** `tab_number` is set only when `row_idx == 0` (and the single summary row), so duplicates are impossible; verify in Expanded/Compact/Summary.
- **Invariant 5 (live toggle):** the new `view.rs` notify arm.
- **Invariant 6 (presentation-only):** "Copy tab title" uses `pane_group.display_title(ctx)` (model), not the rendered string; the vertical number is a separate element, never merged into title/search text.
- **Invariant 7 (no number while renaming):** the prefix is only in the non-rename branch of `render_tab_content`.
- **Automated:** `cargo check -p warp`, `cargo test -p warp tab_settings`, `cargo fmt`, and `cargo clippy -p warp --all-targets` all clean.
- **Manual:** build an OSS bundle, toggle the setting, confirm numbers appear and match `Cmd+N` in both layouts, and that copied titles/search exclude the number.

## Follow-ups

- Optional visual polish (badge style, fixed-width gutter for alignment).
- Optionally extend numbers to other tab-like surfaces if desired.
