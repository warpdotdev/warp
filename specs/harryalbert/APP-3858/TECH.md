# APP-3858: Tech Spec — Remote Control Entrypoint

## Problem
The CLI agent footer's "Share session" button opens the share modal, which is an unnecessary step for the primary use case (quick mobile handoff). We need to make it a one-click action that starts sharing without scrollback, auto-copies the link, and shows a stop button while active.

## Relevant Code
- `crates/warp_core/src/ui/icons.rs (10-296)` — `Icon` enum; needs `Phone01` variant
- `app/assets/bundled/svg/phone-01.svg` — phone icon for start-state (on branch)
- `Icon::StopFilled` — existing icon reused for stop-state
- `app/src/search/slash_command_menu/static_commands/commands.rs` — slash command definitions
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs (491-500)` — `share_session_button` creation
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs (1104-1108)` — CLI toolbar `ShareSession` render
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs (1636-1644)` — agent view toolbar `ShareSession` render
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs (1893, 2025-2027)` — `AgentInputFooterAction::ShareSession` handler
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs (2037-2045)` — `AgentInputFooterEvent` enum
- `app/src/ai/blocklist/agent_view/agent_input_footer/toolbar_item.rs (48, 74, 88)` — `AgentToolbarItemKind::ShareSession` display label and icon
- `app/src/terminal/view/use_agent_footer/mod.rs (217-218, 851-852)` — event forwarding
- `app/src/terminal/view/use_agent_footer/mod.rs (1019-1041)` — `UseAgentToolbarEvent` enum
- `app/src/terminal/view/shared_session/view_impl.rs (415-472)` — `attempt_to_share_session` (scrollback guard at 436-448)
- `app/src/terminal/view/shared_session/view_impl.rs (476-520)` — `on_session_share_started` (auto-opens sharing dialog at 515)
- `app/src/terminal/view/shared_session/view_impl.rs (522-529)` — `stop_sharing_session`
- `app/src/terminal/view/shared_session/view_impl.rs (1288-1311)` — `copy_shared_session_link`
- `app/src/terminal/shared_session/mod.rs (39)` — `COPY_LINK_TEXT` constant
- `app/src/terminal/shared_session/mod.rs (267-290)` — `SharedSessionActionSource` enum

## Current State

**Button**: A single static `share_session_button` with `Icon::Share`, tooltip "Share session". Click dispatches `AgentInputFooterAction::ShareSession` → emits `AgentInputFooterEvent::OpenShareSessionModal`.

**Event flow**: `OpenShareSessionModal` bubbles through `UseAgentToolbarEvent::OpenShareSessionModal` → `TerminalView::open_share_session_modal` → opens the sharing dialog.

**Post-share**: `on_session_share_started` unconditionally auto-opens the sharing dialog so the user can copy the link.

**No stop affordance**: The button never changes. Stopping requires the pane header overflow menu.

## Proposed Changes

### 1. Add icon variant
In `crates/warp_core/src/ui/icons.rs`, add one variant:
- `Phone01` → `"bundled/svg/phone-01.svg"`

The stop state reuses the existing `Icon::StopFilled`.

### 2. Two buttons instead of one
Replace the single `share_session_button` field in `AgentInputFooter` with two fields:

**`start_remote_control_button`**: `Phone01` icon, label "/remote-control". On click dispatches `AgentInputFooterAction::StartRemoteControl`.

**`stop_remote_control_button`**: `StopFilled` icon, label "Stop sharing". On click dispatches `AgentInputFooterAction::StopRemoteControl`.

### 3. Split action and event enums
**`AgentInputFooterAction`** (mod.rs:1893): Replace `ShareSession` with:
- `StartRemoteControl`
- `StopRemoteControl`

**`AgentInputFooterEvent`** (mod.rs:2037): Replace `OpenShareSessionModal` with:
- `StartRemoteControl`
- `StopRemoteControl`

**`UseAgentToolbarEvent`** (use_agent_footer/mod.rs:1019): Replace `OpenShareSessionModal` with:
- `StartRemoteControl`
- `StopRemoteControl`

Each action handler is trivial — it just emits the corresponding event up the chain.

### 4. Swap buttons at render time
In `render_cli_toolbar_item` and `render_toolbar_item` for the `ShareSession` variant, instead of always rendering the start button, check `terminal_model.lock().shared_session_status()`:
- Not sharing → render `start_remote_control_button`
- `is_active_sharer()` or `is_share_pending()` → render `stop_remote_control_button`

The footer already re-renders on `CLIAgentSessionsModel` changes. We need to verify shared session status changes also trigger re-renders; if not, subscribe to the relevant model event.

### 5. Handle `StartRemoteControl` in `TerminalView`
In `handle_use_agent_footer_event` (use_agent_footer/mod.rs:217):
- Call `attempt_to_share_session(SharedSessionScrollbackType::None, Some(FooterChip), SessionSourceType::Terminal, ctx)` with a new `bypass_conversation_guard: bool` parameter set to `true`.

**Bypass the scrollback guard**: Add `bypass_conversation_guard: bool` to `attempt_to_share_session` (view_impl.rs:415). When `true`, skip the guard at lines 436-448. All existing callers pass `false`.

### 6. Auto-copy link on share start
Thread `source: Option<SharedSessionActionSource>` through the share-started flow:
- `Event::StartSharingCurrentSession` gains a `source` field
- Terminal manager forwards it to `on_session_share_started`

In `on_session_share_started` (view_impl.rs:476):
- When source is `FooterChip`: skip `toggle_sharing_dialog`, call `copy_shared_session_link`, show toast "Remote control link copied".
- All other sources: keep existing behavior (auto-open sharing dialog).

### 7. Handle `StopRemoteControl` in `TerminalView`
Call `self.stop_sharing_session(SharedSessionActionSource::FooterChip, ctx)`.

### 8. Update toolbar item metadata
In `toolbar_item.rs`:
- Display label: `"Share Session"` → `"/remote-control"`
- Icon: `Icon::Share` → `Icon::Phone01`

### 9. Add `/remote-control` slash command
Add a `REMOTE_CONTROL` static command in `commands.rs` with:
- name: `/remote-control`
- description: "Start remote control for this session"
- icon_path: `"bundled/svg/phone-01.svg"`
- availability: CLI agent sessions only

Register it in the command list. Handle it in the slash command dispatch to emit the same `StartRemoteControl` action as the chip.

### 10. Banner copy
When sharing is started from the remote control entrypoint (`SharedSessionActionSource::FooterChip`), the inline banners show:
- "Remote control active" (instead of "Sharing started")
- "Remote control stopped" (instead of "Sharing ended")

This is threaded via an `is_remote_control: bool` field on `SharedSessionBanners::ActiveShare` and `LastShared`.

## End-to-End Flow
```
Start:
  click start_remote_control_button
  → AgentInputFooterAction::StartRemoteControl
  → AgentInputFooterEvent::StartRemoteControl
  → UseAgentToolbarEvent::StartRemoteControl
  → attempt_to_share_session(None, FooterChip, bypass=true)
  → terminal manager establishes connection
  → on_session_share_started(source=FooterChip)
    → skips sharing dialog, copies link, shows toast
  → footer re-renders: swaps to stop_remote_control_button

Stop:
  click stop_remote_control_button
  → AgentInputFooterAction::StopRemoteControl
  → AgentInputFooterEvent::StopRemoteControl
  → UseAgentToolbarEvent::StopRemoteControl
  → stop_sharing_session(FooterChip)
  → footer re-renders: swaps back to start_remote_control_button
```

## Risks and Mitigations
- **Scrollback guard bypass**: Deliberately allowing no-scrollback shares for remote control. Guard still applies for all other entry points (share modal, context menu, etc.).
- **State sync**: Reading `shared_session_status()` at render time means the button reflects state changes from any entry point. Must verify shared session status changes trigger footer re-renders.
- **Threading source through share-started flow**: `Event::StartSharingCurrentSession` and the terminal manager callback need to carry the source. Keep it optional so existing callers are unaffected.

## Testing and Validation
- Existing shared session tests pass (protocol unchanged).
- Manual: start CLI agent → click "Remote control" → verify link copied + toast → button shows stop state → click stop → session ends + button reverts.
- Verify starting share from pane menu causes footer to show stop button.

## Follow-ups
- "Text link to my device" stretch goal.
- Rename agent view (non-CLI) share button if desired.
