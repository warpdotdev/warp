# APP-3742: Tech Spec — Circular Pane Icons and Metadata Slot Rules

## Problem

The vertical tabs panel currently renders each pane row with a small 12px inline icon next to the title text. All pane types look similar at a glance. The product spec (PRODUCT.md in this directory) calls for:

1. A prominent circular "avatar" icon per pane row in both compact and expanded modes, with branded variants for agent sessions.
2. Deterministic per-pane-type rules for what data goes in each text slot (title, description, metadata).
3. A `Branch` variant added to the "Pane title as" setting.
4. An unread-activity dot for agent terminal panes, using existing notification infrastructure.

## Relevant code

**Primary file — all rendering logic lives here:**
- `app/src/workspace/view/vertical_tabs.rs` — the entire vertical tabs rendering module

**Key types in vertical_tabs.rs:**
- `TypedPane` enum (line 1022) — resolved pane type with access to typed data (TerminalPane, CodePane, etc.)
- `PaneProps` struct (line 199) — props bag assembled per pane row, containing title, subtitle, typed pane, etc.
- `PaneProps::new()` (line 1123) — constructs props from `PaneConfiguration` title/secondary
- `TerminalPrimaryLineData` enum (line 219) — determines the terminal title content and font
- `terminal_primary_line_data()` (line 1284) — priority logic for terminal title resolution
- `TerminalKindBadgeState` struct (line 245) — tracks whether terminal is Oz, ambient, or CLI agent

**Rendering entry points:**
- `render_pane_row()` (line 981) — expanded mode row
- `render_compact_pane_row()` (line 2376) — compact mode row
- `render_pane_row_element()` (line 96) — shared wrapper (hover, click, background, border)
- `render_terminal_row_content()` (line 1394) — expanded terminal 3-line content
- `render_non_terminal_primary_row()` (line 1861) — non-terminal title with inline icon

**Setting:**
- `app/src/workspace/tab_settings.rs:186-200` — `VerticalTabsPrimaryInfo` enum (currently `Command`, `WorkingDirectory`) and its synced setting registration

**Notification infrastructure (for unread dot):**
- `app/src/ai/agent_management/notifications/item.rs` — `NotificationItems` with `is_read` per item, `mark_all_terminal_view_items_as_read()`, `items_filtered(Unread)`
- `app/src/ai/agent_management/agent_management_model.rs:27` — `AgentNotificationsModel` singleton, emits `AgentManagementEvent::NotificationUpdated`

**Conversation status (for status badge icons):**
- `app/src/ai/agent/conversation.rs:3491` — `ConversationStatus` enum (`InProgress`, `Success`, `Error`, `Cancelled`, `Blocked`)
- `app/src/ai/agent/conversation.rs:3531` — `status_icon_and_color()` returns `(Icon, ColorU)` per status
- `app/src/ai/conversation_status_ui.rs:14` — `render_status_element()` helper

**CLI agent icons:**
- `vertical_tabs.rs:1918` — `cli_agent_warp_icon()` maps `CLIAgent` to branded `WarpIcon`

**Language file icons:**
- `app/src/code/mod.rs` — `icon_from_file_path()` returns language-specific icon element
- Used in `vertical_tabs.rs:1846` by `resolve_non_terminal_icon()`

**Pane configuration:**
- `app/src/pane_group/pane/mod.rs:681` — `PaneConfiguration` struct with `title`, `title_secondary`

## Current state

### Expanded mode layout

`render_pane_row()` builds a `Flex::column` with 2–3 child lines, wrapped in `render_pane_row_element()` for hover/click/background. Terminal panes get 3 lines (primary, secondary, tertiary); non-terminal panes get 1–2 lines (title + optional subtitle). There is no circular icon — the 12px icon is inlined into the title row via `render_non_terminal_primary_row()`.

### Compact mode layout

`render_compact_pane_row()` renders a single primary row. For terminals it delegates to `render_terminal_primary_line_for_view()` (with a 16px prefix icon); for non-terminals to `render_non_terminal_primary_row()` (with a 12px inline icon). No circular icon.

### Terminal title resolution

`terminal_primary_line_data()` returns the title text with priority:
1. CLI agent title
2. Oz conversation title (with status)
3. Terminal title if it differs from working directory
4. Last completed command
5. "New session" fallback

Per PRODUCT.md, items 3–5 collapse to just "terminal title" (no differing-from-WD check, no command fallback, no "New session").

### "Pane title as" setting

`VerticalTabsPrimaryInfo` has `Command` and `WorkingDirectory` variants. Used in `render_terminal_row_content()` (expanded) and `render_compact_pane_row()` (compact) to swap which data goes in line 1 vs line 2. Registered as a synced cloud setting.

