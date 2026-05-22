# Summary
Warp should ship an allowlisted standalone local control CLI binary, provisionally named `warpctrl`, that lets developers script the same classes of user-visible actions they can already perform inside the running app: manipulating windows, tabs, panes, sessions, appearance, settings, and selected UI surfaces. The CLI should operate against one or more already-running local Warp app processes through a stable machine protocol, with deterministic target selection and clear errors when a process or target is ambiguous.
## Problem
Warp already has rich interactive actions, but they are primarily reachable through UI, keybindings, menus, or deeplinks. Developers cannot reliably compose those same actions into shell scripts, demos, automation, or agent workflows, and there is no general local protocol for addressing a specific running Warp instance, window, pane, or session.
## Goals / Non-goals
Goals:
- Provide a first-class, scriptable standalone `warpctrl` binary for controlling running Warp app processes.
- Keep CLI startup lightweight by avoiding GUI-app startup or full terminal initialization for routine control commands.
- Keep the surface allowlisted and finite instead of exposing arbitrary internal actions.
- Make targeting explicit and deterministic across multiple Warp processes, windows, tabs, panes, and sessions.
- Support both ergonomic active-target defaults and precise selectors for automation.
- Define a complete protocol/catalog up front, while shipping the implementation incrementally.
Non-goals:
- Replacing the Oz CLI or mixing cloud-agent management into this CLI.
- Exposing every internal app action, debug action, developer-only helper, or privileged state mutation.
- Treating the CLI as a general RPC escape hatch into Warp internals.
- Requiring developers or automation to spawn the Warp GUI executable in CLI mode for ordinary control commands.
- Requiring the first implementation slice to ship every action in the catalog.
## Behavior
1. The Warp control CLI operates only on running local Warp app processes. If no compatible Warp process is available, the CLI exits non-zero with a clear “no running Warp instance found” error.
2. The CLI exposes only explicitly allowlisted actions. Unknown action names, unsupported parameter combinations, or requests for non-allowlisted capabilities fail with structured errors; they are never forwarded to arbitrary internal dispatch.
3. Every successful mutating request identifies:
   - The Warp process instance that executed it.
   - The resolved target, when the action addresses a window, tab, pane, or session.
   - A success payload suitable for JSON output.
4. Every failure identifies:
   - A stable machine-readable error code.
   - A human-readable explanation.
   - Any selector that was ambiguous, missing, stale, unsupported, or invalid.
5. The CLI supports human-readable output by default and JSON output for scripts. JSON output has stable field names and is available for discovery commands, read commands, successful mutations, and failures.
6. The CLI supports process discovery and instance selection:
   - `warpctrl instance list` returns all reachable local Warp app processes that support the protocol.
   - Each process has an opaque `instance_id`, a channel/build identity, and enough display metadata for a developer to choose it.
   - If exactly one compatible process is available, commands may target it implicitly.
   - If multiple compatible processes are available, the CLI may select a single clearly active/frontmost instance when that state is unambiguous; otherwise it fails and asks the developer to pass an explicit instance selector.
   - Developers can explicitly choose an instance by opaque instance ID. Channel or PID filters may be offered as convenience filters, but opaque instance ID is the canonical selector.
7. The CLI supports introspection for target discovery:
   - `warpctrl window list`
   - `warpctrl tab list`
   - `warpctrl pane list`
   - `warpctrl session list`
   - `warpctrl app active`
   These commands return opaque protocol-facing IDs and enough metadata for subsequent commands without requiring knowledge of internal Warp identifiers.
8. The target selector model is hierarchical:
   - Instance selector resolves a running Warp process.
   - Window selector resolves within the instance.
   - Tab selector resolves within the window.
   - Pane selector resolves within the tab or active pane group context.
   - Session selector resolves within the pane when the pane hosts terminal session state.
9. Every selector family supports an ergonomic `active` form when that concept exists:
   - Active instance, if unambiguous.
   - Active window in the selected instance.
   - Active tab in the selected window.
   - Active pane in the selected tab.
   - Active session in the selected pane.
