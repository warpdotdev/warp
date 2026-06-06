# Summary
Warp should ship an allowlisted local control CLI command, provisionally named `warpctrl`, that acts as an agent control plane for operating Warp itself. `warpctrl` is exposed as an Oz-style wrapper script that invokes the existing channel-specific Warp binary in control mode rather than as a separate standalone binary. `warpctrl` lets agents and developers script the same classes of user-visible actions they can already perform inside the running app: manipulating windows, tabs, panes, sessions, terminal blocks, appearance, settings, Warp Drive views, and selected UI surfaces. The CLI should operate against one or more already-running local Warp app processes through a stable machine protocol, with deterministic target selection and clear errors when a process or target is ambiguous.
## Problem
Warp already has rich interactive actions, but they are primarily reachable through UI, keybindings, menus, or deeplinks. Agents can use native tools for files, code, shell commands, MCP calls, and many context reads, but they cannot reliably operate Warp's own product surfaces: arranging the user's workspace, focusing the correct pane, opening Warp Drive objects, presenting settings, or recovering from ambiguous UI state. Developers also cannot reliably compose those same actions into shell scripts, demos, automation, or agent workflows, and there is no general local protocol for addressing a specific running Warp instance, window, pane, session, terminal block, Warp Drive object, or other uniquely named Warp entity.
## Goals / Non-goals
Goals:
- Provide a first-class, scriptable `warpctrl` command for controlling running Warp app processes.
- Make Warp's own UI and app state available to agents through a typed, permissioned control plane instead of brittle screen automation or arbitrary internal dispatch.
- Keep CLI startup lightweight by avoiding GUI-app startup or full terminal initialization for routine control commands.
- Keep the surface allowlisted and finite instead of exposing arbitrary internal actions.
- Make targeting explicit and deterministic across multiple Warp processes, windows, tabs, panes, terminal sessions, terminal blocks, Warp Drive objects, files, command surfaces, and other uniquely addressable Warp nouns.
- Support both ergonomic active-target defaults and precise selectors for automation.
- Define a complete protocol/catalog up front, while shipping the implementation incrementally.
Non-goals:
- Replacing the Oz CLI or mixing cloud-agent management into this CLI.
- Exposing every internal app action, debug action, developer-only helper, or privileged state mutation.
- Treating the CLI as a general RPC escape hatch into Warp internals.
- Replacing native agent tools for code editing, file operations, shell execution, web/MCP calls, or attached conversation/block context when those tools already solve the task better.
- Requiring developers or automation to directly invoke the Warp app executable path for ordinary control commands; the packaged `warpctrl` wrapper should hide that implementation detail.
- Requiring the first implementation slice to ship every action in the catalog.
## Primary user stories
These stories define the most compelling product uses for `warpctrl`. The command catalog below is intentionally broader, but the product should prioritize surfaces that agents cannot already operate well through native tools.
1. **Agent workspace orchestration.** When a user asks an agent to work on a task, the agent can inspect the current Warp state, create or reuse an appropriate window/tab layout, split panes, name and focus targets, open relevant Warp surfaces, and leave the workspace in a readable task-shaped state for the user. The agent should continue to use native tools for code edits, file reads/writes, shell execution, MCP calls, and other work that does not require operating Warp's UI or local-control authorization model.
2. **Existing-session debugging and repair.** When a user asks for help with an existing Warp session, the agent can understand Warp-specific UI and session structure before acting: which instance/window/tab/pane/session is active, whether the relevant pane still exists, whether the correct surface is focused, which panels or settings pages are open, and which selector should be used for follow-up actions. The story should focus on UI/session structure, focus, panels, settings, and deterministic targeting; native agent context tools should remain the preferred way to read attached blocks, conversations, and other content when they are available.
3. **Warp Drive creation, navigation, and sharing.** When an agent notices reusable knowledge during normal work, it can help the user turn that knowledge into a Warp Drive object, open it for review, and guide sharing with the right scope. This includes workflows from repeated command sequences, notebooks from task writeups, prompts/rules/facts from user or project preferences, environment variable collections, MCP setup objects, folders, and spaces. Existing object navigation remains important, but creation and sharing are first-class because reusable team knowledge cannot be used until users are guided into creating it.
4. **Deterministic demos and walkthroughs.** When a user, teammate, or go-to-market workflow needs a reliable Warp demo, an agent or script can put Warp into a known presentation state: theme, zoom, windows, tabs, panes, focused targets, panels, command palette/search, and Warp Drive surfaces. The walkthrough can then advance using structured target IDs and recover from stale or missing targets instead of relying on screen coordinates, manual setup, or brittle UI automation.
5. **Personalization, onboarding, and preference migration.** When a user wants Warp to feel familiar, an agent can inspect user-approved settings from tools such as VS Code, iTerm, Ghostty, or shell configuration, propose Warp equivalents, apply allowlisted changes through `warpctrl`, and report unsupported mappings explicitly instead of guessing. The same flow can support team onboarding presets, presentation preferences, accessibility-related settings, themes, font and zoom, keybindings, notifications, and panels.
Human power-user scripting is a secondary beneficiary of the same design. Scripts get reliable JSON, target selectors, structured errors, and exact-action credentials because the API is strong enough for agents, but the primary product narrative remains agent-led operation of Warp itself.
Persistent settings changes, Warp Drive creation or sharing, cross-app preference migration, terminal command execution, and other actions with durable or external effects must be visibly reviewable or require explicit action-specific authorization. `warpctrl` should support full typed control over time, but each command must be progressively unlocked through exact-action grants, deterministic target resolution, Agent Profile policy, Scripting settings, authenticated-user requirements, and action-specific approval rather than broad unchecked authority.
## Behavior
1. The Warp control CLI operates only on running local Warp app processes. If no compatible Warp process is available, the CLI exits non-zero with a clear “no running Warp instance found” error.
2. The CLI exposes only explicitly allowlisted actions. Protocol-level unknown action names, unsupported local-control parameter combinations, or requests for non-allowlisted capabilities fail with structured local-control errors; they are never forwarded to arbitrary internal dispatch. Clap parser usage errors, such as an unknown CLI subcommand or invalid flag syntax, may use the parser's normal CLI error behavior unless a later branch explicitly wraps them.
3. Every successful mutating request identifies:
   - The Warp process instance that executed it.
   - The resolved target, when the action addresses a window, tab, pane, terminal session, terminal block, file, Warp Drive object, surface, or other targetable noun.
   - A success payload suitable for JSON output.
