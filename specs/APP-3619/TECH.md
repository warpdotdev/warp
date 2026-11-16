# Tech Spec: Plugin Installation Fallback Modal

See `specs/APP-3619/PRODUCT.md` for the full product spec.

## 1. Install Instructions Data Model

`plugin_manager/mod.rs`

Add `PluginInstallStep` and `PluginInstallInstructions` structs. Replace the existing `post_install_hint()` trait method with `install_instructions() -> &'static PluginInstallInstructions` that returns all modal content for this agent.

```rust
pub(crate) struct PluginInstallStep {
    pub description: &'static str,
    pub command: &'static str,
}

pub(crate) struct PluginInstallInstructions {
    pub title: &'static str,
    pub subtitle: &'static str,
    pub steps: &'static [PluginInstallStep],
    pub success_toast: &'static str,
}
```

Each implementation uses `LazyLock` so the data is built once. Claude Code's steps are the in-session slash commands (`/plugins marketplace add ...`, `/plugins install ...`, `/reload-plugins`) since the user is already in a running session.

The existing `post_install_hint` is removed. The auto-install success toast reads from `install_instructions().success_toast`.

## 2. Modal View

New file: `workspace/view/plugin_install_modal.rs`, following the `CodexModal` pattern (standalone view, centered overlay, backdrop, Escape to close).

### View struct

```rust
pub struct PluginInstallModal {
    agent: Option<CLIAgent>,
    close_button_mouse_state: MouseStateHandle,
    step_code_handles: Vec<CodeSnippetButtonHandles>,
}
```

`agent` is `Option` so the view can be constructed once in workspace init and updated via `set_agent()` before each open. All `MouseStateHandle`s are stored in the struct (not created inline during render). Copying a command shows a "Copied to clipboard" ephemeral toast via `ToastStack`. The modal uses `Dismiss` to close on outside click.

### Code block rendering

Reuse `render_code_block_plain` (`ai/blocklist/code_block.rs:117`) with `CodeBlockOptions` — the same component already used by `cloud_setup_guide_view.rs:393` for its step-by-step CLI guide. Each step gets `on_copy` + a `CodeSnippetButtonHandles`.

### Per-agent extensibility

`render()` dispatches to per-agent render functions that compose shared helpers (modal shell, numbered step layout, code blocks). Adding a new agent means adding a render function + `PluginInstallInstructions` implementation.

### Actions & Events

```rust
enum PluginInstallModalAction { Close, CopyCommand(usize) }
enum PluginInstallModalEvent { Close }
```

## 3. Workspace Wiring

Follows the standard modal pattern (like `CodexModal` at `workspace/view.rs:817,1573,12878,18110`):

- `is_plugin_install_modal_open` on `WorkspaceState` (add to `is_any_non_palette_modal_open` and `close_all_modals` in `workspace/util.rs`)
- `plugin_install_modal: ViewHandle<PluginInstallModal>` on `Workspace`
- `open_plugin_install_modal(agent)` sets the agent, flips the bool, focuses the modal
- Rendered conditionally in the workspace render method's modal stack

## 4. Event Plumbing: Footer → Workspace

New `ShowPluginInstallModal(CLIAgent)` variant propagated through the standard event chain:

`AgentInputFooterEvent` (`mod.rs:1325`) → `Input::Event` (`input.rs:1039`) → `TerminalView::Event` → `pane_group::Event` → Workspace handler

Follows the existing `OpenAutoReloadModal` pattern (`view.rs:18411`, `workspace/view.rs:10182`).

## 5. Remote Session Detection

`CLIAgentSession` has an `is_remote: bool` field set at session creation from `TerminalView::active_session_is_local()`. This uses `SessionType::WarpifiedRemote` and `IsLegacySSHSession` — the same logic as the SSH host chip (`context_chips/builtins.rs:76`).

This avoids relying on `terminal_model.is_ssh_block()` (which only tracks the pre-warpification login phase) or `is_warpified_ssh()` (which misses legacy SSH). The `is_remote` flag is threaded through `set_session` and `register_listener` at all call sites in `terminal/view.rs`.

## 6. Two-Mode Chip

`agent_input_footer/mod.rs` — the install chip has two modes based on whether auto-install is viable.

### Mode selection

`should_use_manual_install_mode(&self, app)` returns `true` when:

- `plugin_install_failed` is set (auto-install already failed this session), OR
- `session.is_remote` is true (checked via `CLIAgentSessionsModel`)

### Remote visibility fix

`should_show_install_plugin_button` checks `session.is_remote` — when true, skips the local `is_installed()` filesystem check and relies solely on listener presence.

### Two buttons

`ActionButton` doesn't support changing label/icon after construction, so we create two `ActionButton` views and conditionally render the right one:

- **Auto-install** (existing `install_plugin_button`): "Install Warp plugin" / `Icon::Download` → triggers `handle_install_plugin`
- **Manual** (new `plugin_instructions_button`): "Plugin install instructions" / `Icon::Info` → emits `ShowPluginInstallModal`

In `render_cli_mode_footer`, branch on `should_use_manual_install_mode()` to pick which button to render.

On install failure (`handle_install_plugin` error callback), set `plugin_install_failed = true` to switch to manual mode for the rest of the session.

## 7. Files Changed

- **New:** `workspace/view/plugin_install_modal.rs`
- **Modified:** `plugin_manager/mod.rs` — `PluginInstallStep`, `PluginInstallInstructions`, new trait method
- **Modified:** `plugin_manager/claude.rs` — `LazyLock` implementation
- **Modified:** `cli_agent_sessions/mod.rs` — `is_remote` field on `CLIAgentSession`, threaded through `register_listener`
- **Modified:** `agent_input_footer/mod.rs` — two-mode chip, failure tracking, remote detection via `session.is_remote`, new action/event variants
- **Modified:** `terminal/input.rs`, `terminal/view.rs` — event forwarding, `is_remote` computation at session creation
- **Modified:** `workspace/util.rs`, `workspace/view.rs`, `workspace/mod.rs` — modal integration
