# HOA Onboarding Flow — Tech Spec

Linear: APP-3809
Product spec: `specs/harryalbert/APP-3809/PRODUCT.md`

## Problem

We need a 4-step guided onboarding flow for existing users that introduces HOA features (vertical tabs, agent inbox, tab configs). The flow must be shown exactly once, gated behind a feature flag, and must reuse existing rendering code from `SessionConfigModal` and the FTU callout pattern.

## Relevant Code

### Triggering & persistence
- `app/src/workspace/one_time_modal_model.rs` — `OneTimeModalModel`: existing pattern for one-time modals (Oz launch, build plan migration). Subscribes to auth events, waits for cloud preferences sync, then triggers.
- `app/src/root_view.rs:1614-1631` — `HAS_COMPLETED_ONBOARDING_KEY` / `private_user_preferences`: the other persistence approach (local-only, no cloud sync needed).
- `app/src/root_view.rs:2143-2155` — post-onboarding flow: opens vertical tabs panel + shows `SessionConfigModal` for new users.

### Session config modal (for extraction)
- `app/src/tab_configs/session_config_modal.rs` — `SessionConfigModal`: the full modal with session type pills, directory picker, worktree checkbox. All rendering is in private methods (`render_session_type_section`, `render_directory_section`, `render_checkboxes`).
- `app/src/tab_configs/session_config.rs` — `SessionConfigSelection`, `SessionType`, `is_git_repo`, `build_tab_config`, `write_tab_config`.

### FTU callout (for extraction)
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs:1758-1840` — `render_ftu_callout`: renders a bubble with a triangle arrow using stacked `CalloutTriangleBorderDown` / `CalloutTriangleFillDown` icons. Positions via `OffsetPositioning::offset_from_save_position_element`.

### Tab layout setting
- `app/src/workspace/tab_settings.rs:245` — `use_vertical_tabs: bool` setting.
- `app/src/workspace/view.rs:1607-1614` — `open_vertical_tabs_panel_if_enabled`: opens the vertical tabs panel based on the setting.

### Anchoring targets
- `app/src/workspace/view.rs:535` — `NOTIFICATIONS_MAILBOX_POSITION_ID = "workspace:notifications_mailbox"`: the inbox icon already uses `SavePosition` for anchoring (line 15262).
- `app/src/workspace/view.rs:515` — `TAB_BAR_POSITION_ID`: the tab bar already has a `SavePosition`.

### Feature flags
- `crates/warp_features/src/lib.rs:9` — `FeatureFlag` enum.
- `crates/warp_features/src/lib.rs:797` — `DOGFOOD_FLAGS` array.

### Workspace integration
- `app/src/workspace/view.rs:779-901` — `Workspace` struct: holds all modal view handles, panel state.
- `app/src/workspace/view.rs:1463-1494` — `build_session_config_modal`: how modals are constructed and subscribed.
- `app/src/workspace/view.rs:1617-1638` — `show_session_config_modal` / `close_session_config_modal`.

## Current State

- New users go through `AgentOnboardingView` → then post-onboarding opens vertical tabs + `SessionConfigModal` (`root_view.rs:2143-2155`).
- Existing users get no introduction to HOA features.
- The `SessionConfigModal` rendering methods are private and tightly coupled to the modal's `View` impl — they can't be reused without extraction.
- The FTU callout rendering is a standalone `fn render_ftu_callout` but hardcodes text, colors, and arrow direction. The general pattern (bubble + positioned triangle) is reusable.
- `OneTimeModalModel` handles one-time modal triggering for existing users (auth → cloud sync → check flag → show). It uses `AISettings` to persist "shown" state via cloud-synced settings.

## Proposed Changes

### 1. Feature flag

Add `HOAOnboardingFlow` to the `FeatureFlag` enum and `DOGFOOD_FLAGS`. Read the create feature flag skill for information on how to do this in a complete way.

File: `crates/warp_features/src/lib.rs`

### 2. Shared callout bubble component

Extract from the FTU callout into a general-purpose helper in a new file.

File: `app/src/view_components/callout_bubble.rs`

```rust
pub enum CalloutArrowDirection {
    Up,
    Left,
}

pub enum CalloutArrowPosition {
    Start(f32),
    Center,
    End(f32),
}

