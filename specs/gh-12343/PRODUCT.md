# Prefill Tab Config parameters from external URIs

GitHub: [warpdotdev/warp#12343](https://github.com/warpdotdev/warp/issues/12343)

Figma: none provided. This first pass reuses the existing Tab Config parameter confirmation modal.

## Summary

Extend the existing `warp://tab_config/<file-stem>` URI so scripts and launchers can prefill declared Tab Config parameters. URI-provided values always remain visible and editable in the existing confirmation modal before any parameterized config runs.

## Goals / Non-goals

Goals:

- Let shell aliases, `open`, Raycast, Alfred, and similar tools supply declared Tab Config parameter values through one stable URI format.
- Preserve the existing Tab Config selection, modal, parameter types, rendering, and window-target behavior.
- Treat external URIs as untrusted input: validate them strictly and require user confirmation before a parameterized config opens.
- Keep control query parameters separate from config parameter names.

Non-goals:

- No dedicated `warp-terminal --tab-config` or `warp --tab-config` command in this first pass; callers invoke the existing custom URI through their platform's normal URL launcher.
- No flag or query parameter that skips confirmation for a Tab Config that declares parameters.
- No support for undeclared parameters, environment-variable expansion, command substitution, parameter files, stdin, or secret injection.
- No change to Tab Config TOML schema, template syntax, parameter types, default values, or command quoting.
- No remote download or arbitrary filesystem path to a Tab Config; selection remains limited to configs loaded from Warp's Tab Config directory.

## Behavior

1. Existing `warp://tab_config/<file-stem>` links continue to select a saved Tab Config by its case-insensitive on-disk file stem, with or without the `.toml` extension.
2. A caller prefills a declared parameter using a namespaced query key:

   ```text
   warp://tab_config/<file-stem>?param.<name>=<value>
   ```

   Multiple declared parameters use multiple `param.<name>=<value>` pairs.
3. Parameter names and values use standard URL query percent-encoding. Warp decodes each name and value exactly once. `+` follows standard query decoding and represents a space; a literal plus must be percent-encoded.
4. The `param.` namespace is required. Unprefixed keys are never interpreted as Tab Config values. This leaves existing and future URI controls separate from user-authored parameter names.
5. `new_window` remains the existing URI control. `new_window=1` and `new_window=true` request a new Warp window; any other value keeps the existing active-window behavior. A config parameter literally named `new_window` is supplied as `param.new_window`.
6. Every `param.<name>` key must name a parameter declared by the selected Tab Config. If any supplied name is undeclared, empty, malformed, duplicated, or contains a Unicode control character, Warp rejects the complete request before opening a modal, tab, or new window. The serialized query is limited to 8 KiB, each decoded parameter name to 128 bytes, and each decoded value to 4 KiB; over-limit requests are rejected as a whole.
7. Query keys other than `new_window` and `param.<declared-name>` are rejected rather than silently ignored. The `new_window` control may appear at most once. An unknown key or duplicate control rejects the complete request, making misspellings and conflicting intent visible.
8. On rejection, Warp shows a non-blocking, generic error toast in an existing Warp window when one is available. The message may identify the error category, but it never repeats an attacker-controlled parameter name, value, or complete URI. Warp does not open the Tab Config.
9. A Tab Config that declares parameters always opens the existing parameter modal, even when the URI supplies every required parameter. There is no URI-controlled auto-submit or confirmation bypass.
10. URI-provided values replace the corresponding TOML defaults only for this one modal invocation. Missing parameters retain their existing defaults; a missing parameter with no default remains unsatisfied and continues to block submission.
11. An explicitly supplied value that is blank after trimming whitespace is different from an omitted parameter:
    - For a parameter with a default, clearing or supplying a blank value preserves the modal's existing fallback-to-default behavior. Repo and Branch pickers show that effective default rather than a blank custom item.
    - For a required parameter without a default, the blank value remains an unselected, unsatisfied field and the modal cannot submit until the user enters or selects a value.
12. Text parameters display the URI value in their text field. The user can edit, clear, or replace it before opening the tab.
13. Repo parameters initialize their existing repo picker from the URI value. Branch parameters initialize their existing branch picker from the URI value. When a config declares multiple Repo parameters, the lexicographically first Repo parameter supplies the initial repository for every Branch picker, matching the modal's existing stable ordering. If the user subsequently selects any Repo parameter, the existing behavior of refreshing every Branch picker from that most recently selected repository remains unchanged.
14. Every effective URI-derived value must be visibly represented by its field before it can be submitted. A non-blank Repo or Branch value that is not in the currently discovered list appears as an explicit selected custom value rather than being stored only in hidden picker state; the user may keep or replace it. A blank value follows invariant 11 and is never rendered or submitted as an invisible custom selection. This phase adds no new semantic rule that a non-blank Repo path or Branch name must already exist, because manually configured defaults are also free-form strings. The mandatory modal confirmation is the trust boundary.
15. The modal marks no value as trusted merely because it came from a custom URI. It presents URI values in the same editable controls and uses the same final submit path as manually entered values.
16. Selecting **Open Tab** submits the effective values currently visible in the modal, not the original URI payload. Editing a prefilled field changes the value used to render the Tab Config.
17. Cancelling or dismissing the modal creates no tab and runs no config command. Prefilled values are discarded and are not persisted as new defaults.
18. When `new_window=true` is present, the existing target-window behavior is preserved: the parameter modal appears in the selected new window, and successful confirmation opens the Tab Config there. Cancelling leaves that window as an ordinary Warp window.
19. When no Warp window is available, Warp may create the normal fallback window needed to show a valid request's modal. Invalid requests are rejected before a new window is created.
20. A selected Tab Config with no declared parameters preserves today's direct-open behavior. Supplying any `param.*` key for such a config is invalid and does not open it.
21. After confirmation, values flow through the same Tab Config renderer as manual values. Existing command-value shell quoting, title/directory rendering, worktree handling, pane creation, tab color, and telemetry behavior remain unchanged.
22. A URI cannot select a Tab Config by arbitrary path, traverse outside the Tab Config directory, or cause Warp to load a config that was not already discovered by the normal loader.
23. Parsing is all-or-nothing. Warp never opens a config with only the subset of URI values that happened to validate.
24. URI parameter values are invocation-local. They are not written to the TOML file, settings, history, clipboard, logs, crash metadata, or telemetry. This applies at the initial custom-URI ingress as well as in Tab Config-specific parsing and errors; the generic incoming-URI logger must not serialize a parameterized Tab Config URL.
25. Telemetry may record that a Tab Config was opened from a parameterized URI, the number of supplied parameters, whether a new window was requested, and coarse validation status. It must not record parameter names or values.
26. If the `TabConfigs` feature is unavailable, the URI retains the existing unsupported-host behavior. Parameter handling does not bypass the feature gate.
27. Existing menu-based Tab Config launches and non-parameterized Tab Config URIs behave exactly as before.
