# Auth Secret Creation Flow — Product Spec

## Summary

When a user selects a non-Oz harness (e.g. Claude Code) in cloud mode, they must provide an authentication secret (API key) before the agent can run. This feature adds a first-time setup (FTUX) flow that guides users through selecting or creating an auth secret, and a compact selector chip for returning users who have already configured one.

## Problem

Today the Warp desktop client has no way to associate an auth secret with a third-party harness in cloud mode. The oz web app supports this, but the native client silently launches runs without credentials, causing failures. Users need a clear, guided way to provide auth credentials the first time they use a non-Oz harness, and an efficient way to change credentials afterward.

## Goals

- Guide first-time users through selecting or creating an auth secret when they pick a non-Oz harness.
- Let returning users quickly see and change their selected auth secret via a compact chip.
- Support all three Claude Code auth secret types: Anthropic API Key, Bedrock API Key, and Bedrock Access Key.
- Persist FTUX completion per-harness so users are not re-prompted unnecessarily.
- Include the selected auth secret in the agent spawn config so the server can inject it.

## Non-goals

- Editing or deleting existing secrets (users go to Warp Drive for that).
- Auth secret flows for harnesses other than Claude Code (will extend the same pattern later).
- A dedicated secrets management UI beyond the existing Warp Drive secrets pane.
- Changes to the warp-server — all necessary GraphQL queries and mutations already exist.

## Figma

- FTUX flow: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=8466-53234&m=dev
- Dropdown open: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7239-48787&m=dev
- Creation flow (raw value input): https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7239-48345&m=dev
- Toast after creation: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7239-49211&m=dev

**Not in Figma** (filled in during design):
- The "New" dropdown item with plus icon and submenu for each secret type.
- The auth secret selector chip in the top row (non-FTUX mode).
- Multiple input fields for secret types requiring more than one env var (e.g. Bedrock Access Key needs 4 fields).

## User Experience

### Trigger: selecting a non-Oz harness

1. The user opens cloud mode composing and selects a non-Oz harness (e.g. "Claude Code") from the harness selector.
2. The system checks a per-harness setting to determine whether this harness has had its auth secret FTUX completed.

### FTUX flow (first time)

3. If FTUX has not been completed for the chosen harness, the input editor area is replaced by the FTUX view. The top row of selector chips (host, harness, MCP config) remains visible above.
4. The FTUX view displays:
   - Description text explaining what is needed (e.g. "Please enter your Claude API key to use it as a remote agent.").
   - A dropdown input labeled with the primary env var name (e.g. `ANTHROPIC_API_KEY`). The dropdown has a search icon on the left and a key icon on the right.
   - A bottom row with: "Already logged in? Click here to skip and continue" on the left; Cancel and Continue buttons on the right.
5. Clicking the dropdown opens a menu (720px wide) populated with:
   - Existing auth secrets for the selected harness (fetched from the server).
   - A "New" item at the bottom with a `+` icon. Hovering or selecting it opens a submenu listing each auth secret type (e.g. "New Anthropic API Key", "New Bedrock API Key", "New Bedrock Access Key").
6. **Selecting an existing secret**: updates the dropdown selection. The user hits Continue to proceed.
7. **Selecting a "New" secret type**: enters the creation sub-flow.

### Creation sub-flow

