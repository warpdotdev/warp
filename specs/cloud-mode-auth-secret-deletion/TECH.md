# Auth Secret Deletion From Selector — Tech Spec

## Context

This spec implements the behavior in `PRODUCT.md` for the ambient cloud-mode auth selector chip.

Relevant current code:

- `app/src/terminal/view/ambient_agent/auth_secret_selector.rs:61` defines `AuthSecretSelectorAction`; it currently supports toggling the menu, selecting a secret, clearing/inheriting, opening the "New" sidecar, and selecting a new-secret type.
- `app/src/terminal/view/ambient_agent/auth_secret_selector.rs:147` subscribes the selector to `HarnessAvailabilityModel` auth-secret load/create/fetch-failure events and refreshes the menu/button.
- `app/src/terminal/view/ambient_agent/auth_secret_selector.rs:389` builds the menu. Loaded secrets are rendered as `MenuItem::Item` rows with `SelectSecret(name)` actions; the "New" row is separate and opens the sidecar.
- `app/src/terminal/view/ambient_agent/auth_secret_selector.rs:498` handles selector actions and persists selected/cleared secrets in `CloudAgentSettings.last_selected_auth_secret`.
- `app/src/ai/harness_availability.rs:55` stores per-harness auth secret fetch state as `AuthSecretFetchState::Loaded(Vec<AuthSecretEntry>)`.
- `app/src/ai/harness_availability.rs:184` fetches harness auth secrets from `ManagedSecretsClient::list_harness_auth_secrets`.
- `app/src/ai/harness_availability.rs:250` already centralizes secret creation through `ManagedSecretManager`, updates the local cache, and emits create/failure events.
- `crates/managed_secrets/src/manager.rs:87` exposes `ManagedSecretManager::delete_secret(owner, name)`.
- `app/src/server/server_api/managed_secrets.rs:132` wires `delete_managed_secret` to the server GraphQL mutation.
- `app/src/menu.rs:394` defines reusable `MenuItemFields`. It already has right-side label/icon rendering, but right-side icons are decorative and do not dispatch a separate action from the row.

The current `AuthSecretEntry` contains only `name`, but server `ManagedSecret` results include `owner`. Deleting team-owned secrets correctly requires preserving enough owner information to call `delete_secret` with `SecretOwner::Team { team_uid }` instead of always assuming `CurrentUser`.

## Proposed Changes

1. Extend auth-secret cache entries with owner metadata.

   Update `AuthSecretEntry` in `app/src/ai/harness_availability.rs` to include `owner: SecretOwner`. Convert `warp_graphql::object::Space` from fetched/created `ManagedSecret` values into `SecretOwner`:

   - `SpaceType::User` -> `SecretOwner::CurrentUser`
   - `SpaceType::Team` -> `SecretOwner::Team { team_uid: space.uid.into_inner() }`

   This keeps deletion ownership close to the existing fetch/create cache and avoids guessing owner at click time.

2. Add deletion to `HarnessAvailabilityModel`.

   Add a method similar to `create_auth_secret`:

   ```rust
   pub fn delete_auth_secret(
       &mut self,
       harness: Harness,
       name: String,
       owner: SecretOwner,
       ctx: &mut ModelContext<Self>,
   )
   ```

   The method should call `ManagedSecretManager::delete_secret(owner, name.clone())`. On success, remove the matching entry from `AuthSecretFetchState::Loaded` for that harness and emit a new event. On failure, leave the cache unchanged, report the error, and emit a failure event with the harness/name/error.

   Add events:

   - `AuthSecretDeleted { harness, name }`
   - `AuthSecretDeletionFailed { harness, name, error }`

   Existing subscribers that only care about menu freshness should refresh on `AuthSecretDeleted`; subscribers that do not care should explicitly ignore both new events so match exhaustiveness remains clear.

3. Add a selector action for deleting a secret.

   Extend `AuthSecretSelectorAction` with enough data to delete the row, for example:

   ```rust
   DeleteSecret {
       name: String,
       owner: SecretOwner,
   }
   ```

   Track pending deletes in `AuthSecretSelector` with a small set keyed by `(harness, name, owner)` or by a stable local row id if one is introduced. This satisfies PRODUCT behavior 7 and avoids duplicate delete requests from the same menu.

