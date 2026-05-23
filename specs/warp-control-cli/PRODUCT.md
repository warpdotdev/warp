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
10. Every selector family supports explicit opaque IDs returned by introspection. Selector families may also support scoped indices, titles/names, or paths where those concepts are already user-visible, but IDs remain the preferred automation surface.
   - Window selectors support `active`, opaque window IDs, window indices from `window list`, and exact window titles for interactive use.
   - Tab selectors support `active`, opaque tab IDs, tab indices scoped to the resolved window, and exact tab titles for interactive use.
   - Pane selectors support `active`, opaque pane IDs, and pane indices scoped to the resolved tab or pane group.
   - Session selectors support `active`, opaque session IDs, and session indices scoped to the resolved pane when sessions are user-visible as an ordered list.
   - Block selectors use opaque block IDs from block/session introspection.
   - File selectors use paths, plus optional line/column coordinates where the command supports opening or reading a location.
   - Warp Drive selectors use opaque object IDs, with optional type-scoped exact name/path lookups for interactive use.
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
   - `warpctrl tab rename --window-id <window_id> --tab-id <tab_id> "Build logs"`
   - `warpctrl tab rename --window active --tab-index 0 "Build logs"`
   - `warpctrl window close --window-title "Scratch"`
   - `warpctrl pane split --direction right`
   - `warpctrl pane split --instance <id> --window active --pane active --direction right`
   - `warpctrl input replace --session-id <session_id> "cargo check"`
   - `warpctrl block output --pane-id <pane_id> --block-id <block_id> --plain`
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
## API command surface
The public `warpctrl` API is organized around nouns that map to stable user-facing entities. Command names are intentionally not a dump of every internal `WorkspaceAction`, `TerminalAction`, keybinding, or command-palette binding. Internal actions inform the catalog, but a command is added only when it has a stable user-facing behavior, typed parameters, deterministic target resolution, and an explicit risk classification.
### State and data taxonomy
The product surface must distinguish what kind of state a command touches. This distinction is part of the public API and the permission model, not just an implementation detail.
- **Metadata reads** inspect app structure or configuration metadata without exposing user content: instances, windows, tabs, panes, sessions, capability metadata, action metadata, keybinding metadata, theme names, setting keys, current project identity, and other structural state.
- **Underlying data reads** expose user content or data-bearing state without changing it: terminal output, block contents, command history, input buffer contents, file contents, Warp Drive object contents, AI conversation content, and any other content that could contain user data or secrets.
- **App-state mutations** change visible local Warp UI state without directly changing user data: opening or focusing windows, creating or closing tabs, splitting panes, focusing panes, opening panels, opening command surfaces, opening files in Warp, and editing the input buffer without submitting it.
- **Metadata/configuration mutations** change persistent configuration or metadata, but not primary user content: changing themes, font size, zoom, allowlisted settings, keybindings, tab names, pane names, and tab colors.
- **Underlying data mutations** can change user data or cause external side effects: executing terminal commands, writing/creating/deleting files, running workflows that execute commands, CRUD operations on Warp Drive objects, mutating AI conversation history, and any action that can modify data outside transient app UI state.
A command that touches multiple categories must require the strongest applicable permission. For example, `file open` is an app-state mutation, while `file write` is an underlying data mutation; `input insert` is an app-state mutation, while `input run` is an underlying data mutation because it executes a command in the target session.
### Targeting flags
All commands that address a running app target accept the same selector flags where meaningful. Generic `--window`, `--tab`, `--pane`, and `--session` flags accept the selector grammar below; explicit typed aliases are provided so scripts can avoid string parsing ambiguity:
- `--instance <instance_id>` selects a running Warp process from `warpctrl instance list`.
- `--pid <pid>` is a convenience instance selector and conflicts with `--instance`.
- `--window <active|id:<id>|index:<n>|title:<title>>` selects a window inside the instance.
- `--window-id <id>`, `--window-index <n>`, and `--window-title <title>` are exact aliases for the corresponding `--window ...` forms.
- `--tab <active|id:<id>|index:<n>|title:<title>>` selects a tab inside the resolved window.
- `--tab-id <id>`, `--tab-index <n>`, and `--tab-title <title>` are exact aliases for the corresponding `--tab ...` forms.
- `--pane <active|id:<id>|index:<n>>` selects a pane inside the resolved tab or pane-group context.
- `--pane-id <id>` and `--pane-index <n>` are exact aliases for the corresponding `--pane ...` forms.
- `--session <active|id:<id>|index:<n>>` selects a terminal or agent session inside the resolved pane when the command is session-scoped.
- `--session-id <id>` and `--session-index <n>` are exact aliases for the corresponding `--session ...` forms.
- `--block-id <id>` selects a terminal block for block-scoped read commands.
- File commands use path arguments or `--path <path>` where the path is the selected file entity; `--line <n>` and `--column <n>` refine the location when supported.
- Drive commands use object ID arguments or `--drive-id <id>` where the ID is the selected Warp Drive entity; name/path lookup must be type-scoped when supported.
- `--output-format <pretty|json|ndjson|text>` controls output shape and remains globally available.
Within a selector family, specifying more than one form is invalid. For example, `--tab-id` conflicts with `--tab-index`, `--tab-title`, and `--tab`. Omitted lower-level selectors use active defaults only when that active target is unambiguous. Explicit IDs must resolve exactly or fail with `stale_target`; index/title/name/path selectors that match zero targets fail with `missing_target`, and selectors that match multiple targets fail with `ambiguous_target`.
### Read-only command set
The read-only branch `zach/warp-cli-readonly` should implement the following commands before mutating catalog expansion begins. Read-only does not mean one permission: metadata reads and underlying data reads are separate grant categories.
Metadata and capability reads:
- `warpctrl instance list`
- `warpctrl instance inspect [--instance <id>|--pid <pid>]`
- `warpctrl app ping [selectors]`
- `warpctrl app version [selectors]`
- `warpctrl app active [selectors]`
- `warpctrl capability list [selectors]`
- `warpctrl capability inspect <action> [selectors]`
Window, tab, pane, and session reads:
- `warpctrl window list [selectors]`
- `warpctrl window inspect [--window <selector>] [selectors]`
- `warpctrl tab list [--window <selector>] [selectors]`
- `warpctrl tab inspect [--tab <selector>] [selectors]`
- `warpctrl pane list [--tab <selector>] [selectors]`
- `warpctrl pane inspect [--pane <selector>] [selectors]`
- `warpctrl session list [--pane <selector>] [selectors]`
- `warpctrl session inspect [--session <selector>] [selectors]`
Underlying data reads, gated separately from structural metadata reads:
- `warpctrl block list [--pane <selector>] [--limit <n>] [selectors]`
- `warpctrl block inspect <block_id> [selectors]`
- `warpctrl block output <block_id> [--plain|--ansi|--json] [selectors]`
- `warpctrl input get [--session <selector>] [selectors]`
- `warpctrl history list [--session <selector>] [--limit <n>] [selectors]`
Appearance, settings, and command-surface reads:
- `warpctrl theme list [selectors]`
- `warpctrl theme get [selectors]`
- `warpctrl appearance get [selectors]`
- `warpctrl setting list [--namespace <namespace>] [selectors]`
- `warpctrl setting get <key> [selectors]`
- `warpctrl keybinding list [selectors]`
- `warpctrl keybinding get <binding_name> [selectors]`
- `warpctrl action list [selectors]`
- `warpctrl action inspect <action> [selectors]`
Local file and project reads that expose only app/editor state, not arbitrary filesystem traversal:
- `warpctrl file list [selectors]`
- `warpctrl project active [selectors]`
- `warpctrl project list [selectors]`
Authenticated read-only Warp Drive metadata and data reads, enabled only when the selected app has a logged-in Warp user and the grant allows authenticated reads. Listing is metadata; inspecting object content is an underlying data read:
- `warpctrl drive list --type <workflow|notebook|env-var-collection|prompt|folder> [selectors]`
- `warpctrl drive inspect <id> [selectors]`
### Mutating command set
The stacked branch `zach/warp-cli-read-write` should build on `zach/warp-cli-readonly` and implement the following mutating commands. Mutating commands are split by what they mutate: app-state, metadata/configuration, or underlying data. Underlying data mutations require a separate and stronger permission than app-state or metadata/configuration mutations.
App-state mutations for app, window, and surfaces:
- `warpctrl app focus [selectors]`
- `warpctrl window create [--shell <name>] [selectors]`
- `warpctrl window focus --window <selector> [selectors]`
- `warpctrl window close --window <selector> [selectors]`
- `warpctrl surface settings open [--page <page>] [--query <query>] [selectors]`
- `warpctrl surface command-palette open [--query <query>] [selectors]`
- `warpctrl surface command-search open [--query <query>] [selectors]`
- `warpctrl surface warp-drive open [selectors]`
- `warpctrl surface warp-drive toggle [selectors]`
- `warpctrl surface resource-center toggle [selectors]`
- `warpctrl surface ai-assistant toggle [selectors]`
- `warpctrl surface code-review toggle [selectors]`
- `warpctrl surface left-panel toggle [selectors]`
- `warpctrl surface right-panel toggle [selectors]`
- `warpctrl surface vertical-tabs toggle [selectors]`
App-state mutations for tabs:
- `warpctrl tab create [--type terminal|agent|cloud-agent|default] [--shell <name>] [selectors]`
- `warpctrl tab activate --tab <selector> [selectors]`
- `warpctrl tab activate --previous [selectors]`
- `warpctrl tab activate --next [selectors]`
- `warpctrl tab activate --last [selectors]`
- `warpctrl tab move --tab <selector> --direction <left|right> [selectors]`
- `warpctrl tab close --tab <selector> [selectors]`
- `warpctrl tab close --active [selectors]`
- `warpctrl tab close --others --tab <selector> [selectors]`
- `warpctrl tab close --right-of --tab <selector> [selectors]`
Metadata mutations for tabs:
- `warpctrl tab rename --tab <selector> <title> [selectors]`
- `warpctrl tab reset-name --tab <selector> [selectors]`
- `warpctrl tab color set --tab <selector> <color> [selectors]`
- `warpctrl tab color clear --tab <selector> [selectors]`
App-state mutations for panes:
- `warpctrl pane split --direction <left|right|up|down> [--shell <name>] [selectors]`
- `warpctrl pane focus --pane <selector> [selectors]`
- `warpctrl pane navigate --direction <left|right|up|down|previous|next> [selectors]`
- `warpctrl pane resize --direction <left|right|up|down> [--amount <cells>] [selectors]`
- `warpctrl pane maximize [--pane <selector>] [selectors]`
- `warpctrl pane unmaximize [selectors]`
- `warpctrl pane close --pane <selector> [selectors]`
Metadata mutations for panes:
- `warpctrl pane rename --pane <selector> <title> [selectors]`
- `warpctrl pane reset-name --pane <selector> [selectors]`
App-state mutations for sessions and input buffers:
- `warpctrl session activate --session <selector> [selectors]`
- `warpctrl session previous [selectors]`
- `warpctrl session next [selectors]`
- `warpctrl session reopen-closed [selectors]`
- `warpctrl input insert <text> [--session <selector>] [selectors]`
- `warpctrl input replace <text> [--session <selector>] [selectors]`
- `warpctrl input clear [--session <selector>] [selectors]`
- `warpctrl input mode set <terminal|agent> [--session <selector>] [selectors]`
Underlying data mutations for terminal execution:
- `warpctrl input run <command> [--session <selector>] [selectors]`
Metadata/configuration mutations for appearance and settings:
- `warpctrl theme set <theme_name> [selectors]`
- `warpctrl theme system set <true|false> [selectors]`
- `warpctrl theme light set <theme_name> [selectors]`
- `warpctrl theme dark set <theme_name> [selectors]`
- `warpctrl appearance font-size increase [selectors]`
- `warpctrl appearance font-size decrease [selectors]`
- `warpctrl appearance font-size reset [selectors]`
- `warpctrl appearance zoom increase [selectors]`
- `warpctrl appearance zoom decrease [selectors]`
- `warpctrl appearance zoom reset [selectors]`
- `warpctrl setting set <key> <value> [selectors]`
- `warpctrl setting toggle <key> [selectors]`
App-state mutations for files, projects, and Warp Drive views:
- `warpctrl file open <path> [--line <line>] [--column <column>] [--new-tab] [selectors]`
- `warpctrl project open <path> [selectors]`
- `warpctrl drive open <id> [selectors]`
- `warpctrl drive notebook open <id> [selectors]`
- `warpctrl drive env-var-collection open <id> [selectors]`
Underlying data mutations for files and authenticated Warp Drive objects:
- `warpctrl file create <path> [--content <text>] [selectors]`
- `warpctrl file write <path> --content <text> [selectors]`
- `warpctrl file append <path> --content <text> [selectors]`
- `warpctrl file delete <path> [selectors]`
- `warpctrl drive workflow run <id> [--arg <name=value>...] [selectors]`
- `warpctrl drive object create --type <workflow|notebook|env-var-collection|prompt|folder> [selectors]`
- `warpctrl drive object update <id> [selectors]`
- `warpctrl drive object trash <id> [selectors]`
- `warpctrl drive object restore <id> [selectors]`
### Excluded from the public command surface
The command surface must continue to exclude debug-only, crash-only, auth-token, heap-dump, and arbitrary internal dispatch actions even when those actions are available in command palette or keybinding registries. Examples that remain excluded are app crash/panic helpers, access-token copy helpers, heap profile dumps, debug reset actions, raw view-tree debugging, and broad internal action-by-string execution.
## Built-in Warp Agent skill
Warp should include a built-in Agent skill for `warpctrl`, analogous to the bundled `oz-platform` skill. The skill should teach Warp Agent when to use `warpctrl`, how to discover and target running instances, how to prefer read-only commands before mutating commands, how to request explicit user approval for underlying data mutations, and how to interpret structured errors. The skill should document the stable command hierarchy above, include concise recipes for common automation tasks, and avoid instructing agents to bypass the CLI by calling local-control HTTP endpoints directly.
## Action classification and permission model
Agents, scripts, and human developers are expected to be major consumers of `warpctrl`. The action catalog must therefore classify every action by risk posture, state/data category, permission category, and authenticated-user requirement so Warp can enforce local-control permissions in the app bridge.
Every action definition must include:
- a stable action name and namespace;
- a risk posture;
- a state/data category: metadata read, underlying data read, app-state mutation, metadata/configuration mutation, or underlying data mutation;
- whether a true logged-in Warp user is required;
- whether the action may run from external clients, verified Warp-terminal clients, or both;
- whether inside-Warp and outside-Warp scripting settings can enable the action;
- the required local-control permission category;
- any target-scope restrictions.
By default, new actions require an authenticated Warp user. An action may be marked logged-out-safe only after deliberate review confirms it does not touch Warp Drive, AI conversation traces, synced settings, team/account data, cloud-backed user state, terminal content, or other user-sensitive data.
### Permission categories
Every action in the catalog belongs to exactly one of the following permission categories, from least to most sensitive:
1. **Read-only / metadata.** Actions that return local app structure, app state, or configuration metadata without exposing terminal content, file content, Warp Drive object content, AI conversation content, or other user data.
   - Instance discovery and health: `instance list`, `app active`, `app version`, `app ping`.
   - Layout enumeration: `window list`, `tab list`, `pane list`, `session list`.
   - Metadata reads: `theme list`, `setting list`, `keybinding list`, `action list`, `project active`, and Drive object listing that returns object IDs/names/types but not content.