### Unread tracking

`AgentNotificationsModel` stores `NotificationItems` with `is_read: bool` per item, keyed by `terminal_view_id`. `mark_all_terminal_view_items_as_read()` clears unread state for a terminal. The model emits `AgentManagementEvent::NotificationUpdated` on changes. Currently only consumed by the notification mailbox UI — the vertical tabs panel does not subscribe to it.

## Proposed changes

### 1. Circular icon rendering

Add a new function `render_pane_circle_icon()` in `vertical_tabs.rs` that returns a `Box<dyn Element>` — a `Stack` containing:
- A circular `Container` (background + `CornerRadius::with_all(Radius::Pixels(CIRCLE_RADIUS))`) holding the inner icon.
- An optional positioned status badge overlay in the bottom-right.

**Variants dispatched by a new enum:**

```rust
enum CircleIconVariant<'a> {
    /// Neutral circle: fg_overlay_2 background, 16px type icon
    Neutral { icon: WarpIcon },
    /// Oz agent: dark background, 10px Oz icon, status badge
    OzAgent { status: Option<&'a ConversationStatus>, is_ambient: bool },
    /// CLI agent: brand-colored background, 10px agent icon, status badge
    CLIAgent { agent: CLIAgent, status: Option<&'a ConversationStatus> },
    /// Language-specific file icon in neutral circle
    LanguageFile { icon_element: Box<dyn Element> },
}
```

The badge uses `status_icon_and_color()` from `ConversationStatus`, rendered at 9px inside a 12px cutout container. The cutout ring uses the panel background color.

**Integration:** Both `render_pane_row()` and `render_compact_pane_row()` will prepend the circle icon to the left of the text column using `Flex::row().with_child(circle_icon).with_child(text_column)`.

### 2. Refactor row content into a unified slot model

Replace the current divergent code paths for terminal vs non-terminal with a single `PaneRowSlots` struct:

```rust
struct PaneRowSlots {
    circle_icon: Box<dyn Element>,
    title: Box<dyn Element>,
    /// Indicator shown inline with title (unread dot, unsaved dot)
    title_indicator: Option<Box<dyn Element>>,
    /// Line 2: shown in both compact (10px muted) and expanded (12px sub-text)
    subtitle: Option<Box<dyn Element>>,
    /// Line 3: shown only in expanded mode
    metadata: Option<Box<dyn Element>>,
}
```

A new function `resolve_pane_row_slots()` takes `PaneProps`, the view mode, the "Pane title as" setting, and `&AppContext`, and returns `PaneRowSlots`. This function centralizes all per-pane-type logic from the product spec:

- For terminals: resolves icon variant, title text (per simplified priority), subtitle/description/metadata per the "Pane title as" setting.
- For code panes: resolves language icon, filename, path, "and N more", unsaved dot.
- For notebooks, settings, and other types: maps `PaneConfiguration` title/secondary.

Two rendering functions consume `PaneRowSlots`:
- `render_compact_row_from_slots()` — circle icon + title row (with indicator) + subtitle
- `render_expanded_row_from_slots()` — circle icon + title row + description + metadata

Both delegate to `render_pane_row_element()` for the shared hover/click wrapper.

### 3. Terminal title priority simplification

Update `terminal_primary_line_data()` to remove fallbacks 3–5 and replace with:

```rust
fn terminal_primary_line_data(...) -> TerminalPrimaryLineData {
    if let Some(cli_agent_title) = cli_agent_title {
        return TerminalPrimaryLineData::StatusText { text: cli_agent_title, status: cli_agent_status };
    }
    if let Some(conversation_title) = conversation_display_title {
        return TerminalPrimaryLineData::StatusText { text: conversation_title, status: conversation_status };
    }
    TerminalPrimaryLineData::Text {
        text: terminal_title.trim().to_string(),
        font: TerminalPrimaryLineFont::Monospace,
    }
}
```

The `StatusText` variant no longer renders an inline status icon prefix in the title — conversation status is shown exclusively via the circular icon's status badge.

### 4. "Pane title as" — add `Branch` variant

Add `Branch` to `VerticalTabsPrimaryInfo`:

```rust
#[derive(Default, Debug, serde::Serialize, serde::Deserialize, PartialEq, Copy, Clone)]
pub enum VerticalTabsPrimaryInfo {
    #[default]
    Command,
    WorkingDirectory,
    Branch,
}
```

