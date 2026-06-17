# Notification Toast & Mailbox UI Updates — Tech Spec

## Problem
The notification toast and mailbox UI need visual updates to match new Figma designs. The changes span container sizing, a new branch context row, line-clamped text with expand affordances, shared avatar rendering with vertical tabs, and various padding/spacing adjustments. See `specs/harryalbert/APP-3752/PRODUCT.md` for the full product spec.

## Relevant Code

### Notification module
- `app/src/ai/agent_management/notifications/mod.rs` — module root, re-exports
- `app/src/ai/agent_management/notifications/item.rs` — `NotificationItem`, `NotificationItems`, `NotificationCategory`, `NotificationSourceAgent`
- `app/src/ai/agent_management/notifications/item_rendering.rs` — shared rendering: `render_notification_item_content`, `render_agent_avatar`
- `app/src/ai/agent_management/notifications/toast_stack.rs` — `AgentNotificationToastStack`, `render_toast`, `render_close_button`, `render_keybinding_hint`
- `app/src/ai/agent_management/notifications/view.rs` — `NotificationMailboxView`, header, filter bar, item rendering

### Avatar rendering (vertical tabs)
- `app/src/ui_components/icon_with_status.rs` — `IconWithStatusVariant`, `IconWithStatusSizing`, `render_icon_with_status`
- `app/src/workspace/view/vertical_tabs.rs:84-240` — vertical-tab sizing presets and `render_pane_icon_with_status`
- `app/src/workspace/view/vertical_tabs.rs:1227-1327` — `resolve_icon_with_status_variant` (maps pane types to avatar variants)

### Status icons
- `app/src/ai/agent/conversation.rs:3531-3538` — `ConversationStatus::status_icon_and_color`
- `app/src/ai/conversation_status_ui.rs` — `render_status_element` (used by agent management views)

### Notification creation
- `app/src/ai/agent_management/agent_management_model.rs:287-408` — `handle_history_event_for_mailbox`, `add_notification` (where Oz conversation notifications are created)
- `app/src/ai/agent_management/agent_management_model.rs:123-191` — `handle_cli_agent_session_event` (where CLI session notifications are created)

### CLI session context
- `app/src/terminal/cli_agent_sessions/mod.rs:39-48` — `CLIAgentSessionContext` (cwd, project, query, etc. — no branch field currently)

### Existing text primitives
- `warp_core/src/ui/builder.rs:826-840` — `wrappable_text` (the text component used in notifications)
- No existing max-lines / line-clamp primitive in the UI framework.

## Current State

### Notification items
`NotificationItem` (`item.rs:64-78`) holds: `id`, `origin`, `title`, `message`, `category`, `agent`, `is_read`, `created_at`, `terminal_view_id`, `artifacts`. No branch context.

### Shared rendering
`render_notification_item_content` (`item_rendering.rs:62-164`) is used by both toasts and the mailbox. It renders: avatar + title row (title | timestamp + unread dot) + message + optional artifact buttons. The title and message text wrap freely with no line limits.

### Avatar rendering
Notifications use `render_agent_avatar` (`item_rendering.rs:166-236`) which has its own icon/color logic. Vertical tabs use `render_pane_icon_with_status` with `IconWithStatusVariant` (`vertical_tabs.rs:108-240`) which has better styling: proper CLI brand colors, cutout rings on status badges, and background color matching the panel. These two implementations are independent and visually inconsistent.

### Toast
`render_toast` (`toast_stack.rs:347-435`): 360px wide, 10v/16h padding, 6px radius, default shadow. Close button at top-right via `OffsetPositioning`. Keybinding hint on newest toast.

### Mailbox
`NotificationMailboxView::render` (`view.rs:340-408`): 420px wide, 500px max height. Header has "Notifications" (semibold) + X close button. Filter bar has 4px button gap, 16px horizontal padding. "All" filter doesn't show a count. Items use 12v/16h padding.

## Proposed Changes

### 1. Add `branch` field to `NotificationItem`

**File**: `app/src/ai/agent_management/notifications/item.rs`

