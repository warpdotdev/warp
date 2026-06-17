# APP-3742: Vertical Tabs v2 — Circular Pane Icons and Metadata Slot Rules

## Summary

Redesign the vertical tabs pane item layout with prominent circular per-pane icons and a refined set of per-pane-type rules governing which metadata appears in each "slot" of the item lockup. Both compact and expanded modes get the new icon system and updated slot content.

## Problem

The current vertical tabs pane rows use small inline icons that are hard to distinguish at a glance. Agent sessions, file editors, notebooks, and terminals all look similar until the user reads the text. The new design elevates the pane icon into a prominent, branded circular element that communicates the pane type — and for agent sessions, the agent identity and status — before the user reads any text.

Additionally, the metadata shown per pane type lacks consistent rules: expanded mode shows the same 3-line structure for all panes even when some lines have no meaningful content (e.g. working directory for Settings). This spec defines explicit rules for what each slot contains per pane type.

## Goals

- Replace the current inline pane-type icon with a circular "avatar" icon system that visually distinguishes pane types, agent identities, and agent status at a glance.
- Define deterministic rules for which metadata is shown in each slot for every pane type, in both compact and expanded modes.
- Add an unread-activity indicator (filled dot) for agent panes with new output the user hasn't viewed.
- Ensure pane types without meaningful data for a slot gracefully omit that slot rather than showing empty or misleading content.

## Non-goals

- **Agent status badge design**: The exact set of status badge icons (running, complete, error, etc.) and their colors are defined elsewhere. This spec covers *where* the badge appears, not every status variant.
- **Group headers or group-by changes**: Out of scope.
- **Compact/expanded toggle UI**: Already shipped per APP-3656.
- **Compact mode configuration popup**: The compact mode settings popup (see [compact config mock](https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7363-74353&m=dev)) is out of scope for this spec.

## Figma / design references

- Compact mode: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7363-71448&m=dev
- Expanded mode: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7363-74446&m=dev
- Expanded mode settings popup ("Pane title as"): https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7363-75462&m=dev

### Intentional deviations from Figma

The mocks contain several inconsistencies that are resolved as follows:

1. **"Code editor" pane**: The mock shows a pane titled "Code editor" with a generic `</>` icon and subtitle "Multiple files". There is no separate "Code editor" pane type — this is the Code pane (file editor). The correct title is the active file name (e.g., `view_ui.rs`), and the subtitle should read `and N more` when multiple tabs are open.
2. **"Testing block unfurling"**: The mock uses a file icon but shows "Last updated 5 mins ago by Zach Bai" metadata, which is notebook-specific. This item is a Notebook pane. The icon should be the notebook/file icon, and the metadata is correct for notebooks.
3. **Settings directory line**: The expanded mock shows `~/warp-internal` as the description line for Settings. Settings has no meaningful working directory. The description line should show the active settings page name instead.
4. **"No unsaved changes" text**: The expanded mock shows explicit "No unsaved changes" text for a file pane. This text should not appear. Instead, only show the unsaved-changes dot indicator when the file IS dirty; show nothing when clean.

## User experience

### Layout anatomy

Each pane item consists of a **circular icon** on the left and a **text column** on the right.

**Compact mode** (icon + 2 text lines):
```
[CIRCLE ICON]  Title ..................... [indicator]
               Subtitle (10px, muted)
```

**Expanded mode** (icon + 3 text lines):
```
[CIRCLE ICON]  Title ..................... [indicator]
               Description (12px, lighter gray)
               metadata-left ............ badges-right
```

Lines that have no content for a given pane type are omitted entirely (the item shrinks vertically). The text column is flexible-width and truncates with ellipsis.

### Circular icon system

The circular icon replaces the current small inline icon. It is the leftmost element in every pane row in both compact and expanded modes.

#### Neutral circle (most pane types)

- Circular background in `fg_overlay_2`
- 16px pane-type icon centered inside
- No status badge

Used for: plain Terminal, Code, File, Notebook, Settings, Workflow, AI Document, AI Fact, MCP Server, Env Var Collection, Environment Management, Execution Profile Editor, Code Diff.

#### Oz agent circle

- Circular background in dark/black (`background` color)
- 10px Oz logo centered inside
- Status badge in the bottom-right corner showing conversation status (clock = running, check = complete, etc.)