Because the enum is already registered via `implement_setting_for_enum!` with `SyncToCloud::Globally`, the new variant is automatically synced. Existing users with persisted `Command` or `WorkingDirectory` are unaffected; `Branch` only activates when explicitly selected.

In `resolve_pane_row_slots()`, when building terminal expanded slots:

| Setting          | Line 1 (title)       | Line 2 (description)      | Line 3 metadata left     |
|------------------|----------------------|---------------------------|--------------------------|
| Command          | command/conversation | working directory          | git branch               |
| WorkingDirectory | working directory    | command/conversation       | git branch               |
| Branch           | git branch           | working directory          | (omit — already in title)|

Compact mode ignores this setting (per PRODUCT.md).

Update the settings popup (`render_settings_popup()` and `render_primary_info_option()`) to add the third option with label "Branch".

### 5. Unread-activity dot

Add a query method to `NotificationItems`:

```rust
pub(crate) fn has_unread_for_terminal_view(&self, terminal_view_id: EntityId) -> bool {
    self.items.iter().any(|item| item.terminal_view_id == terminal_view_id && !item.is_read)
}
```

In `resolve_pane_row_slots()`, for terminal panes with an agent session, query `AgentNotificationsModel::as_ref(app).notifications().has_unread_for_terminal_view(terminal_view.id())`. If true, set `title_indicator` to a `CircleFilled` icon element.

The existing `mark_all_terminal_view_items_as_read()` is already called when a terminal pane gains focus (via `AgentNotificationsModel::mark_items_from_terminal_view_read`). To trigger re-renders, subscribe the `VerticalTabsPanelState` (or the workspace) to `AgentManagementEvent::NotificationUpdated` and call `ctx.notify()`.

### 6. Unsaved-changes dot

Already partially implemented via `TypedPane::badge()` which returns `Some("Unsaved")` for dirty code panes. Reuse this in `resolve_pane_row_slots()`: when `badge()` returns `Some(_)`, set `title_indicator` to the same `CircleFilled` icon.

### 7. Settings popup label update

In `render_settings_popup()`, rename the "Show first" header text to "Pane title as". Add a third `render_primary_info_option()` call for `VerticalTabsPrimaryInfo::Branch` with label "Branch".

## End-to-end flow

**Rendering a terminal pane row (expanded, "Pane title as: Command"):**

1. `render_tab_group()` iterates pane IDs, builds `PaneProps` via `PaneProps::new()`.
2. `PaneProps::new()` reads `PaneConfiguration` title/secondary, resolves `TypedPane`.
3. `resolve_pane_row_slots()` is called:
   - Detects `TypedPane::Terminal(terminal_pane)`.
   - Reads `TerminalView` from pane → gets working directory, git branch, conversation status, CLI agent session.
   - Builds `CircleIconVariant::OzAgent { status, is_ambient: false }` (or CLI/Neutral depending on session type).
   - Calls `render_pane_circle_icon()` → circle icon element with status badge.
   - Calls `terminal_primary_line_data()` → title text.
   - Checks `has_unread_for_terminal_view()` → sets `title_indicator` if unread.
   - Sets `subtitle` = working directory text element.
   - Sets `metadata` = git branch + diff stats badge + PR badge row.
4. `render_expanded_row_from_slots()` assembles: `Flex::row(circle_icon, Flex::column(title_row, subtitle, metadata))`.
5. `render_pane_row_element()` wraps in hover/click/background.

**Unread dot lifecycle:**

1. Agent finishes a task → `AgentNotificationsModel` creates a `NotificationItem` with `is_read: false`.
2. Model emits `AgentManagementEvent::NotificationUpdated`.
3. Workspace (or vertical tabs panel via subscription) receives event → calls `ctx.notify()`.
4. `resolve_pane_row_slots()` re-runs → `has_unread_for_terminal_view()` returns true → dot shown.
5. User clicks the pane row → `WorkspaceAction::FocusPane` → terminal gains focus → `mark_items_from_terminal_view_read()` clears unread → event emitted → dot disappears on next render.

## Risks and mitigations

**Risk: Circular icon adds vertical height to compact rows.** The circle is ~25px tall. Compact rows currently have 8px top/bottom padding. If the circle makes rows taller than intended, adjust vertical padding or use `CrossAxisAlignment::Center` in the horizontal flex to let the icon vertically center without forcing extra height.

**Risk: `has_unread_for_terminal_view()` is O(n) over all notifications.** With the 100-item cap on `NotificationItems`, this is negligible. No mitigation needed.

