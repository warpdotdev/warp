# TECH.md — Configurable Toolbar Chips (Header Toolbar)

**Product spec:** `specs/harryalbert/APP-3865/PRODUCT.md`

## Problem

The header bar (in both horizontal and vertical tabs mode) renders panel toggle buttons in a hardcoded order and side assignment. We need to make these configurable (reorderable, moveable between sides, hideable) using the same `ChipConfigurator` pattern already used for the agent input footer. Critically, the side a button is placed on determines which side its panel opens on and the order determines the panel stacking order.

The entire feature is gated behind `FeatureFlag::ConfigurableToolbar` (enabled by default in all release builds).

## Relevant code

### Panel rendering (two-layer system that needs unification)
- `app/src/workspace/view.rs:16993` — `render_panels`: outer layer composing `VTabs | theme chooser | Main | right panels`.
- `app/src/workspace/view.rs:16545` — `render_banner_and_active_tab`: inner layer composing `LeftPanel(tools) | Terminal | RightPanel(code review)`.
- `app/src/workspace/view/vertical_tabs.rs:1120` — `render_vertical_tabs_panel`: renders the vertical tabs sidebar with a `Resizable` wrapper and `DragBarSide::Right`.

### Panel toggle state
- `app/src/pane_group/mod.rs:843` — `PaneGroup::left_panel_open: bool` (tools panel)
- `app/src/pane_group/mod.rs:845` — `PaneGroup::right_panel_open: bool` (code review)
- `app/src/workspace/view.rs` — `Workspace::vertical_tabs_panel_open: bool`
- `app/src/workspace/view.rs` — `WorkspaceState::is_agent_management_view_open: bool` (replaces main content)
- `app/src/workspace/view.rs` — `WorkspaceState::is_notification_mailbox_open: bool` (popover)

### Panel toggle logic
- `app/src/workspace/view.rs:6711` — `toggle_left_panel`: tools panel open/close
- `app/src/workspace/view.rs:6947` — `toggle_right_panel`: code review open/close
- `app/src/workspace/view.rs:6669` — `toggle_vertical_tabs_panel`
- `app/src/workspace/view.rs:18256` — `ToggleNotificationMailbox` handler

### Notification toast positioning
- `app/src/workspace/view.rs:20265` — mailbox popover anchors to `NOTIFICATIONS_MAILBOX_POSITION_ID` (follows button naturally).
- `app/src/workspace/view.rs:20342` — notification toasts anchor to `TAB_BAR_POSITION_ID` at `BottomRight`. This needs to become dynamic.

### Toolbar button rendering (current hardcoded layout)
- `app/src/workspace/view.rs:15417` — `render_tab_bar_contents`: left-side buttons at 15522–15535, right-side via `add_right_side_tab_bar_controls` at 15543.
- `app/src/workspace/view.rs:15639` — `add_right_side_tab_bar_controls`

### Existing chip configurator infrastructure
- `app/src/chip_configurator/mod.rs` — `ChipConfigurator` with `LeftRightZones` layout.
- `app/src/ai/blocklist/agent_view/agent_input_footer/editor.rs` — `AgentToolbarEditorModal` (template for our modal).

### Existing footer chip selection settings (pattern to follow)
- `app/src/terminal/session_settings.rs:82` — `ToolbarChipSelection` trait, `AgentToolbarChipSelection` enum.
- `app/src/terminal/session_settings.rs:277` — `agent_footer_chip_selection` setting.

### Onboarding
- `app/src/settings/onboarding.rs:62` — `apply_ui_customization_settings`

## Current state

### Panel rendering: two-layer system

The workspace renders panels in two separate composition steps:

**Outer layer** (`render_panels`, line 16993):
```
Flex::row: [ VTabs panel | theme chooser(left) | Main content | resource center/AI assistant(right) ]
```

**Inner layer** (`render_banner_and_active_tab`, line 16545):
```
Flex::row: [ LeftPanelView(tools) | terminal_content | RightPanelView(code review) ]
```

The vertical tabs panel sits OUTSIDE the left/right panel pair, at a different level. Tools panel and code review are inside `render_banner_and_active_tab`. This two-layer split exists for a reason:

**Per-tab vs workspace-level state:** The inner layer panels (tools, code review) have **per-tab** open state — `PaneGroup::left_panel_open` and `PaneGroup::right_panel_open` are on `PaneGroup`, and each tab has its own `PaneGroup`. The tools panel can be open in tab 1 and closed in tab 2. The outer layer panels (vertical tabs, theme chooser, resource center) have **workspace-level** state that persists across tab switches.

To support arbitrary ordering, we need all configurable panels at the same composable level while preserving the per-tab open state for tools panel and code review. The `WorkspaceState::is_left_panel_open()` method (`workspace/util.rs:206`) only checks theme chooser state; `is_right_panel_open()` (`util.rs:202`) only checks resource center / AI assistant. These are workspace-global concerns separate from the configurable toolbar panels.

### Panel types and their current rendering

Each panel type has distinct rendering and state management:

- **Vertical tabs** (`render_vertical_tabs_panel`): `Resizable` wrapper with `DragBarSide::Right`. Self-contained rendering in `vertical_tabs.rs`. Open state: `Workspace::vertical_tabs_panel_open`.
- **Tools panel** (`LeftPanelView`): rendered via `ChildView::new(&self.left_panel_view)`. Has its own `Resizable` wrapper inside `LeftPanelView`. Open state: `PaneGroup::left_panel_open`.
- **Code review** (`RightPanelView`): rendered via `ChildView::new(&self.right_panel_view)`. Has its own `Resizable` wrapper. Open state: `PaneGroup::right_panel_open`. Can be maximized.
- **Agent management**: replaces the terminal content area entirely (`ChildView::new(&self.agent_management_view)` at line 16555). Not a side panel.
- **Notifications mailbox**: popover overlay anchored to `NOTIFICATIONS_MAILBOX_POSITION_ID`. Not a side panel.

### Toolbar button rendering

In the vertical tabs branch of `render_tab_bar_contents` (line 15522):
- Left side: tabs panel toggle, tools panel toggle, agent management toggle
- Right side (via `add_right_side_tab_bar_controls`): code review toggle, notifications mailbox, avatar

These are hardcoded in order and side.

## Proposed changes

### 1. New `HeaderToolbarItemKind` enum

New file: `app/src/workspace/header_toolbar_item.rs`

```rust
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum HeaderToolbarItemKind {
    TabsPanel,
    ToolsPanel,
    AgentManagement,
    CodeReview,
    NotificationsMailbox,
}
```

Methods:
- `display_label(&self) -> &str` — human-readable name for the configurator.
- `icon(&self) -> Icon` — icon for the configurator chip.
- `is_available(app: &AppContext) -> bool` — checks feature flags and prerequisite settings.
- `default_left() -> Vec<Self>` — `[TabsPanel, ToolsPanel, AgentManagement]`.
- `default_right() -> Vec<Self>` — `[CodeReview, NotificationsMailbox]`.
- `all_items() -> Vec<Self>` — all variants (availability filtering done at call site).
- `is_panel(&self) -> bool` — true for items that open a side panel (TabsPanel, ToolsPanel, CodeReview). False for AgentManagement (replaces content) and NotificationsMailbox (popover).

### 2. New setting: `HeaderToolbarChipSelection`

In `app/src/workspace/tab_settings.rs`:

```rust
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum HeaderToolbarChipSelection {
    #[default]
    Default,
    Custom {
        left: Vec<HeaderToolbarItemKind>,
        right: Vec<HeaderToolbarItemKind>,
    },
}
```

With `left_items()`, `right_items()` methods that resolve `Default` to hardcoded defaults. Add to `TabSettings` under `appearance.tabs` hierarchy.

### 3. Flatten the panel rendering into a single composition step

This is the core architectural change. Currently panels render at two levels. We unify them.

**Replace the two-layer system with a single `render_configurable_panels` method:**

In `render_banner_and_active_tab` (or a new unified method), build the panel row dynamically:

```
let config = resolved toolbar config;
let left_items = config.left_items filtered by is_available;
let right_items = config.right_items filtered by is_available;

Flex::row:
  for item in left_items where item.is_panel() && item.is_open():
    render_panel_for_item(item, PanelPosition::Left)
  Main content (terminal or agent management)
  for item in right_items where item.is_panel() && item.is_open():
    render_panel_for_item(item, PanelPosition::Right)
```

Where `render_panel_for_item` dispatches to:
- `TabsPanel` → `render_vertical_tabs_panel` (with `DragBarSide` flipped if on right)
- `ToolsPanel` → `ChildView::new(&self.left_panel_view)` (with drag bar side adjusted)
- `CodeReview` → `ChildView::new(&self.right_panel_view)` (with drag bar side adjusted)

This moves vertical tabs out of `render_panels` and tools/code review out of `render_banner_and_active_tab`, into a unified composition.

**Preserving per-tab state:** The unified rendering must still read tools panel open state from `PaneGroup::left_panel_open` and code review from `PaneGroup::right_panel_open` (per-tab), while reading vertical tabs from `Workspace::vertical_tabs_panel_open` (workspace-level). The rendering loop checks the appropriate open state per item type, regardless of which side it's configured on.

**Drag bar side:** When a panel is on the left, its drag bar (resize handle) should be on the right (`DragBarSide::Right`). When on the right, it should be on the left (`DragBarSide::Left`). The vertical tabs panel currently hardcodes `DragBarSide::Right` (`vertical_tabs.rs:1174`). Tools panel and code review panel handle this internally in their respective views — these will need a `set_panel_position` method or constructor parameter.

### 4. Adapt panel toggle logic to be side-aware

Currently `toggle_left_panel` and `toggle_right_panel` are tightly coupled to the left/right concept. We need toggle methods that are panel-type-aware rather than side-aware.

Add a method that resolves which side a panel is on from the config:

```rust
fn panel_side(&self, item: HeaderToolbarItemKind, app: &AppContext) -> Option<PanelPosition> {
    let config = TabSettings::as_ref(app).header_toolbar_chip_selection.value();
    if config.left_items().contains(&item) { Some(PanelPosition::Left) }
    else if config.right_items().contains(&item) { Some(PanelPosition::Right) }
    else { None }
}
```

The existing `ToggleLeftPanel` / `ToggleRightPanel` workspace actions continue to work — they just consult the config to determine which side the panel actually renders on. The `PaneGroup::left_panel_open` / `right_panel_open` booleans track open state independently of side placement.

### 5. Notification toast positioning

Currently notification toasts anchor at `PositionedElementAnchor::BottomRight` of `TAB_BAR_POSITION_ID` (`view.rs:20350`). When the mailbox button is on the left, this should become `BottomLeft`.

In the toast rendering code, read the toolbar config to determine which side the mailbox is on, and adjust the anchor accordingly.

### 6. Shared chip-editor modal rendering + `HeaderToolbarEditorModal`

The `AgentToolbarEditorModal` (`agent_input_footer/editor.rs`) and the new `HeaderToolbarEditorModal` (`workspace/header_toolbar_editor.rs`) render an identical modal UI (title, chip sections with left/right drop zones, restore-default link, cancel/save buttons, blur overlay). The duplication is purely in rendering — the domain logic (which settings to read/write, action types, event types) is genuinely different.

New file `app/src/chip_configurator/modal_shell.rs` provides:
- `ChipEditorModalConfig<A>` — struct carrying everything that varies: title, available-section label, `is_at_defaults`, `is_dirty`, action values (cancel, save, reset, activate), `chip_action_wrapper`, and three `MouseStateHandle`s.
- `render_chip_editor_modal<A>(configurator, config, appearance) -> Box<dyn Element>` — free function that renders the full modal (blur overlay, centered card, title, chip sections, buttons). Generic over the action type `A`.

Each modal view keeps its own `ChipConfigurator`, `MouseStateHandle`s, `is_dirty`, and domain state. Their `View::render` builds a `ChipEditorModalConfig` and calls `render_chip_editor_modal`. Their `TypedActionView::handle_action` handles save/open/reset with domain-specific logic.

