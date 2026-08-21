# APP-5380: TUI custom LLM endpoints

## Summary

Warp Agent CLI users can define custom LLM endpoints in the TUI `settings.toml` file and manage only the endpoint API keys in `/api-keys`. Endpoint definitions and credentials remain local to the TUI. A configured endpoint becomes available in the model picker only after it has a valid definition, an API key, the required entitlement, and permission from the active workspace.

## Goals

- Let users and `/modify-settings` author custom endpoint definitions without a multi-field TUI form.
- Keep every custom endpoint API key out of `settings.toml`.
- Reuse the existing `/api-keys` set, edit, and clear workflow.
- Make valid keyed endpoint models available to local Warp Agent CLI requests.
- Fail closed for invalid definitions without disabling unrelated settings or valid endpoints.

## Non-goals

- Sharing custom endpoint settings or credentials between the GUI and the TUI.
- Importing GUI custom endpoint definitions or credentials into the TUI.
- Changing the GUI custom endpoint storage format in v1.
- Managing endpoint definitions from `/api-keys`.
- Team-managed or enterprise custom endpoints.
- Using TUI custom endpoints for Warp cloud agents. TUI custom endpoints depend on local configuration and credentials.

## Follow-up

- **TODO (not part of v1):** Evaluate and likely migrate GUI custom endpoints to the same file-backed definition plus split secure-key format. This is the intended direction, not a committed scope item.
  - Follow the execution-profile precedent: use one shared model and a surface-selected persistence source, import legacy GUI data once, and retain a rollback path while migration is gated.
  - The follow-up must define how to handle duplicate legacy endpoint names, preserve or rewrite existing model selections when UUID `config_key` values become deterministic identities, split legacy definitions from keys, and expose the setting on the GUI surface.
  - GUI and TUI settings and secure-storage namespaces remain isolated after migration.

## Behavior

### Define endpoints

1. The TUI settings schema exposes `cloud_platform.custom_endpoints` only on the TUI surface.

2. `cloud_platform.custom_endpoints` is a map. Each map key is the endpoint's required, user-visible, unique name.
   - The name is case-sensitive.
   - The name must not be empty.
   - The name must not start or end with whitespace.
   - TOML map semantics reject duplicate names.
   - A quoted TOML key permits names that contain spaces.

3. Each endpoint contains:
   - `url`: a required HTTPS URL.
   - `schema`: one of `openai_chat_completions`, `openai_responses`, or `anthropic_messages`. The default is `openai_chat_completions`.
   - `models`: one or more model records.

4. Each model record contains:
   - `name`: the required model slug sent to the endpoint.
   - `alias`: an optional model-picker label. A missing alias or `alias = ""` falls back to `name`. A non-empty alias must not start or end with whitespace.

5. Model names must be unique within one endpoint. The same model name can exist under different endpoints.

6. The settings schema does not expose `api_key`, `config_key`, `id`, or another secret or identity field. If a hand-authored endpoint or model includes an unknown field, including `api_key` or `config_key`, Warp marks that endpoint invalid and does not load it.

7. A user or `/modify-settings` can author this complete endpoint:

```toml
[cloud_platform.custom_endpoints."Acme Gateway"]
url = "https://llm.acme.example/v1"
schema = "openai_chat_completions"
models = [
  { name = "gpt-4o", alias = "Acme GPT-4o" },
  { name = "o3-mini" },
]
```

8. Saving a valid definition hot-reloads the endpoint. The user does not restart the TUI.

### Validate definitions

9. Warp validates each endpoint independently.
   - The endpoint name follows Behavior 2.
   - The URL parses as an absolute URL, uses `https`, and contains a host.
   - The URL host is not `localhost`.
   - A literal IP host is not loopback, unspecified, private, link-local, IPv6 unique-local, or an IPv4-mapped restricted address.
   - The schema is a supported value.
   - The models list is not empty.
   - Every model name is non-empty after trimming and has no leading or trailing whitespace.
   - Model names are unique within the endpoint.

