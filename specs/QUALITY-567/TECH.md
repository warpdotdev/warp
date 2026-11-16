# TECH — Orchestration Pill Bar

See `PRODUCT.md` in this directory for user-visible behavior. This document covers implementation and validation only.

## Context

The pill bar lives inside the existing pane header chrome rendered for the fullscreen agent view. The header is built in `app/src/terminal/view/pane_impl.rs` (`render_terminal_pane_header`), which composes a 3-column row via `crate::pane_group::pane::view::header::components::render_three_column_header` and then optionally wraps it via `maybe_add_parent_navigation_card`. The wrapped element is returned to `PaneHeader::render` (`app/src/pane_group/pane/view/header/mod.rs`), which constrains the result to `PANE_HEADER_HEIGHT = 34.` for the standard `HeaderContent::Custom` path.

Relevant existing code:

- `app/src/terminal/view/pane_impl.rs (501-556)` — `maybe_add_parent_navigation_card`, the splice point where the pill bar gets injected below the standard header. Already wraps the header in a `Flex::column` for the pre-existing parent-conversation card.
- `app/src/terminal/view/pane_impl.rs (269-391)` — `render_header_title`, where the pane title is built. Breadcrumbs short-circuit this when the active conversation has a parent.
- `app/src/pane_group/pane/view/header/components.rs (152-213)` — `render_three_column_header`. The center column wraps the title in `Align::new(center_row).finish()` so the title vertically centers within the row's stretched height.
- `app/src/pane_group/pane/view/header/mod.rs:52` — `PANE_HEADER_HEIGHT = 34.` and the `ConstrainedBox::with_height(PANE_HEADER_HEIGHT)` wrap on `HeaderContent::Custom`.
- `app/src/ai/blocklist/agent_view/orchestration_conversation_links.rs` — already exposes `parent_conversation_id` and the existing `parent_conversation_navigation_card` used by the legacy orchestration UI.
- `app/src/ai/blocklist/history_model.rs` — `BlocklistAIHistoryModel` exposes `child_conversations_of`, `conversation`, and the events the pill bar subscribes to.
- `app/src/terminal/view.rs:25419` — existing handler stub for `TerminalAction::SwitchAgentViewToConversation`, calling `enter_agent_view_for_conversation` to navigate the same pane.
- `crates/warp_features/src/lib.rs` — `FeatureFlag` enum and `DOGFOOD_FLAGS`.

The feature is gated by a new `FeatureFlag::OrchestrationPillBar`. Existing `Orchestration` and `AgentView` flag behavior is preserved when the new flag is off.

## Proposed changes

### 1. Feature flag

Add `OrchestrationPillBar` to `FeatureFlag` in `crates/warp_features/src/lib.rs:725`. All new code paths gate on `FeatureFlag::OrchestrationPillBar.is_enabled()`.

### 2. New view: `OrchestrationPillBar`

New file `app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs` exposes:

- `pub struct OrchestrationPillBar` — implements `View` with `Entity::Event = ()`.
  - Holds `agent_view_controller: ModelHandle<AgentViewController>` and `mouse_states: HashMap<AIConversationId, MouseStateHandle>` for persistent per-pill hover state (per WARP.md's `MouseStateHandle` rule — inline `MouseStateHandle::default()` would silently break clicks).
  - Subscribes to `BlocklistAIHistoryModel` for `UpdatedConversationStatus`, `AppendedExchange`, `SetActiveConversation`, `StartedNewConversation`, and to removal events to drop stale mouse states.
  - Subscribes to `AgentViewController` for `EnteredAgentView` / `ExitedAgentView` to clear hover state across view transitions.
- A private `pill_specs(&self, app)` helper that:
  - Resolves the active conversation, walks up to its orchestrator via `parent_conversation_id`.
  - Returns `None` when the active conversation has a parent (child views render breadcrumbs instead) or when the orchestrator has no children.
  - Builds an ordered list: orchestrator first, then children sorted by `first_exchange().start_time`.
- `render_pill(spec, mouse_state, app)` — builds a `Hoverable` whose closure rebuilds the pill on each render (selected vs hovered vs idle styling). Click dispatches `PaneHeaderAction::<TerminalAction, TerminalAction>::CustomAction(TerminalAction::SwitchAgentViewToConversation { conversation_id })`. The action wrapper is required because the pill bar lives inside the pane header chrome — `BackingView::handle_custom_action` unwraps it (mirrors `agent_view_back_button`).
- `render_avatar_disc` — renders the colored circle as a `Stack` of (1) a `ConstrainedBox(Container(bg + corner_radius))` and (2) a centered glyph (letter `Text` or `Icon`). The glyph is centered using nested `Flex::column` / `Flex::row` with both `MainAxisAlignment::Center` and `CrossAxisAlignment::Center` on each axis.

Module wiring: `pub mod orchestration_pill_bar;` in `app/src/ai/blocklist/agent_view/mod.rs` and a `pub use orchestration_pill_bar::{render_orchestration_breadcrumbs, OrchestrationPillBar};`.

### 3. Breadcrumb rendering

Same file. `pub fn render_orchestration_breadcrumbs(agent_view_controller, parent_crumb_mouse_state, app) -> Option<Box<dyn Element>>`:

- Returns `None` unless the flag is on, the view is fullscreen, and the active conversation has a parent.
- Builds two `CrumbSpec`s (parent + active child) and wires them into a `Flex::row` with a `ChevronRight` icon separator. Crumbs share the same avatar treatment as pills.
- The parent crumb takes a caller-owned `MouseStateHandle` (must be a field on `TerminalView`, not constructed inline) and dispatches `SwitchAgentViewToConversation` on click. The trailing crumb has no `Hoverable` and no click handler.

We render breadcrumbs manually rather than reusing `crate::ui_components::breadcrumb` because the shared helper does not support a chevron separator or per-crumb avatars.

### 4. New `TerminalAction` variant

`app/src/terminal/view/action.rs`: add `SwitchAgentViewToConversation { conversation_id: AIConversationId }` plus a `Debug` arm. Distinct from `RevealChildAgent` because pill clicks must navigate the current pane in place rather than emit `Event::RevealChildAgent`, which the pane group treats as a request to spawn / reveal a separate pane.

`app/src/terminal/view.rs`: add a handler arm in `handle_action` calling `self.enter_agent_view_for_conversation(None, AgentViewEntryOrigin::ConversationListView, *conversation_id, ctx)`. Add the variant to the `update_agent_view_pane_header`-eligible action list around line 24393.

### 5. `TerminalView` field + construction

`app/src/terminal/view.rs`:

- Add `orchestration_pill_bar: ViewHandle<OrchestrationPillBar>` field on `TerminalView` (next to `agent_view_back_button`, ~2738).
- Construct in `TerminalView::new` alongside `agent_view_controller`. Subscribe to its no-op event so the parent view re-renders when the pill bar notifies (`ctx.subscribe_to_view(&orchestration_pill_bar, |_, _, _, ctx| ctx.notify())`).

### 6. Pane header wiring

`app/src/terminal/view/pane_impl.rs`:

- In `render_header_title`, short-circuit at the top: if `render_orchestration_breadcrumbs(self.agent_view_controller.as_ref(app), self.mouse_states.parent_conversation_header_link.clone(), app)` returns `Some(element)`, return it directly. Returning the element directly (instead of wrapping in `MainAxisSize::Min` Flex) is required: the breadcrumbs row internally uses `Shrinkable` children, and `render_three_column_header` already wraps the title in `Shrinkable + Clipped` which provides a finite main-axis constraint. A `MainAxisSize::Min` wrapper here would forward an infinite constraint and panic the inner `Shrinkable`.
- In `maybe_add_parent_navigation_card`, add an early branch for the new flag:
  ```rust
  if FeatureFlag::OrchestrationPillBar.is_enabled()
      && FeatureFlag::AgentView.is_enabled()
      && self.agent_view_controller.as_ref(app).is_fullscreen()
  {
      let pinned_header = ConstrainedBox::new(header)
          .with_height(PANE_HEADER_HEIGHT)
          .finish();
      let pill_bar = ChildView::new(&self.orchestration_pill_bar).finish();
      return Flex::column()
          .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
          .with_child(pinned_header)
          .with_child(pill_bar)
          .finish();
  }
  ```
  Pinning the header to `PANE_HEADER_HEIGHT` is **load-bearing**, not cosmetic. `Flex::column` passes `max.y = INFINITY` to its non-flex children (`SizeConstraint::child_constraint_along_axis` in `crates/warpui_core/src/presenter.rs:794`). Without the explicit `ConstrainedBox`, the inner `Align` in `render_three_column_header` collapses to the title's small line-box height and the outer row's `CrossAxisAlignment::Stretch` paints children at offset 0 (top) — the title visibly clings to the top of the row instead of being centered. See `crates/warpui_core/src/elements/flex/mod.rs (467-473)` for the cross-axis offset math and `align.rs (77-89)` for Align's infinite-constraint fallback.

### 7. Mouse state wiring

The breadcrumb's parent crumb needs a persistent `MouseStateHandle`. We reuse `TerminalViewMouseStates::parent_conversation_header_link` (already a field on `TerminalView` for the legacy parent-card link), threaded through `render_orchestration_breadcrumbs`. Per-pill mouse state lives on the pill bar view itself (in its `mouse_states` HashMap, ensured/cleared on history events).

## Testing and validation

### Manual / dogfood verification

To verify in a local build, run an orchestrator (e.g. via `/orchestrate`) that spawns at least two child agents and walk through the invariants from `PRODUCT.md`:

- (1)–(3): Confirm the pill bar appears only on the orchestrator's view in fullscreen agent mode and disappears when there are zero children or when not in fullscreen.
- (4): Cause status changes on multiple children (commands finishing, in-progress flips). Pill order must not reshuffle.
- (5)–(8): Visual check vs Figma (avatar size, pill height, padding, hover/selected styling).
- (9): Click a sibling child pill — the same pane navigates to it (no split spawned, no new tab). Click the orchestrator pill from a child — same.
- (10)–(11): On a child view, breadcrumbs replace the title. Click the parent crumb → returns to orchestrator and pill bar reappears.
- (12): Hover a pill, then trigger a re-render (e.g. wait for a status update). Hover state must persist; cursor must remain pointing-hand.
- (14): Compare title vertical centering against the `ESC for terminal` button on the same row. Both must be centered.
- (15): Toggle `OrchestrationPillBar` off in settings. Header must render exactly as before, including the legacy parent-conversation card path.
- (17): Enter and exit the agent view multiple times. No stale hover bleeds through.

### Layout-regression test

Add a unit test next to `OrchestrationPillBar` that lays out the view in a `warpui::App::test` with at least one child conversation, asserting it does not panic. This is the standard "UI components need layout validation tests" requirement from the `create-pr` skill, and it specifically guards the load-bearing `ConstrainedBox::with_height(PANE_HEADER_HEIGHT)` fix in `maybe_add_parent_navigation_card` (see Risks).

### Behavior-driven coverage to consider

- `pill_specs` returning `None` when the active conversation has a parent and when the orchestrator has zero children — pure logic, easy to unit test against a mocked `BlocklistAIHistoryModel`.
- `pill_avatar_color` being deterministic for a given name (idempotency).
- `orchestrator_label` falling back through `agent_name` to `"Orchestrator"`.

## Risks and mitigations

- **Regression: title vertical centering.** The `Flex::column` wrap introduced by this feature inadvertently broke the title's centering until the header was pinned to `PANE_HEADER_HEIGHT`. The pinning is the only reason centering still works — any future refactor of `maybe_add_parent_navigation_card` that loses the `ConstrainedBox::with_height(PANE_HEADER_HEIGHT)` will regress. Add an inline comment at the call site (already done) and the layout-regression test above.
- **Mouse state lifetime.** Constructing `MouseStateHandle::default()` inline at render time silently zeros out hover state every frame. Per-pill state lives in the view's `mouse_states` map; the parent crumb's state is sourced from `TerminalViewMouseStates`. This pattern is enforced by the existing WARP.md guidance.

## Follow-ups

- Hover preview popover on a pill (small thumbnail of the conversation).
- Pin / unpin a child to keep its conversation open in a parallel split.
- 3-dot menu on each pill: `Open in new pane`, `Open in new tab`, `Stop agent`, `Kill agent`.
- Consider extending `crate::ui_components::breadcrumb` to support per-crumb avatars and a chevron separator so the manual breadcrumb rendering here can collapse into the shared helper.
