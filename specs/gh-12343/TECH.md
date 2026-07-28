# Prefill Tab Config parameters from external URIs — Tech Spec

See [`PRODUCT.md`](PRODUCT.md) for user-visible behavior.

Code reference: [`2fe6a4f567928c6f11b74021e55092e5f3e5bd79`](https://github.com/warpdotdev/warp/tree/2fe6a4f567928c6f11b74021e55092e5f3e5bd79)

## Context

Warp already recognizes `UriHost::TabConfig` behind `FeatureFlag::TabConfigs` and dispatches it to `handle_tab_config_uri` ([`app/src/uri/mod.rs:87-138`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/uri/mod.rs#L87-L138), [`app/src/uri/mod.rs:223-225`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/uri/mod.rs#L223-L225)). The handler resolves a config by case-insensitive file stem, reads only the `new_window` query control, selects a workspace, and calls `Workspace::open_tab_config` ([`app/src/uri/mod.rs:783-855`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/uri/mod.rs#L783-L855)). Current tests lock file-stem, extension, dotted-name, case, and missing-source-path behavior ([`app/src/uri/uri_tests.rs:95-146`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/uri/uri_tests.rs#L95-L146)).

A `TabConfig` declares named `TabConfigParam` entries of type text, branch, or repo, with optional defaults ([`app/src/tab_configs/tab_config.rs:60-92`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/tab_configs/tab_config.rs#L60-L92), [`app/src/tab_configs/tab_config.rs:138-177`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/tab_configs/tab_config.rs#L138-L177)). Rendering already separates unquoted title/directory context from shell-quoted command context, so this feature must feed the same renderer rather than interpolate URI data itself ([`app/src/tab_configs/tab_config.rs:206-227`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/tab_configs/tab_config.rs#L206-L227)).

`Workspace::open_tab_config` opens parameterless configs directly but always opens `TabConfigParamsModal` when parameters exist ([`app/src/workspace/view.rs:6931-6997`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/workspace/view.rs#L6931-L6997)). The modal currently seeds every field only from its TOML default; repo defaults determine the initial branch picker repository, and submission collects the current field values before emitting the existing submit event ([`app/src/tab_configs/params_modal.rs:183-329`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/tab_configs/params_modal.rs#L183-L329), [`app/src/tab_configs/params_modal.rs:436-452`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/tab_configs/params_modal.rs#L436-L452)).

The current `warp_cli` has no Tab Config command. The custom URI is already the cross-launcher entry point, so this phase does not add a second parser or transport in the CLI crate.

`handle_incoming_uri` currently sends the complete parsed URL to the `full` side of a `safe_info!` call before host-specific dispatch ([`app/src/uri/mod.rs:1221-1226`](https://github.com/warpdotdev/warp/blob/2fe6a4f567928c6f11b74021e55092e5f3e5bd79/app/src/uri/mod.rs#L1221-L1226)). A parameterized Tab Config URI would therefore leak values before any parser-level redaction. Ingress redaction is part of this feature, not a follow-up.

## Proposed changes

### 1. Redact parameterized Tab Config URIs at the shared ingress

In `handle_incoming_uri`, classify the scheme and host before logging the full `Url`. For `warp://tab_config/...`:

- emit only a constant message plus a coarse query-pair count;
- do not include the path, raw query, fragment, decoded parameter names, or values in either the safe or full log channel;
- do not use `Url`'s `Debug` or `Display` representation.

Keep the existing diagnostic behavior for other URI hosts. Put the classification in a small pure helper and test that a sentinel config name, parameter name, parameter value, percent-encoded value, and fragment are all absent from its formatted output.

### 2. Parse a strict parameterized Tab Config intent before choosing a window

Add pure parsing types in `app/src/uri/mod.rs`:

```rust
struct TabConfigUriIntent {
    config: TabConfig,
    prefilled_params: HashMap<String, String>,
    force_new_window: bool,
}

fn parse_tab_config_uri_intent(
    url: &Url,
    configs: Vec<TabConfig>,
) -> Result<TabConfigUriIntent, TabConfigUriError>
```

The helper first reuses `get_launch_config_path` and `find_matching_tab_config`, then consumes all query pairs in one pass:

- `new_window` remains a control and preserves the existing `1`/`true` truthy behavior.
- `new_window` may occur at most once; duplicate controls are rejected rather than resolved by query order.
- Keys beginning with `param.` are stripped to a decoded declared parameter name.
- Every parameter name must be non-empty, appear in `config.params`, and occur once.
- Every other key is an error.
- `param.new_window` is a normal declared parameter and cannot collide with the unprefixed control.
- Validate raw query components for complete `%HH` escapes and UTF-8 before consuming the `url` crate's decoded query pairs. Values are not decoded or expanded a second time.
- Reject the request before allocating the parameter map when the serialized query exceeds 8 KiB. After one decode, reject names over 128 bytes, values over 4 KiB, NUL, and any character for which `char::is_control()` is true. The number of supplied config parameters cannot exceed the number of declarations.
- The parser returns no partial intent on error.

Keep error variants structural (`ConfigPathMissing`, `ConfigNotFound`, `QueryTooLarge`, `UnknownQueryKey`, `DuplicateControl`, `UnknownParam`, `DuplicateParam`, `EmptyParamName`, `InvalidCharacter`, `NameTooLarge`, `ValueTooLarge`). Their `Display` and `Debug` implementations expose only the error category, never an attacker-controlled name, value, config stem, or full URI. Do not derive `Debug` for a type that would expose the values to routine logging, and do not derive it for `TabConfigUriIntent`.

Move intent parsing before target-window creation. On failure, log only the redacted error and request an existing-window toast when possible; do not create a fallback window merely to display an invalid request.

### 3. Carry invocation-local initial values into `Workspace`

Add a focused entry point:

```rust
pub(crate) fn open_tab_config_with_initial_params(
    &mut self,
    tab_config: TabConfig,
    initial_params: HashMap<String, String>,
    ctx: &mut ViewContext<Self>,
)
```

Behavior:

- If `tab_config.params` is empty, require `initial_params.is_empty()` and delegate to the current direct-open path.
- If parameters exist, build the current cwd and modal title exactly as `open_tab_config` does, then call the modal with `initial_params`.
- Never call `open_tab_config_with_params` from the URI handler. Only the modal submit event can reach the renderer for a parameterized URI.

Keep `open_tab_config` as the menu/default wrapper that supplies an empty initial map. This avoids changing menu behavior and gives the external intent an explicit, reviewable trust boundary.

### 4. Overlay prefilled values onto modal defaults

Change `TabConfigParamsModal::on_open` to accept `initial_params: HashMap<String, String>`. Keep the supplied raw value only long enough to derive the same effective value that submission already uses:

```text
raw initial = URI value when present, otherwise TOML default, otherwise empty
resolved initial = resolve_param_value(raw initial, declaration)
```

`resolve_param_value` treats a value whose trimmed form is empty as the declaration's default, or as unsatisfied when no default exists; it preserves a non-blank value byte-for-byte. Use the resolved initial value consistently:

- Text: initialize the editor buffer with the raw initial value when it is non-blank. When an explicitly supplied URI value is blank, keep the field empty and show its existing default placeholder so submission's fallback remains visible. A required blank field remains empty and unsatisfied.
- Repo: pass the resolved value to `RepoPicker::new_with_style`, retain it in `selected`, and ensure `refresh_items` inserts a selected, user-friendly custom item when a non-blank resolved value is absent from `PersistedWorkspace`. Never create or retain a blank custom selection, and never retain an initial value that the dropdown does not render.
- Branch: pass the resolved value to `BranchPicker::new_with_style` and retain it in `selected`. Its existing custom-default item remains visible when a non-blank branch is absent from the fetched list. A required blank Branch has no selected item.
- Initial Branch lookup cwd: sort parameters with the modal's existing stable ordering and use the resolved value of the lexicographically first Repo parameter. A blank URI override with a default therefore uses the default repository, while a required blank never constructs `PathBuf("")`; when there is no resolved Repo, fall back to the active terminal cwd as today.
- Subsequent Repo selection: preserve `sync_branch_pickers_for_repo`, under which selecting any Repo parameter refreshes every Branch picker from that repository.

The modal continues to own confirmation and final values. Repo and Branch parameters remain free-form, as existing TOML defaults are; this change guarantees visibility rather than introducing existence validation. Do not retain the initial map after fields are constructed, and do not add it to `TabConfig`, settings, or persistence. `try_submit` and `TabConfigParamsModalEvent::Submit` remain the only path to `open_tab_config_with_params`.

Existing non-URI callers pass an empty map. Session-config/worktree paths that intentionally bypass the modal continue using their current internal `open_tab_config_with_params` calls.

### 5. Preserve the renderer and command-safety boundary

No URI-specific interpolation is added. After the user confirms, the existing submit event supplies the modal's effective `HashMap` to `render_tab_config`. This preserves:

- shell quoting for values placed in commands;
- current title and directory rendering;
- worktree branch-name generation;
- pane-tree validation and creation;
- tab title/color behavior.

Do not add environment expansion, shell parsing, `~` expansion, or filesystem normalization to URI values. A value from the URI must behave exactly like the same string manually entered in the modal.

### 6. Add visible, redacted validation feedback

`handle_tab_config_uri` should:

1. parse and validate the complete intent;
2. choose the existing/new workspace only after success;
3. call `open_tab_config_with_initial_params`;
4. on parser failure, show one ephemeral error toast in the primary existing window if available and emit a category-only warning.

Use messages such as `Tab Config URI contains an undeclared parameter` or `Tab Config URI contains a duplicate parameter`. Never include an attacker-controlled name, value, config stem, or complete URI.

Add an URI-specific telemetry source or boolean to the existing `ExistingConfigOpened` event only if it can remain value-free. Record at most the supplied parameter count, new-window flag, and coarse success/error category.

## Testing and validation

### URI parser tests

Extend `app/src/uri/uri_tests.rs` with pure tests:

- one and multiple declared `param.*` values parse;
- standard percent encoding and plus/space behavior decode exactly once, while incomplete escapes and invalid UTF-8 fail;
- `new_window=true`/`1` remains a control and a duplicate `new_window` is rejected;
- `param.new_window` reaches a same-named declared parameter without colliding;
- explicitly empty values and omitted parameters remain distinguishable in the returned map;
- unknown, empty, malformed, duplicate, and unprefixed query keys fail the entire request;
- control characters (including NUL, CR, and LF) and the boundaries around the 8 KiB query, 128-byte name, and 4 KiB value limits are covered;
- a config with no declarations rejects `param.*`;
- dotted and case-insensitive config stems retain current behavior;
- error and ingress-log formatting contain only coarse categories/counts and never the config stem, supplied name, value, or full URI.

### Modal and workspace tests

Extend `app/src/tab_configs/params_modal_tests.rs` and `app/src/workspace/view_tests.rs`:

- non-blank URI values override defaults for text, repo, and branch fields and every effective value is visible in its control;
- empty and whitespace-only URI values for defaulted Text, Repo, and Branch params resolve to the visible/effective default, while the same inputs for required params remain unselected and block submission;
- unknown Repo and Branch values appear as explicit selected custom items rather than hidden state;
- omitted fields retain defaults and required empty fields still block submit;
- with multiple Repo parameters, the lexicographically first resolved Repo value seeds all Branch pickers, including when its raw URI value is blank and falls back to a default; a later selection in any Repo picker refreshes all Branch pickers;
- editing a prefilled value changes the submitted map;
- cancelling clears the fields and persists nothing;
- parameterized intents always show the modal and never directly call the renderer;
- parameterless intents keep the direct-open behavior;
- menu launches with an empty initial map behave exactly as before;
- invalid intents are rejected before a new workspace is created.

### Security and manual validation

Run:

```bash
cargo nextest run -p warp -E 'test(tab_config) | test(uri)'
./script/format
cargo clippy -p warp --tests -- -D warnings
git diff --check
```

Create a temporary Tab Config with text, two repo, branch, required, defaulted, and `new_window` parameters. Invoke it through macOS `open` with encoded spaces, plus signs, shell metacharacters, duplicate keys, an unknown key, control characters, over-limit values, and `new_window=true`. Record that every valid value is visible/editable before launch, malformed requests run nothing and do not appear in logs, cancelling runs nothing, and the final confirmed values follow the existing renderer's quoting.

## Parallelization

Use one implementation worktree, `codex/gh12343-tab-config-uri-params`, because intent parsing feeds the same `Workspace::open_tab_config` and modal initialization API that the tests exercise. A validation agent can independently audit URI parsing and redaction after implementation, but splitting the three tightly coupled API edits across branches would add merge churn.

## Risks and mitigations

- **Untrusted URI silently runs commands:** parameterized configs always stop at the existing modal; no skip-confirmation option is introduced.
- **Typos silently use defaults:** unknown, duplicate, malformed, and unprefixed keys reject the complete request.
- **Control/parameter name collision:** only unprefixed `new_window` controls window choice; all config values live under `param.`.
- **Double decoding or unintended expansion:** consume `Url::query_pairs` once and perform no shell, environment, tilde, or second URL decoding.
- **Secrets leak before parser dispatch:** redact Tab Config URIs in `handle_incoming_uri`; parser errors, telemetry, and full logs never include config stems, names, values, or the full URI.
- **Invisible picker state is submitted:** compute one effective initial value per field and require Repo/Branch custom values to be rendered as selected items.
- **URI size/control-character abuse:** reject bounded raw/decoded input before building fields or editor buffers.
- **Multiple Repo parameters ambiguously seed branches:** codify the modal's stable first-Repo initialization and existing most-recently-selected Repo refresh behavior.
- **Invalid request leaves an empty window:** validate before target-window creation.

## Follow-ups

- Consider a dedicated Warp CLI command only if custom-URI invocation proves insufficient for automation.
- Consider a separately designed trust mechanism for non-interactive execution; do not add a modal-bypass query flag to this flow.