2. **Read-only / underlying data.** Actions that return user content or data-bearing state without changing it.
   - Terminal reads: block output, scrollback, command history, input editor contents, session replay, or terminal-derived traces.
   - File reads, Warp Drive object content reads, AI conversation reads, and any authenticated-user data read.
   This category is separate from metadata because read-only content can contain secrets, source code, customer data, command output, or other sensitive data.
3. **Mutating / app state.** Actions that change visible local Warp UI state without directly changing underlying user data.
   - Layout and focus: `window create`, `window focus`, `tab create`, `tab activate`, `tab move`, `window close`, `tab close`, `pane split`, `pane focus`, `pane navigate`, `pane maximize`, `pane resize`, and panel/surface toggles.
   - Input-buffer staging: `input insert`, `input replace`, and `input clear` as long as they do not submit or execute the buffer.
   - Opening views: opening settings, command palette, command search, Warp Drive, code review, files, projects, notebooks, and env-var collections.
4. **Mutating / metadata or configuration.** Actions that change persistent metadata or configuration but do not directly mutate primary user data.
   - Tab and pane names, tab colors, themes, system-theme settings, font size, zoom, allowlisted app settings, and keybindings.
   Metadata/configuration writes need a stronger permission than app-state-only changes because they persist beyond the current UI interaction, but they are still distinct from data writes.