10. Every selector family supports explicit opaque IDs returned by introspection. Tabs may also support index selectors where index-based workflows are already user-visible, but IDs remain the preferred automation surface.
11. “Active session” means the currently selected terminal session for the resolved pane/window context. If the selected target does not contain a terminal session, session-scoped actions fail rather than silently redirecting elsewhere.
12. When a command omits lower-level selectors, it resolves them from the chosen higher-level context using active defaults. Example: a pane split command with only `--instance` uses that instance’s active window, active tab, and active pane.
13. When an explicitly supplied target disappears between discovery and execution, the request fails with a stale-target error. The CLI must not silently choose a different tab, pane, or session.
14. The protocol is command-oriented, not open-ended state mutation. Each action has a named command, validated parameters, and defined target scope.
15. The complete allowlisted action catalog should be organized into these namespaces.
16. Discovery and read-only state actions:
   - List instances.
   - Get protocol/app version information for one instance.
   - List windows, tabs, panes, and sessions.
   - Get the currently active instance/window/tab/pane/session chain when available.
   - Inspect enough target metadata to let a script decide what to address next.
17. Window actions:
   - Create a new window.
   - Focus a target window.
   - Close a target window.
18. Tab actions:
   - Create a new terminal tab.
   - Create a new agent tab where that is already a user-visible app capability.
   - Activate a target tab.
   - Activate previous, next, or last tab.
   - Move a target tab left or right.
   - Rename or reset a tab title.
   - Set or clear active-tab color where that is already supported in the UI.
   - Close the active tab, a target tab, other tabs, or tabs to the right of a target tab.
19. Pane actions:
   - Split a target pane left, right, up, or down.
   - Optionally choose the shell/session profile for split operations when that already maps to user-facing behavior.
   - Focus a target pane.
   - Navigate focus left, right, up, or down among panes.
   - Close a target pane.
   - Toggle maximize for a target pane.
   - Resize pane dividers left, right, up, or down when that is supported by the app.
20. Session and terminal-input actions:
   - Cycle to the previous or next session where the app exposes session cycling.
   - Insert text into the active input without executing it.
   - Replace the active input buffer.
   - Clear the active input buffer where that matches existing user behavior.
   - Run a command in the target session where the app already supports user-triggered command execution.
   - Switch input mode between terminal and agent modes only where that mode switch is already user-visible and valid for the selected target.
   These commands are part of the protocol catalog, but command execution should be treated as a higher-risk mutating action with explicit confirmation in spec/review before rollout.
21. Appearance actions:
   - List available themes.
   - Set the current fixed theme.
   - Toggle or set “follow system theme.”
   - Set the light and dark themes used when following the system theme.
   - Increase, decrease, or reset font size.
   - Increase, decrease, or reset UI zoom.
   - Set other allowlisted appearance controls only when they correspond to stable user-facing controls.
22. Settings actions:
   - Read allowlisted user-facing settings.
   - Set allowlisted settings to validated values.
   - Toggle allowlisted boolean settings.
   - Reject attempts to mutate private, debug-only, unsafe, derived, or unsupported settings.
   - Return a stable error when a named setting exists internally but is not part of the public local-control allowlist.
23. The settings allowlist should initially cover settings families that are already plainly user-facing and valuable for scripting:
   - Theme/system-theme configuration.
   - Font/zoom-related controls.
   - Notifications.
   - Syntax highlighting and error-underlining toggles.
   - Accessibility verbosity where exposed to users.
   - Selected panel/layout toggles when the user-facing behavior is already stable.
   Additional settings families can be added only by extending the allowlist.
24. Panel and surface actions:
   - Open the general settings surface.
   - Open a specific settings page or settings search result.
   - Open or toggle the command palette with an optional initial query where the app already supports query seeding.
   - Open or toggle command search where that is already user-visible.
   - Toggle or open the left panel, Warp Drive surface, right panel, resource center, AI assistant panel, code review panel, and vertical tabs panel where valid.
