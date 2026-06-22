# Inline Create-API-Key Flow on Orchestration Cards — Technical Notes

Linear: [QUALITY-702](https://linear.app/warpdotdev/issue/QUALITY-702)
Product spec: `specs/QUALITY-702/PRODUCT.md`

## 1. Overview

This change extends the existing orchestration card auth-secret picker so that users with no managed keys for the active harness can create one without leaving the conversation. The cloud-mode create-key view is decoupled from its previous tight binding to cloud-mode state and re-hosted inside a workspace-level blocking modal. Both orchestration card surfaces (the `RunAgents` confirmation card and the plan card's inline orchestration config block) gain a new picker entry and a new action variant that bubble a "create new key" request up to the workspace. The card's auth-secret selection is reshaped from an `Option<String>` + sibling bool into a three-state enum that makes the picker label, the Accept gate, and persistence all match the product spec.

## 2. Key files

### Reused create-key view, now decoupled
- `app/src/terminal/view/ambient_agent/auth_secret_ftux_view.rs` — takes a `harness: Harness` at construction instead of an `AmbientAgentViewModel` handle; exposes `set_harness`; replaces direct model mutations with `AuthSecretFtuxViewEvent::{Created, Cancelled, Skipped, Failed}` events; gains a `with_skip_hidden(bool)` toggle so the workspace modal can suppress the Skip button (Inherit lives on the picker in this context).
- `app/src/terminal/view/ambient_agent/auth_secret_ftux_dropdown.rs` — same shape change: takes a harness directly, exposes `set_harness`, and removes the previous `subscribe_to_model(AmbientAgentViewModel)` dependency.
- `app/src/terminal/view/ambient_agent/mod.rs` — re-exports updated.

### Cloud-mode re-wiring (preserves existing UX)
- `app/src/terminal/input.rs` — constructs the FTUX view/dropdown with the cloud-mode selected harness and subscribes to the new lifecycle events. The `Created/Cancelled/Skipped/Failed` handlers perform the same side effects (persist selected secret, mark FTUX completed, write `last_selected_auth_secret`, etc.) that the view used to perform inline.

### Orchestration card surfaces
- `app/src/ai/blocklist/inline_action/orchestration_controls.rs` — shared picker logic. Introduces `AuthSecretSelection`, threads it through `OrchestrationEditState`, adds the `+ New API key…` menu entry, adds `apply_create_new_auth_secret_requested` and `apply_created_auth_secret_if_matches`, and adds the `create_new_auth_secret_requested` variant to the `OrchestrationControlAction` trait.
- `app/src/ai/blocklist/inline_action/run_agents_card_view.rs` — confirmation card. Implements the new trait variant, wires the workspace modal dispatch, subscribes to `HarnessAvailabilityEvent::AuthSecretCreated`, and owns the one-shot auto-open guard.
- `app/src/ai/document/orchestration_config_block.rs` — plan card inline config block. Same wiring as the confirmation card for the picker, action handler, and `AuthSecretCreated` adoption.

### Workspace modal host
- `app/src/workspace/action.rs` — adds `WorkspaceAction::OpenCreateAuthSecretModal { harness }`.
- `app/src/workspace/view.rs` — owns a `ModalViewState<Modal<AuthSecretFtuxView>>`; opens it in response to the new action; subscribes to the FTUX view's lifecycle events to close the modal and persist the new selection.

### Button affordance plumbing
- `app/src/view_components/compactible_action_button.rs` — adds `set_disabled` and `set_tooltip` so existing single-state buttons can re-derive their state from a parent gate.
- `app/src/view_components/compactible_split_action_button.rs` — delegates `set_disabled`/`set_tooltip` to both the primary and the menu button so the entire split button reflects the gate.

## 3. `AuthSecretSelection` enum

`OrchestrationEditState` previously carried `auth_secret_name: Option<String>` plus an `auth_secret_explicit_inherit: bool` sibling. That two-field encoding had several subtle issues: `None + false` and `None + true` had different meanings, the proto carried only the name, and the picker label / Accept gate / persistence each had to special-case both fields.

The new enum collapses these into a single value:

```
pub enum AuthSecretSelection {
    Unset,                // no choice yet — picker shows "+ New API key…", Accept disabled
    Inherit,              // user explicitly chose to inherit — Accept enabled
    Named(String),        // user picked a managed key by name — Accept enabled
}
```

`AuthSecretSelection::from_optional_name(Option<String>)` maps wire-format payloads (where absent always means "no choice yet") into the enum. `OrchestrationEditState::auth_secret_name()` returns the `Named` payload (or `None` for the other two variants) so dispatch code that only cares about the on-wire field doesn't have to match the full enum.

Only `Named(_)` is persisted via `CloudAgentSettings.last_selected_auth_secret`. `Inherit` and `Unset` are per-session, per-harness UI state.

## 4. Picker contents and label derivation

`populate_auth_secret_picker_for_harness` rebuilds the dropdown's items each time the harness or secrets list changes. The ordering is:

1. "Inherit key from environment" — always present; dispatches `auth_secret_changed(None)` on click.
2. Loaded managed keys (or a single disabled placeholder for Loading/Failed states).
3. A separator and a "+ New API key…" entry, but only for harnesses whose `auth_secret_types_for_harness(...)` is non-empty.

The picker's trigger label is computed directly from `AuthSecretSelection`:

- `Named(name)` → that name.
- `Inherit` → "Inherit key from environment".
- `Unset` with a create-new-capable harness → "+ New API key…".
- `Unset` otherwise → "Inherit key from environment".

The label always uses the dropdown's default text color. A previous iteration tried to override the trigger color to dim the placeholder; that was removed because the override path re-entered the dropdown's view while still inside the dropdown's own dispatched action and tripped warpui's "Circular view update" guard.

## 5. Action trait and handler wiring

`OrchestrationControlAction` (implemented by both `RunAgentsCardViewAction` and `OrchestrationConfigBlockAction`) gains:

```
fn create_new_auth_secret_requested() -> Self;
```

Both implementers add a `CreateNewAuthSecretRequested` variant and handle it identically: call `oc::apply_create_new_auth_secret_requested(...)` to reset the selection to `Unset` and clear the persisted name (so cancelling the modal does not silently leave a stale name selected), parse the active harness, and dispatch `WorkspaceAction::OpenCreateAuthSecretModal { harness }`. The card then refreshes the Accept gate and notifies.

`apply_auth_secret_change` and `apply_create_new_auth_secret_requested` deliberately do not re-enter the picker view (no `populate_*` or `sync_*` calls inside them) — those helpers are invoked from inside the dropdown's own dispatched action, and re-entry would trip the same circular-update guard noted above. The dropdown updates its own displayed label as part of its menu click; the orchestrator's job is just to record state and persist.

## 6. Workspace modal

`WorkspaceAction::OpenCreateAuthSecretModal { harness }` is dispatched only by the two card action handlers. The workspace view owns a `ModalViewState<Modal<AuthSecretFtuxView>>` constructed lazily when the action arrives. The modal is parameterized with the requested harness; `AuthSecretFtuxView::with_skip_hidden(true)` removes the Skip button (the picker's "Inherit key from environment" entry already plays that role outside the modal).

The workspace subscribes to the view's lifecycle events:

- `Created { harness, name }` — persists the new key as the active selection for that harness via `CloudAgentSettings.last_selected_auth_secret`, then closes the modal. The settings write happens before the modal close so the subsequent `HarnessAvailabilityEvent::AuthSecretCreated` event finds the persisted value already in place.
- `Cancelled` / `Skipped` — closes the modal without side effects. The originating card's selection is unchanged (still `Unset`).
- `Failed { error }` — leaves the modal open and renders an inline error via the view itself.

The two card views subscribe to `HarnessAvailabilityEvent::AuthSecretCreated` and call `oc::apply_created_auth_secret_if_matches(...)` so the freshly-created key is adopted as the active selection on the card without waiting for a manual repopulate.

## 7. Auto-open one-shot guard

Each card owns `has_auto_opened_create_modal: bool`. `maybe_auto_open_create_modal` is the single chokepoint that:

1. Returns early if the guard is set.
2. Returns early if the card is not in an interactive confirmation state (denied, auto-launched, spawning, restored from history, action already finished or running async).
3. Returns early if the active harness has no auth-secret picker (e.g. Oz).
4. Returns early if `auth_secret_selection` is not `Unset`.
5. Returns early if the harness's secrets list is anything other than `Loaded(secrets)` with `secrets.is_empty()`. `NotFetched`, `Loading`, and `Failed` are deliberately treated as "not yet decidable" — the `HarnessAvailabilityEvent::AuthSecretsLoaded` subscription re-fires the check once secrets actually arrive.
6. Sets the guard and dispatches `WorkspaceAction::OpenCreateAuthSecretModal { harness }`.

The guard is reset:
- At construction (set to `false`).
- In `update_request` whenever the harness, model, or execution mode changes via streaming.
- In `try_auto_launch_on_stream_complete` (the stream-complete snapshot is the authoritative final state and gets a fresh evaluation).
- In the `ExecutionModeToggled` and `HarnessChanged` action handlers.

`maybe_auto_open_create_modal` is invoked from those same code paths and from the `AuthSecretsLoaded` / `AuthSecretsFetchFailed` subscription handlers.

## 8. Accept gate and tooltip

`oc::accept_disabled_reason_with_auth(&state.orch, ctx)` extends the existing `OrchestrationEditState::accept_disabled_reason` with a new branch: when `auth_secret_selection` is `Unset` for a harness that exposes the picker, it returns a human-readable reason. Both card views call this helper from a small `refresh_accept_button_state` method which sets `disabled` and `tooltip` on the Accept button (a `CompactibleSplitActionButton` on the confirmation card; the plan card uses the same gate to render an inline validation error instead of a disabled button).

`refresh_accept_button_state` is called from every action handler and from every model-subscription handler that touches `state.orch`, including the `AuthSecretCreated`, `AuthSecretsLoaded`, and `AuthSecretsFetchFailed` branches. `set_disabled` / `set_tooltip` on the button are cheap no-ops when the value hasn't changed.

`CompactibleSplitActionButton::set_disabled` / `set_tooltip` delegate to both the primary and the menu trigger so the split button reads as a single gated affordance.

## 9. FTUX view decoupling details

Previously the cloud-mode FTUX view kept an `Rc<dyn AmbientAgentViewModel>` and read the selected harness from it inside `render` / event handlers. Side effects on submit were performed directly against the model (`set_harness_auth_secret_name`, `mark_harness_auth_ftux_completed`, the `last_selected_auth_secret` write, and the cloud-mode-specific `set_harness Oz` post-action).

The decoupling moves the harness into a plain `Harness` field with a `set_harness(harness, ctx)` setter that the parent invokes when the cloud-mode harness selector changes. Side effects are no longer performed inside the view; instead it emits `AuthSecretFtuxViewEvent::{Created{harness, name}, Cancelled, Skipped{harness}, Failed{error}}` and the host decides what to do.

Cloud-mode UX is preserved by `input.rs` subscribing to these events and performing exactly the side effects that used to be inline. The workspace modal subscribes to the same events and performs the modal-specific behavior (close + persist) described above. The same applies to `AuthSecretFtuxDropdown`.

## 10. Cloud-mode parity

Two cloud-mode behaviors are mirrored on the orchestration cards:

- **Default-selection logic.** `resolve_default_auth_secret_for_harness` only promotes a persisted `last_selected_auth_secret` value; it does not fall back to "first loaded secret". This matches both warp-server's webapp (`HarnessAuthSecretSelector` + `use-agent-form-state.ts`) and cloud-mode's `auth_secret_selector.rs::maybe_restore_auth_secret_from_settings`. Without an explicit choice, the picker stays on `+ New API key…` (or Inherit on harnesses with no managed types).
- **Persistence shape.** Selecting a managed key on either card writes to the same `CloudAgentSettings.last_selected_auth_secret` map keyed by `harness.config_name()` that cloud mode reads on its next launch. Selecting Inherit clears that key; switching to `Unset` (via `+ New API key…`) also clears it so cancelling the modal does not leave a stale name persisted.

## 11. Validation

### Automated

- `cargo check -p warp`
- `cargo fmt`
- `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`

### Manual

Covered in `PRODUCT.md §7`.

## 12. Follow-ups

- Consider extracting the workspace-owned modal into a small reusable host (it currently lives inline on `Workspace`); a second consumer would justify the abstraction.
- Consider a small visual treatment for the picker's `+ New API key…` entry (e.g. a leading plus icon) once the rest of the orchestration picker visuals are finalized.
- Long-term, the cloud-mode FTUX view's "Skipped" path could be removed entirely now that the workspace modal hides Skip and the orchestration picker exposes Inherit directly.