Add `pub branch: Option<String>` to `NotificationItem` and the `new()` constructor. This determines whether the item renders with the "rich" layout (branch row) or "simple" layout (current style).

### 2. Thread branch context through notification creation

**File**: `app/src/ai/agent_management/agent_management_model.rs`

Update `add_notification` to accept an `Option<String>` branch parameter. Pass it through to `NotificationItem::new`.

The branch name should come from the same source as the branch chip in the prompt and the vertical tabs subtitle. The vertical tabs already call `TerminalView::current_git_branch(ctx)` (`app/src/terminal/view/tab_metadata.rs:49-63`), which reads the `ShellGitBranch` context chip or falls back to `GitRepoStatusModel`. Since `add_notification` already receives `terminal_view_id`, the implementation should:

1. Resolve the `TerminalView` from `terminal_view_id`.
2. Call `terminal_view.current_git_branch(ctx)` to get the branch name.
3. Pass it as the `branch` parameter.

This works for both Oz conversations and CLI sessions since both are associated with a terminal view that has prompt chip state.

**Fallback**: If the terminal view is not accessible (e.g. window closed), pass `None`. The UI gracefully falls back to the simple layout.

### 3. Extract shared avatar rendering

**New file**: `app/src/ui_components/icon_with_status.rs`

Extract the circle icon rendering into a shared UI component that both toasts/mailbox and vertical tabs can use.

Extract from `vertical_tabs.rs`:
- `render_pane_icon_with_status` and its helpers (`render_with_optional_status_badge`)
- The `IconWithStatusVariant` enum and the icon sizing constants

The extracted functions should accept a **size parameter** rather than using hardcoded constants, since vertical tabs and notifications use different sizes:
- Vertical tabs: `CIRCLE_NEUTRAL_ICON_SIZE = 16.`, `CIRCLE_AGENT_ICON_SIZE = 10.`, `CIRCLE_NEUTRAL_PADDING = 4.`, `CIRCLE_AGENT_PADDING = 5.`
- Notifications (current `render_agent_avatar`): icon 16px, padding 8px → 32px circle total

The shared rendering function should take a size config struct or individual size parameters so both call sites can specify their dimensions.

Keep a notification-specific mapping helper in `item_rendering.rs`:
```rust
fn render_notification_avatar(
    agent: NotificationSourceAgent,
    category: NotificationCategory,
    theme: &WarpTheme,
) -> Box<dyn Element>
```

This function maps `NotificationSourceAgent` → `IconWithStatusVariant` and `NotificationCategory` → status badge, then delegates to the shared circle icon renderer with notification-appropriate sizing.

The mapping:
- `NotificationSourceAgent::Oz` → `IconWithStatusVariant::OzAgent` with `status` derived from category
- `NotificationSourceAgent::CLI(agent)` → `IconWithStatusVariant::CLIAgent` with `status` derived from category

For the `status` parameter, add a helper that converts `NotificationCategory` → `ConversationStatus`:
- `NotificationCategory::Complete` → `ConversationStatus::Success`
- `NotificationCategory::Request` → `ConversationStatus::Blocked { blocked_action: String::new() }`
- `NotificationCategory::Error` → `ConversationStatus::Error`

This reuses `ConversationStatus::status_icon_and_color` (`conversation.rs:3531`) which the vertical tabs already use.


### 4. Split `render_notification_item_content` into rich and simple variants

**File**: `app/src/ai/agent_management/notifications/item_rendering.rs`

Currently `render_notification_item_content` is one function. Refactor into:

**`render_rich_text_column`** (when `item.branch.is_some()`):
- Branch row: git-branch icon (10px) + branch name (12px, sub_text color) on left.
  - Toast: chevron-right icon on right (when content is truncated).
  - Mailbox: timestamp + unread dot on right.
- Title: 14px semibold, character-count truncated.
- Message: 14px regular, sub_text, character-count truncated.
- Artifact buttons with 48px left padding.

**`render_simple_text_column`** (when `item.branch.is_none()`):
- Current layout: title | timestamp + unread dot in SpaceBetween row.
- Message below.
- Title and message use the same character-count truncation behavior as rich items.
- Mailbox simple rows do not show an expand chevron; toast simple rows show the chevron in the title row when content is truncated.