Used for: Terminal panes with an active Oz agent conversation.

#### Ambient Oz agent circle

- Same as Oz agent circle but uses the OzCloud icon variant.

Used for: Terminal panes that are ambient agent sessions.

#### CLI agent circle (Claude, Gemini, etc.)

- Circular background in the agent's brand color (e.g., `#e8704e` for Claude)
- 10px agent logo centered inside
- Status badge in the bottom-right corner showing conversation status

Used for: Terminal panes running a recognized CLI agent (Claude Code, Gemini CLI, etc.).

#### Language-specific file icon

For Code panes, the neutral circle's inner icon is replaced with the language-specific file icon derived from the active file's extension (e.g., Rust gear for `.rs`, TypeScript logo for `.ts`). Falls back to the generic `Code2` (`</>`) icon if no language icon is available.

### Status badge

The small overlay anchored at the bottom-right of the circular icon:
- Surrounded by a thin ring matching the panel background (creating a "cutout" effect)
- Contains a 9px status icon:
  - `clock_loader`: Agent is running/thinking
  - `check`: Agent has completed successfully
  - Other icons for error, stopped, etc. (as defined by the existing conversation status system)
- Only shown on terminal panes with an active agent session (Oz or CLI agent)

### "Pane title as" setting

The existing `VerticalTabsPrimaryInfo` setting is extended with a third option and renamed to **"Pane title as"** in the settings popup UI. It controls which piece of terminal metadata occupies the title (line 1). The three options are:

- **Command** (default)
- **Working Directory**
- **Branch**

This setting applies to terminal panes in both compact and expanded modes. Font size and color treatment are coupled to line position, not semantic content: line 1 always uses 12px main text color, line 2 always uses 12px lighter sub-text color.

**Expanded mode mapping:**

In expanded mode, the two remaining metadata categories (the ones not selected as title) are always shown — one as the description (line 2) and one as the metadata left (line 3). There is no additional selection needed.

- **Command**: Title = command/conversation. Description = working directory. Metadata left = git branch.
- **Working Directory**: Title = working directory. Description = command/conversation. Metadata left = git branch.
- **Branch**: Title = git branch. Description = command/conversation. Metadata left = working directory.

**Compact mode mapping:**

In compact mode, only one of the two remaining metadata categories can be shown as the subtitle (line 2). The **"Additional metadata"** setting controls which one is displayed.

### "Additional metadata" setting (compact mode only)

A new synced cloud setting (`VerticalTabsCompactSubtitle`) controls which metadata category is shown as the compact subtitle for terminal panes. The available options depend on the current "Pane title as" selection — the two categories not used as the title are offered as choices.

**Available options per "Pane title as" selection:**

- **Pane title as: Command** → Additional metadata options: Branch (default), Working Directory
- **Pane title as: Working Directory** → Additional metadata options: Branch (default), Command/Conversation
- **Pane title as: Branch** → Additional metadata options: Command/Conversation (default), Working Directory

**Defaults:** Each "Pane title as" selection has a sensible default subtitle so the setting works out of the box:
- Command → Branch
- Working Directory → Branch
- Branch → Command/Conversation

**Settings popup behavior:**

The "Additional metadata" section appears in the settings popup **only when compact mode is active**. When expanded mode is selected, this section is hidden (since expanded mode shows all three metadata categories across its 3 lines).

The section renders as a set of selectable options (same style as "Pane title as") with the header "Additional metadata". Only the two options relevant to the current "Pane title as" selection are shown.

**Subtitle rendering rules:**

- When the subtitle is a git branch: render with `[git-branch icon]` prefix, 10px, sub-text color.
- When the subtitle is a working directory: render as plain text, 10px, sub-text color, clip from start.
- When the subtitle is command/conversation: render using the terminal primary line data (same as title but at 10px sub-text color).

**Persistence:** Synced cloud setting, consistent with `VerticalTabsPrimaryInfo`. If the persisted value is incompatible with the current "Pane title as" selection (e.g., user had Branch subtitle but switches title to Branch), fall back to the default for that title selection.

### Pane-type slot rules

#### Terminal pane

**Icon:** Agent circle (Oz, CLI, or ambient) if an agent session is active; neutral circle with Terminal icon otherwise.

