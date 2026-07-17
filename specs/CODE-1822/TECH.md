# TECH: TUI Orchestration Permission and Configuration
Linear: [CODE-1822 — Orchestration](https://linear.app/warpdotdev/issue/CODE-1822/orchestration)
Product: [specs/CODE-1822/PRODUCT.md](./PRODUCT.md)
Inspected commit: `27da0f4885aa23603c4feb442c7806b0170cde70`

## Context
### Shared wire types and execution (already frontend-agnostic)
- [`crates/ai/src/agent/action/mod.rs (214-249) @ 27da0f48`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/crates/ai/src/agent/action/mod.rs#L214-L249) — `RunAgentsRequest`, `RunAgentsExecutionMode`, `RunAgentsAgentRunConfig`.
- [`crates/ai/src/agent/orchestration_config.rs @ 27da0f48`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/crates/ai/src/agent/orchestration_config.rs) — `OrchestrationConfig`, `OrchestrationConfigStatus`, `matches_active_config`.
- [`app/src/ai/blocklist/action_model.rs (684-745) @ 27da0f48`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/app/src/ai/blocklist/action_model.rs#L684-L745) — `execute_run_agents` (replaces the queued request with the user-edited one, then executes) and `deny_run_agents` (records a `Denied` result; used by the GUI for "accept without orchestration" and disapproved configs, not for plain rejection).
- [`app/src/ai/blocklist/action_model.rs (1036-1066) @ 27da0f48`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/app/src/ai/blocklist/action_model.rs#L1036-L1066) — `cancel_action_with_id`; the GUI reject path (`RunAgentsCardViewEvent::RejectRequested` → `AIBlock::cancel_action`, [`block.rs:4845-4854`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/app/src/ai/blocklist/block.rs#L4845-L4854), [`block.rs:7102-7106`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/app/src/ai/blocklist/block.rs#L7102-L7106)).
- [`app/src/ai/blocklist/action_model/execute/run_agents.rs @ 27da0f48`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/app/src/ai/blocklist/action_model/execute/run_agents.rs) — `RunAgentsExecutor`: validation, plan publication wait, per-child fan-out via `StartAgentExecutor`, `SpawningStarted`/`SpawningFinished` events. `resolve_request_from_config` consumes the shared `OrchestrationConfigState` from `app/src/ai/orchestration/`.

### Shared orchestration domain and selector (landed earlier in this stack)
The frontend-neutral edit state, option snapshots, and the reusable selector this card consumes landed in the three PRs below this one; see their specs for details:
- [specs/code-1822-edit-state/TECH.md](../code-1822-edit-state/TECH.md) — `OrchestrationConfigState`, `OrchestrationEditState`, `AuthSecretSelection`, transitions, providers, and validation helpers in `app/src/ai/orchestration/`.
- [specs/code-1822-option-snapshots/TECH.md](../code-1822-option-snapshots/TECH.md) — `OptionSnapshot`/`OptionRow`/`OptionSourceStatus`/`OptionFooter` and the per-page snapshot builders, plus the GUI picker adaptation onto them.
- [specs/code-1822-tui-option-selector/TECH.md](../code-1822-tui-option-selector/TECH.md) — the reusable `TuiOptionSelector` list primitive (`crates/warp_tui/src/option_selector.rs`) the card embeds for its configuration pages.

Live catalogs come from `HarnessAvailabilityModel` ([`app/src/ai/harness_availability.rs`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/app/src/ai/harness_availability.rs)), `LLMPreferences`, `CloudAmbientAgentEnvironment`, `ConnectedSelfHostedWorkersModel`, `CloudAgentSettings`, `UserWorkspaces`.

### TUI plumbing
- [`crates/warp_tui/src/agent_block.rs (105-306) @ 27da0f48`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/crates/warp_tui/src/agent_block.rs#L105-L306) — `TuiToolCallView` enum plus `sync_action_views`, the lazy per-action child-view registration seam (currently `FileEdits`, `ShellCommand`).
- [`crates/warp_tui/src/terminal_session_view.rs @ 27da0f48`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/crates/warp_tui/src/terminal_session_view.rs) — renders transcript, inline menu, input box, footer; focuses the input at startup (620) and after restore flows (808, 839, 867).
- [`crates/warp_tui/src/inline_menu.rs @ 27da0f48`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/crates/warp_tui/src/inline_menu.rs) — `TuiInlineMenuHandle`/`TuiInlineMenuSnapshot`; scroll/selection math shared with GUI via `warp_search_core::inline_menu::InlineMenuSelection`.
- [`crates/warp_tui/src/tool_call_labels.rs (503-577) @ 27da0f48`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/crates/warp_tui/src/tool_call_labels.rs#L503-L577) — existing static RunAgents status labels (kept for restored/terminal fallbacks).
- [`crates/warp_tui/src/tui_builder.rs @ 27da0f48`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/crates/warp_tui/src/tui_builder.rs) — `TuiUiBuilder` theme→style recipes; all colors derive from `WarpTheme`, no raw hex.
- [`app/src/tui_export.rs @ 27da0f48`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/app/src/tui_export.rs) — the sole `warp` → `warp_tui` export seam.

There is no TUI permission/confirmation UI for `RunAgents` today and no generalized input-hiding mechanism; the closest precedent is the inline-menu overlay, which keeps the input visible and focused.

## Proposed changes
### 1. TUI orchestration block `crates/warp_tui/src/orchestration_block.rs`
New `TuiToolCallView::OrchestrationBlock(ViewHandle<TuiOrchestrationBlock>)` variant, constructed in `TuiAIBlock::sync_action_views` for `AIAgentActionType::RunAgents` actions (mirroring `ensure_run_agents_card_view`'s active-config lookup via `conversation.orchestration_config_for_plan(&request.plan_id)` at [`block.rs:7069-7083`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/app/src/ai/blocklist/block.rs#L7069-L7083), including `update_request` re-syncs while streaming).

View state: `action_id`, an `OrchestrationEditState` + card fields (`agent_run_configs`, `base_prompt`, `summary`, `skills`, `plan_id`, `original_tool_call_request`), `mode: Acceptance | Configuring { page }`, the active `TuiOptionSelector` handle, model handles (`BlocklistAIActionModel`, `RunAgentsExecutor`), and the identity palette captured at construction.

- Shared card chrome: a persistent yellow-square permission title on a header row tinted with the surface overlay applied twice, over a 10%-magenta body in both modes; the body is inset three cells with one row of vertical padding. Acceptance renders the wrapping colored agent-identity line and one wrapping inline `Label: value` metadata row (bold values, muted bullets); the request summary is not repeated inside the card. Configuration renders `Edit agent configuration`, right-aligned `← n of m →`, a blank row, a bold singular/plural-aware question, and the selector. Each mode's styled key hints render below, outside the tinted surface (acceptance: `Enter to accept  Ctrl + E to edit Ctrl + C to reject`).
- Keybindings registered in `orchestration_block::init` (added to `keybindings.rs`, `tui:`/`TUI_BINDING_GROUP` conventions): acceptance owns `enter`/`numpadenter` → Accept and `ctrl-e` → Configure; configuration owns `esc` → Back, `left` → confirm then PreviousPage, `right` → confirm then NextPage, and `tab` → NextPage without confirmation; `ctrl-c` → Reject applies in either mode. The embedded selector owns configuration-page Enter/Numpad Enter confirmation. Arrow navigation applies the current option selection, recomputes the dynamic page sequence, then moves in the requested direction and clamps at sequence boundaries; Tab preserves the current unconfirmed highlight.
- Page sequencing: `ConfigPage { Location, Harness, ApiKey, Host, Environment, Model }`; `sequence(state)` returns the dynamic page list (Cloud: 5 + API-key page when `should_show_auth_secret_picker`; Local: `[Location, Model]`). Confirmations call the shared transition methods (`state.apply_execution_mode_change`, `session.apply_harness_change`, `state.apply_auth_secret_change`, `set_worker_host` + `persist_host_selection`, `set_environment_id` + `persist_environment_selection`, `model_id` assignment). Enter advances and returns to Acceptance after the final page; arrows navigate in their requested direction after committing. Every interactive Configuration → Acceptance transition reclaims focus on `TuiOrchestrationBlock` before the selector stops rendering, so a hidden search/custom-text editor cannot keep its more-specific editor bindings and shadow acceptance bindings such as `Ctrl+E`.
- Search: only `ConfigPage::Model` opts into `TuiOptionSelector` search. The pinned
  `Search:` editor stays above the model viewport; the list starts on the selected
  model so numeric shortcuts remain immediate. Search is the final item in the
  navigation cycle: Up from the first model focuses Search, Up from Search selects
  the last filtered model, Down from the last model focuses Search, and Down from
  Search selects the first filtered model.
- Accept: guard with `accept_disabled_reason_with_auth`; on `Some(reason)` render the reason inline and stay active (PRODUCT 53); on `None` build the request exactly as `RunAgentsEditState::to_request` does (auth via `state.auth_secret_name()`, preserved `computer_use_enabled`) and call `action_model.execute_run_agents(&action_id, request, ctx)` — the same shared path the GUI uses.
- Reject: emit an event the owning `TuiAIBlock` maps to `cancel_action_with_id(conversation_id, &action_id, CancellationReason::ManuallyCancelled, ctx)`, matching the GUI's `RejectRequested` semantics (`deny_run_agents` remains reserved for disapproved-config denial, which the TUI does not surface).
- Subscriptions: `RunAgentsExecutorEvent` (spawning presentation), `BlocklistAIActionEvent` (blocked/finished transitions), `HarnessAvailabilityEvent` (`Changed`, `AuthSecretsLoaded`, `AuthSecretsFetchFailed`, `AuthSecretDeleted` → `revalidate_after_catalog_change` + refresh the active selector snapshot), `LLMPreferencesEvent` (Oz model catalog), `ConnectedSelfHostedWorkersEvent` (host list). Retry from a `Failed` API-key page calls `HarnessAvailabilityModel::ensure_auth_secrets_fetched` — the same lazy fetch the GUI triggers on picker population.
- Terminal states reuse the pure result-matching copy already in `tool_call_labels.rs` (503-577); restored blocks keep the existing fallback label path.
- The card never locks the terminal model; it renders from its own state and shared singletons.

### 2. Generalized input replacement (derived, no stored flag)
Input visibility is a pure function of the front-of-queue blocker rather than a suppression boolean:
- `TuiAIBlock` gains `active_blocking_child(&self, ctx) -> Option<TuiBlockingChild>` (`{ action_id, view_id }`): the front pending action for the conversation (`BlocklistAIActionModel::get_pending_action`) when its status is `Blocked` and its registered child view reports `wants_focus(ctx)`. `TuiOrchestrationBlock::wants_focus` is true in Acceptance/Configuring and false once accepted, rejected, spawning, or finished — matching PRODUCT (1-8). Deriving from the action queue (not transcript order) keeps semantics identical to the GUI's `focus_subview_if_necessary` ([`block.rs:4913-4954`](https://github.com/warpdotdev/warp/blob/27da0f4885aa23603c4feb442c7806b0170cde70/app/src/ai/blocklist/block.rs#L4913-L4954)).
- `TuiTranscriptView` exposes the same query over its agent blocks; `TuiTerminalSessionView::render` calls it once per pass. When `Some`, the session view omits the input box and normal footer from its element tree and the card renders its own hint footer; when `None`, it renders input + footer as today.
- Focus: on the `None → Some` transition the session view records that the input was focused and focuses the blocker view; on `Some(a) → Some(b)` it focuses `b` directly (no intermediate editable input, PRODUCT 6); on `Some → None` it restores focus to the input (PRODUCT 5). Draft/cursor/selection/scroll are untouched by construction — nothing in this path writes to the input model.
- Re-derivation is driven by the session view's existing `BlocklistAIActionModel` subscription (`ActionBlockedOnUserConfirmation`, `FinishedAction`, queue changes → `ctx.notify()`). No terminal-model locks are added.

### 3. Theming and agent identity
`TuiUiBuilder` gains orchestration recipes, all derived from `WarpTheme` (no raw design hex): `orchestration_surface_background()` (one 10% magenta overlay over the probed base background), `orchestration_header_background()` (the overlay applied twice for the title row), `orchestration_selected_value_style()`, and `agent_identity_palette()`, while selected configuration rows use the shared `option_selector_selected_style()` recipe. The palette crosses the design's seven glyphs (`⊹ ⟡ ✶ ◊ ⊛ * ✠`) with seven themed ANSI roles: normal cyan, blue, and magenta; bright magenta for lilac; and normal red, green, and yellow for the design's pink, green, and yellow swatches. This yields 49 deterministic combinations; assignment is `stable_hash(agent_name) % len`, collision-free ordering within one request via first-come index fallback, cycling beyond exhaustion. The card captures the palette once at construction so identities stay stable across re-renders and edits.

### 4. Export seam
`tui_export.rs` re-exports the neutral surface only: `OrchestrationConfigState`, `OrchestrationEditState`, `AuthSecretSelection`, snapshot types and builders, validation helpers, `RunAgentsExecutor`/`RunAgentsExecutorEvent`/`RunAgentsSpawningSnapshot`, `HarnessAvailabilityModel` + events, `RunAgentsRequest`/`RunAgentsExecutionMode`/`RunAgentsAgentRunConfig`, `OrchestrationConfig`/`OrchestrationConfigStatus`, and the shared orchestration telemetry types. No GUI element types cross the seam.


## Testing and validation
Focused unit coverage:
- `orchestration_block_tests.rs` covers page sequencing, approved-config and auth-secret resolution, request reconstruction, selector-to-edit-state navigation, and decision/focus behavior. Focus regression coverage drives the model-page search editor, confirms a result as a row click does, then verifies that Acceptance owns focus so `Ctrl+E` is no longer shadowed by the hidden editor. The interaction tests inject a local controller, so they exercise the real block, selector, and typed actions without exporting app test infrastructure.
- `option_selector_tests.rs` covers the reusable selector's navigation, confirmation, search, disabled/loading/failure states, custom text, scrolling, and refresh behavior.
- `agent_identity_tests.rs` covers palette size, deterministic assignment, uniqueness, and cycling.
- `keybindings_tests.rs` validates that the orchestration block's bindings remain TUI-owned.

Live verification via `./script/run-tui` (per `tui-verify-change`): accept-without-edit, full Cloud edit loop, Local collapse, model search → click result → `Ctrl+E` reopening configuration from Acceptance, retry on failed secret fetch, narrow-terminal reflow, input draft preservation across a full accept/reject cycle.

Commands: `cargo nextest run -p warp -E 'test(orchestration) + test(run_agents)'`, `cargo nextest run -p warp_tui`, `cargo nextest run -p warpui_core --features tui` (if element changes land there), `./script/format`, `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`, `./script/presubmit` before PR.

## Orchestration
The work ships as a four-PR Graphite stack, each mergeable on its own:
1. `harry/code-1822-edit-state` — the frontend-neutral orchestration domain module (`app/src/ai/orchestration/`: edit state, session, transitions, providers, validation) and the executor retarget; specified in [specs/code-1822-edit-state/TECH.md](../code-1822-edit-state/TECH.md).
2. `harry/code-1822-option-snapshots` — option snapshots and their builders, plus the behavior-preserving GUI picker adaptation onto them; specified in [specs/code-1822-option-snapshots/TECH.md](../code-1822-option-snapshots/TECH.md).
3. `harry/code-1822-tui-option-selector` — the reusable `TuiOptionSelector` primitive; specified in [specs/code-1822-tui-option-selector/TECH.md](../code-1822-tui-option-selector/TECH.md).
4. The final PR (this spec's remaining scope) — the TUI orchestration block, generalized input replacement, theming, and agent identity, reviewed against the PRODUCT invariants.

## Risks and mitigations
- Catalog events arriving mid-configuration can reshape option lists — the selector preserves the selected id when still present; disappearance surfaces the PRODUCT (50) unavailability copy rather than silently reselecting.
- Focus derivation vs. event ordering: `SpawningStarted` must flip `wants_focus` before the next render; both arrive through the same entity-event loop, and the render-time derivation (not cached state) makes late events self-correcting.
- Theme switches would rebuild the identity palette; the card pins its palette at construction so in-flight requests keep stable identities, at the cost of using pre-switch colors until the next request.
