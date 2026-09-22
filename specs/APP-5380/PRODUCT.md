# Shared custom inference endpoints
## Summary
Migrate Warp's existing GUI custom-inference endpoint capability to a shared, file-backed definition format that both the GUI and Warp Agent CLI understand. Endpoint credentials remain local secure data, while the existing GUI editor continues to provide native definition management and the TUI exposes credential and connection status only.
## Goals
- Preserve the complete existing generic custom-endpoint capability and all existing GUI configurations.
- Make the same endpoint-definition format available to GUI and TUI settings.
- Keep API keys out of settings files and cloud synchronization.
- Avoid provider-specific behavior, including special treatment for OpenRouter.
## Non-goals
- Native endpoint-definition editing in the TUI.
- Endpoint connectivity tests, model discovery, or arbitrary HTTP headers.
- Sharing credentials between GUI and TUI secure-storage namespaces.
- Making local custom endpoints available to cloud agents.
## Figma
Figma: none provided. The GUI retains its existing custom-endpoint editor, while the TUI extends existing `/api-keys` and zero-state patterns.
## Behavior
1. Custom endpoint definitions use one settings format supported by both the GUI and TUI. A definition contains a stable identity, display name, base URL, supported request/response schema, and one or more models with stable model identities.

2. API keys never appear in the settings value, generated settings schema examples, settings synchronization payloads, logs, telemetry, or error messages.

3. The GUI retains its existing ability to add, edit, and remove generic custom endpoints, including all currently supported schemas, model names, and aliases. These operations update the shared setting instead of creating a new provider-specific object.

4. Existing GUI custom endpoint definitions and API keys migrate automatically. Existing model identities remain unchanged so saved model selections and execution profiles continue to resolve to the same models.

5. Migration is idempotent. Restarting Warp, retrying after an interrupted write, or receiving settings synchronization events cannot duplicate endpoints, replace an explicit settings collection, or lose an existing API key.

6. An explicitly configured empty endpoint collection is authoritative and remains empty; Warp does not re-import legacy endpoints over it.

7. The GUI and TUI use their existing settings-mode behavior. They understand the same setting and domain model, but their separate settings files and secure-storage namespaces do not implicitly copy definitions or credentials between the two applications.

8. TUI users define or edit endpoints through `settings.toml`, directly or through `/modify-settings`. The TUI does not provide an interactive form for endpoint names, URLs, schemas, or models.

9. `/api-keys` lists custom endpoints using their user-provided display names. No row is hardcoded for OpenRouter or any other compatible gateway.

10. A valid endpoint without a local API key appears as not connected. Selecting it opens the existing masked credential-entry flow.

11. A valid endpoint with a local API key appears as connected. Selecting it replaces the key, and the existing clear action removes the key without removing the endpoint definition.

12. Connection status means only that a non-empty local credential exists. Warp does not claim that the URL is reachable, the credential is accepted, or any configured model exists upstream.

13. Endpoint definitions blocked by plan entitlement or workspace policy remain visible for status and credential cleanup but are unavailable for model selection and requests.

14. When no definitions exist, `/api-keys` does not show a custom-endpoint row. When the endpoint setting is invalid, it shows a non-selectable error state instead of individual endpoint rows.

15. The TUI zero state includes a compact Custom endpoints section only when at least one endpoint is configured or the explicit endpoint setting is invalid. It reports needs keys, connected, unavailable, or configuration-error status and directs users to `/api-keys` or the invalid setting as appropriate.

16. Only definitions that are valid, locally keyed, entitled, and permitted by workspace policy contribute models to model pickers or custom-provider request data.

17. Invalid endpoint settings fail closed. No custom endpoint model remains selectable and no endpoint URL or secret is sent while the setting contains a syntax or validation error.

18. Fixing an invalid setting restores eligible endpoints without requiring an application restart.

19. Removing or renaming an endpoint definition immediately prevents its models and credential from being sent. Orphaned secure credentials may remain stored during the migration and rollback period but are never resolved or transmitted without a matching definition.

20. Custom endpoints remain generic. OpenAI Chat Completions-compatible services such as OpenRouter work by supplying an ordinary endpoint definition; Warp does not infer providers from names or URLs.
