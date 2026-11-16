# Configurable Toolbar Chips (Header Toolbar)

**Linear:** [APP-3865](https://linear.app/warpdotdev/issue/APP-3865)
**Figma:** none provided

## Summary

The app header bar shows a set of panel toggle buttons (tabs panel, tools panel, agent management, code review, notifications mailbox). Users should be able to add/remove, re-arrange, and move these items between the left and right sides of the header, using the same configurator UI pattern as the agent input footer chip editor. This applies to both horizontal and vertical tab modes.

## Problem

The toolbar layout is currently hardcoded, outside of customizing which items appear. Users cannot customize item order, or which side items are on. Users with different workflows may want different toolbar layouts — for example, a user who never uses code review shouldn't have to see that button, and a user who frequently uses agent management may want it on the right side near their cursor.

## Goals

- Let users reorder toolbar items within a side (left or right).
- Let users move toolbar items between left and right sides.
- Let users hide toolbar items they don't need.
- Let users restore hidden toolbar items.
- Integrate with the existing onboarding show/hide mechanism so that onboarding choices and configurator choices stay consistent.
- Persist configuration across sessions via settings.

## Non-goals

- The search bar remains centered and is not configurable (vertical tabs mode only; not shown in horizontal tabs).
- The avatar/profile button remains fixed at the far right and is not configurable.

## User experience

### Configurable items

The following toolbar items are configurable:

1. **Tabs panel** — toggles the vertical tabs sidebar (hamburger icon)
2. **Tools panel** — toggles the left panel containing project explorer, global search, warp drive, conversation history (tool icon)
3. **Agent management** — toggles the agent management view (grid icon)
4. **Code review** — toggles the code review panel (right panel button)
5. **Notifications mailbox** — toggles the notification mailbox (inbox icon)

Each item is gated on the same feature flags and settings that currently control its visibility (e.g. agent management requires AI enabled + `FeatureFlag::AgentManagementView`, notifications requires `FeatureFlag::HOANotifications`). Items whose prerequisites are not met do not appear in the toolbar or in the configurator.

### Default layout

The default layout matches the current hardcoded layout:
- **Left:** Tabs panel, Tools panel, Agent management
- **Right:** Code review, Notifications mailbox

### Side placement determines panel open side

Moving a toolbar item to the left or right side determines which side of the main content area its panel opens on:

- **Tabs panel:** Opens its sidebar on whichever side its button is placed. Not pinned to the left.
- **Tools panel:** Opens on whichever side its button is placed (left or right of main content).
- **Code review:** Opens on whichever side its button is placed.
- **Agent management:** Replaces the main content area entirely — side placement only affects button position, not where it renders.
- **Notifications mailbox:** The popover anchors to its button position — top-left corner under the button's bottom-left when on the left side, top-right under bottom-right when on the right. Notification toasts also appear on the same side as the mailbox button.

Within a side, panels open in the order their buttons appear in the toolbar. For example, if the left side is configured as `[Code Review, Tabs Panel, Tools Panel]`, then when all three are open, the panels render left-to-right as: Code Review | Tabs Panel | Tools Panel | Main Content.

### Side-aware UI element flipping

When a panel or its parent container moves to the opposite side, all associated UI elements flip to avoid rendering off-screen:

- **Tabs panel detail sidecar:** When the tabs panel is on the left, the hover-detail sidecar opens to the right. When on the right, it opens to the left.
- **Conversation list tooltips:** Item hover tooltips open toward the center of the screen (right when panel is on the left, left when on the right).
- **Conversation list kebab menu:** The overflow button and its opened menu anchor toward the center.
- **Warp Drive item kebab button:** When the tools panel is on the right, the kebab `⋮` button renders as a left-aligned overlay (flush with the left edge) instead of appending to the right end of the row. The opened menu also opens toward the center.
- **Warp Drive hover previews and dialogs:** Hover preview sidecars and share/naming dialogs flip to open toward the center.
- **Vertical tabs action buttons:** The kebab/close button container appears on the opposite side of the tab group, and the right-click menu anchors accordingly.
- **Panel borders and resize handles:** The drag bar and border line render on the edge facing the main content (right edge when panel is on the left, left edge when on the right).

### Configuration entry points

1. **Right-click context menu:** Right-clicking on any toolbar item or on the toolbar area between the left items and the search bar / between the search bar and the right items shows a context menu with an "Edit toolbar" option that opens the configurator modal.
2. **Settings page:** The configurator is also accessible from the Settings page under Appearance > Tabs, using the same pattern as the agent input footer toolbelt editors (the "Edit agent toolbelt" / "Edit CLI agent toolbelt" buttons accessible under Appearance > Input type). A button like "Edit toolbar" opens the same configurator modal.

### Configurator modal

The modal uses the same drag-and-drop `ChipConfigurator` pattern as the agent input footer editor (`app/src/ai/blocklist/agent_view/agent_input_footer/editor.rs`):

- **Title:** "Edit toolbar"
- **Available items** section: shows items not currently placed on either side. Chips can be dragged from here to either drop zone.
- **Left side** drop zone: shows items that render on the left side of the header bar. Items can be reordered via drag-and-drop.
- **Right side** drop zone: shows items that render on the right side of the header bar. Items can be reordered via drag-and-drop.
- Each placed item has an X button to remove it (moves it back to available items).
- Items can be dragged between zones (left → right, right → available, available → left, etc.).
- **Restore default** link: resets to the default layout. Disabled when already at defaults.
- **Cancel** button: discards changes and closes.
- **Save changes** button: persists the configuration and closes. Disabled when no changes have been made.

### Interaction with existing show/hide settings

The existing settings that control item visibility (`TabSettings::show_code_review_button`, `AISettings::show_agent_notifications`, etc.) remain the source of truth for whether an item is shown. When a user removes an item from the toolbar via the configurator, the corresponding existing setting is toggled off. When a user adds an item back, the setting is toggled on.

This ensures that:
- Onboarding choices are respected: if onboarding hides code review, it won't appear in the toolbar or in the configurator's "available" section until the prerequisite is met.
- Settings page toggles remain consistent with the toolbar configuration.

### Ordering and side placement persistence

A new setting stores the user's custom ordering and side placement. The setting stores a list of item identifiers for the left side and right side. When the setting is `Default`, the hardcoded default layout is used. When `Custom`, the stored lists determine order and placement.

Items that are "available" (prerequisites met + existing show/hide setting is on) but not present in either the left or right list are treated as hidden by the user.

### Edge cases

- If an item's prerequisite becomes unmet after configuration (e.g. AI is disabled, removing agent management), that item silently disappears from the toolbar. It remains in the stored configuration so it reappears if the prerequisite is met again.
- If all items are removed from both sides, the toolbar area remains but is empty (just spacing).
- If a panel is open and the user reconfigures its button to the opposite side, the panel should close and reopen on the new side on next toggle.

## Success criteria

1. Right-clicking the toolbar area (in both horizontal and vertical tabs mode) opens a context menu with "Edit toolbar".
2. The configurator modal opens and displays the correct current layout.
3. Items can be dragged between left, right, and available zones, and reordered within zones.
4. Clicking X on a placed item moves it to available.
5. Saving persists the configuration; reopening the configurator shows the saved layout.
6. The toolbar renders items in the configured order and sides.
7. "Restore default" resets to the hardcoded default layout.
8. Removing an item from the toolbar via the configurator also toggles off the corresponding existing setting (e.g. `show_code_review_button` becomes false).
9. Adding an item back toggles the setting on.
10. Onboarding settings are respected: items hidden during onboarding are not shown until re-enabled.
11. Items gated behind feature flags that are not enabled do not appear in the toolbar or configurator.
12. Configuration survives app restart.
13. Moving code review to the left side causes the code review panel to open on the left.
14. Moving tools panel to the right side causes the tools panel to open on the right.
15. Moving tabs panel to the right side causes the vertical tabs sidebar to open on the right.
16. Panel ordering within a side is respected (if tabs panel is to the right of tools panel on the left side, the tabs sidebar renders to the right of the tools panel).
17. Notification toasts appear on the same side as the mailbox button.
18. Warp Drive item hover previews open toward the center of the screen (left when panel is on the right).
19. Conversation list tooltips and kebab menus open toward the center.
20. Vertical tabs detail sidecar opens toward the center.
21. Warp Drive kebab button appears as a left-edge overlay when the panel is on the right side.

## Validation

- Manual testing: open the configurator, rearrange items, save, verify the toolbar updates. Restart app and verify persistence.
- Verify onboarding integration: complete onboarding with code review disabled, then open the configurator and confirm code review is not in the "available" section until re-enabled in settings.
- Verify feature flag gating: with `HOANotifications` disabled, confirm notifications mailbox does not appear in toolbar or configurator.
- Verify panel side placement: move code review to the left, verify it opens on the left side of the main content.
- Verify panel ordering: configure left as `[Code Review, Tabs Panel]`, open both, verify code review renders to the left of tabs panel.
- Unit tests for the new setting type (serialization, default resolution, migration).

## Feature flag

The entire feature is gated behind `FeatureFlag::ConfigurableToolbar`, which is enabled by default in all release builds. When disabled, the toolbar renders the default hardcoded layout and the right-click context menu / settings entry point are hidden.

## Open questions

None — all questions have been resolved.