8. When a "New" secret type is selected, input fields appear below the dropdown — one per required env var for that type:
   - **Anthropic API Key**: 1 field (`ANTHROPIC_API_KEY`).
   - **Bedrock API Key**: 2 fields (`AWS_BEARER_TOKEN_BEDROCK`, `AWS_REGION`).
   - **Bedrock Access Key**: 4 fields (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN` (optional), `AWS_REGION`).
9. Each field has a label above it showing the env var name in gray text (10px, `#6f6e6d`).
10. If the user switches the "New" type selection in the dropdown, the input fields re-render to match the newly selected type.
11. If the user types a raw value into the dropdown search and no existing secrets match, helper text appears: "No secrets found. Save to use this value directly or click the key to add a secret."
12. Hitting Continue with filled fields creates the secret on the server, shows a success toast ("API key saved. Manage secrets"), and transitions to the normal composing input with the newly created secret selected.

### Skip and Cancel

13. "Click here to skip and continue" skips auth secret selection and proceeds to the normal composing input with no auth secret set.
14. Cancel dismisses the FTUX and returns to Oz harness selection.
15. After any successful selection or creation, the per-harness FTUX setting is marked as completed.

### Returning user flow (FTUX already completed)

16. If the FTUX setting is already completed for the harness, the normal composing input is shown with an **auth secret selector chip** in the top row, next to the harness selector.
17. The chip shows: key icon + selected secret name + chevron-down. It uses the same `NakedHeaderButtonTheme` as the host and harness selectors.
18. Clicking the chip opens a dropdown with the same items as the FTUX dropdown (existing secrets + "New" submenu), but at a narrower width.
19. Selecting "New {type}" from the chip reopens the FTUX creation flow, but **without** the "Click here to skip" link (since the user has already completed FTUX before).

### Toast notification

20. After successfully creating a new secret, an ephemeral toast appears: "API key saved." with a "Manage secrets" button that opens the Warp Drive secrets pane.

### Spawn integration

21. When the user submits their prompt, the selected auth secret name is included in the `AgentConfigSnapshot.harness_auth_secrets` field sent to the server.

## Success Criteria

- Selecting Claude Code in cloud mode when FTUX has not been completed shows the FTUX view with description, dropdown, and Cancel/Continue buttons. The top row (host/harness selectors) remains visible.
- The FTUX dropdown is populated with existing auth secrets fetched from `harnessAuthSecrets` for the selected harness.
- The "New" item in the dropdown opens a submenu with all three Claude Code secret types.
- Selecting "New Anthropic API Key" shows 1 input field; "New Bedrock API Key" shows 2 fields; "New Bedrock Access Key" shows 4 fields (one optional).
- Filling in the fields and hitting Continue creates the secret via `createManagedSecret`, shows the success toast, and returns to the normal composing input with the secret selected.
- After completing FTUX, re-selecting Claude Code shows the normal input with the auth secret chip in the top row pre-populated with the previously selected secret.
- The auth secret chip dropdown lets the user change their selection or create a new secret.
- Creating a new secret from the chip opens the FTUX creation flow without the "Click here to skip" link.
- The selected auth secret name is included in `harness_auth_secrets.claude_auth_secret_name` when the agent is spawned.
- Cancelling the FTUX returns to Oz harness selection. Skipping proceeds with no auth secret.
- The FTUX setting persists across app restarts.

## Validation

- **Manual**: select Claude Code, verify FTUX appears, select an existing secret, hit Continue, verify normal input returns. Repeat with creating a new secret.
- **Manual**: after completing FTUX, reopen cloud mode with Claude Code, verify the chip appears with the previously selected secret.
- **Manual**: from the chip, select "New Anthropic API Key", fill in the field, hit Continue, verify toast and that the chip updates.
- **Manual**: verify Cancel returns to Oz, Skip proceeds with no secret.
- **Compile check**: `cargo check` passes with the V2 feature flag on and off.
- **Unit tests**: FTUX setting read/write helpers round-trip correctly. Auth secret type metadata returns the correct fields for each type.

## Resolved Decisions

- **Skip behavior**: "Skip" marks the per-harness FTUX as completed, just like a successful selection or creation. The user is not re-prompted on subsequent harness reselections.
- **Empty / failure state**: The dropdown always shows the "New" item with the sidecar of secret types. When the fetch fails (network error or `Failed` state), the existing-secrets section shows a single disabled "Unable to load secrets" row but the "New" item remains so the user can still create. When secret creation fails, an error toast is shown via `ToastStack`.
- **Initial selection**: The FTUX always starts with no secret selected. The user must explicitly pick an existing secret or create a new one (or use Skip / Cancel).