pub struct CalloutBubbleConfig {
    pub width: f32,
    pub arrow_direction: CalloutArrowDirection,
    pub arrow_position: CalloutArrowPosition,
}
```

A `render_callout_bubble(content, config, appearance) -> Box<dyn Element>` function that:
- Wraps `content` in a bordered, rounded container with accent-tinted background (matching FTU style).
- Appends/prepends a triangle arrow (using the existing `CalloutTriangleBorderDown` / `CalloutTriangleFillDown` icon stacking technique) positioned per the config.

This is reused by the HOA flow's Steps 2–4.

### 3. Shared session config rendering

Extract the rendering portions of `SessionConfigModal` into free functions that both the modal and the onboarding step can call.

File: `app/src/tab_configs/session_config_rendering.rs`

Extract these from `SessionConfigModal`:
- `render_session_type_pills(session_types, selected_index, pill_mouse_states, appearance) -> Box<dyn Element>`
- `render_directory_picker(selected_directory, mouse_state, appearance) -> Box<dyn Element>`
- `render_worktree_checkbox(enabled, is_git_repo, checkbox_mouse_state, tooltip_mouse_state, appearance) -> Box<dyn Element>`

`SessionConfigModal` calls these extracted functions in its `render` method instead of its current private methods. The onboarding Step 4 view calls the same functions.

The action dispatch, state management, and file picker logic remain in each caller (`SessionConfigModal` and the onboarding view) — only the rendering is shared.

### 4. HOA onboarding view

New module: `app/src/workspace/hoa_onboarding/`
- `mod.rs` — re-exports.
- `hoa_onboarding_flow.rs` — the main `HoaOnboardingFlow` view.
- `welcome_banner.rs` — Step 1 rendering.
- `tab_config_step.rs` — Step 4 rendering (uses shared session config rendering).

#### `HoaOnboardingFlow` view

```rust
enum HoaOnboardingStep {
    WelcomeBanner,
    VerticalTabsCallout,
    AgentInboxCallout,
    TabConfig,
}

struct HoaOnboardingFlow {
    step: HoaOnboardingStep,
    // Step 1 state
    close_button: ViewHandle<ActionButton>,
    // Step 2 state
    switch_to_horizontal_checkbox_mouse_state: MouseStateHandle,
    // Step 4 state (mirrors SessionConfigModal fields)
    session_types: Vec<SessionType>,
    selected_session_type_index: usize,
    selected_directory: PathBuf,
    is_git_repo: bool,
    enable_worktree: bool,
    session_pill_mouse_states: Vec<MouseStateHandle>,
    directory_button_mouse_state: MouseStateHandle,
    worktree_checkbox_mouse_state: MouseStateHandle,
    worktree_tooltip_mouse_state: MouseStateHandle,
}
```

**Events**:
```rust
enum HoaOnboardingFlowEvent {
    Completed(Option<SessionConfigSelection>),
    Dismissed,
}
```

- `Completed(Some(selection))` — user clicked Finish with a valid tab config.
- `Completed(None)` — user completed the flow but didn't reach the tab config (shouldn't happen in normal flow, but covers edge cases).
- `Dismissed` — user clicked X.

**Rendering** dispatches to different renderers per step:
- Steps 1: a centered modal with scrim (similar to how `SessionConfigModal` is wrapped in a `Modal`).
- Steps 2–3: the shared `render_callout_bubble` positioned via `OffsetPositioning::offset_from_save_position_element`, anchored to the vertical tabs panel or `NOTIFICATIONS_MAILBOX_POSITION_ID`.
- Step 4: the shared `render_callout_bubble` with the tab config form body, anchored to the vertical tabs panel in vertical tabs mode or the tab bar in horizontal tabs mode.

### 5. Persistence

Add a new `private_user_preferences` key: `HasCompletedHOAOnboarding`.

Use the same pattern as `HAS_COMPLETED_ONBOARDING_KEY` in `root_view.rs:1614-1631` — a simple boolean read/write via `ctx.private_user_preferences()`. This should be cloud synced.

Helper functions (placed near the onboarding flow or in a shared utils module):
```rust
const HAS_COMPLETED_HOA_ONBOARDING_KEY: &str = "HasCompletedHOAOnboarding";

