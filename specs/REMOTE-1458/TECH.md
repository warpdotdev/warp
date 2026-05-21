# REMOTE-1458: Tech Spec — Unified agent icon-with-status
## Problem
Agent-icon rendering is duplicated across four surfaces and each surface re-derives the underlying `{agent kind, status, is_ambient}` tuple from different primitives. Today's logic is also incomplete: the vertical tab icon doesn't flip to the selected third-party harness until the harness CLI is detected in `CLIAgentSessionsModel`, leaving a stale "Oz cloud" icon during the entire setup phase for Claude / Gemini cloud runs. See `specs/REMOTE-1458/PRODUCT.md` for desired behavior.
## Relevant code
- `app/src/ui_components/agent_icon.rs` — source-facing helpers: `terminal_view_agent_icon_variant`, `conversation_or_task_agent_icon_variant`, plus the pure inner functions exercised by tests.
- `app/src/ui_components/agent_icon_tests.rs` — cross-surface equivalence test suite.
- `app/src/ui_components/icon_with_status.rs` — `IconWithStatusVariant`, `render_icon_with_status`, `render_with_cloud_status_badge`, `OZ_AMBIENT_BACKGROUND_COLOR`, sub-component size ratios.
- `app/src/workspace/view/vertical_tabs.rs` — `resolve_icon_with_status_variant`, `TypedPane::summary_pane_kind`, `SummaryPaneKind`, `ambient_agent_variant`, `VERTICAL_TABS_ICON_SIZE`.
- `app/src/terminal/view/pane_impl.rs` — `render_header_title`, `is_in_cloud_agent_setup_phase`, `selected_conversation_status`, `selected_conversation_status_for_display`, `PANE_HEADER_AGENT_SIZE`.
- `app/src/terminal/view/ambient_agent/model.rs` — `selected_harness`, `is_third_party_harness`, `selected_third_party_cli_agent`, `mark_harness_command_started`, `harness_command_started`.
- `app/src/terminal/view/ambient_agent/view_impl.rs::handle_ambient_agent_event` — re-emits `TerminalViewEvent::TerminalViewStateChanged` on icon-affecting state transitions.
- `app/src/terminal/cli_agent_sessions/mod.rs` — `CLIAgentSessionsModel::session`.
- `app/src/terminal/cli_agent.rs` — `CLIAgent` enum + `CLIAgent::from_harness`, brand colors, icons.
- `crates/warp_cli/src/agent.rs` — `Harness` enum, `Harness::config_name` / `Harness::from_config_name`.
- `app/src/workspace/view/conversation_list/item.rs` — `render_item` (inline conversation list row leading-slot icon), `LIST_ITEM_AGENT_SIZE`, `LIST_ITEM_OVERLAY_EXTRA_OVERHANG`.
- `app/src/ai/agent_conversations_model.rs` — `ConversationOrTask::title`, `status`, `display_status`, `harness`.
- `app/src/ai/ambient_agents/task.rs` — `AmbientAgentTask`, `AgentConfigSnapshot.harness: Option<HarnessConfig>`, `serialize_harness` / `deserialize_harness` (now route through `Harness::config_name` / `Harness::from_config_name`).
- `app/src/ai/agent_management/notifications/item.rs` — `NotificationSourceAgent::{Oz, CLI}` (both carry `is_ambient`), `NotificationItem`.
- `app/src/ai/agent_management/notifications/item_rendering.rs` — `render_agent_avatar`, `NOTIFICATION_AVATAR_SIZE`.
- `app/src/ai/agent_management/agent_management_model.rs` — `handle_cli_agent_session_event`, `handle_history_event_for_mailbox`, `add_notification`, `find_terminal_view_by_id`, `TerminalViewMetadata` (consolidates the `is_ambient`/branch lookup).
## Current state (pre-implementation)
Vertical tabs contains the only surface that uses the full `IconWithStatusVariant` shape; `resolve_icon_with_status_variant` walks a CLI-session-then-conversation waterfall, and `TypedPane::summary_pane_kind` reimplements the same logic without status. The two waterfalls agree on the happy path but diverge on the ambient-but-no-session case (both return OzAgent regardless of the user's selected harness, which is the bug driving this spec).
The pane header's indicator slot uses plain glyphs. `render_ambient_agent_indicator` renders `Icon::OzCloud` in a constrained box with no brand circle, no status, and no harness specialization. `render_agent_indicator` renders either `WarpIcon::Oz` / `WarpIcon::OzCloud` OR a raw status element — never a combined circle+status.
The inline conversation list row's leading slot (in `conversation_list/item.rs::render_item`) switches between two plain icons based on whether the row represents a `ConversationOrTask::Task` (ambient) or `ConversationOrTask::Conversation` (local): a raw `Icon::Cloud` glyph for ambient rows, or `render_status_element(&conversation.status(app), font_size, appearance)` for local rows. There is no brand color, harness identity, or cloud-lobe treatment in that slot at all.
The Agent Management View's card leading slot (`AgentManagementView::render_header_row` in `app/src/ai/agent_management/view.rs`) also calls `render_status_element` with the display status. That surface is out of scope for this pass — its richer card layout (action buttons, session status labels, creator avatars) owns its own status treatment and we leave it untouched.
Notifications call `render_agent_avatar(NotificationSourceAgent, NotificationCategory, theme)` which maps to `IconWithStatusVariant::{OzAgent, CLIAgent}` with `is_ambient: false` hardcoded and a category-derived status.
## Architecture: helpers, not a singleton
Before diving into the per-surface changes, one architectural decision: centralization happens via a small set of source-facing *helpers* that return `Option<IconWithStatusVariant>`, not via a new singleton model that caches computed descriptors.
A singleton was considered. It would offer "guaranteed centralization" and "one call site per surface" in exchange for keeping a cached `AgentIconDescriptor` keyed by some notion of run identity. The tradeoffs that kept us away from it:
- **Keying.** A logical run has multiple identities (`terminal_view_id` when open, `AmbientAgentTaskId` remotely, `AIConversationId` in history, plus the notification's own `terminal_view_id`). A singleton would need overlapping indices and a merge policy for the same run viewed via different keys. On-demand helpers sidestep this entirely because each surface already holds exactly one identity.
- **Update plumbing.** Every emit path that mutates the underlying state would have to push into the singleton: `CLIAgentSessionsModel` (6 event variants), `AmbientAgentViewModel` (11 event variants), `BlocklistAIHistoryModel` (conversation status updates), `AgentConversationsModel` (task state updates). ~25 new write sites, each a potential source of stale state.
- **Derivation cost.** The computation is a small `match` over a few field reads. It is cheaper to recompute at render time than to maintain a cache. Cached derived state is also not an idiomatic pattern in WarpUI.
- **Testability.** Source helpers with a source argument are trivially testable by constructing the source. Singleton state requires mocking.
The Option-3 trait path (see *Follow-ups*) is the natural escalation if we ever grow past two source types and want a shared descriptor contract without a cache. Do that later, not now.
## Proposed changes
### 1. Extend `IconWithStatusVariant` with `is_ambient` + cloud-lobe rendering
`app/src/ui_components/icon_with_status.rs`
Extend `IconWithStatusVariant::OzAgent` and `IconWithStatusVariant::CLIAgent` to carry `is_ambient: bool`. Replace the previous `IconWithStatusSizing` struct with a single `total_size: f32` plus an `overlay_extra_overhang_ratio: f32` parameter on `render_icon_with_status`; every sub-component (brand circle, status badge, cloud lobe, status icon inside the lobe) is derived proportionally from `total_size` via module-private ratio constants (`CIRCLE_RATIO`, `BADGE_RATIO`, `CLOUD_RATIO`, etc.). Surfaces just pick the size they want and pass `0.0` for the overhang in the common case; the conversation list passes a small positive overhang to push the badge to the bounding-box corner.
Introduce `render_with_cloud_status_badge` as a private helper that overlays a white `WarpIcon::CloudFilled` at the bottom-right of the brand circle with the status icon (if any) centered inside; invoke it from the agent variants when `is_ambient` is true in place of the normal status ring. For ambient Oz runs, swap the circle background to the brand purple `OZ_AMBIENT_BACKGROUND_COLOR = ColorU { r: 203, g: 176, b: 247, a: 255 }` and render `WarpIcon::OzCloud` instead of `WarpIcon::Oz`. Each surface defines a single `*_AGENT_SIZE: f32` (or `NOTIFICATION_AVATAR_SIZE`) constant; threading `is_ambient: true` is what actually enables lobe rendering.
### 2. Centralize derivation via per-source helpers (no new types)
Keep `IconWithStatusVariant` as the canonical render-time shape. Introduce one pure helper per data source, each returning `Option<IconWithStatusVariant>` covering only the agent variants (`OzAgent`, `CLIAgent`). Non-agent variants (`Neutral`, `NeutralElement`) stay at the call site.
The helpers share three primitive mappers — each mapper lives in exactly one place and is invoked by every helper that needs it:
- `CLIAgent::from_harness(Harness) -> Option<CLIAgent>`, added on `CLIAgent` in `app/src/terminal/cli_agent.rs`. Maps every `Harness` variant exhaustively (Oz → None; Claude/Gemini/OpenCode → their corresponding CLIAgent; Unknown → `Some(CLIAgent::Unknown)`, which the icon waterfall filters out). `AmbientAgentViewModel::selected_third_party_cli_agent` is a thin wrapper that just delegates to `CLIAgent::from_harness(self.harness)` — it does NOT gate on `FeatureFlag::AgentHarness`, so the icon change ships independently of that flag.
- `Harness::config_name` / `Harness::from_config_name` on `crates/warp_cli/src/agent.rs`. The exhaustive `config_name` match forces every new `Harness` variant to declare a canonical name, and a round-trip test in `warp_cli::agent::tests` locks the inverse pair. `task.rs::serialize_harness` / `deserialize_harness` route through these helpers, so no separate string parser exists.
- `ConversationOrTask::status(app) -> ConversationStatus` (already exists in `agent_conversations_model.rs`).
- `ConversationOrTask::harness() -> Option<Harness>` (already exists in `agent_conversations_model.rs`); the task-card helper consumes this directly without round-tripping through a string.
- `TerminalView::selected_conversation_status_for_display(ctx) -> Option<ConversationStatus>` (already exists in `pane_impl.rs`; surfaces InProgress whenever the run is busy, including the cloud-setup phase).
### 3. `AmbientAgentViewModelEvent` re-emission through `TerminalViewStateChanged`
`app/src/terminal/view/ambient_agent/view_impl.rs::handle_ambient_agent_event`
The vertical tabs (and every other subscriber that renders from `terminal_view_agent_icon_variant`) re-render on `TerminalViewEvent::TerminalViewStateChanged`. Emit that event from the ambient-model event handlers for every state transition that can change the icon output: `EnteredSetupState`, `EnteredComposingState`, `DispatchedAgent`, `SessionReady`, `ProgressUpdated`, `Failed`, `Cancelled`, `NeedsGithubAuth`, `HarnessSelected`, `HarnessCommandStarted`. The event propagates pane-group → workspace → `ctx.notify()` via existing wiring in `app/src/pane_group/pane/terminal_pane.rs:720-722` and `app/src/workspace/view.rs:12920`.
### 4. `TerminalView::is_in_cloud_agent_setup_phase` + status override
`app/src/terminal/view/pane_impl.rs`
Add `is_in_cloud_agent_setup_phase(ctx) -> bool` that returns true while `ambient_agent_view_model.is_waiting_for_session()` OR `is_cloud_agent_pre_first_exchange` is true. Update `selected_conversation_status` (line 996) and `selected_conversation_status_for_display` (line 1031) to treat the setup phase as `ConversationStatus::InProgress`, matching how active long-running shell commands surface today. This is what makes the spinner show up inside the cloud lobe on every surface that consults the terminal view for status (vertical tabs, pane header). Remote-task surfaces (conversation list cards) already return InProgress during `AmbientAgentTaskState::{Queued, Pending, Claimed, InProgress}` via `ConversationOrTask::status`, so they inherit the same semantic for free.
### 5. `terminal_view_agent_icon_variant` helper
New free function in a new module `app/src/ui_components/agent_icon.rs`, re-exported via `app/src/ui_components/mod.rs`. Signature:
```rust
pub(crate) fn terminal_view_agent_icon_variant(
    terminal_view: &TerminalView,
    app: &AppContext,
) -> Option<IconWithStatusVariant>;
```
The public function gathers primitives from the live `TerminalView` into a `TerminalIconInputs` struct, then delegates to a pure inner function (`agent_icon_variant_from_terminal_inputs`) that's exercised directly by the cross-surface tests without an `AppContext`. Resolution order:
1. If `CLIAgentSessionsModel::session(terminal_view.id())` returns a session with a known (non-`Unknown`) agent, return `CLIAgent { agent, status, is_ambient }` where `status` is `Some(session.status.to_conversation_status())` only when the session is plugin-backed AND the handler exposes rich status (Codex's OSC 9 handler does not). Plugin-backed and command-detected sessions share a single match arm here.
2. Else if the terminal is ambient and `ambient_agent_view_model.selected_third_party_cli_agent()` returns a non-`Unknown` agent, return `CLIAgent { agent, status: selected_conversation_status_for_display, is_ambient: true }`. **This is the fix for the pre-setup bug.** `Unknown` is filtered to avoid rendering an unbranded gray circle for a future-server harness this client doesn't recognize.
3. Else if the terminal has a selected conversation OR `is_ambient`, return `OzAgent { status: selected_conversation_status_for_display, is_ambient }`.
4. Else `None` (caller falls through to `Neutral { Terminal }` / `Neutral { Shell }` / error indicator).
### 6. `conversation_or_task_agent_icon_variant` helper
Same module as above. Signature:
```rust
pub(crate) fn conversation_or_task_agent_icon_variant(
    src: &ConversationOrTask<'_>,
    app: &AppContext,
) -> Option<IconWithStatusVariant>;
```
Rules:
- `ConversationOrTask::Task(_)`: ambient run. Pull the `Harness` directly from `ConversationOrTask::harness()` (already returns `Option<Harness>` from the deserialized `agent_config_snapshot.harness.harness_type`, no string parsing) and pass it to a private `agent_icon_variant_for_task(harness, status)` that maps via `CLIAgent::from_harness`. Filter out `CLIAgent::Unknown` so future-server harnesses fall back to the Oz treatment instead of an unbranded gray circle.
- `ConversationOrTask::Conversation(_)`: local Oz conversation. Emit `OzAgent { status: Some(src.status(app)), is_ambient: false }`. Local conversations have no `CLIAgent` signal on this surface (per product spec).
### 7. Wire vertical tabs through the helper
Replace the two inline derivations:
- `resolve_icon_with_status_variant` in `vertical_tabs.rs:2254-2299` for the Terminal branch becomes:
  ```rust
  TypedPane::Terminal(terminal_pane) => {
      let terminal_view = terminal_pane.terminal_view(app);
      let terminal_view = terminal_view.as_ref(app);
      if let Some(variant) = terminal_view_agent_icon_variant(terminal_view, app) {
          return variant;
      }
      IconWithStatusVariant::Neutral {
          icon: WarpIcon::Terminal,
          icon_color: main_text,
      }
  }
  ```
- `TypedPane::summary_pane_kind` in `vertical_tabs.rs:2488-2509` for the Terminal branch becomes a thin wrapper that maps the helper's variant into `SummaryPaneKind::OzAgent`/`CLIAgent` with matching `is_ambient`:
  ```rust
  if let Some(variant) = terminal_view_agent_icon_variant(terminal_view, app) {
      return match variant {
          IconWithStatusVariant::OzAgent { is_ambient, .. } => SummaryPaneKind::OzAgent { is_ambient },
          IconWithStatusVariant::CLIAgent { agent, is_ambient, .. } => SummaryPaneKind::CLIAgent { agent, is_ambient },
          _ => unreachable!("helper only returns agent variants"),
      };
  }
  // else: existing conversation/title fallback / Terminal
  ```
No other logic in these two functions changes — the centralization removes the duplicated waterfall without moving the call sites.
### 8. Wire pane header through the helper
Replace the agent-only indicator branches in `render_header_title` (`pane_impl.rs`):
- Today the branch chain is: shared ambient agent indicator → shared-session avatar / Sharing icon → conversation-selected `render_agent_indicator` → terminal-mode indicator.
- New: when the shared-session branch fires for a viewed/transcript ambient session, OR when the conversation-bound branch fires, call `terminal_view_agent_icon_variant(self, app).map(render_agent_circle)` where `render_agent_circle` is a small closure that calls `render_icon_with_status(variant, PANE_HEADER_AGENT_SIZE, 0., theme, theme.background())`. This replaces both `render_ambient_agent_indicator` and `render_agent_indicator`.
- Plain-terminal / shell / error indicators (`render_terminal_mode_indicator`) stay untouched.
Define `PANE_HEADER_AGENT_SIZE: f32 = 26.` in `pane_impl.rs` (slightly larger than `VERTICAL_TABS_ICON_SIZE` to fit the pane header row). Delete the now-unused `render_agent_indicator` and `render_ambient_agent_indicator` after the migration, plus the `mouse_states.ambient_agent_indicator_mouse_handle` field on `TerminalView`.
### 9. Wire the inline conversation list menu through the helper
In `conversation_list/item.rs::render_item`, replace the two-branch icon derivation (`Icon::Cloud` for ambient rows, `render_status_element` for local rows) with:
```rust
let icon_element: Box<dyn Element> =
    match conversation_or_task_agent_icon_variant(conversation, app) {
        Some(variant) => render_icon_with_status(
            variant,
            LIST_ITEM_AGENT_SIZE,
            LIST_ITEM_OVERLAY_EXTRA_OVERHANG,
            theme,
            theme.background(),
        ),
        None => render_status_element(&conversation.status(app), font_size, appearance),
    };
```
`LIST_ITEM_AGENT_SIZE: f32 = 22.` is sized so the icon footprint stays close to the existing `status_element_size = font_size + STATUS_ELEMENT_PADDING * 2.` and row heights don't shift. `LIST_ITEM_OVERLAY_EXTRA_OVERHANG: f32 = 0.05` pushes the badge fully to the bounding-box corner, which reads better in this denser layout than the default centered overhang. The `None` branch keeps the surface future-proof; every current `ConversationOrTask` produces an agent variant today.
The Agent Management View (`AgentManagementView::render_header_row`) is explicitly out of scope in this pass — keep its existing `render_status_element` call untouched. See `PRODUCT.md` § "Inline conversation list menu" for rationale.
### 10. Thread `is_ambient` through `NotificationSourceAgent`
Expand the enum to carry the flag so downstream rendering can honor it:
```rust
// app/src/ai/agent_management/notifications/item.rs
pub enum NotificationSourceAgent {
    Oz { is_ambient: bool },
    CLI { agent: CLIAgent, is_ambient: bool },
}
```
Both notification emit paths (`handle_cli_agent_session_event` and `handle_history_event_for_mailbox`) need `is_ambient` on the source. They also already need the git branch for the rich-layout header row, and that lookup walks the same workspace tree. Consolidate both into a single `TerminalViewMetadata { is_ambient, branch }` struct and a `TerminalViewMetadata::lookup(terminal_view_id, app)` helper that calls `find_terminal_view_by_id` once per notification, then reads `view.is_ambient_agent_session(app)` (the existing `TerminalView` helper, which gates on `FeatureFlag::CloudMode`) and `view.current_git_branch(app)`. `add_notification` takes the `branch: Option<String>` as a parameter so it doesn't perform a second lookup; callers thread the metadata through.
Update `render_agent_avatar` in `item_rendering.rs` to read `is_ambient` from the enum variant rather than hardcoding `false`. The existing `IconWithStatusVariant::{OzAgent, CLIAgent}` code path already produces the cloud-lobe rendering when `is_ambient` is true; no changes to `render_icon_with_status` itself.
The telemetry event `TelemetryEvent::AgentNotificationShown { agent_variant: agent.into() }` keeps its existing schema by dropping the ambient flag in the `From<NotificationSourceAgent>` impl.
### 11. Telemetry / logs
No new telemetry events. Existing `AgentNotificationShown` keeps firing with the agent variant. If schema allows, extend it with `is_ambient: bool` to enable future cloud-vs-local notification analytics; otherwise defer.
### 12. Cross-surface equivalence tests
The suite's job is to lock the invariant *"same logical run → identical icon on every surface"* and catch drift when a new surface or state combo is added. Three essential pieces, all in a new `app/src/ui_components/agent_icon_tests.rs`.
#### 12.1 Canonical state fixture
A test-only enum `CanonicalRunState` enumerates every conceptually distinct run (plain terminal; local Oz conversation; local CLI agent (plugin-backed in-progress / plugin-backed blocked / command-detected); cloud Oz in-progress; cloud third-party pre-dispatch and in-progress). It exposes:
```rust
impl CanonicalRunState {
    fn all() -> &'static [Self];
    fn terminal_inputs(&self) -> TerminalIconInputs;
    fn task_inputs(&self) -> Option<(Harness, ConversationStatus)>;
    fn expected(&self) -> Option<AgentIconFields>; // single canonical answer
}
```
`expected` is spec-in-code — editing it requires editing `PRODUCT.md`. `AgentIconFields` is a `PartialEq` projection of `IconWithStatusVariant`'s agent-variant fields (`IconWithStatusVariant` itself can't derive `PartialEq` because `NeutralElement` carries a `Box<dyn Element>`).
#### 12.2 Canonical-state equivalence table
One parameterized test (`every_canonical_state_produces_consistent_icon_across_surfaces`) drives every canonical state through both the terminal-side helper (`agent_icon_variant_from_terminal_inputs`) and the task-side helper (`agent_icon_variant_for_task`) and asserts they project to the same `AgentIconFields`. Adding a surface = one more comparison; adding a state = one more enum variant + `expected` arm. A second test (`terminal_is_ambient_matches_inputs_for_every_state`) locks the structural invariant that `is_ambient` on the rendered variant always matches the input flag.
#### 12.3 Spot tests
- `cli_agent_from_harness_maps_known_harnesses` covers Oz/Claude/Gemini/OpenCode.
- `local_claude_vs_cloud_claude_differ_only_by_is_ambient` asserts a locally-registered Claude CLI session and an ambient Claude run produce the same `CLIAgent { Claude, .. }` variant differing only by `is_ambient`.
- `task_with_oz_or_unknown_harness_renders_as_oz` asserts both Oz and Unknown harnesses fall back to the Oz variant on task cards.
- `summary_pane_kind_icons_distinguish_ambient_claude_from_local_claude` in `vertical_tabs_tests.rs` stays as-is. Existing `notifications/item_tests.rs` tests are extended with `is_ambient: true` and `false` cases for each variant.
## End-to-end flow (Claude cloud run)
1. User selects Claude in harness selector. `AmbientAgentViewModel::set_harness(Harness::Claude, ctx)` fires `HarnessSelected`.
2. The `HarnessSelected` handler in `ambient_agent/view_impl.rs:262` emits `TerminalViewStateChanged` → pane group → workspace → `ctx.notify()`.
3. Vertical tabs re-render. `terminal_view_agent_icon_variant` walks its waterfall: no CLI session yet (step 1/2 skipped), `is_third_party_harness()` → true with `selected_third_party_cli_agent() → Some(CLIAgent::Claude)`, `selected_conversation_status_for_display` → `None` (nothing in progress yet since Composing), so variant is `CLIAgent { Claude, None, is_ambient: true }`. Tab shows Claude-orange circle, cloud lobe (no status icon inside).
4. User submits prompt. `DispatchedAgent` fires; view-model transitions to `WaitingForSession`; `is_in_cloud_agent_setup_phase` → true; `selected_conversation_status_for_display` → `Some(InProgress)`. Helper returns `CLIAgent { Claude, Some(InProgress), is_ambient: true }`. Tab shows Claude circle, cloud lobe with spinner inside.
5. Pane header renders the same helper output, same visual.
6. Conversation list card appears as soon as the `AmbientAgentTask` is registered. `conversation_or_task_agent_icon_variant` reads `agent_config_snapshot.harness` → "claude" → `Harness::Claude` → `CLIAgent::Claude`; `ConversationOrTask::status` → `InProgress`. Card shows same visual.
7. Session ready; harness block starts; `CLIAgentSessionsModel::set_session(CLIAgent::Claude)` fires. Helper's step 1 takes over (plugin-backed session). Same brand + status, so no visual flip.
8. Notification fires (e.g. blocked permission request). Emit path resolves `terminal_view.ambient_agent_view_model().is_ambient_agent() → true`; `NotificationSourceAgent::CLI { Claude, is_ambient: true }` goes into the notification. `render_agent_avatar` → `CLIAgent { Claude, Some(Blocked), is_ambient: true }`. Notification shows Claude cloud avatar.
9. Run completes. Status propagates from `AmbientAgentTask.state` → `ConversationOrTask::status` → Success; card icon updates. `AmbientAgentViewModel` transitions to terminal state; `HarnessCommandStarted` already fired earlier, so `is_cloud_agent_pre_first_exchange` is false and the tab/pane-header helpers pick up the real conversation status.
## Risks and mitigations
**`NotificationSourceAgent` schema change is a breaking API for existing call sites.** Every construction site is in `agent_management_model.rs` (8 call sites between CLI events and history events). We'll update them all in the same patch. The helper function `find_terminal_view_by_id` keeps the lookup cost low (already paid once per notification for branch resolution).
**Look-up races for is_ambient at notification emit time.** If the terminal view has been torn down before the notification fires (unlikely — notifications fire off in-flight events), the lookup returns `None` and we default `is_ambient: false`. This is safe; the notification still renders, just without the cloud lobe.
**Pane header sizing regression.** The current indicator uses `appearance.ui_font_size()` (~12-14px) as the icon size; the new circle is 24-26px overall. The header row height is fixed by other elements (title text at 13-14px), so a larger circle is taller. We may need to adjust vertical alignment or cross-axis sizing. Validate during implementation; if the header grows, either reduce `PANE_HEADER_AGENT_SIZE` to match the font size or re-lay the header row to center the circle against the title.
**Removing `render_ambient_agent_indicator` drops its tooltip.** The indicator currently shows a "Cloud agent run" tooltip on hover (pane_impl.rs:881-892). If retaining the tooltip is important, wrap the new circle in the same `Hoverable` and tooltip. Otherwise drop; the cloud lobe makes the ambient nature visually obvious.
**Card icon replaces the plain status-only visual.** Cards that don't have an agent (none today, but defensively) fall through to the existing `render_status_element`. No visual regression expected.
**Helper takes a full `TerminalView` reference.** This creates a transitive dependency from `ui_components/agent_icon.rs` back to `terminal::view::TerminalView`. Acceptable because `ui_components` already has terminal-aware code (`icon_with_status.rs` imports `CLIAgent` from `terminal`). Alternative: invert the dependency by making the helper a method on `TerminalView` in `pane_impl.rs`. Pick based on where the compile dependencies look cleanest; prefer the free function in `ui_components/agent_icon.rs` for testability.
## Testing and validation
Unit coverage:
- Add `CLIAgent::from_harness` tests for Oz/Claude/Gemini inputs.
- Extend `vertical_tabs_tests.rs` (or add `ui_components/agent_icon_tests.rs`) with the cross-surface equivalence suite described in §12.
- Test `conversation_or_task_agent_icon_variant` with:
  - A `ConversationOrTask::Task` whose `agent_config_snapshot.harness` is `"claude"` → `CLIAgent::Claude` + `is_ambient: true`.
  - A `ConversationOrTask::Task` whose harness is `"oz"` or missing → `OzAgent` + `is_ambient: true`.
  - A `ConversationOrTask::Conversation` → `OzAgent` + `is_ambient: false`.
- Test that `terminal_view_agent_icon_variant` returns `CLIAgent { Claude, ... is_ambient: true }` when the view-model's selected harness is Claude and no CLIAgent session exists yet.
- Test idempotency of `NotificationSourceAgent` schema: old tests in `notifications/item_tests.rs` need updates for the new `is_ambient` field; add one new test case asserting the cloud-lobe path is taken when `is_ambient: true`.
Integration / manual validation:
- Full flows from the product spec's Validation section. Concretely: spawn a Claude cloud run; observe tab + pane header + card all render the same Claude circle + cloud lobe + status through Composing → Waiting → Running → Success.
- Start a local `claude` CLI session and confirm tab + pane header render the Claude circle with a bottom-right badge (no cloud lobe).
- Trigger a blocking permission request on a Claude cloud run; observe the notification in the mailbox shows the Claude cloud avatar.
- Re-run the REMOTE-1454 validation cases to confirm no regression in the cloud setup UX.
Invoke `verify-ui-change-in-cloud` after the implementation lands to spot-check the visual result across surfaces.
## Follow-ups
- Consider collapsing `NotificationSourceAgent` + `is_ambient` + status into a single `AgentIconDescriptor` struct shared by all four surfaces once a third or fourth call site appears (Option 3 in the design discussion). Not worth it at two call sites.
- If the helper function pattern proves brittle at the trait-boundary for test fakes, consider introducing an `AgentIconSource` trait and formalizing the descriptor in a follow-up PR.
- Telemetry: extend `AgentNotificationShown` with `is_ambient` once the schema is updated, to track cloud-notification volume.