4. Every protocol or runtime local-control failure identifies:
   - A stable machine-readable error code.
   - A human-readable explanation.
   - Any selector that was ambiguous, missing, stale, unsupported, or invalid.
5. The CLI supports human-readable output by default and JSON output for scripts. JSON output has stable field names and is available for discovery commands, read commands, successful mutations, and protocol or runtime local-control failures.
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
   - `warpctrl block list`
   - `warpctrl drive list`
   - `warpctrl app active`
   These commands return opaque protocol-facing IDs and enough metadata for subsequent commands without requiring knowledge of internal Warp identifiers.
8. The target selector model is hierarchical:
   - Instance selector resolves a running Warp process.
   - Window selector resolves within the instance.
   - Tab selector resolves within the window.
   - Pane selector resolves within the tab or active pane group context.
   - Session selector resolves within the pane when the pane hosts terminal session state.
   - Block selector resolves within the terminal session when the command is block-scoped.
   Non-hierarchical selectors such as file paths, Warp Drive objects, and global app surfaces still resolve inside the selected instance and must not silently borrow lower-level pane/session defaults unless the action definition explicitly requires that scope.
9. Every selector family supports an ergonomic `active` form when that concept exists:
   - Active instance, if unambiguous.
   - Active window in the selected instance.
   - Active tab in the selected window.
   - Active pane in the selected tab.
   - Active session in the selected pane.
   - Active or selected terminal block in the selected session when a current block is unambiguous.
   For window-scoped mutations, an omitted or active window selector may fall back to the sole existing window when no active window is reported, because that target is still unambiguous. If there are no windows, the request fails with `missing_target`; if multiple windows exist and none is active, it fails with `ambiguous_target`.
10. Every selector family supports explicit opaque IDs returned by introspection. Selector families may also support scoped indices, titles/names, or paths where those concepts are already user-visible, but IDs remain the preferred automation surface.
   - Window selectors support `active`, opaque window IDs, window indices from `window list`, and exact window titles for interactive use.
   - Tab selectors support `active`, opaque tab IDs, tab indices scoped to the resolved window, and exact tab titles for interactive use.
   - Pane selectors support `active`, opaque pane IDs, and pane indices scoped to the resolved tab or pane group.
   - Session selectors support `active`, opaque session IDs, and session indices scoped to the resolved pane when sessions are user-visible as an ordered list.
   - Block selectors support `active`, opaque block IDs, and block indices scoped to the resolved terminal session when blocks are user-visible as an ordered list. A block command may also support read-only filters such as command text, status, time range, or “last completed” for interactive lookup, but those filters must fail on ambiguity and resolve to concrete block IDs before reading output.
   - File selectors use paths, plus optional line/column coordinates where the command supports opening a location.
   - Warp Drive selectors use opaque object IDs, with optional type-scoped exact name/path lookups for interactive use. Type scopes must include the user-facing object families Warp exposes today: spaces, folders, notebooks, workflows, agent-mode workflows/prompts, environment variable collections, AI facts/rules, MCP servers, MCP server collections, and trash entries when trash operations are supported.