**`HeaderToolbarEditorModal`** specifics:
- On `open()`: reads `HeaderToolbarChipSelection` from `TabSettings`, builds `ConfigurableItem` list via `ControlItemRenderer::new_with_label_and_icon`.
- On `save()`: writes back to `TabSettings::header_toolbar_chip_selection`. Syncs `show_code_review_button` and `show_agent_notifications`.
- On `reset_default()`: restores default layout.

### 7. Integrate modal into Workspace

- Add `header_toolbar_editor_modal: ViewHandle<HeaderToolbarEditorModal>` field.
- Add `WorkspaceAction::OpenHeaderToolbarEditor`.
- Add `is_header_toolbar_editor_open` to `WorkspaceState`.
- Render in the overlay stack.

### 8. Refactor `render_tab_bar_contents` to read from settings

Replace hardcoded button rendering in the vertical tabs branch with a loop:

```rust
let config = TabSettings::as_ref(ctx).header_toolbar_chip_selection.value();
for item in config.resolved_left(ctx) {
    tab_bar.add_child(self.render_header_toolbar_button(item, appearance, ctx));
}
// ... search bar ...
for item in config.resolved_right(ctx) {
    right_controls.add_child(self.render_header_toolbar_button(item, appearance, ctx));
}
```

`render_header_toolbar_button` dispatches to existing per-item render methods.

### 9. Right-click context menu

Wrap toolbar items/area with an `EventHandler` that intercepts right-click and opens a `Menu` with a single "Edit toolbar" entry dispatching `WorkspaceAction::OpenHeaderToolbarEditor`.

### 10. Settings page entry point

Add an "Edit toolbar" button widget in `appearance_page.rs` under the "Tabs" category, dispatching `WorkspaceAction::OpenHeaderToolbarEditor`.

### 11. Show/hide setting sync on save

When saving:
- `CodeReview` → sync `TabSettings::show_code_review_button`
- `NotificationsMailbox` → sync `AISettings::show_agent_notifications`
- `ToolsPanel` → composite (no single toggle to sync)
- `TabsPanel`, `AgentManagement` → no existing individual show/hide setting

### 12. Side-aware UI element flipping

When panels move sides, all overlay/popup UI elements associated with them must flip to point toward the center of the screen. This is implemented via helpers that read `HeaderToolbarChipSelection` from `TabSettings` at render time.

**`tools_panel_menu_direction(app) -> MenuDirection`** (`drive/items/item.rs`): Returns `MenuDirection::Right` when the tools panel is on the left, `MenuDirection::Left` when on the right. Used by Warp Drive items, conversation list items, and the sorting button.

**`tabs_panel_side(app) -> PanelPosition`** (`workspace/view.rs`): Returns the side the tabs panel button is on. Used by the detail sidecar, action buttons, and right-click menu.

**`is_mailbox_on_left(app) -> bool`** (`workspace/view.rs`): Returns whether the mailbox button is configured on the left. Used for mailbox popover and toast anchoring.

Flipped elements:
- **Notification mailbox popover** (`view.rs`): Anchors `BottomLeft`/`TopLeft` when on left, `BottomRight`/`TopRight` when on right.
- **Notification toasts** (`view.rs`): Same side-aware anchoring.
- **Tabs panel detail sidecar** (`vertical_tabs.rs`): `detail_sidecar_offset_and_max_height` accepts `PanelPosition` and flips `TopRight→TopLeft` / `BottomRight→BottomLeft` (and negates the horizontal gap) when on the right.
- **Vertical tabs action buttons** (`vertical_tabs.rs`): Overlay position flips from `TopRight` to `TopLeft`.
- **Vertical tabs right-click menu** (`view.rs`): Anchors flip from `BottomLeft`/`TopLeft` to `BottomRight`/`TopRight`.
- **Conversation list tooltips** (`conversation_list/item.rs`): `tooltip_opens_right` field on `ItemProps` flips `MiddleRight→MiddleLeft`.
- **Conversation list kebab button + menu** (`conversation_list/item.rs`): Button position flips from `TopRight` to `TopLeft`; `MenuDirection` flips.
- **Warp Drive item overflow button** (`drive/items/item.rs`): When `overflow_on_left`, the button renders as a `Stack` overlay at `MiddleLeft` (flush with edge) instead of appending to the flex row. This avoids pushing item content.
- **Warp Drive hover previews and dialogs** (`drive/index.rs`): `add_row_overlay_to_stack` flips the X-axis anchor pair from `(Right, Left)` to `(Left, Right)` and negates the pixel offset.
- **Panel borders** (`left_panel.rs`, `right_panel.rs`): Border side and `DragBarSide` are driven by `self.panel_position`.
- **Vertical tabs panel** (`vertical_tabs.rs`): Border and drag bar side are driven by the `side` parameter.