10. One invalid endpoint does not disable valid endpoints or unrelated settings.
    - Warp skips the invalid endpoint.
    - The TUI shows the existing transient `Settings failed to load: invalid values.` error.
    - `/api-keys` includes a non-selectable `Invalid custom endpoint: <name>` row with a `(Skipped)` state and directs the user to the matching `cloud_platform.custom_endpoints."<name>"` entry.
    - Warp does not send the invalid endpoint or show its models in the model picker.
    - Warp preserves an existing API key for that name while the definition is invalid. Fixing the definition reconnects the preserved key.

11. A TOML syntax error in a live TUI keeps the last-known-good settings state and uses the existing invalid-syntax TUI error. A startup syntax error loads no endpoint definitions and preserves all stored endpoint keys for recovery. A typed validation error keeps the valid endpoint subset from the newly parsed map.

### Manage keys in `/api-keys`

12. `/api-keys` keeps the current built-in provider rows and Warp credit fallback row.

13. When custom endpoints are allowed and at least one valid definition exists, `/api-keys` inserts valid custom endpoint rows after the built-in provider rows and before Warp credit fallback.
    - Rows sort by endpoint name.
    - The row title is the endpoint name.
    - A muted italic `custom endpoint` annotation appears beside the title.
    - The existing `(Connected)` state appears when a non-empty key exists. An endpoint without a key has no connection-state suffix.
    - Search matches the endpoint name and the `custom endpoint` annotation.

14. Selecting a custom endpoint row opens the same masked credential editor used by pasted provider keys.
    - An existing key is prefilled for replacement.
    - Enter saves the input.
    - Escape cancels and returns to the provider list without changing the key.
    - A successful save returns to the provider list and updates the state to `(Connected)`.
    - A successful save notifies other running TUI processes to reload TUI keys.
    - If cross-process notification fails after secure persistence succeeds, the current TUI keeps the saved key and shows `The API key changed, but other running Warp processes could not be notified.`
    - A failed secure-storage write keeps the editor open and shows `Could not save this API key. Try again.`

15. A connected custom endpoint row exposes the existing `ctrl + x` clear action.
    - Clearing removes the key but not the endpoint definition.
    - A successful clear keeps `/api-keys` open and removes the `(Connected)` state.
    - A successful clear notifies other running TUI processes to reload TUI keys.
    - If cross-process notification fails after secure persistence succeeds, the current TUI keeps the cleared state and shows the partial-success message from Behavior 14.
    - A failed clear keeps the previous key and shows `Could not clear the selected API key. Try again.`
    - Saving an empty credential has the same effect as clearing it.

16. `/api-keys` never adds, edits, renames, or removes endpoint definitions. It manages only API keys.

17. When custom endpoints are allowed but there are no valid or invalid definitions, `/api-keys` shows one non-selectable `Custom endpoints` row with `(None configured)` and the instruction `Add one in settings.toml or use /modify-settings.`

### Entitlement and team policy

18. Custom endpoints are usable only when both conditions are true:
    - The current user has custom inference access through `BYO_ENDPOINT`.
    - The active workspace allows member-provided custom endpoints.

19. When `BYO_ENDPOINT` is unavailable, `/api-keys` does not show valid endpoint names or connection states. It shows one non-selectable `Custom endpoints (Unavailable)` row with `Custom endpoints are not available for this workspace.`

20. When workspace policy disallows member custom endpoints, `/api-keys` does not show valid endpoint names or connection states. It shows one non-selectable `Custom endpoints (Disabled by organization)` row with `Your organization does not allow member custom endpoints.`

21. Invalid-definition rows from Behavior 10 remain visible after the policy or entitlement row so the user can still repair `settings.toml`.

22. Losing entitlement or workspace permission does not delete endpoint definitions or keys. It immediately removes the endpoint models from the picker and requests. Restoring access reconnects still-valid definitions and stored keys.

### Model picker and requests