5. **Mutating / underlying data.** Actions that can change user data, execute code, or cause external side effects.
   - Terminal execution: `input run`, workflow execution in a terminal session, and any command execution path.
   - File writes: create, write, append, delete, rename, or otherwise modify local files.
   - Warp Drive CRUD: create, update, trash, restore, permanently delete, or otherwise mutate workflows, notebooks, prompts, env-var collections, folders, or other Drive objects.
   - AI conversation history mutation and any action that modifies cloud-backed user content.
   This category must be explicitly separate from app-state mutation. A client allowed to open or focus Warp UI must not automatically be allowed to execute commands, write files, or mutate Warp Drive content.
### Authenticated-user requirement
An authenticated user means a true logged-in Warp user in the selected Warp app, not merely the local OS user or a `warpctrl` process authenticated to localhost.
The allowlist must clearly indicate `requires_authenticated_user` for every action:
- `false` only for logged-out-safe actions that operate on local app structure, local appearance metadata, or local-only settings that do not expose user-sensitive data.
- `true` for actions that read or mutate Warp Drive, AI conversation traces, synced settings, team/account data, user identity data, or any cloud-backed Warp state.
- `true` for actions that execute user-authored Warp Drive content, even if the execution target is a local terminal session.
If an authenticated-user action is invoked while the selected app has no logged-in user, the CLI reports a structured authenticated-user error. It must not silently return partial logged-out data as success.
### Execution context policy
`warpctrl` should distinguish verified invocations from inside Warp-managed terminal sessions from external invocations.
- **Verified Warp-terminal invocation:** a `warpctrl` process started inside a Warp-managed terminal session and able to present an app-issued execution-context proof. The top-level setting for this context should default to on. When the selected app has a logged-in Warp user, this context can receive authenticated-user grants if the user's Scripting permissions allow that grant.
- **External invocation:** a `warpctrl` process started outside Warp's terminal, such as from another terminal app, launch agent, IDE, or background script. The top-level setting for this context must default to off. When disabled, external invocations receive no local-control credentials, including logged-out-safe metadata credentials.
- The app must not trust a caller-declared label. Environment variables may help discover the context, but the broker must verify a session-bound capability or equivalent proof before issuing in-Warp-only grants.
### Settings surface
Warp should add a new top-level Settings pane page named **Scripting**. This page should own settings for local scripting and automation surfaces, including Warp control. For Warp control, it should include two top-level toggles:
- **Allow Warp control from inside Warp:** default on. Controls `warpctrl` invocations from verified Warp-managed terminal sessions.
- **Allow Warp control from outside Warp:** default off. Controls `warpctrl` invocations from external terminals, scripts, IDEs, launch agents, and other same-user processes.
The Scripting page should explain that inside-Warp control is scoped to commands launched from Warp-managed terminals, while outside-Warp control allows other local apps and scripts to talk to Warp's control plane. Disabling either top-level toggle should invalidate credentials for that invocation context.
### Granular local-control permissions
The Scripting settings page should expose granular permissions beneath the inside-Warp and outside-Warp toggles. Recommended controls:
- Allow metadata reads.
- Allow underlying data reads.
- Allow app-state mutations.
- Allow metadata/configuration mutations.
- Allow underlying data mutations.
- Allow authenticated-user actions from verified Warp terminals.
- Allow authenticated-user actions from external clients, default off and separate from the in-Warp permission.
These settings define the maximum grants the broker may issue. The app bridge still enforces the action's risk posture, state/data category, authenticated-user requirement, execution-context requirement, and target scope for every request. Enabling app-state mutation must not imply permission to mutate underlying data.
### Scoped credentials
The local discovery record must not expose a reusable full-access credential. `warpctrl` should request scoped credentials from an app-owned broker or equivalent trusted path.
Scoped credentials should include:
- the selected Warp instance;
- granted permission categories;
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
- `warpctrl file open <path>` — app-state mutation that opens a file in a Warp editor tab, equivalent to clicking a file link.
- `warpctrl file open <path> --line <n>` — app-state mutation that opens at a specific line.
- `warpctrl file list` — metadata read that lists files currently open in editor tabs across the instance.
- `warpctrl file read <path>` — underlying data read that returns file contents.
- `warpctrl file create|write|append|delete <path>` — underlying data mutations that modify the filesystem.
File selectors would use filesystem paths (absolute or relative to the working directory of the target pane/session). Unlike window/tab/pane selectors, file selectors are not opaque IDs — they are user-visible paths. The protocol should support a `file` field in the target selector that accepts a path string, distinct from the opaque ID selectors used for windows, tabs, and panes.
**Warp Drive objects.** Warp Drive stores typed objects (workflows, notebooks, environment variable sets, prompts) that users can reference, execute, and share. A future `drive` namespace could support:
- `warpctrl drive list --type workflow` — authenticated metadata read that lists Warp Drive objects by type.
- `warpctrl drive inspect <id>` — authenticated underlying data read when it returns object content.
- `warpctrl drive workflow run <workflow-id>` — authenticated underlying data mutation that executes a workflow in a target session.
- `warpctrl drive object create|update|trash|restore <id>` — authenticated underlying data mutations that change cloud-backed user content.
- `warpctrl drive notebook open <notebook-id>` — app-state mutation that opens a view of an existing notebook without modifying it.
Drive object selectors should support both opaque IDs (for automation stability) and human-friendly name/path lookups (for interactive use). The type field (`workflow`, `notebook`, `env_var`, `prompt`) acts as a namespace filter. Drive actions that execute content in a terminal session (e.g., running a workflow) inherit the underlying-data-mutation permission from the action classification model.
**Design constraints for both:**
- File and Drive selectors are orthogonal to the window/tab/pane hierarchy — a file open action targets an instance (which window to open in), not a specific pane. A Drive workflow execution targets a session (which pane to run in).
- The `TargetSelector` type in the protocol should be extensible with optional fields for these new selector families without breaking existing requests that omit them.
- The action classification categories apply, and Drive actions require authenticated-user grants by default: listing Drive objects is metadata plus authenticated user, reading Drive object content is underlying-data-read plus authenticated user, opening an existing Drive object in the app is app-state mutation plus authenticated user, and executing or changing a Drive object is underlying-data-mutation plus authenticated user.
### Settings: protocol-first
Settings reads and writes should go through the local-control protocol like other actions, not bypass it via direct file manipulation.
- `warpctrl setting get <key>`, `warpctrl setting set <key> <value>`, and `warpctrl setting toggle <key>` send requests to the running Warp instance through the standard authenticated control endpoint.
- The app bridge validates the key against the allowlist and the value against the expected type before applying the change.
- This keeps authorization enforcement consistent: the same permission category, execution-context, and authenticated-user policies apply to settings mutations as to any other action, rather than creating a second unguarded path through the filesystem.
- The app owns the write to the settings file and any side effects (e.g., theme reload, layout reflow) as a single atomic operation, avoiding races between a CLI file write and the app's file watcher.
- If a future need arises for offline settings manipulation (no running Warp process), a separate file-based path can be added later with its own validation, but it should not be the default.
- The action classification still applies: settings reads are metadata reads, and settings writes are metadata/configuration mutations. Settings writes must not be authorized by app-state mutation permission alone.
