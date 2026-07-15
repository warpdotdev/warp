# TECH: TUI Orchestration Conversation Tab Bar
Linear: [CODE-1822 — Orchestration](https://linear.app/warpdotdev/issue/CODE-1822/orchestration)
Product: [specs/code-1822-tui-orchestration-tab-bar/PRODUCT.md](./PRODUCT.md)
Component: [specs/code-1822-tui-tab-bar-component/TECH.md](../code-1822-tui-tab-bar-component/TECH.md)
Inspected commit: `0bfc788907e2b27c3488c581fee92e2f67a18ef1`

## Context
The preceding stack gives every native local child a retained full TUI session, but only the focused session is projected and there is no navigation chrome between related sessions:
- [`crates/warp_tui/src/sessions.rs (20-175) @ 0bfc7889`](https://github.com/warpdotdev/warp/blob/0bfc788907e2b27c3488c581fee92e2f67a18ef1/crates/warp_tui/src/sessions.rs#L20-L175) — `TuiSessionId`, retained `TuiSession` views/managers, and `TuiSessions::focus_session`. `FocusChanged` updates projection state but currently has no focus-routing subscriber.
- [`crates/warp_tui/src/root_view.rs (36-82) @ 0bfc7889`](https://github.com/warpdotdev/warp/blob/0bfc788907e2b27c3488c581fee92e2f67a18ef1/crates/warp_tui/src/root_view.rs#L36-L82) — the root renders and exposes only `TuiSessions::focused_session()`, so switching sessions is a full-view swap rather than transcript replacement.
- [`crates/warp_tui/src/orchestration_model.rs (30-266) @ 0bfc7889`](https://github.com/warpdotdev/warp/blob/0bfc788907e2b27c3488c581fee92e2f67a18ef1/crates/warp_tui/src/orchestration_model.rs#L30-L266) — `TuiOrchestrationModel` materializes local native children, retains the child-conversation/session mapping needed for failed-launch cleanup, and subscribes to every session's `StartAgentExecutor`.
- [`app/src/ai/blocklist/history_model.rs (403-568) @ 0bfc7889`](https://github.com/warpdotdev/warp/blob/0bfc788907e2b27c3488c581fee92e2f67a18ef1/app/src/ai/blocklist/history_model.rs#L403-L568) — the shared history model owns loaded conversations, parent resolution, and the restart-durable `children_by_parent` index.
- [`app/src/ai/blocklist/history_model.rs (963-1018) @ 0bfc7889`](https://github.com/warpdotdev/warp/blob/0bfc788907e2b27c3488c581fee92e2f67a18ef1/app/src/ai/blocklist/history_model.rs#L963-L1018) — `terminal_surface_id_for_conversation` and `active_conversation` provide the authoritative conversation↔surface mapping. This mapping remains correct when a session restores a different conversation, unlike `TuiOrchestrationModel::child_session_by_conversation`.

The GUI already owns the canonical orchestration ordering and tree traversal:
- [`app/src/ai/blocklist/orchestration_topology.rs (107-180) @ 0bfc7889`](https://github.com/warpdotdev/warp/blob/0bfc788907e2b27c3488c581fee92e2f67a18ef1/app/src/ai/blocklist/orchestration_topology.rs#L107-L180) — `descendant_conversation_ids_in_pill_order` implements pin/status/recency/spawn ordering, and adjacent navigation establishes wraparound semantics.
- [`app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs (600-670) @ 0bfc7889`](https://github.com/warpdotdev/warp/blob/0bfc788907e2b27c3488c581fee92e2f67a18ef1/app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs#L600-L670) — the GUI renders an orchestrator first, then descendants in the shared canonical order. Its current active-child lookup climbs one parent; PRODUCT (2) requires the TUI to climb to the top orchestration root so every member session shows the same complete tree.

The focused session already owns the relevant input and focus transitions:
- [`crates/warp_tui/src/terminal_session_view.rs (746-1037) @ 0bfc7889`](https://github.com/warpdotdev/warp/blob/0bfc788907e2b27c3488c581fee92e2f67a18ef1/crates/warp_tui/src/terminal_session_view.rs#L746-L1037) — input construction, history subscriptions, background-session focus guards, blocker precedence, and input restoration.
- [`crates/warp_tui/src/terminal_session_view.rs (2166-2342) @ 0bfc7889`](https://github.com/warpdotdev/warp/blob/0bfc788907e2b27c3488c581fee92e2f67a18ef1/crates/warp_tui/src/terminal_session_view.rs#L2166-L2342) — the current session render tree, including alt-screen replacement, transcript, blocker handling, input, and footer.
- [`crates/warp_tui/src/input/view.rs (61-170) @ 0bfc7889`](https://github.com/warpdotdev/warp/blob/0bfc788907e2b27c3488c581fee92e2f67a18ef1/crates/warp_tui/src/input/view.rs#L61-L170) and [`crates/warp_tui/src/input/view.rs (584-675) @ 0bfc7889`](https://github.com/warpdotdev/warp/blob/0bfc788907e2b27c3488c581fee92e2f67a18ef1/crates/warp_tui/src/input/view.rs#L584-L675) — `Shift+Up` currently dispatches `SelectUp` unconditionally and the input applies vertical selection directly.
- [`crates/warp_tui/src/editor_element.rs (271-408) @ 0bfc7889`](https://github.com/warpdotdev/warp/blob/0bfc788907e2b27c3488c581fee92e2f67a18ef1/crates/warp_tui/src/editor_element.rs#L271-L408) — the char-cell display lattice is the source of truth for wrapped visual rows and cursor placement.

## Proposed changes
### 1. Export and reuse the GUI topology policy
Re-export `descendant_conversation_ids_in_pill_order` through `app/src/tui_export.rs`; do not duplicate its sort keys in `warp_tui`.

`TuiOrchestrationModel` reuses the GUI's descendant ordering while resolving the complete tree required by PRODUCT (2):
1. Receive the focused session's selected conversation ID from `TuiTerminalSessionView`; do not substitute the history model's “most recently streamed” active pointer for the input selection.
2. Follow resolved parent conversation IDs until reaching the top orchestration root, with a visited set to make malformed cycles fail closed.
3. Read all descendants from `descendant_conversation_ids_in_pill_order`.
4. Filter the orchestrator and descendants through the authoritative history surface mapping and a retained-session lookup.
5. Omit the bar unless the orchestrator and at least one child are immediately navigable.

Filtering occurs after canonical ordering so the relative order of navigable descendants remains identical to the GUI. The model does not use `child_session_by_conversation` for rendering or navigation; that map remains narrowly scoped to failed-child cleanup.

Add `TuiSessions::session_id_for_surface(EntityId)` and a narrow session-view lookup so the model can compose `BlocklistAIHistoryModel::terminal_surface_id_for_conversation` with retained TUI ownership. This makes restored conversations self-correcting without another conversation/session index.

### 2. Orchestration-owned tab state and actions
Extend `TuiOrchestrationModel` from launch coordination to the owner of semantic orchestration tab state. Add a per-orchestrator entry containing:
- The child page-anchor conversation ID.
- Whether the user explicitly paged away from the active tab.

Expose a plain-data snapshot for `TuiTerminalSessionView` containing the root, ordered child tabs, active conversation ID, spawn-order-derived identity indices, and page anchor. The session view maps identity indices and semantic tab states to current-theme styles before updating the generalized component. Snapshot construction reads live history and session state each time; cached UI state never becomes the source of truth for membership, status, ordering, or selection.

Model operations:
- `select_conversation` resolves the target's retained session, calls `TuiSessions::focus_session`, clears explicit paging, and anchors the page so the selected child is visible.
- `select_first_child`/`select_last_child` exclude the orchestrator.
- `set_explicit_page` records the page anchor emitted by the tab bar and marks it explicit. It never calls `focus_session`.
- A dynamic ordering update automatically re-anchors to the active child only when explicit paging is false.
- Removed roots prune their per-tree state; removed children clamp invalid anchors through fresh snapshot resolution.

Subscribe to `BlocklistAIHistoryModel` events that can change membership, parent linkage, labels, status, recency, pin order, or active selection, and to all `TuiSessionsEvent` variants. Emit a narrow `TabBarChanged` event so every retained session view in the affected tree redraws. These subscriptions also make child materialization/removal visible without polling.

### 3. Session switching and focus handoff
The model owns tab selection and page state, but actual responder focus remains view-owned because only a `ViewContext` can focus a view.

Add orchestration-tab actions and a keymap-context flag to `TuiTerminalSessionView`:
- `FocusInput`
- `SelectPrevious` / `SelectNext`
- `SelectFirstChild` / `SelectLastChild`
- `SelectConversation(AIConversationId)`
- `SetPage(AIConversationId)`

Register `Left`, `Right`, `Tab`, `Shift+Tab`, `Shift+Left`, `Shift+Right`, and `Shift+Down` only under the tab-focused context. Keep the existing session-level `Ctrl+C` binding unchanged; the focused footer omits kill copy in this PR.

The input's `FocusAboveRequested` event directly enters tab focus. `SelectPrevious` and `SelectNext` ask the tab-bar component for a target from its private settled layout; the session view then delegates that semantic selection to `TuiOrchestrationModel`. The session and model never read a visible range.

Track whether the session view itself currently owns tab-bar focus. A switch performs two coordinated operations:
1. Ask `TuiOrchestrationModel` to select/focus the target retained session.
2. Focus the target `TuiTerminalSessionView` and set its tab-focused mode for keyboard switches or already-focused mouse switches; otherwise invoke a target-session helper that applies existing precedence: alternate screen, active blocker, then input.

This closes the current `TuiSessionsEvent::FocusChanged` projection/focus gap for tab-driven switches without letting a background session steal focus. Add an explicit test that the projected root child and responder chain move together.

### 4. Input boundary handoff
Add a generic `FocusAboveRequested` event/availability flag to `TuiInputView`; do not import orchestration types into the input module.

When handling `SelectUp`, the input requests focus above only if:
- Its owner marked an above-target available.
- The selection is empty.
- The cursor's display point is display row zero.

Use the editor's char-cell render state and `display_lattice(...).offset_to_display_point(...)`, the same projection used by `TuiEditorElement`, rather than counting newlines or duplicating soft-wrap math. If any condition is false, call the existing `select_up` path unchanged. `TuiTerminalSessionView` updates availability from the orchestration snapshot and maps `FocusAboveRequested` to `FocusTabs`.

### 5. Session rendering and owner callbacks
Render the generalized bar from `TuiTerminalSessionView`, matching the architecture that each rendered terminal session asks `TuiOrchestrationModel` for its current tree.

Restructure the normal render tree into:
- A full-width optional orchestration tab row.
- The existing horizontally padded transcript/input/footer column beneath it.

The alt-screen early return remains unchanged and therefore owns the complete pane. Blocking cards remain inside the normal session column, so the tab bar stays available above them.

When the bar is unfocused, render the normal footer with the conditional `Shift + ↑ sub-agents` hint. When focused, keep the input visible but blurred and replace the normal footer with the PRODUCT (18) navigation footer.

Tab and overflow callbacks dispatch typed actions back to the owning `TuiTerminalSessionView`:
- Tab clicks select immediately. The owner preserves tab focus only if it was already active; otherwise it focuses the target session's normal interaction target.
- Overflow clicks carry the page anchor computed by the component to the model's `set_explicit_page` operation and never call a focus API, so input focus remains unchanged.

### 6. Styling and identities
Add semantic orchestration-tab style recipes to `TuiUiBuilder` rather than embedding raw colors in the element. Supply those styles to `TuiTabBar`:
- Magenta-tinted bar background.
- Focused selected tab background and contrasting text.
- Unfocused active-tab emphasis.
- Muted divider, overflow, and non-selected labels.

Reuse `agent_identity_palette` and `assign_agent_identity_indices`. Assign identities from stable spawn order, then project them into dynamic pill order so a status change never changes a child's glyph or color. The generic tab element receives only the resulting optional leading glyph/style.

## Testing and validation
### Integration tests
Extend `orchestration_model_tests.rs`, `sessions_tests.rs`, `terminal_session_view_tests.rs`, and `input/view_tests.rs`:
- Exact reuse of GUI pin/status/error/active/done-recency/spawn ordering after filtering non-navigable sessions — PRODUCT (4, 9-12).
- The same root and tabs from orchestrator and child sessions; newly materialized and removed sessions — PRODUCT (1-5, 47-49).
- Shared per-root page state, explicit-page persistence, active reveal, and first/last-visible keyboard origin — PRODUCT (26-27, 39-42).
- Wrapped-row and selection-aware `Shift+Up` behavior; `Shift+Down` restoration — PRODUCT (13-18).
- Wrapped adjacent navigation, child-only first/last jumps, and tab focus preserved across full-session projection — PRODUCT (19-25).
- Focused and unfocused mouse switches, no-op active clicks, overflow clicks that neither select nor move focus, and blocker precedence — PRODUCT (28-32, 38, 45-46).
- Tab bar above normal blockers and omitted by alt-screen replacement — PRODUCT (50).
- Existing `Ctrl+C` behavior and the absence of kill-agent footer copy remain regression-covered.

### Live verification
Use `tui-testing` render-to-lines coverage for 40-, 80-, and 132-column widths and dark/light themes. Then use `tui-verify-change` with `./script/run-tui` to verify:
- `Shift+Up` from wrapped and selected input states.
- Continuous keyboard switching while child agents reorder dynamically.
- Mouse switching from input focus and tab focus.
- Multiple overflow pages, explicit paging with the active tab off-page, and terminal resize.
- Input drafts and running child state preserved across repeated switches.

Run focused validation first:
- `cargo nextest run -p warp_tui orchestration`
- `cargo nextest run -p warp_tui terminal_session`
- `cargo nextest run -p warp_tui input`

Before PR submission, run `./script/format`, the repository-prescribed Clippy command, and `./script/presubmit`.

## Parallelization
The reusable component lands in the lower `harry/code-1822-tui-tab-bar-component` PR. Do not split this integration PR across child agents: topology snapshots, shared page state, full-session focus switching, input handoff, and render tests cross the same TUI views and should be implemented coherently. Implement sequentially in one checkout: model/session lookup and paging, input/focus integration, then rendering and end-to-end validation. Long-running focused test groups may run concurrently after implementation.

## Risks and mitigations
- **Dynamic order versus explicit paging:** store page anchors by conversation ID, not index, and distinguish automatic reveal from explicit paging. Re-resolve every anchor against the fresh canonical order.
- **Layout/event drift:** the tab bar resolves navigation against the same private settled range and ordered snapshot it rendered, then emits a stable tab ID. The orchestration model resolves that ID against fresh state and no-ops unavailable targets.
- **Focus without projection:** make every tab selection update `TuiSessions` and the target view's focus mode in one owner action; test both root projection and responder focus.
- **Stale child-session cache:** use history's authoritative conversation→surface mapping plus `TuiSessions` lookup for navigation. Keep the existing child map only for launch cleanup.
- **Identity changes during reorder:** assign identities in stable spawn order, independent of canonical display order.
- **Terminal-model deadlock:** tab membership, paging, and focus require no new `TerminalModel::lock()` call; visual-row detection reads the editor's char-cell render state instead.

## Follow-ups
- Add `Ctrl+C` to kill the selected child while the tab bar is focused, including its focused-footer hint and terminal-state behavior, in the next PR above this one.
