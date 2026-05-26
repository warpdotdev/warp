# Orchestration Pill Bar Pinning — Tech Spec
Linear: [QUALITY-672](https://linear.app/warpdotdev/issue/QUALITY-672)
PR: [#10777](https://github.com/warpdotdev/warp/pull/10777)
## Context
The orchestration pill bar renders a horizontal row of pills above the agent view header — one for the orchestrator and one for each child agent. With long-running parent agents that spawn many children, the row gets long enough that frequently-used child agents scroll off-screen, and the user has no way to keep them anchored.
The feature adds pinning so frequently-used children stay anchored to the leading section of the bar, with pin state shared across panes and persisted across app restarts.
### Relevant files
**Pill bar rendering**
- `app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs` — `OrchestrationPillBar` view (per-`TerminalView`). `pill_specs()` builds the spec list, `render_pill()` renders each pill, `View::render()` partitions and lays out the bar.
**Per-conversation persistence**
- `app/src/ai/agent/conversation.rs:262-301` — `AIConversation::new()` initializer; `new_restored()` rehydrates from `AgentConversationData`.
- `app/src/ai/agent/conversation.rs:2921-3008` — `write_updated_conversation_state()` builds an `UpdateMultiAgentConversation` `ModelEvent` and sends it to the SQLite writer thread.
- `crates/persistence/src/model.rs` — `AgentConversationData` (the JSON blob persisted per conversation).
**History model**
- `app/src/ai/blocklist/history_model.rs:462-477` — `update_event_sequence()`, the existing template for a "mutate one field + persist" method.
- `app/src/ai/blocklist/history_model.rs:1655-1732` — `RemoveConversation` / `DeletedConversation` events emitted from `remove_conversation_from_memory` and `delete_conversation`.
**Startup and logout**
- `app/src/lib.rs:~1683` — `BlocklistAIHistoryModel` registered at startup with the restored `multi_agent_conversations` vec.
- `app/src/auth/mod.rs:213-281` — `log_out()` calls `.reset()` on all singletons that hold user state.
**Icons**
- `crates/warp_core/src/ui/icons.rs` — `Icon` enum; new SVGs bundled at `app/assets/bundled/svg/`.
## Proposed changes
### 1. Per-conversation persistence
**File**: `crates/persistence/src/model.rs`
Add `pinned: bool` to `AgentConversationData` with `#[serde(default, skip_serializing_if = "is_false")]`. The `default` makes existing rows deserialize cleanly; the `skip_serializing_if` keeps unpinned conversations from bloating the persisted JSON.
**File**: `app/src/ai/agent/conversation.rs`
Add `pinned: bool` to `AIConversation`. Wire it through:
- `new()` initializer (defaults to `false`).
- `new_restored()` reads `conversation_data.pinned`.
- `write_updated_conversation_state()` includes `pinned: self.pinned` in the emitted `AgentConversationData`.
- New accessors `is_pinned()` / `set_pinned(bool)`.
**File**: `app/src/ai/blocklist/history_model.rs`
Add `set_conversation_pinned(conversation_id, pinned, ctx)` that updates the in-memory `AIConversation.pinned` and calls `write_updated_conversation_state(ctx)`. Mirrors the existing `update_event_sequence()` pattern. Early-return with `log::warn!` when the conversation isn't loaded so dropped writes are visible in logs rather than silent.
### 2. Cross-pane singleton (`OrchestrationPinModel`)
**File**: `app/src/ai/blocklist/agent_view/orchestration_pin_model.rs` (new)
New `SingletonEntity`. Holds `pinned: HashSet<AIConversationId>` as an in-memory mirror of the per-conversation `pinned` flag. Emits `OrchestrationPinEvent::PinSetChanged` on toggle and on history-driven prune.
Why singleton, not per-`TerminalView`:
- Pin state needs to be cross-pane (pinning in one pane should immediately reflect in every other pane's bar).
- Centralizes deleted-conversation cleanup so each pill bar doesn't race to clobber sibling panes' sets.
API:
```rust path=null start=null
pub fn new(initial_pinned: HashSet<AIConversationId>, ctx: &mut ModelContext<Self>) -> Self;
pub fn is_pinned(&self, conversation_id: &AIConversationId) -> bool;
pub fn toggle_pin(&mut self, conversation_id: AIConversationId, ctx: &mut ModelContext<Self>);
pub fn reset(&mut self); // called from log_out
```
`toggle_pin` flips the in-memory set and calls `BlocklistAIHistoryModel::set_conversation_pinned` to persist. `new` subscribes to `BlocklistAIHistoryEvent::{RemoveConversation, DeletedConversation}` and removes the affected id from `pinned` (emitting `PinSetChanged` only if the set actually changed).
**File**: `app/src/lib.rs`
Register the singleton at startup, after `BlocklistAIHistoryModel`. Seed `initial_pinned` by walking the restored `multi_agent_conversations` vec, deserializing `AgentConversationData` for each, and collecting ids where `data.pinned == true`.
**File**: `app/src/auth/mod.rs`
In `log_out`, call `OrchestrationPinModel::handle(app).update(app, |model, _| model.reset())` alongside the existing `*::reset()` calls. The persisted per-conversation flags are wiped by the existing SQLite reset; this just clears the in-memory mirror so the next user doesn't inherit the previous account's pins.
### 3. Pill bar rendering
**File**: `app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs`
- Subscribe to `OrchestrationPinModel`'s `PinSetChanged` event so every pane's bar re-renders together.
- Partition pills in `View::render()`: orchestrator → pinned children (spawn order) → vertical divider (only when both sides are non-empty) → unpinned children (spawn order).
- Add `PillKind::Child`'s pin glyph behavior in `render_pill()`:
  - At rest: avatar disc shown for both pinned and unpinned pills.
  - On hover: avatar swaps to a pin glyph — outline (`Icon::Pin`) when unpinned, solid (`Icon::PinFilled`) when pinned.
  - Pin state is also communicated by position (left of the divider).
**Click-handler scoping (subtle):** Only wrap the avatar/pin-glyph element in `Hoverable` *and* attach the `TogglePin` click handler when `show_pin_glyph` is true. If we wrap unconditionally, the inner `Hoverable` steals clicks during the 300ms `with_hover_in_delay` window — a user clicking the avatar to navigate would land on the toggle. With the conditional wrap, clicks on the avatar (rest state) bubble to the outer pill's navigate handler, and clicks on the pin glyph (hover state) toggle pin.
### 4. Icons
**Files**: `app/assets/bundled/svg/pin-01.svg`, `app/assets/bundled/svg/pin-filled.svg` (new), `crates/warp_core/src/ui/icons.rs`
Add `Icon::Pin` (outline) and `Icon::PinFilled` (solid) variants, bundled from Figma SVGs.
## Testing and validation
### Unit tests
**`orchestration_pin_model_tests.rs`** (new):
- `toggle_pin_in_set_flips_membership_for_each_call` — pure-function helper test for toggle semantics.
- `toggle_pin_in_set_only_affects_target_id` — guards against accidental cross-id mutation.
- `toggle_pin_persists_pinned_state_to_sqlite_event` — e2e: wires up settings + a mock `GlobalResourceHandles` channel, restores a conversation, calls `toggle_pin`, asserts (a) `AIConversation.is_pinned()` flips, (b) an `UpdateMultiAgentConversation` `ModelEvent` is sent with `conversation_data.pinned == true`, (c) a second toggle emits a follow-up event with `pinned: false`.
**`crates/persistence/src/model.rs`** — round-trip tests for `AgentConversationData.pinned`:
- Default deserialization (no `pinned` field in JSON) → `pinned: false`.
- `pinned: false` is skipped in serialized output.
- `pinned: true` round-trips correctly.
### Manual validation
- Start two panes with the same parent conversation. Pin a child in pane A → verify the child immediately moves to the leading section in pane B's bar.
- Pin a child, then quit and relaunch Warp → verify the child is still pinned (persistence).
- Hover a pinned child → solid pin glyph appears, hover background; click → unpinned and moves back across the divider.
- Hover an unpinned child → outline pin glyph appears; click → pinned.
- Click the avatar area of a pinned child during the hover-in delay window → verify navigation happens (not toggle).
- Delete a pinned conversation → verify it's removed from the pin set in all panes (no orphan).
- Log out and log in as a different user → verify pins do not carry over.
### Presubmit
`cargo fmt`, `cargo build -p warp`, `cargo nextest run -p warp orchestration_pin_model`, `cargo clippy -p warp --tests --all-features`.
## Risks and mitigations
**Risk: Click during the 300ms hover-in delay toggles pin instead of navigating.** First implementation had this bug.
*Mitigation:* Inner `Hoverable` (with the toggle handler) is only present when `show_pin_glyph` is true. At rest, clicks bubble to the outer navigate handler. Covered by manual validation; consider an integration test if regressions appear.
**Risk: `set_conversation_pinned` no-ops silently when the conversation isn't loaded into `conversations_by_id`.** This can happen for historical conversations.
*Mitigation:* `log::warn!` on the early-return path so dropped writes are visible. The in-memory `OrchestrationPinModel` set still toggles, so the UI is correct until that conversation rehydrates; the persisted state will simply be missing until the next time the conversation is loaded.
**Risk: Logged-out user's pins persist into the next login.**
*Mitigation:* `OrchestrationPinModel::reset()` is invoked from `log_out` alongside the other singleton resets. The SQLite-level reset that runs alongside logout wipes the persisted `pinned` flags.
**Risk: Forked conversations don't carry pin state forward.** `fork_conversation*` paths in `history_model.rs` build `AgentConversationData` with `pinned: false` regardless of source.
*Mitigation:* Intentional — forks are user-initiated "fresh start" semantics. Not a hidden gotcha because the new conversation has a different `AIConversationId` and isn't expected to share pin state.
## Parallelization
Not used. Single-PR change spanning ~12 files, mostly tightly coupled: the persistence schema change (`AgentConversationData.pinned`), the conversation accessor wiring, and the singleton seed in `lib.rs` all have to land together to avoid an intermediate broken state. The pill-bar UI work depends on the singleton existing. Splitting into sub-agents would just add coordination overhead for a change one engineer can land in a single sitting.
## Follow-ups
- **Drag-to-reorder within the pinned section.** Today pinned pills are in spawn order; the user can't manually reorder.
- **Pin from overflow menu / keyboard shortcut.** Pin is only reachable via hover-click. A right-click menu item or `cmd+shift+P` style binding would help keyboard-first users.
- **Telemetry on pin/unpin actions** to measure feature adoption.