11. “Active session” means the currently selected terminal session for the resolved pane/window context. If the selected target does not contain a terminal session, session-scoped actions fail rather than silently redirecting elsewhere.
12. When a command omits lower-level selectors, it resolves them from the chosen higher-level context using active defaults. Example: a pane split command with only `--instance` uses that instance’s active window, active tab, and active pane.
13. When an explicitly supplied target disappears between discovery and execution, the request fails with a stale-target error. The CLI must not silently choose a different tab, pane, or session.
14. The protocol is command-oriented, not open-ended state mutation. Each action has a named command, validated parameters, and defined target scope.
15. The complete allowlisted action catalog should be organized around stable public nouns rather than internal view/action names. The target taxonomy includes instances, windows, tabs, panes, terminal sessions, terminal blocks, input buffers, command history entries, file/path intents, Warp Drive spaces, folders, notebooks, workflows, agent-mode workflows/prompts, environment variable collections, AI facts/rules, MCP servers, MCP server collections, settings, themes, keybindings, command surfaces such as the command palette and command search, panels/surfaces such as Warp Drive, resource center, AI assistant, code review, left/right panels, and vertical tabs, plus action/capability metadata. The initial implementation may expose only a subset, but new command families should extend this noun taxonomy instead of inventing unrelated selector conventions.
16. Discovery and read-only state actions:
   - List instances.
   - Get protocol and build identity metadata for one instance.
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
   - Switch input mode between terminal and agent modes only where that mode switch is already user-visible and valid for the selected target.
   Input staging commands must not submit terminal input or press Enter. The separate `input run` execution action may submit a command only in the later execution-underlying branch, after authenticated scripting identity, an exact `input.run` grant, approval or configured policy, audit coverage, and explicit target resolution are implemented. Accepted-command submission and agent-prompt submission remain future protocol concepts that require separate product/security review.
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
26. The following actions are explicitly excluded from the public allowlist even when internal implementations exist:
   - Crash, panic, heap-dump, token-copying, debug-reset, and similar developer/debug helpers.
   - Arbitrary auth manipulation outside the explicit authenticated-scripting flows.
   - Arbitrary cloud object mutation or broad Warp Drive CRUD outside the typed Drive actions in this spec.
   - Arbitrary internal view dispatch by string.
   - Arbitrary setting names outside the allowlist.
   - Accepted-command submission and agent-prompt submission until they receive a separate product/security review.
   Terminal command execution and typed Warp Drive object mutations are no longer excluded from the full product scope, but they belong to later authenticated underlying-data mutation branches and require stronger authenticated-user, approval, targeting, and audit requirements than ordinary UI actions. Local file content reads, writes, appends, deletes, and other filesystem-content mutations are excluded from the public `warpctrl` catalog; file/path support is limited to app-state intents such as opening a path in Warp and metadata reads of files already open in Warp.
27. CLI command names should be noun-oriented and discoverable. During the provisional wrapper-script phase, the control CLI should expose a `warpctrl ...` command surface:
   - `warpctrl instance list`
   - `warpctrl app ping`
   - `warpctrl app version`
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
   - Issuing a session-scoped action against a non-terminal pane.
   - Focusing a window that has closed.
   - Setting a theme that is not available in that instance.
33. The first `warpctrl` implementation slice should ship the smallest end-to-end vertical slice that proves:
   - The current implementation supports outside-Warp local-control requests only; verified inside-Warp requests are specified for future work and rejected until the app-issued terminal proof broker exists.
   - The authoritative local-control mode is read only from protected storage, never imported from ordinary or private preferences, and defaults to disabled when no valid protected value is available.
   - Process discovery and target resolution work.
   - The wrapper-script command can reach a running local Warp process through the existing Warp binary's early control-mode dispatch without launching or initializing the GUI app.
   - `warpctrl tab create` creates a new terminal tab in the selected running instance.
   - The command returns a structured success or failure payload suitable for human-readable and JSON output.
   The first slice should include the minimum health/introspection commands needed to discover a running instance and exercise `tab.create`.