25. File/path intent actions may be included when they already mirror existing user-visible deep-link behavior:
   - Open a path in a new tab or window.
   - Open a repository picker or repo path flow where the current app already supports it.
   These should remain allowlisted intent actions rather than arbitrary filesystem RPCs.
26. The following categories are explicitly excluded from the initial public allowlist even if there are internal actions for them:
   - Crash, panic, heap-dump, token-copying, debug-reset, and similar developer/debug helpers.
   - Arbitrary auth manipulation.
   - Arbitrary cloud object mutation or broad Warp Drive CRUD.
   - Arbitrary internal view dispatch by string.
   - Arbitrary setting names outside the allowlist.
27. CLI command names should be noun-oriented and discoverable. During the provisional standalone-binary phase, the control CLI should expose a `warpctrl ...` command surface:
   - `warpctrl instance list`
   - `warpctrl app active`
   - `warpctrl tab create`
   - `warpctrl pane split --direction right`
   - `warpctrl pane split --instance <id> --window active --pane active --direction right`
   - `warpctrl theme set "Warp Dark"`
   - `warpctrl setting set appearance.themes.system_theme true`
   - `warpctrl input insert "cargo check" --replace`
   Channelized install names or aliases may vary during packaging. If the product later converges on `warp ...`, update packaging, shell completions, and operator docs together.
28. The wire protocol mirrors the CLI model. A mutating request contains:
   - An action name from the allowlist.
   - A structured target selector.
   - Validated parameters.
   A response contains:
   - Success/failure status.
   - Resolved instance and target metadata.
   - Result data or structured error data.
29. The protocol is versioned. Clients must be able to determine whether a running Warp process supports the protocol version and action they intend to call.
30. Multiple running Warp processes are a supported normal case, not an error state. A local stable build and local dev build, or multiple supported local app instances, can coexist; the CLI provides deterministic discovery and addressing rather than assuming one global server.
31. Requests should be scoped to local-user control of the running app, with separate enforcement for actions that require a true logged-in Warp user. A command that fails local authentication, local authorization, execution-context checks, or authenticated-user checks reports that condition explicitly and does not degrade into a less-specific request.
32. If a selected action is valid in general but impossible in the current UI state, the CLI reports a state-specific failure. Examples include:
   - Splitting a pane that no longer exists.
   - Running a session-scoped command against a non-terminal pane.
   - Focusing a window that has closed.
   - Setting a theme that is not available in that instance.
33. The first `warpctrl` implementation slice should ship the smallest end-to-end vertical slice that proves:
   - Process discovery and target resolution work.
   - A standalone CLI binary can reach a running local Warp process without launching or initializing the GUI app.
   - `warpctrl tab create` creates a new terminal tab in the selected running instance.
   - The command returns a structured success or failure payload suitable for human-readable and JSON output.
   The first slice should include the minimum health/introspection commands needed to discover a running instance and exercise `tab.create`.
34. Follow-up PRs should fill out the remaining catalog in parallelizable groups once the protocol, discovery model, target resolution, error model, `tab.create` action path, and standalone `warpctrl` packaging shape have been validated by the first slice.
35. The protocol transport should be designed so that the default target is localhost but the CLI can be extended in the future to target remote URLs (e.g., a Warp instance on another machine or a hosted control endpoint). This is not in scope for the first implementation but should not be precluded by the architecture.
## Action classification and permission model
Agents, scripts, and human developers are expected to be major consumers of `warpctrl`. The action catalog must therefore classify every action by both risk tier and authenticated-user requirement so Warp can enforce local-control permissions in the app bridge.
Every action definition must include:
- a stable action name and namespace;
- a risk tier;
- whether a true logged-in Warp user is required;
- whether the action may run from external clients, verified Warp-terminal clients, or both;
- the required local-control permission category;
- any target-scope restrictions.
By default, new actions require an authenticated Warp user. An action may be marked logged-out-safe only after deliberate review confirms it does not touch Warp Drive, AI conversation traces, synced settings, team/account data, cloud-backed user state, terminal content, or other user-sensitive data.
### Classification tiers
Every action in the catalog belongs to exactly one of the following tiers, from least to most sensitive:
1. **Read-only / metadata.** Actions that return local app structure or configuration without exposing terminal content or user-authenticated data.
   - Instance discovery and health: `instance list`, `app active`, `app version`, `app ping`.
   - Layout enumeration: `window list`, `tab list`, `pane list`, `session list`.
   - Appearance reads that return local configuration values but not user data: `theme list`, selected `setting get` actions for logged-out-safe local settings.
   These are the primary default actions for external clients.