**Compact:**
- **Title:** Determined by "Pane title as" setting. Priority for command/conversation: (1) CLI agent title, (2) Oz conversation title, (3) terminal title.
- **Subtitle:** Determined by "Additional metadata" setting. See the setting section above for the full mapping and defaults.

**Expanded (default — "Pane title as: Command"):**
- **Title:** Same as compact title. Optionally includes the unread-activity dot (see Indicators below).
- **Description (line 2):** Working directory path (e.g., `~/warp-internal`).
- **Metadata (line 3):**
  - Left: `[git-branch icon] branch-name`
  - Right: diff stats badge (`+N -M`, green/red colored) if the working tree has changes; PR badge (`[GitHub icon] #NNNN`) if a pull request is associated.

**Expanded — "Pane title as: Working Directory":** Line 1 and line 2 swap: the working directory becomes the title and the command/conversation title becomes the description. Metadata row is unchanged.

**Expanded — "Pane title as: Branch":** Line 1 becomes the git branch name. Line 2 becomes the command/conversation title. Line 3 shows the working directory on the left (with git branch icon) and badges on the right.

#### Code pane (file editor)

**Icon:** Neutral circle with language-specific file icon from the active file's extension. Falls back to `Code2` if no match.

**Single file open — Compact:**
- **Title:** Filename (e.g., `shared_sessions.rs`). Includes unsaved-changes dot if dirty.
- **Subtitle:** File path (e.g., `/peterrajani/warp-internal/src`).

**Single file open — Expanded:**
- **Title:** Filename. Includes unsaved-changes dot if dirty.
- **Description:** File path.
- **Metadata:** Omitted.

**Multiple tabs open — Compact:**
- **Title:** Active filename. Includes unsaved-changes dot if any tab is dirty.
- **Subtitle:** `and N more` (where N = total tab count − 1).

**Multiple tabs open — Expanded:**
- **Title:** Active filename. Includes unsaved-changes dot if any tab is dirty.
- **Description:** `and N more`.
- **Metadata:** Diff stats badge if available.

#### Notebook pane

**Icon:** Neutral circle with Notebook icon.

**Compact:**
- **Title:** Notebook name.
- **Subtitle:** `Last updated X ago by Author` (if last-updated metadata is available); otherwise the pane's secondary title.

**Expanded:**
- **Title:** Notebook name.
- **Description:** `Last updated X ago by Author` if available; otherwise the pane's secondary title.
- **Metadata:** Omitted.

#### Settings pane

**Icon:** Neutral circle with Gear icon.

**Compact:**
- **Title:** "Settings".
- **Subtitle:** Active settings page name (e.g., "MCP servers", "Appearance", "AI").

**Expanded:**
- **Title:** "Settings".
- **Description:** Active settings page name.
- **Metadata:** Omitted.

#### All other pane types

Applies to: Workflow, AI Document, AI Fact, MCP Server, Code Diff, Env Var Collection, Environment Management, Execution Profile Editor, File (non-code), and any future pane types.

**Icon:** Neutral circle with the pane type's icon (from the existing `TypedPane::icon()` mapping).

**Compact:**
- **Title:** Pane configuration title (falls back to type label, e.g., "Plan", "Workflow").
- **Subtitle:** Pane configuration secondary title (the existing `title_secondary()` value).

**Expanded:**
- **Title:** Pane configuration title.
- **Description:** Pane configuration secondary title.
- **Metadata:** Omitted.

### Indicators

#### Unread-activity dot

- **Visual:** Filled circle icon (`CircleFilled`, 16px) rendered inline with the title text, right-aligned within the title row. Uses a blue/accent color.
- **When shown:** A terminal pane has an agent conversation that produced new output since the user last focused that pane (e.g., agent completed a task, new streaming output arrived).
- **Cleared:** When the user activates/focuses the pane.
- **Scope:** Terminal panes with agent sessions only. Does not apply to plain terminals or non-terminal panes.

#### Unsaved-changes dot

- **Visual:** Same filled circle icon (`CircleFilled`, 16px), inline with the title text, right-aligned. Same visual treatment as the unread-activity dot.
- **When shown:** A Code pane has at least one tab with unsaved changes.
- **Cleared:** When all tabs in the Code pane are saved.
- **No "clean" text:** When there are no unsaved changes, nothing is shown — no "No unsaved changes" text, no indicator.

