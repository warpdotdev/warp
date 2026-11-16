# REMOTE-1591: Environment creation flow for local-to-cloud handoff

## Summary

When a user enters `&` handoff-compose mode but has no cloud environments, they should be able to create one inline via a modal overlaying the main window. After creating the environment, the handoff auto-submits with the new environment. Dismissing the modal without creating leaves the input unchanged.

Figma: none provided

## Behavior

### Entry into `&` compose mode with no environments

1. Typing `&` in agent view activates handoff-compose mode regardless of whether the user has any environments. The `&` prefix indicator, message bar hints, and locked-AI input behavior all apply the same as when environments exist.

2. The ghost text / placeholder in the input reads **"Handoff to cloud"** (the generic fallback), since there is no environment name to display in the "Hand off to \<env\>" pattern.

### Opening the creation modal

3. When the user presses Enter while in `&` handoff-compose mode and no environments exist:
   - If the input buffer is non-empty, the prompt text is preserved (held in `&` compose state).
   - A modal dialog overlays the main window containing the `UpdateEnvironmentForm` in Create mode.
   - The modal uses the same form as Settings → Environments → Create (name, description, GitHub repos, Docker image, setup commands), rendered without the settings-page header (back button / page title) and instead with a modal-style close button and title.

4. If the input buffer is empty and the user presses Enter with no environments, nothing happens.

### Modal behavior

5. The modal renders as a centered overlay with a dark background overlay, a close button, and an Esc keybinding hint — the same pattern as other modals in the app (e.g. tab config modal, `EnvironmentSetupModeSelector`). Clicking outside the dialog or pressing Escape closes it.

6. The form inside the modal is the existing `UpdateEnvironmentForm` in Create mode, with `show_header = false` and `should_handle_escape_from_editor = true`. The submit button renders at the bottom-right of the form body (the existing non-header layout).

7. Focus moves into the form's name field when the modal opens. Tab order cycles through form fields as it does in the settings page.

8. Pressing Escape anywhere in the form, or clicking outside the dialog, closes the modal. This is the "dismiss without creating" path — see (13).

### Successful environment creation

9. When the user fills out the form and submits (Create button or Enter on the focused submit button), the modal closes immediately.

10. The environment is created on the server via an online-only API call (`create_ambient_agent_environment_online`). The modal waits for the server to confirm creation and return a `ServerId` before proceeding — this avoids race conditions with unsynced client IDs.

11. Once the server confirms, the handoff auto-submits: the prompt and any pending attachments are sent to `WorkspaceAction::OpenLocalToCloudHandoffPane` with the new environment's `ServerId`, following the same path as a normal `& query` submit.

### Failure during server creation

12. If the server call fails (network error, validation failure, etc.), the error is logged and an error toast is shown ("Failed to create environment: \<error\>"). The `&` compose input is preserved so the user can retry.

### Dismissal without creating

13. If the user closes the modal without creating an environment (Escape, clicking outside, or the close button), the input returns to `&` handoff-compose mode unchanged — the prompt text, attachments, and chip state are all preserved exactly as they were before the modal opened.

14. The user can re-trigger the modal by pressing Enter again, or exit `&` mode entirely via Backspace-on-empty / Escape as usual.

### Interaction with existing environment flows

15. If the user already has one or more environments, Enter in `&` compose mode submits the handoff as it does today — the modal never opens when environments exist.

16. If the user creates their first environment through Settings → Environments while `&` compose mode is active, the environment chip should update reactively (via the existing `CloudModel` subscription) to show the new environment name. At that point, Enter submits the handoff normally without opening the modal.

17. The modal does not interfere with the "share with team" checkbox — it renders the same as in the settings page create flow. If the user is on a team, the option appears.

### Edge cases

18. If the user is not logged in or not authenticated, the form's GitHub auth flow (repo selection) works the same as in settings — the OAuth redirect opens in the browser and the form refreshes on app focus.