2. **Read-only / terminal data.** Actions that return content from terminal sessions, command history, pane output buffers, input editor state, session replay, or terminal-derived traces.
   - Reading pane output or scrollback content.
   - Reading the current input buffer contents.
   - Reading command history or session replay data.
   Even though these are read-only, they cross a privacy boundary that metadata reads do not.
3. **Mutating / non-destructive.** Actions that change app state in visible, reversible, or low-risk ways without executing terminal content or destroying user state.
   - Layout mutations: `tab create`, `tab activate`, `tab move`, `tab rename`, `window create`, `window focus`, `pane split`, `pane focus`, `pane navigate`, `pane maximize`, `pane resize`.
   - Appearance mutations: `theme set`, `font-size increase/decrease/reset`, `zoom increase/decrease/reset`.
   - Settings writes for allowlisted non-destructive local settings.
   - Panel/surface toggles where they do not expose authenticated user data.
4. **Mutating / destructive or high-risk.** Actions that destroy user state, close active work, inject terminal input, execute commands, or execute user-authored content.
   - Closing targets: `tab close`, `window close`, `pane close`.
   - Terminal input injection: `input insert`, `input replace`, `input clear`.
   - Command execution in a session.
   - Input mode switching between terminal and agent modes.
   - Executing Warp Drive workflows or notebooks in a terminal session.
   Any action that can cause data loss or execute arbitrary code belongs here regardless of how simple the API looks.