### Interactions

All row interactions remain unchanged from the current implementation:
- **Click:** Focus the pane.
- **Right-click:** Open the tab context menu.
- **Hover:** Row background highlight.
- **Selected state:** Focused pane in active tab has `fg_overlay_2` background + border.
- **Drag:** Only tab groups are draggable, not individual pane rows.

Diff stats and PR badges in the metadata row are interactive (clickable) — diff stats opens the code review panel, PR badge opens the PR URL in the browser. These behaviors are unchanged.

## Success criteria

1. Every pane row in both compact and expanded modes displays a circular icon to the left of the text column.
2. Plain terminal panes show a neutral circle with Terminal icon. Oz agent terminals show the Oz circle with status badge. CLI agent terminals show the branded agent circle with status badge.
3. Code panes show a language-specific file icon in the neutral circle, falling back to the generic code icon.
4. Terminal compact mode title and subtitle respect the "Pane title as" and "Additional metadata" settings. Default: command/conversation title + git branch subtitle.
5. Terminal panes in expanded mode show working directory as the description line and git branch + badges as the metadata line (default). The "Pane title as" setting with Command, Working Directory, and Branch options correctly controls which data occupies line 1 vs line 2.
6. Code panes with a single file show the filename as title and file path as subtitle/description. Code panes with multiple tabs show the active filename and `and N more`.
7. Notebook panes show "Last updated X ago by Author" as the subtitle/description when available.
8. Settings panes show the active settings page name as subtitle/description, NOT a directory path.
9. Pane types without meaningful metadata for a slot omit that slot — no empty lines or placeholder text.
10. The unread-activity dot appears on agent terminal panes with new unviewed output and clears on focus.
11. The unsaved-changes dot appears on Code panes with dirty tabs and clears on save. No "No unsaved changes" text is shown.
12. Agent status badges (clock, check, etc.) appear on the circular icon for all agent terminal panes and update in real time as the agent status changes.
13. All existing row interactions (click, right-click, hover, drag) and badge interactions (diff stats click, PR badge click) continue to work.

## Validation

- **Icon differentiation:** Open a mix of terminal, agent, code, notebook, and settings panes. Verify each has the correct circular icon variant (neutral vs branded, with or without status badge).
- **Language icons:** Open `.rs`, `.ts`, `.py`, and `.json` files. Verify each gets the appropriate language icon in the circle.
- **Agent status badge:** Start an Oz agent conversation. Verify the clock badge appears while running and changes to check on completion. Start a Claude Code session and verify the orange-branded circle appears.
- **Compact slot content:** Switch to compact mode. Verify terminal shows command + branch, code shows filename + path, settings shows "Settings" + page name, notebook shows name + "Last updated...".
- **Expanded slot content:** Switch to expanded mode. Verify terminal shows 3 lines (command, directory, branch+badges), code shows 2 lines (filename, path), settings shows 2 lines (Settings, page name).
- **"Pane title as" setting:** In expanded mode, switch between Command, Working Directory, and Branch. Verify terminal line 1 content changes to the selected data type. Verify font size/color treatment stays coupled to line position (line 1 is always main text, line 2 is always lighter sub-text). Verify Branch mode omits the redundant git branch from the metadata row.
- **Multi-tab code pane:** Open multiple files in a code editor pane. Verify title is the active filename and subtitle/description reads `and N more`.
- **Unread dot:** Start an agent, switch to another pane, let the agent complete. Verify the blue dot appears on the agent pane. Click the pane — verify the dot clears.
- **Unsaved dot:** Open a code file, make an edit without saving. Verify the blue dot appears. Save the file — verify the dot clears.
- **No "No unsaved changes" text:** Open a code file with no unsaved changes in expanded mode. Verify no third-line metadata text appears.
- **Settings no directory:** Open the Settings pane in expanded mode. Verify the description line shows the settings page name, not a directory path.

## Resolved decisions

1. **Unread-activity tracking:** Existing infrastructure from the notifications modal provides the unread/viewed state for agent conversations. No new tracking state is needed.
2. **Line styling vs content:** Font size and color treatment are coupled to line position, not semantic content. Line 1 always renders as 12px main text, line 2 always renders as 12px lighter sub-text, regardless of which data type (command, directory, branch) occupies each line.

## Open questions

None.
