# Handoff-Compose Footer Chip Filtering — Tech Spec
Product spec: `specs/REMOTE-1595/PRODUCT.md`
Linear: [REMOTE-1595](https://linear.app/warpdotdev/issue/REMOTE-1595)
## Context
`AgentInputFooter::render_toolbar_item` (`app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs:1945`) is called for each user-configured left/right toolbar item during footer rendering. It already filters by `available_in()` and `available_to_session_viewer()`. The footer already has access to `handoff_compose_state: ModelHandle<HandoffComposeState>` (added in the REMOTE-1558 PR).
The environment selector for `&` mode is rendered separately at `mod.rs:2108`, outside the toolbar item loop, so it is unaffected by toolbar filtering.
## Proposed changes
### 1. Add `AgentToolbarItemKind::is_available_during_handoff_compose`
Add a method on `AgentToolbarItemKind` in `toolbar_item.rs` that returns whether the item should render during `&` handoff-compose mode:
```rust path=null start=null
fn is_available_during_handoff_compose(&self) -> bool {
    matches!(
        self,
        Self::ModelSelector | Self::VoiceInput | Self::FileAttach
    )
}
```
### 2. Guard in `render_toolbar_item`
At the top of `render_toolbar_item` (`mod.rs:1945`), after the existing `available_in()` / `available_to_session_viewer()` guard, add:
```rust path=null start=null
if self.handoff_compose_state.as_ref(app).is_active()
    && !item.is_available_during_handoff_compose()
{
    return None;
}
```
This is purely a render-time filter; it does not mutate the user's configured layout.
## Testing
- Unit test: verify `is_available_during_handoff_compose` returns `true` only for `ModelSelector`, `VoiceInput`, `FileAttach`.
- Integration/manual: enter `&` mode, confirm only those three items plus the environment selector render. Exit `&`, confirm full footer restores.