4. Render the right-aligned `X` affordance using the menu system's existing styling.

   Preferred implementation: extend `MenuItemFields` with an optional right-side action, keeping the visual rendering path for `with_right_side_icon(Icon::X)` consistent with other menus while allowing the right icon to dispatch a different action than the row.

   The right-side action should:

   - Stop mouse event propagation so clicking `X` does not also dispatch `SelectSecret`.
   - Close the menu or disable the row while the delete request is pending.
   - Preserve the existing row action when the user clicks anywhere outside the `X`.

   If changing `MenuItemFields` is too broad during implementation, an acceptable alternative is a custom-label row in `auth_secret_selector.rs` that composes the existing text/icon primitives and explicitly separates row-click from `X` click. Prefer the reusable menu extension only if it stays small and does not alter existing right-side icon behavior.

5. Wire deletion action handling in `AuthSecretSelector`.

   In the `DeleteSecret` branch:

   - Read the active harness from `AmbientAgentViewModel`.
   - Mark the row pending.
   - Call `HarnessAvailabilityModel::delete_auth_secret(harness, name, owner, ctx)`.
   - Close the menu or refresh it into a disabled pending state.

   In the selector's `HarnessAvailabilityModel` subscription:

   - On `AuthSecretDeleted` for the active harness, clear pending state for the matching secret, refresh menu/button, and show a success toast.
   - If the deleted name equals `AmbientAgentViewModel::selected_harness_auth_secret_name()`, call `set_harness_auth_secret_name(None, ctx)`.
   - Remove `CloudAgentSettings.last_selected_auth_secret[harness.config_name()]` if it equals the deleted name.
   - On `AuthSecretDeletionFailed`, clear pending state, refresh menu, and show a failure toast containing the user-facing error when available.

   Use `DismissibleToast::success(...)` and `DismissibleToast::error(...)` via `ToastStack::add_ephemeral_toast`, matching nearby toast patterns.

6. Keep adjacent auth-secret surfaces consistent.

   `AuthSecretFtuxDropdown`, orchestration pickers, and model selector subscribers must compile with the new deletion events. They can ignore failure events and refresh on success where they render auth secret lists. This ensures a secret deleted from the chip no longer appears in other already-mounted selectors after the model emits.

## Testing and Validation

1. Unit-test or view-model-test the cache update path for PRODUCT behaviors 6, 8, and 9:
   - Successful deletion removes only the matching auth secret from the loaded list and emits `AuthSecretDeleted`.
   - Failed deletion leaves the list unchanged and emits `AuthSecretDeletionFailed`.
   - Team-owned entries call delete with `SecretOwner::Team`, user-owned entries call delete with `SecretOwner::CurrentUser`.

2. Test `AuthSecretSelector` action behavior for PRODUCT behaviors 4, 5, 7, and 8:
   - Clicking/selecting the row still selects the secret.
   - Dispatching the delete action does not select the secret.
   - Duplicate delete dispatches for a pending secret are ignored.
   - Deleting the currently selected secret clears the selected model value and persisted setting.

3. Add or update menu rendering tests if the reusable `MenuItemFields` right-side action API is introduced:
   - Existing right-side icon rendering remains decorative when no right-side action is set.
   - A right-side action dispatches independently from the row action.

4. Manual validation against the screenshot state:
   - Open the auth selector chip menu with multiple secrets.
   - Confirm only existing secret rows have a right-aligned `X`.
   - Confirm the "New" row and sidecar rows do not show an `X`.
   - Delete an unselected secret and verify it disappears plus a success toast appears.
   - Delete the selected secret and verify the chip label falls back to no-secret/inherit state.
   - Force a delete failure/offline state and verify the failure toast appears and the row remains.

5. Run focused checks:

   ```sh
   cargo check -p warp_app
   cargo test -p warp_app auth_secret
   ```

   Adjust package/test targets to the repository's current presubmit conventions if those commands are too broad or renamed.

## Parallelization

Parallel agents are not recommended. The work is small and tightly coupled across one UI view, one shared model, and a small optional menu extension; splitting would increase merge risk around action/event names without materially reducing wall-clock time.

## Risks and Mitigations

- **Accidental row selection when clicking `X`:** stop event propagation on the `X` hit target and test the separate action path.
- **Deleting the wrong owner scope:** store `SecretOwner` on each fetched `AuthSecretEntry`; do not infer owner from the active workspace at click time.
- **Stale selected secret after deletion:** clear both `AmbientAgentViewModel` selection and `CloudAgentSettings.last_selected_auth_secret` when the deleted name matches the active harness selection.
- **Over-broad menu API changes:** keep any `MenuItemFields` addition opt-in so existing menus with right-side icons remain visually and behaviorally unchanged.

