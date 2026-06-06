# warpctrl security architecture
`warpctrl` is a local-control CLI for an already-running Warp app instance. Its security architecture is designed to support the control catalog: discovery, structural metadata reads, underlying data reads, app-state mutations, metadata/configuration mutations, underlying data mutations, input-buffer staging, file operations, and Warp Drive operations. Terminal command execution, workflow execution, accepted-command submission, and agent-prompt submission are future high-risk capabilities and must not be included in the initial public implementation; the initial version may stage text in an active input buffer only for the user to review and confirm manually.
The correct architecture is not a single shared localhost bearer token with client-side conventions. The CLI, app bridge, and protocol must treat security as a local app-enforced capability system: discovery finds compatible instances, secure storage protects raw credential material, broker-issued credentials identify the granted scopes, the running Warp app's local-control bridge enforces action categories before dispatch, and target resolution never silently retargets a request.
The action-category model is primarily a safety and intent mechanism, not a hard security boundary against malicious same-user software. It lets a user, script, or agent intentionally request metadata-only, data-read, app-state mutation, metadata/configuration mutation, or underlying-data mutation access so it does not accidentally mutate state, expose sensitive content, or execute commands. It should not be described as strong access control against a process that can already run arbitrary commands as the user.
`warpctrl` has two distinct authorization dimensions: local-control authority and Warp user authority. Local-control authority proves the request is allowed to control the local app. Warp user authority proves the selected Warp app has a real logged-in Warp user and the request is allowed to act on user-authenticated data such as Warp Drive objects, AI conversation traces, synced settings, or cloud-backed user state. Logged-out users should retain a smaller local-only control surface, but authenticated-user actions require a true logged-in Warp user in the selected app.
## Security goals
- Allow trusted local users and approved automation to control a running Warp instance through a stable, scriptable interface.
- Prevent unauthenticated localhost clients from invoking read or mutating control actions.
- Prevent browser-origin JavaScript from becoming an ambient localhost control client.
- Support multiple running Warp processes without a shared global mutating port or global credential.
- Separate discovery metadata from control authority so enumerating an instance does not automatically grant full control.
- Require explicit in-app user enablement before local control scripting from outside Warp can issue credentials or accept control requests.
- Allow local control scripting from verified Warp-managed terminal sessions by default, subject to granular permission settings.
- Store the authoritative enablement states in protected local storage so external apps cannot enable outside-Warp control by editing ordinary settings.
- Keep raw credential material out of plaintext discovery records and protect it with platform secure storage where available.
- Distinguish verified `warpctrl` invocations that originate from a Warp-managed terminal session from external same-user invocations.
- When outside-Warp control is enabled, allow external invocations only for a smaller local-only action set by default that does not touch user-authenticated data.
- Allow in-Warp invocations to receive authenticated-user grants when the selected Warp app has a true logged-in user and the user's local-control settings permit that grant.
- Support least-privilege safety modes for automation and interactive use without relying on an unenforceable identity label.
- Classify every action by state/data category and enforce the required permission category in the local app bridge, not in the CLI frontend.
- Classify every action by whether it requires an authenticated Warp user. New actions should default to requiring an authenticated user unless they are deliberately reviewed as safe for logged-out or external use.
- Prevent `warpctrl` from becoming an ambient full-power confused deputy that any same-user process can invoke for high-risk actions.
- Prohibit terminal command execution, workflow execution, accepted-command submission, and agent-prompt submission in the initial public implementation; input-buffer actions may stage text only and must not submit it.
- Preserve deterministic targeting so a request never silently mutates or reads the wrong window, tab, pane, session, file, or Warp Drive object.
- Keep the action surface allowlisted and typed rather than exposing arbitrary internal app dispatch.
- Make high-risk operations auditable and configurable without logging sensitive terminal contents or credentials.
## Meaningful security boundaries
The most important security boundary is preventing control from places that should have no ambient authority over the user's Warp instance:
- arbitrary web apps running in a browser;
- other OS users on the same machine;
- unauthenticated clients that discover or guess the localhost control port;
- stale discovery records from exited Warp processes;
- malformed or unallowlisted direct protocol calls.
The local-control design can provide meaningful protection for those cases by binding only to loopback, avoiding permissive CORS, requiring local credentials, keeping credentials out of browser-readable and world-readable locations, pruning stale records, and validating every request in the local Warp app process.
The boundary is much weaker for a different local app running as the same OS user. Same-user local apps may already have access to user-owned files such as logs, may be able to observe the screen or UI through OS permissions such as Accessibility or Screen Recording, and can often invoke user-installed command-line tools. `warpctrl` should not imply strong isolation from such software.
For same-user local apps, the realistic goal is narrower:
- do not leave a raw bearer token in plaintext discovery records;
- prevent arbitrary direct HTTP calls to the localhost control listener by requiring a credential those apps cannot simply read;
- use platform secure storage, such as macOS Keychain, so raw credentials are accessible only to Warp-owned signed code where practical;
- make high-risk operations go through `warpctrl` or a Warp-owned helper where user approval, configured policy, and safety grants can be applied;
- avoid giving `warpctrl` ambient non-interactive full-control authority.
In other words, the security model can make arbitrary direct localhost protocol calls fail, and it can make direct credential theft harder. It cannot make a same-user malicious app safe if that app can invoke `warpctrl`, automate the user's desktop, read other local state, or wait for the user to approve prompts.
## Comparison with other local scripting models
Other developer tools expose local automation through a few recurring patterns. The `warpctrl` design should borrow the parts that match Warp's needs while avoiding designs that assume localhost or same-user access is enough by itself.
### VS Code
VS Code's `code` command is primarily a launch and routing CLI: it opens files, folders, diffs, merge views, chat sessions, extension-management commands, and remote/tunnel workflows. It is not a general unauthenticated localhost API for arbitrary UI control of an already-running desktop app.
VS Code's richer local automation runs through extension APIs and extension hosts. Extensions are installed into a trusted editor environment and run with broad access to the workspace or UI side depending on extension kind. Workspace Trust and remote extension placement help users reason about whether code should run locally, remotely, or in a browser sandbox, but they do not create a fine-grained same-user security boundary against arbitrary local software.
Lessons for `warpctrl`:
- a narrow, typed CLI command surface is safer to reason about than exposing arbitrary internal app commands;
- agent and script workflows should request explicit capabilities instead of inheriting ambient full-control authority;
- local UI control should remain distinct from remote/tunnel control because remote transports need stronger identity, approval, and network-security semantics.
### Chrome DevTools Protocol
Chrome DevTools Protocol is a powerful debugging and automation API. When Chrome is launched with remote debugging enabled, clients can discover targets over local HTTP endpoints and then control the browser over WebSocket. That protocol is intentionally high-power: it can inspect pages, navigate, execute JavaScript, observe network state, and interact with browser storage.
Chrome's security history is a useful warning for `warpctrl`: a local debugging port is dangerous if it becomes reachable by unexpected clients. Recent Chrome versions restrict remote debugging against the default user data directory and recommend isolated user data directories for automation, because debugging a real browser profile can expose sensitive cookies and credentials. Chrome also distinguishes command-line remote debugging from user-confirmed debugging flows.
Lessons for `warpctrl`:
- loopback binding is necessary but not sufficient;
- unauthenticated localhost endpoints should not expose powerful state or mutation;
- browser-origin protections matter because web pages can attempt localhost requests;
- high-power automation should prefer explicit, isolated, user-approved, or short-lived authority over a reusable full-profile control channel.
### Ghostty and macOS AppleScript
Ghostty exposes platform-native scripting on macOS through AppleScript. That model relies on macOS Automation/TCC prompts to decide whether one app may control another app, and Ghostty can disable AppleScript entirely with configuration. This is a good fit for macOS-native scripting, but it is platform-specific and inherits the limits of OS automation permission: once an app is allowed to automate another app, the boundary is not a per-action capability system.
Ghostty also supports terminal-oriented features such as shell integration and command-line window creation flows. Those are useful local automation conveniences, but they are not a general cross-platform authenticated control protocol with scoped credentials.
Lessons for `warpctrl`:
- use platform security mechanisms where they exist, such as macOS Keychain and Automation prompts;
- keep a user-visible kill switch or policy path for scripting/control surfaces;
- do not rely only on platform automation permission if Warp needs cross-platform, action-scoped safety grants.
### iTerm2 Python API
iTerm2's Python API is a close comparison for terminal automation. The API is disabled by default. When enabled, iTerm2 listens on a Unix domain socket and requires authentication by default. Scripts launched by iTerm2 receive a random cookie in the environment, while external programs can request a cookie through AppleScript so macOS Automation permission mediates access. iTerm2 also documents an administrator-gated escape hatch to allow unauthenticated local apps.
This model directly acknowledges that terminal contents are sensitive and that any local automation API can affect local and remote hosts connected through terminal sessions.
Lessons for `warpctrl`:
- default-off or policy-controlled high-power automation is reasonable for sensitive capabilities;
- random local credentials are useful, but the path that grants or unwraps them is just as important as the token itself;
- underlying data reads and input/command execution should be treated as higher-risk than structural metadata reads;
- macOS Automation can be part of the approval path, but Warp still needs local app-side enforcement because direct protocol clients can bypass the official CLI.
### tmux
tmux is a useful lower-level comparison because its clients and server communicate through local sockets. The default socket lives in a per-user directory under `/tmp`, and that directory must not be world readable, writable, or executable. tmux control mode then exposes a text protocol where clients can issue normal tmux commands and receive asynchronous pane/session notifications. Newer tmux versions also have explicit server-access controls for sharing across users.
tmux's model is mostly an OS-user and socket-permission model. Once a client can access the socket with write authority, it can generally control the session. Read-only modes are useful operational guardrails but are not a reason to trust untrusted users or processes with the socket.
Lessons for `warpctrl`:
- per-user discovery directories and sockets protect meaningfully against other OS users;
- structured control protocols are scriptable and durable, but broad socket access quickly becomes broad control access;
- read-only and low-risk modes are valuable “do not accidentally interfere” controls, not a complete hostile-client sandbox.
### Overall direction for `warpctrl`
Compared with these systems, `warpctrl` should combine:
- tmux-style local filesystem/socket hygiene for protecting against other OS users;
- Chrome's lesson that local debugging/control endpoints need authentication and browser-origin hardening;
- iTerm2's use of explicit local credentials and macOS Automation-style approval for external control;
- Ghostty's use of platform-native scripting controls where available;
- VS Code's preference for typed public commands and separate treatment of remote control.
The resulting architecture should not claim to solve arbitrary same-user malicious-app isolation. Its real value is preventing web-origin and other-user access, preventing unauthenticated direct localhost calls, keeping raw credentials out of plaintext discovery, giving honest scripts and agents narrow safety grants, and routing high-risk operations through local Warp app validation and user/policy approval.
## Authoritative enablement model
Warp control has two top-level enablement states based on invocation context:
- **Allow scripting from inside Warp:** controls `warpctrl` invocations from verified Warp-managed terminal sessions. This should default to on so commands run inside Warp can use local control subject to granular permissions.
- **Allow scripting from outside Warp:** controls `warpctrl` invocations from external terminals, scripts, launch agents, IDEs, or other same-user processes. This must default to off.
Both controls should live in a new top-level Settings pane page named **Scripting**. The Scripting page owns the user-facing controls for local scripting surfaces, including Warp control, and should explain the difference between commands run inside Warp and commands run from other apps.
The visible UI settings are not enough by themselves. The authoritative enablement states must be stored in protected local storage that ordinary same-user apps cannot update by writing a plist, settings database row, registry key, JSON file, or synced cloud preference. This avoids turning outside-Warp control into a feature that any process can silently enable before invoking `warpctrl`.
Enablement requirements:
- The settings are local-only and must not sync through Settings Sync, Warp Drive, or server-backed user preferences.
- Only the running Warp app, through the Settings > Scripting UI, should be able to enable or disable the authoritative states.
- `warpctrl`, shell scripts, config files, command-line flags, registry edits, defaults writes, and direct local-control protocol requests must not be able to enable either setting.
- The in-Warp setting may default to enabled, but turning it off should prevent verified Warp-terminal invocations from receiving local-control grants.
- The outside-Warp setting defaults to disabled and should require an intentional user gesture before enabling; the UI should explain that it allows scripts and automation from other apps to control Warp.
- The Scripting page should expose granular local-control permission settings rather than a single all-powerful switch.
- Each setting should be easy to disable from the same UI, and disabling either setting should revoke or invalidate active local-control credentials for that invocation context.
- If enterprise or managed-device policy is added later, policy may force-disable either setting or allow an administrator-controlled default, but policy should be separate from user-editable local settings.
Disabled-state behavior:
- Warp should not mint scoped local-control credentials for a request whose invocation context is disabled.
- The control listener should reject requests from disabled contexts with a structured disabled-state error before authentication, selector resolution, or handler dispatch.
- Discovery records should avoid publishing actionable endpoint or credential-reference metadata for disabled outside-Warp control. If a minimal record is needed for UX, it should expose only non-sensitive status such as `outside_warp_control_enabled: false`.
- `warpctrl` may detect a disabled context and print instructions to enable it in Settings > Scripting, but it must not offer a command that flips the setting.
- Previously issued credentials must become unusable when their invocation context is disabled, even if their original expiry has not elapsed.
These enablement gates do not create perfect same-user malicious-app isolation. A hostile process with Accessibility or Screen Recording permission might still try to automate the Warp UI. The outside-Warp gate is still important because it closes the much easier paths where external apps silently edit local preferences, call a config CLI, or write synced settings to enable a powerful control surface.
### Granular permission settings
Once the relevant inside-Warp or outside-Warp enablement setting allows a request context, users should control which categories of `warpctrl` authority can be granted. These permissions should appear under Settings > Scripting. Recommended independent permissions:
- **Metadata reads:** permit external and in-Warp clients to inspect non-sensitive local app structure and configuration metadata such as instances, windows, tabs, panes, app version, theme names, setting keys, action metadata, and Drive object IDs/names/types without content.
- **Underlying data reads:** permit reads of terminal output, scrollback, input buffers, command history, session traces, file contents, Warp Drive object contents, AI conversation content, and other content-bearing state.
- **App-state mutations:** permit local UI/layout/focus changes such as opening windows, creating tabs, closing tabs, focusing panes, splitting panes, opening panels, opening files/projects/views, and staging text in the input buffer without executing it.
- **Metadata/configuration mutations:** permit persistent metadata or configuration changes such as tab/pane names, tab colors, themes, font size, zoom, allowlisted settings, and keybindings.
- **Underlying data mutations:** permit file create/write/append/delete, Warp Drive object CRUD, AI conversation mutations, and any other action that can change user data or cause external side effects. Terminal command execution, Warp Drive workflow execution, accepted-command submission, and agent-prompt submission belong in this category if they are added later, but they are not allowed in the initial public implementation.
- **Authenticated-user actions from Warp terminals:** permit `warpctrl` invocations that originate from a verified Warp-managed terminal session to receive grants backed by the currently logged-in Warp user, enabling actions that read or mutate Warp Drive, AI conversation traces, synced settings, or other user-authenticated state.
- **Authenticated-user actions from external clients:** default off. If supported, this must be a separate explicit permission from in-Warp authenticated actions because external same-user processes are a weaker context than a Warp-managed terminal session.
Granular permissions should be independently configurable for inside-Warp and outside-Warp contexts where the distinction matters. Disabling any category should invalidate active credentials that include that category. The broker and app bridge must enforce these settings locally for every credential request and every presented credential. App-state mutation permission must not imply metadata/configuration mutation or underlying data mutation permission.
## Trust boundaries
`warpctrl` has several distinct trust boundaries.
### Operating-system user boundary
The baseline local trust boundary is the OS user account. Discovery records and local credential material must be readable only by the owning user. This protects against other local users and network peers, but it does not protect against an already-compromised same-user process.
### Invocation boundary
Same-user does not mean same authority. Interactive use and unattended automation may both run commands under the same user account, but they should be able to intentionally request narrower capabilities. The protocol needs scoped credentials that encode concrete grants, target scopes, and lifetimes rather than an abstract caller type that the bridge cannot reliably verify.
These scoped credentials are guardrails for well-behaved clients. They prevent accidental overreach and make user intent explicit, but they are not a defense against malicious same-user code that can automate the CLI, inspect the user's environment, or wait for user approvals.
### Warp-terminal execution context boundary
`warpctrl` should be able to receive special grants when it is invoked from within a Warp-managed terminal session, but the bridge must not trust a caller-supplied string such as `caller_class=warp_terminal`. The app should issue a session-bound execution-context proof to Warp-managed terminal sessions and have the broker verify that proof before minting in-Warp-only grants.
Acceptable designs include a short-lived per-session capability, an app-owned broker handshake tied to the terminal session, or an equivalent proof that arbitrary external processes cannot mint by setting an environment variable. Plain environment variables may be used as handles or hints, but they must not be the sole authority for in-Warp privileges because external processes can spoof them.
Verified in-Warp context can raise the maximum eligible grant set, especially for authenticated-user actions. It does not by itself bypass the user's granular local-control settings, action categories, target scopes, or logged-in-user requirements.
### Warp user authentication boundary
Actions that touch user-authenticated Warp data require a true logged-in Warp user in the selected app. This includes Warp Drive object contents or mutation, AI conversation traces, cloud-backed user settings, team/account data, and any other surface whose normal app access depends on the user's Warp account. Authenticated-user grants must be tied to the same logged-in Warp user that is active in the selected app instance; `warpctrl` must not maintain a separate cloud login that can drift from the controlled process.
The app bridge should execute these actions on behalf of the logged-in app user through existing app auth state. `warpctrl` should receive a local-control credential that carries an `authenticated_user` grant, the verified user identity or stable subject reference, and the allowed authenticated action families. It should not need to export raw Firebase, server, or cloud API tokens to shell scripts.
If the selected app has no logged-in user, authenticated-user actions must fail with a structured error rather than falling back to logged-out behavior. Logged-out users may still use the smaller local-only action set explicitly marked as not requiring an authenticated user.
### Authenticated-user login protocol
`warpctrl` should provide an auth/status flow for users and automation that need authenticated-user actions, but the CLI must not collect Warp credentials or own an independent Warp account session.
Requirements:
- `warpctrl auth status [selectors]` reports whether the selected app instance is logged in and may return a stable, non-secret user subject/identity summary when the caller has the required grant.
- `warpctrl auth login [selectors]` focuses or opens the selected Warp app's normal sign-in UI and waits, or exits with actionable instructions, until the user signs in through Warp itself.
- The credential broker may mint an authenticated-user grant only after confirming the selected app has a true logged-in Warp user and the requested authenticated-user setting is enabled for the verified invocation context.
- Authenticated-user credentials are bound to the selected instance and logged-in user subject. If the app logs out, switches users, loses authenticated state, or the presented credential subject no longer matches the selected app's current user, authenticated-user actions fail with `authenticated_user_mismatch` or another structured authenticated-user error.
- The app bridge executes authenticated-user actions through the selected app's existing auth state. Raw Firebase, server, OAuth, or cloud API tokens must not be exported to `warpctrl`, shell scripts, JSON output, generated docs, or logs.
This protocol applies only to actions whose allowlist entry requires a logged-in Warp user. Logged-out-safe actions continue to use local-control credentials without requiring Warp account login.
### Application identity boundary
On platforms with secure credential storage, especially macOS, the raw local-control credential should be readable only by Warp-owned, correctly signed code. On macOS this means storing raw credential material in Keychain with access constrained by Warp's signing identity, designated requirement, Keychain access group, or equivalent platform mechanism. This narrows token extraction from “any same-user process can read a file” to “only trusted Warp-signed code can unwrap the secret.”
This boundary protects the credential from direct theft and prevents arbitrary apps from making authenticated raw HTTP requests to the local-control listener. It also lets the authoritative enablement state be stored somewhere harder to modify than ordinary user preferences. It does not prove that the user personally intended the specific action. Any same-user process may still be able to invoke the trusted `warpctrl` binary or automate the Warp UI. That confused-deputy risk is reduced by explicit in-app enablement, scoped credential issuance, action-category policy, and local app-side bridge enforcement, but it is not eliminated as a hard same-user security boundary.
### Action boundary
Every action belongs to a state/data category. The bridge must map the requested action to a required permission category and compare that category to the presented credential before selector resolution or handler dispatch.
### Target boundary
A valid credential for one instance or target must not imply authority over another. Credentials should be bound to the issuing Warp instance and may be further scoped to target families such as terminal sessions, files, or Warp Drive objects when those surfaces are exposed.
## Threat model
### In scope
- Other local OS users attempting to control a Warp instance owned by the current user.
- Browser-origin JavaScript attempting to call localhost control endpoints.
- Same-user automation attempting actions without the required scoped grants.
- Same-user processes attempting to extract plaintext credentials from local state.
- Same-user processes invoking `warpctrl` as a confused deputy for actions the process could not authorize directly.
- External same-user processes attempting authenticated-user actions that should be limited to verified Warp-terminal invocations.
- Logged-out requests attempting actions that require a true logged-in Warp user.
- Stale discovery records from exited Warp processes.
- Multiple running Warp instances where ambiguous selection could target the wrong process.
- Malformed clients attempting unknown, unsupported, unallowlisted, or invalid action payloads.
- Valid clients attempting actions above their granted permission category.
- Explicit target IDs that become stale between discovery and execution.
- Future handlers that expose terminal data, settings writes, input mutation, command execution, file intents, or Warp Drive object operations.
### Out of scope
- A malicious process that already has arbitrary same-user filesystem and process access, except that scoped credentials should still reduce accidental over-granting to ordinary automation.
- Kernel, hypervisor, or administrator-level compromise.
- Security semantics for remote URL control endpoints. Remote control requires a separate transport and identity design before it can ship.
## Architecture overview
The security model has eight layers:
1. **Protected enablement:** Use protected local storage for separate inside-Warp and outside-Warp enablement states, with inside-Warp on by default and outside-Warp off by default.
2. **Discovery:** Find compatible live Warp instances without granting broad authority.
3. **Secure credential storage:** Store raw secrets outside plaintext discovery records and restrict access to trusted Warp-owned code where the platform supports it.
4. **Execution context verification:** Distinguish verified Warp-terminal invocations from external same-user invocations without trusting caller-declared labels.
5. **Credential issuance:** Issue scope-specific credentials with explicit grants and lifetimes only when the request's invocation context is enabled and the user's granular permissions allow the requested category.
6. **Transport authentication:** Reject disabled or unauthenticated requests before reading or mutating app state.
7. **Safety and user-auth policy:** Enforce permission categories, target scopes, execution-context requirements, and authenticated-user requirements locally in the app bridge.
8. **Deterministic dispatch:** Resolve targets exactly and invoke only allowlisted typed handlers.
```mermaid
sequenceDiagram
    participant Invoker as User / Automation
    participant CLI as warpctrl
    participant Registry as Per-user discovery registry
    participant Enablement as Protected enablement state
    participant Context as Execution context proof
    participant Broker as Credential broker
    participant Store as Secure credential storage
    participant Auth as App auth state
    participant HTTP as Warp control listener
    participant Bridge as App bridge + safety policy
    participant UI as Warp app state

    Invoker->>CLI: Invoke allowlisted command
    CLI->>Registry: Read instance metadata
    Registry-->>CLI: instance_id, endpoint, protocol version, credential reference
    CLI->>Enablement: Check inside/outside context enablement
    Enablement-->>CLI: Enabled or disabled
    alt Disabled
        CLI-->>Invoker: context disabled; enable in Settings > Scripting
    else Enabled
    CLI->>Broker: Request scoped credential for action
    Broker->>Enablement: Verify protected enablement state
    Broker->>Context: Verify external vs Warp-terminal context
    opt Authenticated-user action
        Broker->>Auth: Verify logged-in Warp user + setting
        Auth-->>Broker: User subject or unavailable
    end
    Broker->>Store: Load or unwrap raw secret with Warp-signed access
    Store-->>Broker: Raw secret or credential capability
    Broker-->>CLI: Scoped credential with grants, context, user scope, expiry
    CLI->>HTTP: Authenticated typed request
    HTTP->>Bridge: Verify credential and protocol envelope
    Bridge->>Bridge: Check permission category + context + authenticated-user + target scope
    alt Denied
        Bridge-->>CLI: structured safety-policy error
    else Allowed
        Bridge->>UI: Resolve target exactly and run allowlisted handler
        UI-->>Bridge: typed result or structured target error
        Bridge-->>CLI: response envelope
    end
    end
```
## Discovery registry
Each participating Warp process writes a discovery record in a secure per-user local-control directory. Discovery records are metadata, not a full control-authority model.
A discovery record should contain:
- opaque `instance_id`;
- PID and process start timestamp;
- channel and build metadata;
- protocol version and supported capability summary;
- loopback endpoint for the instance-local control listener;
- credential reference or bootstrap credential metadata, not necessarily the full control credential.
Discovery rules:
- Records must be readable only by the owning user.
- POSIX records must use owner-only permissions such as `0600` for files and a non-world-readable directory.
- Windows records must live under the current user's app data directory with ACLs limited to the current user, Administrators, and SYSTEM.
- When outside-Warp control is disabled, records must not publish actionable control endpoints or credential references for external clients. A minimal disabled-status record is acceptable only if it contains no authority.
- The CLI must prune or ignore stale records whose PID is gone or whose health/protocol check fails.
- If multiple compatible instances are ambiguous, the CLI must require explicit `--instance` selection.
- Discovery metadata must not expose terminal contents, environment variables, auth tokens for cloud services, raw local-control credentials, or mutating capability grants.
## Credential model
The full `warpctrl` catalog requires scoped credentials. A single shared full-power bearer token is not sufficient once automation, underlying data reads, app-state mutations, metadata/configuration mutations, and underlying data mutations are supported.
### Credential properties
A control credential should encode or reference:
- issuing Warp instance;
- protocol version or accepted version range;
- granted permission categories;
- verified execution context, such as external client or Warp-managed terminal session;
- whether the credential may act on behalf of an authenticated Warp user;
- authenticated Warp user subject or stable user reference when an authenticated-user grant is present;
- optional allowed action families;
- optional target restrictions, such as one session, one workspace, one file path, or one Warp Drive object type;
- issued-at time;
- expiry time or process-lifetime binding;
- unique credential ID for revocation and auditing;
- integrity protection so callers cannot forge or widen grants.
### Credential issuance
Warp should issue credentials through an app-owned local broker or equivalent trusted path. The broker decides which grants to issue based on the requested permission category, target scope, user configuration, execution context, and any explicit user approval.
Recommended defaults:
- Credential issuance is unavailable unless the protected enablement state allows the request's invocation context: inside Warp or outside Warp.
- Commands should start from least privilege and request only the grant needed for the requested action.
- External same-user invocations should default to the smaller logged-out-safe local action set unless policy or explicit approval grants more.
- Verified Warp-terminal invocations may receive broader local-control grants when the user's granular settings allow them.
- Authenticated-user grants are available only when the selected Warp app has a true logged-in Warp user and the requested execution context is allowed by local-control settings.
- Metadata reads require an explicit `read_metadata` grant.
- Underlying data reads require an explicit `read_underlying_data` grant.
- App-state mutations require an explicit `mutate_app_state` grant.
- Metadata/configuration mutations require an explicit `mutate_metadata` or `mutate_configuration` grant.
- Underlying data mutations require an explicit `mutate_underlying_data` grant and should require approval or policy for unattended automation.
- User-authenticated data reads or mutations require an explicit `authenticated_user` grant and an allowed authenticated action family in addition to the data-category grant.
- Integrations should receive the narrowest grant needed for the configured workflow.
The broker must not issue broad authority merely because the request came from the signed `warpctrl` binary. It should evaluate the requested permission category, target scope, configured policy, execution context, and whether user approval is required. The CLI must not mint its own authority. It can request, load, and present credentials, but the app bridge remains the enforcement point for these safety grants.
### Safety grants, not strong access control
The category system should be understood as a user-intent and accident-prevention mechanism:
- A user can ask an agent or script to operate with metadata-read grants so it can inspect structure but cannot read terminal content or mutate state.
- A workflow can request underlying-data reads separately from structural metadata reads because terminal output, files, Drive object content, and AI conversations can contain sensitive data.
- A script can request app-state mutation without also receiving permission to change persistent settings, execute commands, write files, or mutate Warp Drive objects.
- Metadata/configuration mutations can be allowed without granting underlying data mutation.
- Underlying data mutations can require explicit approval or configured policy so surprising operations pause before they execute commands or change user data.
This model does not make untrusted same-user software safe. A malicious local process may invoke `warpctrl`, simulate user workflows, or use other OS-level capabilities outside `warpctrl`. The category model is still valuable because it lets honest clients, agents, and scripts constrain themselves and gives Warp a structured point to prompt, deny, or audit risky actions.
### Credential storage
Credential storage should be platform-appropriate:
- Local discovery may store a credential reference rather than the credential itself.
- The authoritative local-control enablement states for inside-Warp and outside-Warp scripting should use the same class of protected local storage as raw credential material, but they should be accessible to the Warp app for the Settings > Scripting UI and not writable by `warpctrl` or arbitrary external apps.
- Raw long-lived credentials should prefer platform-secure storage such as macOS Keychain or Windows Credential Manager when practical.
- On macOS, raw control secrets should be stored in Keychain and restricted to trusted Warp-signed code using a designated requirement, Keychain access group, trusted-application ACL, or equivalent code-signing based mechanism. Restricting by filesystem path alone is insufficient because paths can be replaced or wrapped.
- Keychain item access should include the Warp app, the signed `warpctrl` binary, and any signed Warp-owned local broker/helper that needs to unwrap raw secrets. It should exclude arbitrary same-user applications.
- Short-lived credentials may be stored in owner-only local state if their lifetime and scope are narrow.
- Credentials must never be printed in human-readable output, JSON output, logs, errors, or shell completion data.
### Confused-deputy mitigation
Secure storage prevents arbitrary apps from reading the token; it does not prevent arbitrary apps from asking trusted Warp code to use the token on their behalf.
For example, if `warpctrl` can silently unwrap a full-power credential and execute any action, another same-user process can invoke `warpctrl input run ...` without reading the credential directly. That makes `warpctrl` a confused deputy.
Mitigations:
- Do not give `warpctrl` ambient non-interactive access to an unrestricted full-control credential.
- Prefer action-scoped or session-scoped credentials minted just in time by the broker.
- Require explicit user approval or preconfigured policy for underlying data mutations and other sensitive grants.
- Distinguish user-approved credential requests from ambient unattended invocations through explicit approval prompts, configured policy, terminal/session context, or narrow credential request flows.
- Bind issued credentials to the requested instance, permission category, optional action family, optional target scope, and short expiry.
- Let `warpctrl` preflight and request credentials, but require the local app bridge to enforce scopes because direct protocol clients can bypass the CLI.
- Make denials structured and non-fatal for automation so callers can request narrower or user-approved grants rather than falling back to unsafe behavior.
These mitigations are about routing high-risk operations through intentional `warpctrl` flows rather than exposing a reusable localhost token to any process. They should not be documented as a guarantee that arbitrary same-user applications cannot cause Warp-visible actions.
## Transport authentication
The default transport is an instance-local loopback listener bound to `127.0.0.1` on an ephemeral per-process port.
Transport requirements:
- Bind only to loopback for local control.
- Do not set permissive CORS headers.
- Reject control requests when their inside-Warp or outside-Warp invocation context is disabled, even if the request presents an otherwise valid credential.
- Authenticate every control request locally in the selected Warp app process before selector resolution or action dispatch.
- Reject missing, malformed, expired, revoked, or invalid credentials with structured authentication errors.
- Keep unauthenticated health metadata minimal and non-sensitive.
- Preserve structured error envelopes so the CLI does not collapse security failures into generic transport errors.
Remote URL support is a separate future transport mode. It should not reuse the local same-user credential model without additional identity, encryption, replay protection, and remote approval/policy design.
## Logged-in user requirements
Local-control validation always begins with local protocol state: discovery records, secure local credential references, scoped safety grants, execution-context proof, protocol version, request shape, allowlisted actions, typed parameters, and deterministic target selectors.
Some actions additionally require a true logged-in Warp user in the selected app. The action allowlist must declare this explicitly with a `requires_authenticated_user` field.
Default rule for new actions:
- New actions require an authenticated Warp user unless the implementer deliberately classifies them as logged-out-safe.
- The logged-out-safe set should remain meaningfully smaller and limited to local app structure, local appearance metadata, and other surfaces that do not depend on the user's cloud-backed Warp identity.
- Actions that read or mutate Warp Drive, AI conversation traces, synced settings, team/account data, or other user-authenticated state must require an authenticated user.
- Future actions that can execute user-authored cloud-backed content, such as running Warp Drive workflows or submitting notebook commands, require both the authenticated-user grant and the appropriate high-risk action category. These execution actions are excluded from the initial public implementation.
When an authenticated-user action is requested:
- the selected app must have an active logged-in Warp user;
- the presented local-control credential must include an `authenticated_user` grant for that user or stable subject;
- the user's granular settings must allow authenticated-user actions for the verified execution context;
- the app bridge should execute through the app's existing authenticated state rather than exporting raw cloud auth credentials to `warpctrl`.
If these conditions are not met, the app returns a structured error. It must not fall back to logged-out behavior or silently omit user-authenticated data from a result that claims success.
## Safety policy model
Safety grants are enforced in the app bridge after transport authentication and before target resolution or handler dispatch. This provides consistent “do not accidentally do more than requested” behavior for honest clients, not a sandbox for hostile same-user code.
The bridge must:
1. Parse the typed request envelope.
2. Verify protocol version compatibility.
3. Authenticate the credential.
4. Determine granted permission categories, execution context, target scopes, and authenticated-user grants.
5. Map the requested action to a required permission category, action family, execution-context requirement, and authenticated-user requirement.
6. Check optional target-family restrictions.
7. Reject requests that exceed the credential's grants with `insufficient_permissions`.
8. Reject authenticated-user actions without a logged-in user or authenticated-user grant with a structured authenticated-user error.
9. Only then resolve selectors and invoke the allowlisted handler.
The CLI frontend may provide helpful preflight errors, but those checks are advisory. Local app-side bridge enforcement is mandatory because other tools can bypass the official CLI and speak the protocol directly.
## Action permission categories
Every action belongs to exactly one state/data category for permission enforcement. These categories describe risk and intended safety prompts; they are not a sandbox or a complete OS-level access-control model.
### Metadata reads
Return app structure, app state, or configuration metadata without exposing terminal content, file content, Warp Drive object content, AI conversation content, or other user data.
Examples:
- `instance list`, `app active`, `app version`, `app ping`;
- `window list`, `tab list`, `pane list`, `session list`;
- `theme list`, `setting list`, `keybinding list`, and action/capability metadata;
- Drive object listing that returns object IDs, names, and types but not content.
Default unattended credentials may include this category.
### Underlying data reads
Return user content or data-bearing state without mutating state.
Examples:
- pane output, scrollback, current input buffer, command history, session replay, or transcript reads;
- file content reads;
- Warp Drive object content reads;
- AI conversation content reads.
This category is separate from metadata because content often contains secrets, source code, file paths, command output, customer data, and other sensitive information.
### App-state mutations
Change visible local Warp UI state without directly changing underlying user data.
Examples:
- creating, focusing, activating, moving, or closing windows, tabs, panes, or sessions;
- splitting, navigating, maximizing, or resizing panes;
- opening panels, palettes, files, projects, notebooks, and other user-facing surfaces;
- inserting, replacing, or clearing staged input buffer text without submitting or executing it.
### Metadata/configuration mutations
Change persistent metadata or configuration without directly mutating primary user content.
Examples:
- renaming tabs or panes;
- changing tab colors;
- theme, font, zoom, keybinding, and allowlisted settings writes.
This category should not authorize terminal command execution, file writes, or Warp Drive CRUD.
### Underlying data mutations
Can change user data, execute code, submit prompts, or cause external side effects.
Examples:
- future command execution in a session;
- future agent prompt submission or acceptance of an agent-proposed command;
- future execution of Warp Drive workflows or other user-authored runnable content;
- file create/write/append/delete operations;
- Warp Drive object create/update/trash/restore/permanent-delete operations;
- AI conversation history mutation or other cloud-backed content mutation.
This category should require explicit user or policy approval for unattended automation and integrations. It must remain separate from app-state mutation so a client that can open or focus Warp UI cannot automatically execute commands, submit prompts, write files, or mutate Warp Drive content. Terminal command execution, workflow execution, accepted-command submission, and agent-prompt submission must remain unavailable in the initial public implementation even if the protocol has future reserved action names for them.
## Target scoping and deterministic resolution
Targeting is part of security. The protocol must not convert ambiguous or stale selectors into best-effort mutations.
Rules:
- Instance selection happens before request dispatch and must be explicit when ambiguous.
- `active` selectors may be ergonomic defaults only when the active target is unambiguous.
- If no active target exists for a mutating request, return `missing_target` or `invalid_selector`.
- Explicit opaque IDs must resolve exactly or return `stale_target`.
- Index selectors must resolve to concrete IDs before execution and must not race into a different target silently.
- Session-scoped requests against non-terminal panes return `target_state_conflict`.
- File selectors use paths and must remain distinct from opaque UI IDs.
- Warp Drive selectors must include object type and resolve by opaque ID for automation stability, with name/path lookup only as an interactive convenience.
Target restrictions in credentials should be checked before invoking handlers. For example, a credential scoped to one session must not read another session's output even if the CLI can discover that session ID.
## Allowlisted handlers
The protocol must not expose arbitrary internal app actions by string.
Each supported command requires:
- a typed protocol action;
- typed parameters;
- validation rules;
- a documented state/data category and permission category;
- a documented `requires_authenticated_user` value;
- a documented allowed execution context, including whether external clients can run it or whether it is limited to verified Warp-terminal invocations;
- local app-side safety-grant checks;
- deterministic target resolution;
- a handler that reuses existing user-visible app behavior where possible;
- typed success and error responses.
Adding a new action should be additive and reviewable: extend the protocol enum, implement validation, map the action to a state/data category, declare whether it requires an authenticated user, declare its allowed execution contexts, add a handler, and add tests for authentication, safety-policy denial, authenticated-user denial, selector failure, and success behavior.
## Browser and localhost protections
Loopback is not sufficient by itself because browsers can send requests to localhost.
Required protections:
- No permissive CORS on control endpoints.
- No JSONP or browser-readable fallback formats.
- Valid scoped credentials required for all sensitive endpoints.
- Credentials stored outside browser-readable locations.
- Preflight and error responses must not reveal credentials or sensitive target state.
- The protocol should avoid GET endpoints for mutating actions.
The control plane should assume a malicious webpage can guess common localhost ports and send blind requests. It should not be able to read discovery records or obtain credentials.
## Auditing and logging
High-risk action support should include auditability without leaking sensitive data.
Recommended audit fields:
- timestamp;
- instance ID;
- credential ID or grant profile;
- action name, state/data category, and permission category;
- target type and opaque target ID when safe;
- success or structured error code.
Avoid logging:
- bearer tokens or scoped credentials;
- terminal output;
- command text for command execution unless explicitly approved by policy in a future version that supports execution;
- agent prompt text;
- input buffer contents;
- Warp Drive object contents;
- environment variable values.
Error-level logs should be used only for conditions that need developer attention, not normal denied requests or user-caused selector failures.
## Security- and safety-relevant errors
Structured errors are part of the security contract.
Important errors include:
- `local_control_disabled` when the relevant inside-Warp or outside-Warp scripting context is disabled in Settings > Scripting or has been disabled after credentials were issued;
- `unauthorized_local_client` for missing, malformed, expired, revoked, or invalid credentials;
- `insufficient_permissions` for valid credentials that lack the requested permission category or target scope;
- `authenticated_user_required` when an action requires a logged-in Warp user but the credential lacks an authenticated-user grant;
- `authenticated_user_unavailable` when the selected Warp app has no logged-in Warp user or cannot access the required authenticated user state;
- `authenticated_user_mismatch` when an authenticated-user credential is bound to a different user subject than the user currently logged in to the selected Warp app;
- `execution_context_not_allowed` when the action or requested grant is not allowed from the verified invocation context, such as an external client attempting an in-Warp-only authenticated-user action;
- `ambiguous_instance` when multiple compatible instances cannot be resolved safely;
- `invalid_selector` for malformed or unsupported selector syntax;
- `missing_target` when an active/default target does not exist;
- `stale_target` when an explicit target ID no longer exists;
- `unsupported_action` for actions not implemented by the selected instance;
- `not_allowlisted` for actions intentionally excluded from the public control surface;
- `invalid_params` for malformed parameters;
- `target_state_conflict` when the target exists but cannot support the requested action.
The app must not downgrade these failures into broader default actions, and the CLI must preserve structured server errors in both human-readable and JSON output.
## Required controls before full catalog expansion
Before shipping each action family, verify that these controls are implemented for that family:
- Local control scripting must be enabled for the request's invocation context before the action family can run; inside-Warp control defaults on and outside-Warp control defaults off.
- The authoritative enablement states live under Settings > Scripting, are protected from external writes, and are local-only rather than synced.
- The action has a documented state/data category and required permission category.
- The action has a documented `requires_authenticated_user` value. New actions default to `true` unless explicitly reviewed as logged-out-safe.
- The action documents allowed execution contexts and whether external clients may run it.
- The bridge maps the action to that permission category locally in the selected Warp app process.
- The credential model can express the required grant.
- The credential model can express authenticated-user grants and verified execution context requirements when needed.
- The handler checks optional target restrictions where relevant.
- Requests with invalid credentials or insufficient safety grants fail before selector resolution or mutation.
- Requests that require authenticated-user access fail unless the selected app has a true logged-in Warp user and the credential includes an authenticated-user grant.
- Ambiguous, missing, and stale targets return structured errors.
- Tests cover allowed, insufficient-permission, and denied credential paths.
- Logs and errors do not expose credentials, terminal contents, command text, or sensitive settings.
- Operator docs distinguish available commands from planned catalog entries.
- Initial public action-family docs and tests prove terminal command execution, workflow execution, accepted-command submission, and agent-prompt submission are not allowlisted; input-buffer staging never submits the buffer.
## Platform requirements
### macOS and Linux
Discovery files must be stored in a per-user directory with owner-only permissions.
On macOS, raw credential material and the authoritative local-control enablement states should live in Keychain, not in the discovery record or an ordinary preferences file. Keychain access should be constrained to Warp-owned signed binaries or helpers using code-signing based access control. The enablement states should be writable by the Warp app's Settings > Scripting flow and not writable by `warpctrl`. The discovery record should hold only metadata and a credential reference when the relevant inside-Warp or outside-Warp context is enabled.
On Linux, raw credentials and the authoritative enablement states should prefer platform-secure storage where available; otherwise short-lived scoped credentials may live in owner-only local state with strict file and directory permissions. If an enablement state falls back to owner-only local state, the weaker same-user protection should be documented.
Unix domain sockets with peer credential checks may be considered for stronger same-machine identity than bearer tokens alone.
### Windows
Discovery records and credential material must live under the current user's app data directory with ACLs restricted to the current user, Administrators, and SYSTEM.
The authoritative enablement states should use Credential Manager, DPAPI-backed protected storage, or an equivalent app-controlled protected store rather than normal registry settings that arbitrary same-user processes can write.
Windows support for authenticated local control should not be considered complete until the implementation creates, validates, and tests those ACLs and the protected enablement-state behavior for both inside-Warp and outside-Warp settings.
## Remote control is separate
The local architecture intentionally assumes same-machine, same-user control over a loopback listener. Future remote URLs must use a different security design that includes:
- transport encryption;
- remote identity and authentication;
- replay protection;
- explicit user or admin approval/policy;
- network exposure review;
- separate credential issuance from local discovery;
- remote-safe auditing and revocation.
Remote support should not be enabled by simply allowing `warpctrl` to point the existing local credential at an arbitrary URL.