### Authenticated-user requirement
An authenticated user means a true logged-in Warp user in the selected Warp app, not merely the local OS user or a `warpctrl` process authenticated to localhost.
The allowlist must clearly indicate `requires_authenticated_user` for every action:
- `false` only for logged-out-safe actions that operate on local app structure, local appearance metadata, or local-only settings that do not expose user-sensitive data.
- `true` for actions that read or mutate Warp Drive, AI conversation traces, synced settings, team/account data, user identity data, or any cloud-backed Warp state.
- `true` for actions that execute user-authored Warp Drive content, even if the execution target is a local terminal session.
If an authenticated-user action is invoked while the selected app has no logged-in user, the CLI reports a structured authenticated-user error. It must not silently return partial logged-out data as success.
### Execution context policy
`warpctrl` should distinguish verified invocations from inside Warp-managed terminal sessions from external invocations.
- **External invocation:** a `warpctrl` process started outside Warp's terminal. By default, it can receive only the smaller logged-out-safe local action set that does not touch user-authenticated data. Higher tiers require explicit local-control settings or approvals.
- **Verified Warp-terminal invocation:** a `warpctrl` process started inside a Warp-managed terminal session and able to present an app-issued execution-context proof. When the selected app has a logged-in Warp user, this context can receive authenticated-user grants if the user enabled that permission.
- The app must not trust a caller-declared label. Environment variables may help discover the context, but the broker must verify a session-bound capability or equivalent proof before issuing in-Warp-only grants.
### Granular local-control permissions
The product settings surface should expose granular permissions under the default-off local-control setting. Recommended controls:
- Allow local read-only metadata.
- Allow terminal data reads.
- Allow non-destructive local mutations.
- Allow destructive or execution actions.
- Allow authenticated-user actions from verified Warp terminals.
- Allow authenticated-user actions from external clients, default off and separate from the in-Warp permission.
These settings define the maximum grants the broker may issue. The app bridge still enforces the action's risk tier, authenticated-user requirement, execution-context requirement, and target scope for every request.
### Scoped credentials
The local discovery record must not expose a reusable full-access credential. `warpctrl` should request scoped credentials from an app-owned broker or equivalent trusted path.
Scoped credentials should include:
- the selected Warp instance;
- granted risk tiers;
- allowed action families;
- verified execution context;
- whether authenticated-user access is granted and for which logged-in user subject;
- optional target scopes;
- issuance and expiry metadata;
- revocation/audit identity.
The bridge, not the CLI frontend, enforces these grants. If a request exceeds its credential, the bridge returns `insufficient_permissions`, `authenticated_user_required`, `authenticated_user_unavailable`, or `execution_context_not_allowed` as appropriate.
### Future entity extensibility: files and Warp Drive objects
The selector and action model should be designed to accommodate entity types beyond the current window/tab/pane/session hierarchy. Two important future entity families are **local files** and **Warp Drive objects** (workflows, notebooks, environment variables, prompts). Neither is in scope for the first implementation, but the protocol should not preclude them.
**Files.** Warp already supports file opening via deep links and the built-in editor. A future `file` namespace could support:
- `warpctrl file open <path>` — open a file in a Warp editor tab, equivalent to clicking a file link.
- `warpctrl file open <path> --line <n>` — open at a specific line.
- `warpctrl file list` — list files currently open in editor tabs across the instance.
File selectors would use filesystem paths (absolute or relative to the working directory of the target pane/session). Unlike window/tab/pane selectors, file selectors are not opaque IDs — they are user-visible paths. The protocol should support a `file` field in the target selector that accepts a path string, distinct from the opaque ID selectors used for windows, tabs, and panes.
**Warp Drive objects.** Warp Drive stores typed objects (workflows, notebooks, environment variable sets, prompts) that users can reference, execute, and share. A future `drive` namespace could support:
- `warpctrl drive list --type workflow` — list Warp Drive objects by type.
- `warpctrl drive get <id>` — retrieve a specific Drive object by its opaque ID or by name/path.
- `warpctrl drive run <workflow-id>` — execute a workflow in a target session, equivalent to invoking it from the command palette.
- `warpctrl drive insert <notebook-id>` — insert a notebook's runnable commands into the active input.
Drive object selectors should support both opaque IDs (for automation stability) and human-friendly name/path lookups (for interactive use). The type field (`workflow`, `notebook`, `env_var`, `prompt`) acts as a namespace filter. Drive actions that execute content in a terminal session (e.g., running a workflow) inherit the destructive/high-risk tier from the action classification model.
**Design constraints for both:**
- File and Drive selectors are orthogonal to the window/tab/pane hierarchy — a file open action targets an instance (which window to open in), not a specific pane. A Drive workflow execution targets a session (which pane to run in).
- The `TargetSelector` type in the protocol should be extensible with optional fields for these new selector families without breaking existing requests that omit them.
- The action classification tiers apply, and Drive actions require authenticated-user grants by default: listing Drive objects is tier 1 metadata plus authenticated user, reading Drive object content is tier 1 or 2 depending on whether it contains user data plus authenticated user, and executing a Drive workflow is tier 4 plus authenticated user.
### Settings: protocol-first
Settings reads and writes should go through the local-control protocol like other actions, not bypass it via direct file manipulation.
- `warpctrl setting get <key>`, `warpctrl setting set <key> <value>`, and `warpctrl setting toggle <key>` send requests to the running Warp instance through the standard authenticated control endpoint.
- The app bridge validates the key against the allowlist and the value against the expected type before applying the change.
- This keeps authorization enforcement consistent: the same permission tier, execution-context, and authenticated-user policies apply to settings mutations as to any other action, rather than creating a second unguarded path through the filesystem.
- The app owns the write to the settings file and any side effects (e.g., theme reload, layout reflow) as a single atomic operation, avoiding races between a CLI file write and the app's file watcher.
- If a future need arises for offline settings manipulation (no running Warp process), a separate file-based path can be added later with its own validation, but it should not be the default.
- The action classification still applies: settings reads are tier 1 (metadata), settings writes are tier 3 (non-destructive mutation).