Both variants call the shared avatar renderer from step 3.

Extract shared sub-rendering into free functions that both variants can use:
- `render_branch_row(branch, right_side_content, appearance)` — the git-branch icon + text row
- `render_clamped_title(title, max_lines, appearance)` — title text with line clamping
- `render_clamped_message(message, max_lines, expand_affordance, appearance)` — message with optional expand
- `render_timestamp_with_dot(created_at, is_read, appearance)` — timestamp + optional unread dot

The existing `render_git_branch_text` in `vertical_tabs.rs:1899-1921` is a good reference for the branch row rendering and could be reused or extracted to a shared location.

A public entry point selects the variant:
```rust
fn render_notification_item_content(
    item: &NotificationItem,
    artifact_buttons: Option<&ViewHandle<ArtifactButtonsRow>>,
    context: NotificationRenderContext,
    appearance: &Appearance,
) -> Box<dyn Element>
```

Where `NotificationRenderContext` is an enum:
```rust
enum NotificationRenderContext {
    Toast,
    Mailbox,
}
```

This context determines whether the branch row's right side shows a chevron (toast) or timestamp + unread dot (mailbox).

### 5. Implement character-count truncation with expand

Truncate title and message text using a character-count heuristic rather than a visual line-clamp primitive (which doesn't exist in the UI framework).

Constants:
- `COLLAPSED_MAX_CHARS = 100` — max characters when collapsed
- `EXPANDED_MAX_CHARS = 500` — max characters when expanded

A `truncate_text(text, max_chars)` helper appends `…` when the text exceeds the limit.

A `content_is_truncated(title, message)` helper returns true when either exceeds `COLLAPSED_MAX_CHARS`, which controls whether the expand chevron is rendered.

For the expand affordance:
- Add `message_expanded: bool` state to `NotificationToastItem` (toast_stack.rs) and per-item state in `NotificationMailboxView` (view.rs).
- Both toast and mailbox use a chevron icon (ChevronRight when collapsed, ChevronDown when expanded) as the expand affordance.
- The chevron is rendered in the branch row (rich layout) or title row (simple layout, toast only).

### 6. Update toast container and close button

**File**: `app/src/ai/agent_management/notifications/toast_stack.rs`

In `render_toast`:
- Width: `360.` → `420.` (line 390)
- Padding: `.with_vertical(10.).with_horizontal(16.)` → `.with_uniform(12.)` (line 383)
- Corner radius: `Radius::Pixels(6.)` → `Radius::Pixels(8.)` (line 386)
- Drop shadow: removed (was `DropShadow::default()`)

In `render_close_button`:
- Change `PositionedElementAnchor::TopRight` / `ChildAnchor::TopRight` → `PositionedElementAnchor::TopLeft` / `ChildAnchor::TopLeft` (line 414)
- Adjust offset vector from `(4., -4.)` → `(-4., -4.)` or similar to position outside the top-left
- Add a visible border to the close button circle (add `Border::all(0.67)` with outline color)

### 7. Remove timestamp and unread dot from toast

In the rich/simple layout variants, when `context == NotificationRenderContext::Toast`:
- Rich layout: no timestamp/dot in branch row (chevron-right only when truncated).
- Simple layout: no timestamp — show only the title (plus expand chevron when truncated).

### 8. Update mailbox header

**File**: `app/src/ai/agent_management/notifications/view.rs`

In `render_header`:
- Change title font weight from `Weight::Semibold` → remove the weight override (defaults to regular).
- Update padding from `.with_vertical(8.).with_left(16.).with_right(8.)` → `.with_vertical(8.).with_horizontal(12.)` (line 436-440).

### 9. Update mailbox filter bar

**File**: `app/src/ai/agent_management/notifications/view.rs`

In `render_filter_bar`:
- Change filter button spacing from `4.` → `2.` (line 456).
- Update "All" filter label to include count: change the conditional at lines 465-473 so `NotificationFilter::All` also shows `"All tabs ({count})"`.
- Update bar padding from `.with_vertical(12.).with_horizontal(16.)` → `.with_vertical(12.).with_left(12.).with_right(6.)` (line 537).

### 10. Update mailbox item padding

**File**: `app/src/ai/agent_management/notifications/view.rs`

In `render_notification_item`:
- Rich items (branch present): padding `Padding::uniform(12.)` (was `.with_vertical(12.).with_horizontal(16.)`)
- Simple items (branch absent): keep `.with_vertical(12.).with_horizontal(16.)`

### 11. Timestamp font size

In both rich and simple item layouts, change timestamp font size from `14.` → `12.` in `item_rendering.rs`.

## End-to-End Flow

### Notification creation
1. Conversation status changes or CLI agent session event fires.
2. `AgentNotificationsModel::add_notification` is called with the new `branch` parameter.
3. `NotificationItem` is created with `branch: Option<String>`.
4. `AgentManagementEvent::NotificationAdded` is emitted.

### Toast rendering
1. `AgentNotificationToastStack` receives the event, creates a `NotificationToastItem`.
2. `render_toast` builds the toast container with updated sizing (420px, 12px padding, 8px radius, no drop shadow).
3. `render_notification_item_content` is called with `NotificationRenderContext::Toast`.
4. If `item.branch.is_some()`, renders rich layout (branch row with chevron, clamped title/message).
5. If `item.branch.is_none()`, renders simple layout (title only, no timestamp).
6. Avatar uses `render_notification_avatar` → delegates to vertical tabs primitives.
7. Close button positioned at top-left with border.

### Mailbox rendering
1. `NotificationMailboxView::render` builds the popup with updated container (4px top padding, no drop shadow).
2. Header renders with regular-weight title + close button.
3. Filter bar renders with 2px gap, count on "All", updated padding.
4. Each item calls `render_notification_item_content` with `NotificationRenderContext::Mailbox`.
5. Rich items show branch row (with timestamp + unread dot on right), truncated title/message.
6. Simple items show current layout with timestamp in title row.

## Risks and Mitigations

**Text truncation detection**: Precisely detecting whether wrapped text exceeds N lines is non-trivial without a max-lines primitive. Mitigation: use `ConstrainedBox::with_max_height` for visual clamping and a character-length heuristic to decide whether to show the expand affordance. This won't be pixel-perfect but will be correct for the vast majority of cases.

**Avatar extraction scope**: Moving `IconWithStatusVariant` and friends out of `vertical_tabs.rs` touches a high-traffic file. Mitigation: keep the extraction mechanical — move types and functions without changing behavior, then add the notification-specific mapping in the new `avatar.rs` module.

**Branch availability**: Branch data comes from `TerminalView::current_git_branch()` which reads the shell's git branch chip. It is available for any terminal in a git repo but may be `None` for sessions outside a repo. The UI gracefully falls back to the simple layout when branch is `None`.

## Testing and Validation

- `cargo check` to verify compilation after each step.
- Visual comparison against Figma mocks for both toast and mailbox.
- Verify toast: 420px width, 12px padding, 8px radius, close button at top-left on hover, keybinding hint on newest.
- Verify mailbox: 4px top padding, header with title + close, filter bar spacing, "All tabs (N)" count.
- Verify rich items: branch row, truncated title, truncated message, expand chevron works.
- Verify simple items: current layout preserved, timestamp at 12px font.
- Verify avatars match vertical tabs (Oz, CLI brand colors, status badges with cutout rings).
- Existing notification unit tests in `item_tests.rs` should still pass after adding the `branch` field.

## Follow-ups

- **CLI session branch**: Extend the CLI agent plugin protocol to surface the working branch name in `CLIAgentSessionContext`.
- **Settings gear**: Punted from this pass. Add to mailbox header when design is finalized.
- **Approve/decline on toasts**: Punted from this pass. Will need plumbing to dispatch permission responses directly from toasts.
- **Precise line measurement**: If the character-count heuristic for expand affordances proves insufficient, invest in a proper max-lines text primitive in the UI framework.