34. Follow-up PRs should fill out the remaining catalog in parallelizable groups once the protocol, discovery model, target resolution, error model, `tab.create` action path, and wrapper-script `warpctrl` packaging shape have been validated by the first slice.
35. The protocol transport should be designed so that the default target is localhost but the CLI can be extended in the future to target remote URLs (e.g., a Warp instance on another machine or a hosted control endpoint). This is not in scope for the first implementation but should not be precluded by the architecture.
## API command surface
The public `warpctrl` API is organized around nouns that map to stable user-facing entities. Command names are intentionally not a dump of every internal `WorkspaceAction`, `TerminalAction`, keybinding, or command-palette binding. Internal actions inform the catalog, but a command is added only when it has a stable user-facing behavior, typed parameters, deterministic target resolution, and explicit authorization requirements.
Catalog support status is part of the public API contract. An action reported as `implemented` by `warpctrl action list --implemented-only`, `warpctrl capability list --implemented-only`, or app discovery metadata must be reachable through the wrapper-backed `warpctrl ...` parser route, represented in generated help/completions/docs, and backed by an app-side bridge handler in the selected app build. Planned actions without that complete path must be reported as stubs or planned entries, even if an internal app handler already exists.
### Direct action requirements
Every public action must declare the policy inputs that the broker and app bridge actually enforce: stable action identity, implementation status, authenticated-user requirement, allowed invocation contexts, target scope, typed parameters, and typed result. Credentials authorize one exact action; authority for one action never implies authority for another action, even when the actions have similar effects.
Sensitive actions carry stronger requirements directly. Actions that expose terminal output or other user content may require authenticated-user access or explicit approval. Actions that execute commands, mutate or share Warp Drive objects, change persistent settings, or cause external side effects require the identity, invocation context, target restrictions, approval, and audit coverage specified for that exact action. Opening or focusing Warp UI must never imply authority to execute commands or mutate user data.
### Targeting flags
The full product should converge on shared selector flags for every command that addresses a running app target. The current foundation branch is not required to expose that complete CLI grammar yet: it supports instance selection with `--instance` and `--pid` for the implemented commands, while the shared window/tab/pane/session/block selector flags are deferred to the later target-selector branch that implements those target families. When the shared grammar ships, generic `--window`, `--tab`, `--pane`, `--session`, and `--block` flags accept the selector grammar below; explicit typed aliases are provided so scripts can avoid string parsing ambiguity:
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
- `--block <active|id:<id>|index:<n>>` selects a terminal block inside the resolved terminal session when the command is block-scoped.
- `--block-id <id>` and `--block-index <n>` are exact aliases for the corresponding `--block ...` forms.
- File commands use path arguments or `--path <path>` where the path is the selected file entity; `--line <n>` and `--column <n>` refine the location when supported.
- Drive commands use object ID arguments or `--drive-id <id>` where the ID is the selected Warp Drive entity; name/path lookup must be type-scoped when supported.
- `--output-format <pretty|json|ndjson|text>` controls output shape and remains globally available.
Within a selector family, specifying more than one form is invalid. For example, `--tab-id` conflicts with `--tab-index`, `--tab-title`, and `--tab`. Omitted lower-level selectors use active defaults only when the resolved target is unambiguous; window-scoped mutations may use the sole existing window when no active window is reported. Explicit IDs must resolve exactly or fail with `stale_target`; index/title/name/path selectors that match zero targets fail with `missing_target`, and selectors that match multiple targets fail with `ambiguous_target`.
### Read-only command set
The read-only v2 follow-up branches should implement the following commands before mutating catalog expansion begins. `zach/warp-cli-v2/readonly-capability-targets` owns structural metadata and targeting, while `zach/warp-cli-v2/appstate-file-drive-views` owns approved underlying-data reads and app/file/Drive view surfaces. Read-only actions remain independently authorized: a credential for structural metadata does not authorize terminal output, input buffers, history, Drive content, or any other action.
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
Content-bearing reads, each gated by its own exact-action grant:
- `warpctrl block list [--session <selector>|--pane <selector>] [--limit <n>] [selectors]`
- `warpctrl block inspect --block <selector> [selectors]`
- `warpctrl block output --block <selector> [--plain|--ansi|--json] [selectors]`
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
Local file reads that expose only app/editor state, not arbitrary filesystem traversal:
- `warpctrl file list [selectors]`
Authenticated read-only Warp Drive metadata and data reads, enabled only when the selected app has a logged-in Warp user and the grant allows authenticated reads. Listing is metadata; inspecting object content is an underlying data read:
- `warpctrl drive list --type <workflow|notebook|env-var-collection|prompt|folder|ai-fact|mcp-server|space|trash> [selectors]`
- `warpctrl drive inspect <id> [selectors]`
### Authenticated scripting command set
Authenticated actions in the selected public contract are available only to verified Warp-terminal invocations. `warpctrl` presents the app-issued terminal proof described in `TECH.md` and may receive authenticated-user grants only when the selected app is logged into Warp and Settings > Scripting allows authenticated actions from verified Warp terminals. External API-key authenticated scripting and `auth.api_key.*` commands are not allowlisted; adding them requires a separate product/security review and catalog change.
Recommended CLI surface for app-backed authenticated status:
- `warpctrl auth status [selectors]` reports local-control auth state, selected app login state, and verified Warp-terminal authenticated grant availability.
- `warpctrl auth login [selectors]` focuses the selected Warp app's sign-in UI for interactive app-login flows.
### Mutating command set
The mutating v2 follow-up branches should build on the shared contract, auth/security, read-only, and targeting layers. `zach/warp-cli-v2/metadata-config-mutations` owns metadata/configuration mutations, `zach/warp-cli-v2/drive-data-mutations` owns Warp Drive underlying-data mutations, and `zach/warp-cli-v2/execution-underlying` owns terminal command execution and other approved execution-underlying actions. Approved app-state mutations and views land in the earliest v2 branch that owns their required targeting and direct policy prerequisites. Every mutating command requires its own exact-action grant. Commands that mutate user data, execute code, or cause external side effects additionally require authenticated scripting identity, explicit approval or configured action policy, deterministic targets, and audit coverage.
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
These input-buffer commands only stage or edit text and must not submit the buffer. The separate `input run` command belongs only to the execution-underlying branch and requires authenticated scripting identity, an exact `input.run` grant, explicit target resolution, approval or configured policy, and audit coverage. Accepted-command submission and agent-prompt submission remain excluded until separately reviewed.
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
App-state mutations for files and Warp Drive views:
- `warpctrl file open <path> [--line <line>] [--column <column>] [--new-tab] [selectors]`
- `warpctrl drive open <id> [selectors]`
- `warpctrl drive notebook open <id> [selectors]`
- `warpctrl drive env-var-collection open <id> [selectors]`
- `warpctrl drive object share open <id> [selectors]`
Underlying data mutations for authenticated Warp Drive objects:
- `warpctrl drive object create --type <workflow|notebook|env-var-collection|prompt|folder> [--content <text>|--content-file <path>] [selectors]`
- `warpctrl drive object update <id> [--content <text>|--content-file <path>] [selectors]`
- `warpctrl drive object delete <id> [selectors]`
- `warpctrl drive object insert <id> [--target <selector>] [selectors]`
- `warpctrl drive object share-to-team <id> [selectors]`
- `warpctrl drive workflow run <id> [--arg <name=value>...] [selectors]`
Execution-underlying actions:
- `warpctrl input run <command> [--session <selector>] [selectors]`
These are underlying-data mutations because they can modify user data, execute code, trigger external side effects, share cloud-backed content, or run user-authored content. They require authenticated scripting identity, exact-action grants, deterministic target resolution, approval or configured policy, audit records, and explicit tests proving credentials for other actions cannot run them. `drive object share-to-team` is the only direct sharing mutation in the v0 product scope: it may make a personal Warp Drive object available to the user's current team using the app's standard team-sharing semantics. Arbitrary ACL editing, sharing with specific users, sharing with external guests, public-link creation, accepted-command submission, and agent-prompt submission remain excluded until separately reviewed.
### Excluded from the public command surface
The command surface must continue to exclude debug-only, crash-only, auth-token, heap-dump, and arbitrary internal dispatch actions even when those actions are available in command palette or keybinding registries. Examples that remain excluded are app crash/panic helpers, access-token copy helpers, heap profile dumps, debug reset actions, raw view-tree debugging, and broad internal action-by-string execution.
## Branch stacking and delivery model
The Warp Control CLI work should ship as the active raw-git v2 branch stack so the shared contract, security enforcement, read-only expansion, mutating expansion, and final integration remain reviewable independently:
- `zach/warp-cli-v2/contract-spec-sync` is the bottom review branch and targets `master`. It exclusively owns the product, technical, security, and operator specs plus the shared contract/foundation and minimum first-slice smoke path.
- `zach/warp-cli-v2/auth-security` stacks on the contract branch and owns authentication and security enforcement shared across command families.
- `zach/warp-cli-v2/readonly-capability-targets` stacks on auth/security and owns structural metadata, capability/action metadata, selectors, opaque IDs, and read-only target resolution.
- `zach/warp-cli-v2/appstate-file-drive-views` stacks on read-only targeting and owns approved app-state, file-view, Drive-view, and underlying-data-read surfaces without adding local filesystem content operations.
- `zach/warp-cli-v2/metadata-config-mutations` stacks on the approved view/read surfaces and owns allowlisted metadata and configuration mutations.
- `zach/warp-cli-v2/drive-data-mutations` stacks on metadata/configuration mutations and owns authenticated Warp Drive underlying-data mutations.
- `zach/warp-cli-v2/execution-underlying` stacks on Drive mutations and owns authenticated execution-underlying actions.
- `zach/warp-cli-v2/cli-catalog-docs` stacks on the action-family branches and owns final CLI, catalog, completion, documentation, and action-review consistency.
- `zach/warp-cli-v2/fanin-finalize` is the final integration branch used for end-to-end validation and review handoff.
Older pre-recovery branch names are historical source material only and must not be used as the active PR stack. New spec changes originate on `zach/warp-cli-v2/contract-spec-sync` and are propagated upward through raw-git rebases so all higher v2 branches reflect the same product/security contract. Graphite is not part of this stack. If a lower branch merges first, higher branches should rebase onto the merged successor while preserving the approved spec content.
## Built-in Warp Agent skill
Warp should include a built-in Agent skill for `warpctrl`, analogous to the bundled `oz-platform` skill. The skill should teach Warp Agent when to use `warpctrl`, how to discover and target running instances, how to prefer read-only commands before mutating commands, how to request explicit user approval for underlying data mutations, and how to interpret structured errors. The skill should document the stable command hierarchy above, include concise recipes for common automation tasks, and avoid instructing agents to bypass the CLI by calling local-control HTTP endpoints directly.
## CLI implementation and documentation conventions
`warpctrl` should feel consistent with the Oz CLI from a developer's perspective and use the same CLI libraries and conventions:
- Argument parsing, subcommand structure, help text, and shell-completion generation should use the same `clap`/`clap_complete` patterns used by the Oz CLI.
- JSON serialization and machine-readable output should use the same `serde`/`serde_json` conventions and the same output-format vocabulary used by the Oz CLI.
- Human-readable help, examples, errors, and generated completions should follow Oz CLI conventions unless Warp Control has a documented product reason to differ.
CLI documentation should be generated from the command catalog instead of maintained by hand in multiple places:
- The typed action catalog is the source of truth for command names, selector flags, parameters, output formats, authenticated-user requirement, allowed invocation contexts, target scope, support status, and examples.
- `warpctrl help`, shell completions, markdown reference docs, the built-in Warp Agent skill, and the operator README should be generated or checked from that catalog so they cannot drift silently.
- A later branch should add native Warp completions for `warpctrl` in addition to shell completions so Warp can suggest commands, flags, selectors, and action names directly in the input editor.
- Generated documentation must distinguish implemented commands from planned catalog entries. A command may appear in specs as planned, but public operator docs must not imply it is usable until the selected app build advertises support for it.
- CI or presubmit checks should fail when CLI parser/help output, generated reference docs, completions, or the built-in skill are stale relative to the command catalog.
## Exact-action authorization model
Agents, scripts, and human developers are expected to be major consumers of `warpctrl`. Every credential therefore authorizes one exact typed action, and the app bridge verifies that exact action before selector resolution or handler dispatch. Similar actions do not inherit authority from one another.
Every action definition must include:
- a stable action name and namespace;
- whether a true logged-in Warp user is required;
- whether the action may run from external clients, verified Warp-terminal clients, or both;
- whether inside-Warp and outside-Warp scripting settings can enable the action;
- any target-scope restrictions;
- typed parameter and result contracts;
- any action-specific approval, audit, or policy requirements.
By default, new actions require an authenticated Warp user. An action may be marked logged-out-safe only after deliberate review confirms it does not touch Warp Drive, AI conversation traces, synced settings, team/account data, cloud-backed user state, terminal content, or other user-sensitive data. Actions that expose sensitive content, mutate durable data, execute code, or cause external effects require stronger conditions attached directly to the action.
### Authenticated scripting model
Authenticated scripting is required for any command that acts on a true Warp user identity or performs underlying-data mutation. Local-control credentials prove that a process may talk to the selected app; authenticated scripting credentials prove which logged-in Warp user is allowed to request user-backed or high-risk actions.
Inside Warp, authenticated scripting uses the verified terminal proof flow: the selected app is already logged in, the terminal proof binds the CLI to a live Warp-managed session, and the broker may mint an authenticated-user grant for that app user when Settings > Scripting allows it.
Outside-Warp invocations are limited to actions explicitly classified as logged-out-safe. External authenticated scripting is not part of the selected public contract.
### Authenticated-user requirement
An authenticated user means a true logged-in Warp user in the selected Warp app, not merely the local OS user or a `warpctrl` process authenticated to localhost.
The allowlist must clearly indicate `requires_authenticated_user` for every action:
- `false` only for logged-out-safe actions that operate on local app structure, local appearance metadata, or local-only settings that do not expose user-sensitive data.
- `true` for actions that read or mutate Warp Drive, AI conversation traces, synced settings, team/account data, user identity data, or any cloud-backed Warp state.
- `true` for actions that execute user-authored Warp Drive content, even if the execution target is a local terminal session.
If an authenticated-user action is invoked while the selected app has no logged-in user, the CLI reports a structured authenticated-user error. It must not silently return partial logged-out data as success.
### Warp Control authenticated scripting protocol
Authenticated scripting relies on the logged-in user in the selected Warp app and verified terminal proof. The CLI should expose app-backed auth/status flows:
- `warpctrl auth status [selectors]` reports whether the selected Warp app is logged in and returns a stable, non-secret user subject/identity summary when the caller has the required local-control grant.
- `warpctrl auth login [selectors]` does not collect credentials in the CLI or mint a separate CLI account session. It focuses or opens the selected Warp app's normal sign-in UI and waits, or exits with instructions, until the user completes sign-in in that app.
- After app login completes, the app-side credential broker may mint an app-user grant only for the same user subject that is currently logged in to the selected app and a verified Warp-terminal invocation.
- Authenticated credentials are bound to the selected app instance, subject, grant mode, scopes, expiry, and optional target/resource restrictions. If the app logs out, switches users, loses auth state, or the grant's subject no longer matches a grant that requires the selected app's logged-in subject, authenticated actions fail with a structured authenticated-user error rather than using stale authority.
- Raw Firebase, server, OAuth, and cloud API tokens are never exported to `warpctrl` output, shell scripts, generated docs, logs, discovery records, or JSON responses.
This authenticated scripting protocol applies only to actions whose allowlist entry requires a true logged-in Warp user or underlying-data-mutation authority. Logged-out-safe local actions continue to use local-control credentials without requiring Warp account login.
### Execution context policy
`warpctrl` should eventually distinguish verified invocations from inside Warp-managed terminal sessions from external invocations. The current foundation branch implements the setting shape for both contexts, supports external invocation only when the user explicitly enables the broadest mode, and must reject verified Warp-terminal claims until the proof broker is implemented.
- **Verified Warp-terminal invocation:** a `warpctrl` process started inside a Warp-managed terminal session and able to present an app-issued execution-context proof. This is allowed when the user selects **Enabled within Warp** or the broadest mode after the proof broker exists; the default disabled mode blocks it. When the selected app has a logged-in Warp user, this context can receive authenticated-user grants if the selected mode allows the context and the action's catalog policy allows that grant.
- **External invocation:** a `warpctrl` process started outside Warp's terminal, such as from another terminal app, launch agent, IDE, or background script. This is allowed only by **Enabled everywhere, including outside Warp**. When disabled for the selected mode, external invocations receive no local-control credentials, including logged-out-safe metadata credentials.
- The app must not trust a caller-declared label. Environment variables may help discover the context, but the broker must verify a session-bound capability or equivalent proof before issuing in-Warp-only grants.
### Settings surface
Warp should add a new top-level Settings pane page named **Scripting**. This page should own settings for local scripting and automation surfaces, including Warp control. Warp control should be represented as a single private, local-only mode setting with three choices:
- **Disabled:** default. No local-control invocation context can receive credentials.
- **Enabled within Warp:** allows only verified Warp-managed terminal invocations once the proof broker exists. In the current foundation branch, inside-Warp proof verification is not implemented yet, so requests in this mode are rejected rather than silently treated as external.
- **Enabled everywhere, including outside Warp:** allows verified Warp-managed terminal invocations and external local clients such as other terminals, scripts, IDEs, launch agents, and same-user automation to request local-control credentials.
The Scripting page should explain that the default mode blocks local-control credentials, the within-Warp mode is reserved for verified Warp-managed terminals once proof support lands, and the broadest mode allows other local apps and scripts to talk to Warp's control plane. Changing the mode should invalidate or prevent credentials for invocation contexts no longer allowed by the selected mode.
### Local-control action policy
The Scripting settings page should not expose separate risk-group toggles in the foundation stack. The single mode setting defines which invocation contexts may request credentials. For every request, the broker and app bridge still enforce the exact granted action, authenticated-user requirement, execution-context requirement, target restrictions, and any action-specific approval or audit policy. Enabling the broadest mode must not imply permission to run a different action or bypass authenticated scripting identity, logged-in user state, or future review.
### Agent Profile permissions
Agent Profiles should expose a dedicated **Warp control** permission group for agents that can invoke `warpctrl`. Profile policy should evaluate the requested exact action using the same autonomy vocabulary used by other Agent Profile permissions: allow, ask, let the agent decide based on confidence and risk, or deny. Profiles may offer curated UI groupings for usability, but those groupings must not become credential scopes or allow one approved action to authorize another.
Agent Profile permissions and global Scripting settings both apply. Settings > Scripting defines which invocation contexts may request local-control credentials. The selected Agent Profile determines whether that agent may request the specific action within that maximum. If either layer denies the action, authenticated-user requirement, or execution context, the request fails with a structured permission error instead of falling back to a weaker action or a raw `warpctrl` shell command.
The profile-level permission group should preserve the native-tools-first boundary. Agents should prefer native tools for code editing, file reads/writes, shell command execution, web/MCP calls, and attached conversation or block context when those tools are available. Agents should prefer `warpctrl` when the task requires operating Warp product surfaces, preserving visible UI context for the user, using Warp Drive as a first-class app surface, or applying the app's own permissioned control plane.
### Exact-action credentials
The local discovery record must not expose a reusable full-access credential. `warpctrl` requests a short-lived credential for the one typed action it is about to invoke from an app-owned broker or equivalent trusted path.
Exact-action credentials include:
- the selected Warp instance;
- the granted `ActionKind`;
- verified execution context;
- whether authenticated-user access is granted and for which logged-in user subject;
- optional target scopes;
- issuance and expiry metadata;
- revocation/audit identity.
The bridge, not the CLI frontend, enforces these grants. If a request presents a credential for a different action or otherwise exceeds its authority, the bridge returns `insufficient_permissions`, `authenticated_user_required`, `authenticated_user_unavailable`, or `execution_context_not_allowed` as appropriate.
### Future entity extensibility: files, blocks, and Warp Drive objects
The selector and action model should accommodate entity types beyond the current window/tab/pane/session hierarchy. Important entity families are **terminal blocks**, **file/path intents**, and **Warp Drive objects**. Broad Drive mutation and command execution are not in scope for the foundation branch, but they are in scope for later authenticated branches in the expanded stack. Local file content reads, writes, appends, deletes, and other filesystem-content mutations are intentionally out of scope for the public `warpctrl` catalog because native agent file tools are the preferred surface for file content operations. Agent-prompt submission remains excluded until separately reviewed.
**Terminal blocks.** Blocks are first-class targetable terminal entities, not just data hanging off a session. Block selectors should support the same addressing primitives as terminal sessions where meaningful: active/current block, opaque block ID, and block index scoped to the resolved session. Block reads can expose command text, output, status, timing, exit code, and metadata, so block content reads are underlying-data reads while block listing that returns only IDs/status/timestamps may be metadata reads. Stale, missing, or ambiguous block selectors must fail rather than selecting a neighboring block.
**Files.** Warp already supports file opening via deep links and the built-in editor. The `file` namespace is limited to app-state and metadata behaviors that operate Warp's visible UI:
- `warpctrl file open <path>` — app-state mutation that opens a file in a Warp editor tab, equivalent to clicking a file link.
- `warpctrl file open <path> --line <n>` — app-state mutation that opens at a specific line.
- `warpctrl file list` — metadata read that lists files currently open in editor tabs across the instance.
File selectors use filesystem paths (absolute or relative to the working directory of the target pane/session when the command defines that behavior). Unlike window/tab/pane selectors, file selectors are not opaque IDs — they are user-visible paths. The protocol should support a `file` field in the target selector that accepts a path string, distinct from the opaque ID selectors used for windows, tabs, and panes. `warpctrl` must not expose file content reads or filesystem-content mutations; agents and scripts should use native file tools for those operations.
**Warp Drive objects.** Warp Drive stores typed objects that users can reference, execute, edit, and share. The object taxonomy should include, at minimum, spaces, folders, notebooks, workflows, agent-mode workflows/prompts, environment variable collections, AI facts/rules, MCP servers, MCP server collections, and trash entries where trash operations are exposed. A future `drive` namespace could support:
- `warpctrl drive list --type workflow` — authenticated metadata read that lists Warp Drive objects by type.
- `warpctrl drive inspect <id>` — authenticated underlying data read when it returns object content.
- `warpctrl drive workflow run <workflow-id>` — authenticated underlying data mutation that executes a typed workflow in a target session, implemented only in the execution-underlying branch with authenticated scripting identity and audit coverage.
- `warpctrl drive object create|update|trash|restore <id>` — authenticated underlying data mutations that change cloud-backed user content.
- `warpctrl drive object share open <id>` — app-state mutation that opens the sharing dialog for user review without changing sharing state.
- `warpctrl drive object share-to-team <id>` — authenticated underlying data mutation that makes a personal object available to the user's current team using the app's standard team-sharing behavior. This is the only direct sharing mutation in the v0 product scope.
- `warpctrl drive notebook open <notebook-id>` — app-state mutation that opens a view of an existing notebook without modifying it.
Drive object selectors should support both opaque IDs (for automation stability) and human-friendly name/path lookups (for interactive use). The type field (`workflow`, `notebook`, `env_var_collection`, `prompt`, `folder`, `ai_fact`, `mcp_server`, etc.) acts as a namespace filter. Drive actions that execute content in a terminal session, such as running a workflow, require an exact grant for that execution action and are implemented only in the execution-underlying branch after authenticated scripting identity, approval policy, and audit coverage are in place.
**Design constraints for these future entity families:**
- File and Drive selectors are orthogonal to the window/tab/pane hierarchy — a file open action targets an instance (which window to open in), not a specific pane. A Drive workflow execution targets a session (which pane to run in).
- The `TargetSelector` type in the protocol should be extensible with optional fields for these new selector families without breaking existing requests that omit them.
- Drive actions require authenticated-user grants by default. Listing, reading content, opening a view, executing, sharing, and changing a Drive object are separate exact actions; a grant for any one of them does not authorize the others. Executing, sharing, or changing a Drive object additionally requires its action-specific approval and audit policy.
### Settings: protocol-first
Settings reads and writes should go through the local-control protocol like other actions, not bypass it via direct file manipulation.
- `warpctrl setting get <key>`, `warpctrl setting set <key> <value>`, and `warpctrl setting toggle <key>` send requests to the running Warp instance through the standard authenticated control endpoint.
- The app bridge validates the key against the allowlist and the value against the expected type before applying the change.
- This keeps authorization enforcement consistent: the exact requested setting action, execution context, authenticated-user requirement, and action-specific policy are checked like any other action, rather than creating a second unguarded path through the filesystem.
- The app owns the write to the settings file and any side effects (e.g., theme reload, layout reflow) as a single atomic operation, avoiding races between a direct settings-file edit and the app's file watcher.
- If a future need arises for offline settings manipulation (no running Warp process), a separate file-based path can be added later with its own validation, but it should not be the default.
- Settings reads and writes are separate exact actions. A credential for opening or focusing Warp UI must not authorize any settings write.