## End-to-end flow

**Toolbar rendering:**
1. `render_tab_bar_contents` reads `HeaderToolbarChipSelection` from `TabSettings`.
2. For each configured left item (filtered by `is_available`), renders the corresponding button.
3. Search bar renders in the middle.
4. For each configured right item, renders the corresponding button.

**Panel rendering:**
1. Unified panel composition reads the same config.
2. For each left panel item that is open, renders the panel on the left of main content, in order.
3. Main content (terminal or agent management) renders in the middle.
4. For each right panel item that is open, renders the panel on the right, in order.

**Configuration flow:**
1. User right-clicks toolbar → context menu → "Edit toolbar".
2. `WorkspaceAction::OpenHeaderToolbarEditor` opens the modal.
3. Modal reads current config, populates `ChipConfigurator`.
4. User drags items, clicks save.
5. Modal writes `Custom { left, right }` to `TabSettings::header_toolbar_chip_selection`.
6. Toolbar and panels re-render from the new config.

## Risks and mitigations

**Risk:** Flattening the two-layer panel system is a significant refactor that touches the core workspace layout.
**Mitigation:** Both the vertical and horizontal tab paths use the same config-driven rendering. The default config produces identical behavior to the previous hardcoded layout, so existing users see no change unless they customize.

**Risk:** Resizable panel drag bars need to flip direction based on side.
**Mitigation:** The `Resizable` element already accepts `DragBarSide`. Vertical tabs hardcodes `Right` at `vertical_tabs.rs:1174`; tools and code review panels handle it internally. We pass the side through.

**Risk:** `PaneGroup::left_panel_open` / `right_panel_open` naming becomes confusing when panels can be on either side.
**Mitigation:** Keep the field names as-is (they track the tools panel and code review open state respectively, and are per-tab). Rename can be a follow-up. The side a panel renders on is determined by the toolbar config, not by which boolean tracks its state.

**Risk:** Flattening the panel layers might break the per-tab vs workspace-level open state distinction.
**Mitigation:** The unified rendering explicitly reads per-tab state (`PaneGroup::left_panel_open` / `right_panel_open`) for tools and code review, and workspace-level state (`Workspace::vertical_tabs_panel_open`) for the tabs panel. The existing `WorkspaceState::is_left_panel_open()` / `is_right_panel_open()` methods (which only concern theme chooser and resource center) remain unchanged since those panels are not configurable.

**Risk:** Show/hide setting desync if user toggles settings directly outside the configurator.
**Mitigation:** Toolbar rendering always filters by `is_available()`. The configurator's `open()` re-evaluates availability each time.

## Testing and validation

- **Unit tests:** `HeaderToolbarChipSelection` serialization, resolution, availability filtering.
- **Manual testing:**
  - Move code review to left → verify it opens left of main content.
  - Move tabs panel to right → verify sidebar opens right of main content.
  - Configure left as `[CodeReview, TabsPanel]`, open both → verify code review renders left of tabs.
  - Move notifications to left → verify toasts appear on left side.
  - Save config, restart, verify persistence.

## Follow-ups

- Rename `PaneGroup::left_panel_open` / `right_panel_open` to panel-type-specific names (e.g. `tools_panel_open`, `code_review_panel_open`).
- Consider a dedicated `show_agent_management` boolean setting.
- Flip additional secondary elements when on the right side: Warp Drive sorting button menu, create-new button menu, section header dialog positioning.
- Vertical tabs settings popup positioning when the tabs panel is on the right side.
- New-session dropdown menu and sidecar positioning when the tabs panel is on the right side.
