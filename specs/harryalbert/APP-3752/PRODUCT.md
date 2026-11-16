# Notification Toast & Mailbox UI Updates

## Summary
Update the notification toast and mailbox UI to match the new Figma designs. This covers layout, spacing, and content changes across both the floating toast notifications and the persistent mailbox dropdown. The changes introduce a branch name context row, line-clamped text with expand affordances, shared avatar rendering with vertical tabs, and various padding/spacing refinements.

## Problem
The current notification UI was built as a first pass behind the `hoa_notifications` feature flag. The toast and mailbox designs have since been refined with better information density (branch context), improved visual consistency with other surfaces (avatar rendering matching vertical tabs), and more polished spacing/sizing.

## Goals
- Match the Figma spec for both toast and mailbox UI
- Introduce branch name context so users can immediately see which branch a notification relates to
- Improve text density with character-count truncation and expand affordances
- Unify avatar rendering between notifications and vertical tabs

## Non-goals
- Inline approve/decline buttons on toasts (punted)
- Changes to notification data model beyond adding branch context
- Changes to notification timing, auto-dismiss, or sound behavior
- Changes to the notification mailbox's keyboard navigation or action dispatch

## Figma
- Toast: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7411-88337&m=dev
- Mailbox: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7411-91197&m=dev

## User Experience

### Toast

#### Container
- Width: 420px (was 360px).
- Padding: 12px uniform (was 10px vertical / 16px horizontal).
- Corner radius: 8px (was 6px).
- No drop shadow.

#### Branch name header row
- When a notification has branch context, show a row above the title containing:
  - Left: git-branch icon (10×10) + branch name text (12px, sub_text color).
  - Right: chevron icon (expand/collapse toggle, shown only when content is truncated).
- When a notification has no branch context, this row is omitted and the toast shows the title only (no timestamp — see below).

#### Title
- Truncated at 100 characters (collapsed) / 500 characters (expanded).
- Appends `…` when truncated.

#### Message / subtext
- Truncated at 100 characters (collapsed) / 500 characters (expanded).
- Appends `…` when truncated.
- The expand chevron (in the branch/title row) is only rendered when content is actually truncated.

#### Timestamp and unread dot
- Removed from the toast entirely.
(Toasts are inherently unread and short-lived, so these indicators add no value.)

#### Avatar
- Use the same avatar rendering system as vertical tabs (`render_pane_icon_with_status` with `IconWithStatusVariant`).
- This gives proper brand colors for CLI agents, `main_text_color` for Oz, and status badges with cutout rings.
- The notification's `NotificationCategory` (Complete/Request/Error) maps to the same status badge icons/colors.

#### Close button
- Positioned at the **top-left** corner of the toast, partially outside the bounds.
- Only visible on hover (current behavior preserved).
- Has a visible border on the circle.

#### Keybinding hint
- Still shown on the newest toast ("Open conversation" + keyboard shortcut).
- No change from current behavior.

#### Artifact buttons
- 48px left padding to align past the avatar column.

### Mailbox

#### Outer container
- Add 4px top padding before the header.
- No drop shadow.

#### Header
- Left side: "Notifications" text (14px, **regular** weight — was semibold).
- Right side: X close button.
- Padding: `8px vertical, 12px horizontal` (was 8px vertical, 16px left, 8px right).

#### Filter bar
- Button gap: 2px (was 4px).
- "All" filter now shows count: e.g. "All tabs (11)" instead of just "All Tabs".
- Bar padding: `12px vertical, 12px left, 6px right` (was 12px vertical, 16px horizontal).

#### Rich notification items (with branch context)
- Padding: 12px uniform (was 12px vertical, 16px horizontal).
- Branch row at top of the text column:
  - Left: git-branch icon (10px) + branch name (12px, sub_text color).
  - Right: timestamp (12px, disabled gray) + unread dot (when unread).
- Title: 14px semibold, truncated at 100 characters (collapsed) / 500 characters (expanded).
- Message: 14px regular, sub_text color, truncated at 100 characters (collapsed) / 500 characters (expanded).
- Artifact buttons: 48px left padding.

#### Simple notification items (without branch context)
- Padding: stays 12px vertical, 16px horizontal (unchanged).
- Title row: SpaceBetween with title (14px semibold) | timestamp + optional unread dot (current layout, unchanged).
- Title and message use the same 100-character collapsed / 500-character expanded truncation behavior as rich items.
- In the mailbox, simple items do not render an expand chevron; in toasts, the chevron appears in the title row when content is truncated.

#### Shared item changes (both layouts)
- Avatar: use vertical tabs `render_pane_icon_with_status` system.
- Timestamp font size: 12px (was 14px).

### Data model
- `NotificationItem` needs a new `branch: Option<String>` field.
- When present, items render using the "rich" layout (with branch row).
- When absent, items render using the "simple" layout (current style).
- The branch value must be threaded from the conversation or CLI session context when creating notifications.

## Success Criteria

1. Toast container is 420px wide with 12px uniform padding, 8px corner radius, and no drop shadow.
2. When a notification has branch context, the toast shows a branch name header row with git-branch icon, branch name, and chevron-right button when the title or message is truncated.
3. Toast title text is truncated at 100 characters when collapsed.
4. Toast message text is truncated at 100 characters with an expand chevron shown only when content is truncated. Clicking the chevron reveals the full message (up to 500 characters).
5. Toast does not display a timestamp or unread dot.
6. Toast close button is positioned at the top-left, partially outside the toast, visible only on hover, with a border.
7. Toast keybinding hint ("Open conversation" + shortcut) is still shown on the newest toast.
8. Toast artifact buttons have 48px left padding.
9. All toast and mailbox avatars use the vertical tabs `render_pane_icon_with_status` system with proper brand colors, Oz styling, and status badge cutout rings.
10. Mailbox outer container has 4px top padding and no drop shadow.
11. Mailbox header has regular-weight "Notifications" title on the left, close button on the right, padded at 8px vertical / 12px horizontal.
12. Mailbox filter bar has 2px button gap, the "All" filter shows a count, and padding is 12px vertical / 12px left / 6px right.
13. Mailbox rich items (with branch) use 12px uniform padding, show a branch row (with timestamp + unread dot on right), truncated title, truncated message, and 48px left-padded artifact buttons.
14. Mailbox simple items (without branch) keep the current layout with 12px vertical / 16px horizontal padding.
15. Timestamp font size is 12px in both mailbox item layouts.

## Validation
- Build the app and visually compare toasts and mailbox against the Figma mocks.
- Verify toast hover shows close button at top-left with border.
- Verify long titles (>100 chars) are truncated with `…` in both toast and mailbox.
- Verify long messages (>100 chars) show the expand chevron, and clicking expands the full text (up to 500 chars).
- Verify notifications with branch context render the branch row; notifications without branch context render the simple layout.
- Verify avatars match vertical tabs (Oz icon with dark background, CLI agents with brand colors, status badges with cutout rings).
- Verify keybinding hint still appears on the newest toast.
- Verify filter bar "All tabs" shows count and spacing matches Figma.

## Open Questions
1. ~~Where does the branch name come from?~~ Resolved: uses `TerminalView::current_git_branch()`, the same source as the branch chip and vertical tabs.
2. ~~What does the settings gear icon in the mailbox header do?~~ Deferred: settings gear removed from this pass.
3. ~~Should the expand affordance collapse back after expanding?~~ Resolved: yes, toggle behavior — clicking the chevron toggles between collapsed and expanded.