23. A valid endpoint without a key remains visible in `/api-keys`, but none of its models appear in the model picker or requests.

24. After a non-empty key is saved, every valid model for that endpoint appears in the TUI model picker without a restart.
    - The row title is `alias` when it is non-blank. Otherwise, it is `name`.
    - The row shows only `Custom · <endpoint name>`, matching the GUI custom-model description and disambiguating equal model labels.
    - The row does not show `(key connected)`. A custom model cannot appear until its endpoint has a key, so that state is redundant.

25. Selecting a custom model follows the existing TUI model-selection behavior. Local agent requests include the selected model identity and the matching endpoint URL, key, schema, and raw model name.

26. Clearing the key, invalidating or deleting the endpoint, removing the selected model, or losing access immediately removes its models from the picker and request registry. An active selection that no longer resolves falls back to the normal usable default model.

### Rename and delete lifecycle

27. The endpoint's TOML map key is its identity. Changing that key, including only its case, is a delete followed by an add.
    - The renamed endpoint starts without a `(Connected)` state.
    - Warp removes the old name's stored API key.
    - The user must enter the key again for the new name.
    - Every model under the renamed endpoint receives a different deterministic model identity.

28. Changing a model's `name` changes that model's identity. Changing only its `alias`, URL, schema, or order does not.

29. Deleting an endpoint definition removes its models immediately and removes its stored API key. Deleting one model removes only that model.

30. Warp does not write generated identities back to `settings.toml`.

## Assumptions

- The five settled APP-5380 decisions and Harry's PR review decisions are authoritative. No human question remains unresolved.

## Decisions

### Endpoint identity uses the map key

- **Chosen:** Use the unique endpoint name as the TOML map key and secure-key lookup key.
- **Advantages:** Matches the execution-profile collection's map shape; TOML enforces uniqueness; users and `/modify-settings` do not generate an opaque ID.
- **Disadvantages:** Renaming disconnects the key and changes every model identity.
- **Rejected:** A user-authored UUID or opaque stable key. It protects renames but repeats the workflow the requester rejected.
- **Rejected:** An array of endpoint objects. It makes duplicate-name handling and targeted `/modify-settings` edits less reliable.

### Invalid entries recover independently

- **Chosen:** Load valid entries, retain diagnostics for invalid entries, and skip only invalid entries.
- **Advantages:** One typo does not disable unrelated endpoints.
- **Disadvantages:** The setting loader needs endpoint-level diagnostics in addition to its normal whole-setting result.
- **Rejected:** Reject the complete collection. This matches execution profiles but is too disruptive for independent inference endpoints.
- **Rejected:** Silently drop invalid entries. Users could not tell why an endpoint is absent.

## Validation criteria

1. Add the TOML example from Behavior 7. Verify `Acme Gateway custom endpoint` appears without a `(Connected)` state in `/api-keys`, while neither model appears in the model picker.
2. Set, replace, and clear the endpoint key. Verify masked editing, connected-state transitions, and no `api_key` or generated identity appears in `settings.toml`.
3. Set the key and verify both models appear immediately with the labels and sole `Custom · Acme Gateway` annotation from Behavior 24. Verify neither row shows `(key connected)`.
4. Select each model and inspect the local request. Verify the matching URL, schema, raw model name, model identity, and key are sent.
5. Add one valid and one invalid endpoint. Verify the valid endpoint remains usable, the invalid endpoint gets a `(Skipped)` row, and the standard invalid-settings hint appears.
6. Exercise every URL rejection category in Behavior 9 and verify no rejected endpoint reaches a request.
7. Remove and restore the entitlement and team permission independently. Verify the required `/api-keys` state, picker removal, request suppression, and key preservation.
8. Rename an endpoint, rename a model, edit only an alias, edit URL/schema, reorder models, and delete an endpoint. Verify the identity and key lifecycle in Behaviors 27–30.
9. Run two TUI processes on the same profile. Change a key through one process and verify the other reloads the connection state through the existing key revision mechanism.