**Risk: Adding `Branch` to `VerticalTabsPrimaryInfo` is a persisted enum change.** New variant is additive — serde deserialization of `"Command"` or `"WorkingDirectory"` from cloud still works. A user on an older client seeing a `"Branch"` value from cloud sync would fail to deserialize and fall back to the default (`Command`). This is acceptable — older clients simply ignore the new variant.

## Testing and validation

- **Unit tests for `terminal_primary_line_data()`:** Update existing tests in `vertical_tabs_tests.rs` to verify the simplified 3-step priority (CLI agent → conversation → terminal title) and ensure the old fallbacks (last command, "New session") are removed.
- **Unit test for `has_unread_for_terminal_view()`:** Add to `item_tests.rs` — create items with different `terminal_view_id` and `is_read` states, verify query correctness.
- **Visual validation:** Per the validation section in PRODUCT.md — open a mix of pane types, verify circle icons, slot content, indicators, and "Pane title as" setting behavior.
- **Presubmit:** `cargo clippy` and `cargo fmt` must pass. Run `cargo nextest run -p warp` for the workspace tests.

### 8. "Additional metadata" setting for compact subtitle

**New enum in `tab_settings.rs`:**

```rust
#[derive(Default, Debug, serde::Serialize, serde::Deserialize, PartialEq, Copy, Clone)]
pub enum VerticalTabsCompactSubtitle {
    #[default]
    Branch,
    WorkingDirectory,
    Command,
}
```

Registered via `implement_setting_for_enum!` with `SyncToCloud::Globally`, same as the other vertical tabs settings. Added to the `TabSettings` group.

**Resolving the effective subtitle:**

The persisted `VerticalTabsCompactSubtitle` value may be incompatible with the current `VerticalTabsPrimaryInfo` (e.g., user has `Branch` as both title and subtitle). A helper function `resolve_compact_subtitle()` maps the combination:

```rust
fn resolve_compact_subtitle(
    primary: VerticalTabsPrimaryInfo,
    subtitle_pref: VerticalTabsCompactSubtitle,
) -> VerticalTabsCompactSubtitle {
    // If the subtitle preference is the same category as the title, fall back to default.
    let is_conflict = matches!(
        (primary, subtitle_pref),
        (VerticalTabsPrimaryInfo::Command, VerticalTabsCompactSubtitle::Command)
        | (VerticalTabsPrimaryInfo::WorkingDirectory, VerticalTabsCompactSubtitle::WorkingDirectory)
        | (VerticalTabsPrimaryInfo::Branch, VerticalTabsCompactSubtitle::Branch)
    );
    if is_conflict {
        default_compact_subtitle(primary)
    } else {
        subtitle_pref
    }
}

fn default_compact_subtitle(primary: VerticalTabsPrimaryInfo) -> VerticalTabsCompactSubtitle {
    match primary {
        VerticalTabsPrimaryInfo::Command => VerticalTabsCompactSubtitle::Branch,
        VerticalTabsPrimaryInfo::WorkingDirectory => VerticalTabsCompactSubtitle::Branch,
        VerticalTabsPrimaryInfo::Branch => VerticalTabsCompactSubtitle::Command,
    }
}
```

**New action:** `WorkspaceAction::SetVerticalTabsCompactSubtitle(VerticalTabsCompactSubtitle)` — mirrors the existing `SetVerticalTabsPrimaryInfo` pattern.

**Settings popup changes:**

In `render_settings_popup()`, when `current_mode == VerticalTabsViewMode::Compact`, add an "Additional metadata" section between the "Pane title as" options and the divider. The section shows:
- A header label "Additional metadata" (same style as "Pane title as")
- Two selectable options — the two metadata categories not used as the title
- Each option dispatches `SetVerticalTabsCompactSubtitle`

The option labels and values depend on `current_primary_info`:
- Command title → options: "Branch" (Branch), "Working Directory" (WorkingDirectory)
- WorkingDirectory title → options: "Branch" (Branch), "Command / Conversation" (Command)
- Branch title → options: "Command / Conversation" (Command), "Working Directory" (WorkingDirectory)

Two new `MouseStateHandle` fields are needed in `VerticalTabsPanelState` for the two option hover states.

**Compact rendering changes:**

In `render_compact_pane_row()`, read `VerticalTabsCompactSubtitle` from settings and pass through `resolve_compact_subtitle()`. Use the resolved value to determine the subtitle element for terminal panes.

## Follow-ups

- **Animated status badge:** The Figma mock's `clock_loader` icon implies rotation animation. The current `Icon::ClockLoader` is static. Adding animation is a separate task.
- **Group-by modes:** The settings popup mock shows "Group by" options. These will change how tab groups are constructed and are a separate feature.
