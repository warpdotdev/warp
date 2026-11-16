# APP-3781: Tech Spec — Plugin Instructions in a Split Pane

Linear: [APP-3781](https://linear.app/warpdotdev/issue/APP-3781/move-plugin-installation-settings-into-a-dedicated-pane)

## Problem

The plugin install/update instructions are rendered as a modal (`PluginInstallModal`) that overlays the entire workspace. This change replaces the modal with a split terminal pane containing a rich-content instructions block with a manual close button.

## Relevant Code

**Current modal (to be deleted):**
- `app/src/workspace/view/plugin_install_modal.rs` — the modal view, step rendering, copy-to-clipboard, `render_step_number`
- `app/src/workspace/view.rs:851` — `plugin_install_modal` field on `Workspace`
- `app/src/workspace/view.rs:14027-14064` — `handle_plugin_install_modal_event` and `open_plugin_install_modal`
- `app/src/workspace/view.rs:19587-19590` — modal render in workspace overlay stack
- `app/src/workspace/util.rs:119,156,197` — `is_plugin_install_modal_open` in `WorkspaceState`

**Event chain (to be renamed):**
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs:1788,1959` — `ShowPluginInstallModal` / `ShowPluginInstructionsModal` actions and events
- `app/src/terminal/input.rs:1047` — `InputEvent::ShowPluginInstructionsModal`
- `app/src/terminal/view.rs:1892` — `terminal::view::Event::ShowPluginInstructionsModal`
- `app/src/pane_group/pane/terminal_pane.rs:664-668` — forwards to `pane_group::Event`
- `app/src/pane_group/mod.rs:695` — `pane_group::Event::ShowPluginInstructionsModal`
- `app/src/workspace/view.rs:11231` — workspace handler

**Plugin instructions data:**
- `app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs` — `PluginInstructions`, `PluginInstructionStep`, `PluginModalKind`
- `app/src/terminal/cli_agent_sessions/plugin_manager/claude.rs:126-170` — static `INSTALL_INSTRUCTIONS` and `UPDATE_INSTRUCTIONS`

**Patterns to follow:**
- `app/src/terminal/view/zero_state_block.rs` — `TerminalViewZeroStateBlock` (container styling, positioned close/dismiss button via `Stack::with_positioned_child`)
- `app/src/terminal/view/rich_content.rs` — `RichContent`, `RichContentMetadata`, `TerminalView::insert_rich_content`
- `app/src/terminal/model/rich_content.rs` — `RichContentType` enum
- `app/src/ai/blocklist/code_block/mod.rs` — `render_code_block_plain`, `CodeBlockOptions`, `CodeSnippetButtonHandles` (for code block rendering)
- `app/src/pane_group/mod.rs:3750-3766` — `PaneGroup::add_terminal_pane` returns `TerminalPaneId`
- `app/src/pane_group/mod.rs:6078-6085` — `PaneGroup::terminal_view_from_pane_id` returns `ViewHandle<TerminalView>`
- `app/src/pane_group/mod.rs:1057-1072` — `PaneGroup::smart_split_direction`
- `app/src/workspace/view.rs:569` — `WORKFLOW_AND_ENV_VAR_SPLIT_RATIO`

## Current State

Event chain: info (ⓘ) button click → `AgentInputFooterAction::ShowPluginInstallModal` → `AgentInputFooterEvent::ShowPluginInstructionsModal(agent, kind)` → `InputEvent` → `terminal::view::Event` → `TerminalPane` → `pane_group::Event` → `Workspace::open_plugin_install_modal`.

The workspace resolves `PluginInstructions` from `plugin_manager_for(agent)`, sets them on the modal view, sets `is_plugin_install_modal_open = true`, and focuses the modal. The modal renders as a centered overlay with dimmed background.

## Proposed Changes

### 1. New file: `app/src/terminal/view/plugin_instructions_block.rs`

A `PluginInstructionsBlock` view that renders plugin instructions as terminal rich content with a close button.

**Struct:**
```rust
struct PluginInstructionsBlock {
    instructions: &'static PluginInstructions,
    close_button_mouse_state: MouseStateHandle,
    step_code_handles: Vec<CodeSnippetButtonHandles>,
    should_hide: bool,
}
```

**Rendering:** Uses a `Stack` with:
- Main child: a bordered `Container` (matching `TerminalViewZeroStateBlock` style — horizontal terminal padding, vertical padding, top/bottom border) containing a `Flex::column` with title, subtitle, and numbered step rows
- Positioned child: a close button (using `appearance.ui_builder().close_button()`) in the top-right corner via `OffsetPositioning::offset_from_parent`

Each step row reuses the `render_step_number` badge (moved from the modal into this file as a private `fn`, since no other caller exists) and `render_code_block_plain` for the command code block with copy button.

**Actions and events:**
- `PluginInstructionsBlockAction::Close` — sets `should_hide = true`, emits a close event to the owning `TerminalView` so the rich-content item is removed from the blocklist sumtree, and calls `ctx.notify()`
- `PluginInstructionsBlockAction::CopyCommand(usize)` — copies to clipboard via `ctx.clipboard().write()`, shows toast via `ToastStack::handle(ctx)` singleton (same pattern as the modal, `plugin_install_modal.rs:248-257`)
- `Entity::Event = PluginInstructionsBlockEvent` — `Close` bubbles to `TerminalView` for rich-content cleanup; toast and clipboard are still handled directly in the block

When `should_hide` is true, `render` returns `Empty::new().finish()`.

### 2. New `RichContentType` and `RichContentMetadata` variants

In `app/src/terminal/model/rich_content.rs`, add `PluginInstructionsBlock` to the `RichContentType` enum.

In `app/src/terminal/view/rich_content.rs`, add `PluginInstructionsBlock` to the `RichContentMetadata` enum.

### 3. Rename event variants

Rename across the entire chain to reflect pane-based behavior:

- `AgentInputFooterAction::ShowPluginInstallModal` → `OpenPluginInstallInstructionsPane`
- `AgentInputFooterAction::ShowPluginInstructionsModal` → `OpenPluginUpdateInstructionsPane`
- `AgentInputFooterEvent::ShowPluginInstructionsModal` → `OpenPluginInstructionsPane` (carries `PluginModalKind`)
- `InputEvent::ShowPluginInstructionsModal` → `OpenPluginInstructionsPane` (carries `PluginModalKind`)
- `terminal::view::Event::ShowPluginInstructionsModal` → `OpenPluginInstructionsPane` (carries `PluginModalKind`)
- `pane_group::Event::ShowPluginInstructionsModal` → `OpenPluginInstructionsPane` (carries `PluginModalKind`)

### 4. Workspace: replace modal with split pane creation

Replace `Workspace::open_plugin_install_modal` with `open_plugin_instructions_pane`. The method:

1. Resolves `PluginInstructions` from `plugin_manager_for(agent)` (same as before)
2. Creates a new terminal pane via `PaneGroup::add_terminal_pane_ignoring_default_session_mode(direction, None, ctx)` so the pane stays in terminal mode even if the user's default mode for new sessions is Agent Mode. Split panes do not show the homepage zero-state, so no `hide_homepage` option is needed.
3. Gets the `ViewHandle<TerminalView>` via `PaneGroup::terminal_view_from_pane_id(pane_id, ctx)`
4. Inside `terminal_view.update()`, creates a `PluginInstructionsBlock` view and calls `view.insert_rich_content(...)` to add it

This keeps `TerminalView` fully decoupled from plugin concepts — the workspace owns the orchestration, and the block is just another rich content view.

### 5. Delete modal code

Remove:
- `app/src/workspace/view/plugin_install_modal.rs` (entire file)
- `mod plugin_install_modal` declaration (`view.rs:13`)
- `use crate::workspace::view::plugin_install_modal::{PluginInstallModal, PluginInstallModalEvent}` import (`view.rs:129`)
- `plugin_install_modal` field from `Workspace` struct (`view.rs:851`)
- `plugin_install_modal` field initialization in `Workspace::new` (`view.rs:2393-2394`)
- `is_plugin_install_modal_open` from `WorkspaceState` and all references in `is_any_non_palette_modal_open`, `close_all_modals` (`util.rs:119,156,197`)
- `handle_plugin_install_modal_event` (`view.rs:14027-14040`)
- Modal construction and subscription in `Workspace::new` (`view.rs:1896-1901`)
- Modal render in overlay stack (`view.rs:19587-19590`)
- `view::plugin_install_modal::init(app)` call (`workspace/mod.rs:87`)

## End-to-End Flow

1. User clicks info (ⓘ) button on install/update chip
2. `AgentInputFooterAction::OpenPluginInstallInstructionsPane` (or update variant) dispatched
3. Event bubbles: `AgentInputFooter` → `Input` → `TerminalView` → `TerminalPane` → `PaneGroup` → `Workspace`
4. `Workspace::open_plugin_instructions_pane(agent, kind, ctx)` called
5. Workspace resolves `PluginInstructions` from `CliAgentPluginManager`
6. Inside `active_tab_pane_group().update()`:
   - Creates terminal pane with `add_terminal_pane_ignoring_default_session_mode(Direction::Right, None, ctx)` → `TerminalPaneId`
   - Gets `ViewHandle<TerminalView>` via `terminal_view_from_pane_id`
   - Inside `terminal_view.update()`: creates `PluginInstructionsBlock`, calls `insert_rich_content`
7. New pane renders with instructions block at top; user types/runs commands below it
8. User clicks close (X) button → block hides, the corresponding rich-content item is removed from the blocklist sumtree, and the terminal pane remains functional

## Risks and Mitigations

**Block insertion timing.** `insert_rich_content` appends to the block list model and works before session bootstrap. The block will be visible while the session bootstraps (sub-second). No special handling needed.

**Toast access.** The block accesses `ToastStack::handle(ctx)` directly (it's a singleton), same pattern as the modal. No event bubbling required for toasts.

**`PluginInstructions` visibility.** `PluginInstructions` and `PluginInstructionStep` are currently `pub(crate)` in `plugin_manager/mod.rs`. The new block file is within the same crate, so no visibility changes needed.

## Testing and Validation

- `cargo check` — no remaining references to deleted modal types
- `cargo fmt` and `cargo clippy` per presubmit
- Manual test: click install info button → split pane with instructions. Copy command → clipboard + toast. Run commands → instructions block persists. Click close (X) → block hides. Close pane → clean.
- Both flows: verify install and update info buttons show correct instructions.

## Follow-ups

- The two `AgentInputFooterAction` variants (`OpenPluginInstallInstructionsPane` for install, `OpenPluginUpdateInstructionsPane` for update) could be collapsed into a single variant carrying `PluginModalKind`. Left as-is for minimal diff, can unify later.