fn has_completed_hoa_onboarding(ctx: &AppContext) -> bool { ... }
fn mark_hoa_onboarding_completed(ctx: &AppContext) { ... }
```

### 6. Triggering

Integrate into `OneTimeModalModel`:
- Add `is_hoa_onboarding_open: bool` field.
- Add a `check_and_trigger_hoa_onboarding` method that:
  1. Checks `FeatureFlag::HOAOnboardingFlow.is_enabled()`.
  2. Checks `!has_completed_hoa_onboarding(ctx)`.
  3. Checks `has_completed_local_onboarding(ctx)` — ensures this is an existing user (not someone mid-new-user-onboarding).
  4. Sets `is_hoa_onboarding_open = true` and emits the event.
- Call this from `check_and_trigger_all_modals` alongside the existing Oz launch and build plan migration checks.

In `Workspace`, subscribe to `OneTimeModalEvent` and when the HOA onboarding should show:
- Ensure vertical tabs are enabled (set the `use_vertical_tabs` setting to `true` if not already).
- Open the vertical tabs panel.
- Show the `HoaOnboardingFlow` view as an overlay.

### 7. New-user onboarding integration

In `root_view.rs`, after new-user onboarding completes (`AgentOnboardingEvent::OnboardingCompleted`), call `mark_hoa_onboarding_completed(ctx)` to ensure the HOA flow never shows for new users (only if the hoa flow feature flag is enabled).

### 8. Workspace rendering integration

Add `hoa_onboarding_flow: Option<ViewHandle<HoaOnboardingFlow>>` to `Workspace`.

In the workspace's `render` method:
- When `hoa_onboarding_flow` is `Some` and the step is `WelcomeBanner`, render a full-window scrim overlay with the banner centered.
- When the step is `VerticalTabsCallout` or `AgentInboxCallout`, render the callout bubble as a positioned overlay (using `Stack::add_positioned_overlay_child`) anchored to the appropriate `SavePosition` element.
- When the step is `TabConfig`, render the tab config popover as a positioned overlay anchored to the content area.

Handle `HoaOnboardingFlowEvent`:
- On `Dismissed`: call `mark_hoa_onboarding_completed`, drop the flow view.
- On `Completed(Some(selection))`: call `mark_hoa_onboarding_completed`, process the tab config selection (reuse `handle_session_config_completed` logic), drop the flow view.
- On `Completed(None)`: call `mark_hoa_onboarding_completed`, drop the flow view.

## End-to-End Flow

1. Existing user launches Warp with `HOAOnboardingFlow` enabled.
2. `OneTimeModalModel` receives auth + cloud sync completion → calls `check_and_trigger_hoa_onboarding`.
3. Checks pass (flag enabled, not completed, is existing user) → emits `OneTimeModalEvent`.
4. `Workspace` receives event → enables vertical tabs setting → opens vertical tabs panel → creates `HoaOnboardingFlow` in `WelcomeBanner` step → renders scrim + banner.
5. User clicks "See what's new" → flow advances to `VerticalTabsCallout` → callout bubble rendered anchored to vertical tabs panel.
6. User optionally toggles "Switch back to horizontal tabs" → `use_vertical_tabs` setting toggled, UI updates live, callout re-anchors.
7. User clicks "Next" → flow advances to `AgentInboxCallout` → callout bubble rendered anchored to `NOTIFICATIONS_MAILBOX_POSITION_ID`.
8. User clicks "Next" → flow advances to `TabConfig` → popover with session config fields rendered.
9. User configures and clicks "Finish" → `HoaOnboardingFlowEvent::Completed(Some(selection))` emitted → tab config saved, flow closed, preferences key set.

## Risks and Mitigations

1. **Rendering extraction breaking `SessionConfigModal`**: The extraction of rendering functions is a refactor of existing code. Risk: subtle visual regressions. Mitigation: compare screenshots before/after extraction to verify pixel-identical output.

2. **Callout positioning edge cases**: If the vertical tabs panel or inbox icon aren't rendered yet when the callout tries to anchor, `offset_from_save_position_element` may position incorrectly. Mitigation: the flow ensures vertical tabs are opened before Step 2, and the inbox icon is always in the title bar when `HOANotifications` is enabled.

3. **Interaction with other one-time modals**: `OneTimeModalModel` runs checks sequentially — Oz launch modal takes priority. If the Oz launch modal shows, the HOA flow won't trigger until the next app launch. This is acceptable; the flow persists across launches until shown.

4. **Feature flag dependency**: The flow assumes `VerticalTabs`, `HOANotifications`, and `TabConfigs` flags are also enabled. If any are disabled, certain steps may reference UI that doesn't exist. Mitigation: gate the HOA flow on all required flags in `check_and_trigger_hoa_onboarding`.

## Testing and Validation

- **Unit tests**: persistence helpers (`has_completed_hoa_onboarding`, `mark_hoa_onboarding_completed`), step transitions in `HoaOnboardingFlow`, new-user exclusion logic.
- **Visual verification**: local walkthrough of all 4 steps after building.
- **Extraction regression**: verify `SessionConfigModal` still renders identically after extracting shared rendering functions.
- **Edge cases**: dismiss at Step 1, dismiss at Step 4, toggle horizontal tabs back and forth, select non-git directory for worktree.

## Follow-ups

- Remove `HOAOnboardingFlow` feature flag once stable (promote through preview → release).
- Refactor the existing FTU callout in `agent_input_footer` to use the shared `callout_bubble` component.
- Hero art asset: needs a static image to be produced and bundled.
