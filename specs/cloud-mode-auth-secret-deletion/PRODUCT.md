# Auth Secret Deletion From Selector — Product Spec

## Summary

Users can delete Warp-managed harness auth secrets directly from the auth secret selector chip menu. The delete affordance appears as a right-aligned `X` button on each existing secret row, removes the secret from the server, updates the menu state, and confirms success or failure with a toast.

## Figma

Figma: none provided. The prompt includes a screenshot of the current auth selector menu and requests adding a right-aligned `X` button to each non-New submenu item.

## Behavior

1. The feature applies to the auth secret selector chip menu shown for non-Oz cloud harnesses after auth-secret FTUX has been completed.

2. When the auth secret selector chip menu opens, each existing Warp-managed secret row shows:
   - The secret name on the left.
   - A right-aligned `X` button on the same row.
   - Hover, selected, spacing, icon, and text styling consistent with existing Warp menus.

3. The `X` button appears only on existing managed-secret rows. It does not appear on:
   - The header row.
   - The "Inherit key from environment" row.
   - Loading or error placeholder rows.
   - The "New" row.
   - Secret-type rows in the "New" sidecar submenu.

4. Selecting the secret row outside the `X` button preserves current behavior: the row becomes the selected auth secret for the active harness, the per-harness selection is persisted, and the menu closes.

5. Selecting the `X` button deletes that specific secret. It must not select the secret first, must not open the "New" sidecar, and must not accidentally dispatch the row's normal select action.

6. Deletion is server-backed. A secret is considered deleted only after the server delete request succeeds. The UI must not present deletion as successful before the server confirms it.

7. While deletion is pending, the user should not be able to fire duplicate delete requests for the same secret from the same open menu. Acceptable treatments include disabling the `X` button for that row, closing the menu, or otherwise preventing a second click.

8. On successful deletion:
   - The deleted secret disappears from the selector menu the next time the menu is shown, and ideally immediately if the menu remains open.
   - A success toast is shown.
   - If the deleted secret was currently selected for the active harness, the selected auth secret is cleared and the chip falls back to the no-secret/inherit label.
   - Any persisted last-selected secret for the active harness is cleared if it points to the deleted secret.

9. On failed deletion:
   - The secret remains available in the selector menu.
   - The current selected secret does not change.
   - A failure toast is shown.
   - The user can retry deletion after the failure.

10. If the deleted secret is selected by another visible selector instance, that selector should refresh when it observes updated harness secret state. It must not keep showing a deleted secret as a valid selectable item after the secret list is refreshed.

11. If the secret list is stale and the server reports that the secret no longer exists, the delete attempt is treated as a failure unless the server explicitly reports the delete as successful. The failure toast should use the server/user-facing error text when available.

12. If the user is offline or unauthenticated when pressing `X`, the delete fails and shows a failure toast. The menu must not remove the secret locally unless a later server response confirms deletion.

13. The success and failure toasts are ephemeral, consistent with nearby app toasts, and do not require a custom action button.

14. Keyboard behavior must remain usable:
   - Existing keyboard navigation for selecting a secret row remains unchanged.
   - Adding the `X` button must not make the menu trap focus or prevent Escape/close behavior.
   - If the `X` button is reachable by keyboard, it must have an accessible label such as "Delete API key".

15. The menu layout remains stable for long secret names. Long names should truncate or shrink according to existing menu behavior rather than overlapping the right-aligned `X` button.

16. The feature does not add editing, renaming, creating, or bulk-deleting secrets. It only deletes one existing secret at a time from the auth selector menu.

