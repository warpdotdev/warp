# warpctrl security architecture
`warpctrl` is a local-control CLI for an already-running Warp app instance. Its security architecture is designed to support the full control catalog: discovery, structural reads, terminal-data reads, non-destructive mutations, settings changes, input manipulation, command execution, and destructive window/tab/pane operations.
The correct architecture is not a single shared localhost bearer token with client-side conventions. The CLI, app bridge, and protocol must treat security as a local app-enforced capability system: discovery finds compatible instances, secure storage protects raw credential material, broker-issued credentials identify the granted scopes, the running Warp app's local-control bridge enforces action tiers before dispatch, and target resolution never silently retargets a request.
The action-tier model is primarily a safety and intent mechanism, not a hard security boundary against malicious same-user software. It lets a user, script, or agent intentionally request read-only or low-risk access so it does not accidentally mutate state or execute commands. It should not be described as strong access control against a process that can already run arbitrary commands as the user.
`warpctrl` has two distinct authorization dimensions: local-control authority and Warp user authority. Local-control authority proves the request is allowed to control the local app. Warp user authority proves the selected Warp app has a real logged-in Warp user and the request is allowed to act on user-authenticated data such as Warp Drive objects, AI conversation traces, synced settings, or cloud-backed user state. Logged-out users should retain a smaller local-only control surface, but authenticated-user actions require a true logged-in Warp user in the selected app.
## Security goals
- Allow trusted local users and approved automation to control a running Warp instance through a stable, scriptable interface.
- Prevent unauthenticated localhost clients from invoking read or mutating control actions.
- Prevent browser-origin JavaScript from becoming an ambient localhost control client.
- Support multiple running Warp processes without a shared global mutating port or global credential.
- Separate discovery metadata from control authority so enumerating an instance does not automatically grant full control.
- Require explicit in-app user enablement before local control scripting can issue credentials or accept control requests.
- Store the authoritative enablement state in protected local storage so external apps cannot enable local control by editing ordinary settings.
- Keep raw credential material out of plaintext discovery records and protect it with platform secure storage where available.
- Distinguish verified `warpctrl` invocations that originate from a Warp-managed terminal session from external same-user invocations.
- Allow external invocations by default only for a smaller local-only action set that does not touch user-authenticated data.
- Allow in-Warp invocations to receive authenticated-user grants when the selected Warp app has a true logged-in user and the user's local-control settings permit that grant.
- Support least-privilege safety modes for automation and interactive use without relying on an unenforceable identity label.
- Classify every action by risk tier and enforce the required tier in the local app bridge, not in the CLI frontend.
- Classify every action by whether it requires an authenticated Warp user. New actions should default to requiring an authenticated user unless they are deliberately reviewed as safe for logged-out or external use.
- Prevent `warpctrl` from becoming an ambient full-power confused deputy that any same-user process can invoke for high-risk actions.
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
- terminal-data reads and input/command execution should be treated as higher-risk than structural metadata reads;
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
Local control scripting must be explicitly enabled by the user inside the Warp app before `warpctrl` can control a running instance. The setting should be visible in Warp's settings UI as something like “Allow local control scripting,” and it should default to off.
The visible UI setting is not enough by itself. The authoritative enablement state must be stored in protected local storage that ordinary same-user apps cannot update by writing a plist, settings database row, registry key, JSON file, or synced cloud preference. This avoids turning local control into a feature that any process can silently enable before invoking `warpctrl`.
Enablement requirements:
- The setting is local-only and must not sync through Settings Sync, Warp Drive, or server-backed user preferences.
- Only the running Warp app, through an in-app user action, should be able to enable or disable the authoritative state.
- `warpctrl`, shell scripts, config files, command-line flags, registry edits, defaults writes, and direct local-control protocol requests must not be able to enable the setting.
- The app should require an intentional user gesture before enabling the setting, and the UI should explain that it allows local scripts and approved automation to control Warp.
- The UI should expose granular local-control permission settings rather than a single all-powerful switch.
- The setting should be easy to disable from the same UI, and disabling it should revoke or invalidate active local-control credentials.
- If enterprise or managed-device policy is added later, policy may force-disable local control or allow an administrator-controlled default, but policy should be separate from user-editable local settings.
Disabled-state behavior:
- Warp should not mint scoped local-control credentials while local control scripting is disabled.
- The control listener should either not start or should reject all sensitive requests with a structured disabled-state error before authentication, selector resolution, or handler dispatch.
- Discovery records should avoid publishing actionable endpoint or credential-reference metadata while disabled. If a minimal record is needed for UX, it should expose only non-sensitive status such as `local_control_enabled: false`.
- `warpctrl` may detect the disabled state and print instructions to enable local control in Warp settings, but it must not offer a command that flips the setting.
- Previously issued credentials must become unusable when local control is disabled, even if their original expiry has not elapsed.
This enablement gate does not create perfect same-user malicious-app isolation. A hostile process with Accessibility or Screen Recording permission might still try to automate the Warp UI. The gate is still important because it closes the much easier paths where external apps silently edit local preferences, call a config CLI, or write synced settings to enable a powerful control surface.
### Granular permission settings
Once local control scripting is enabled, users should control which categories of `warpctrl` authority can be granted. Recommended independent permissions:
- **Local read-only metadata:** permit external and in-Warp clients to inspect non-sensitive local app structure such as instances, windows, tabs, panes, app version, and theme names.
- **Terminal data reads:** permit reads of terminal output, scrollback, input buffers, command history, and session traces.
- **Non-destructive local mutations:** permit reversible app-state changes such as creating tabs, focusing panes, changing theme, or opening panels.
- **Destructive and execution actions:** permit closing targets, injecting input, running commands, executing workflows, or other high-risk operations.
- **Authenticated-user actions from Warp terminals:** permit `warpctrl` invocations that originate from a verified Warp-managed terminal session to receive grants backed by the currently logged-in Warp user, enabling actions that read or mutate Warp Drive, AI conversation traces, synced settings, or other user-authenticated state.
- **Authenticated-user actions from external clients:** default off. If supported, this must be a separate explicit permission from in-Warp authenticated actions because external same-user processes are a weaker context than a Warp-managed terminal session.
Disabling any category should invalidate active credentials that include that category. The broker and app bridge must enforce these settings locally for every credential request and every presented credential.
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
Verified in-Warp context can raise the maximum eligible grant set, especially for authenticated-user actions. It does not by itself bypass the user's granular local-control settings, action risk tiers, target scopes, or logged-in-user requirements.
### Warp user authentication boundary
Actions that touch user-authenticated Warp data require a true logged-in Warp user in the selected app. This includes Warp Drive object contents or mutation, AI conversation traces, cloud-backed user settings, team/account data, and any other surface whose normal app access depends on the user's Warp account.
The app bridge should execute these actions on behalf of the logged-in app user through existing app auth state. `warpctrl` should receive a local-control credential that carries an `authenticated_user` grant, the verified user identity or stable subject reference, and the allowed authenticated action families. It should not need to export raw Firebase, server, or cloud API tokens to shell scripts.
If the selected app has no logged-in user, authenticated-user actions must fail with a structured error rather than falling back to logged-out behavior. Logged-out users may still use the smaller local-only action set explicitly marked as not requiring an authenticated user.
### Application identity boundary
On platforms with secure credential storage, especially macOS, the raw local-control credential should be readable only by Warp-owned, correctly signed code. On macOS this means storing raw credential material in Keychain with access constrained by Warp's signing identity, designated requirement, Keychain access group, or equivalent platform mechanism. This narrows token extraction from “any same-user process can read a file” to “only trusted Warp-signed code can unwrap the secret.”
This boundary protects the credential from direct theft and prevents arbitrary apps from making authenticated raw HTTP requests to the local-control listener. It also lets the authoritative enablement state be stored somewhere harder to modify than ordinary user preferences. It does not prove that the user personally intended the specific action. Any same-user process may still be able to invoke the trusted `warpctrl` binary or automate the Warp UI. That confused-deputy risk is reduced by explicit in-app enablement, scoped credential issuance, action-tier policy, and local app-side bridge enforcement, but it is not eliminated as a hard same-user security boundary.
### Action boundary
Every action belongs to a risk tier. The bridge must map the requested action to a required tier and compare that tier to the presented credential before selector resolution or handler dispatch.
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
- Valid clients attempting actions above their granted tier.
- Explicit target IDs that become stale between discovery and execution.
- Future handlers that expose terminal data, settings writes, input mutation, command execution, file intents, or Warp Drive object operations.
### Out of scope
- A malicious process that already has arbitrary same-user filesystem and process access, except that scoped credentials should still reduce accidental over-granting to ordinary automation.
- Kernel, hypervisor, or administrator-level compromise.
- Security semantics for remote URL control endpoints. Remote control requires a separate transport and identity design before it can ship.
## Architecture overview
The security model has eight layers:
1. **Protected enablement:** Require explicit in-app opt-in backed by protected local storage before local control is available.
2. **Discovery:** Find compatible live Warp instances without granting broad authority.
3. **Secure credential storage:** Store raw secrets outside plaintext discovery records and restrict access to trusted Warp-owned code where the platform supports it.
4. **Execution context verification:** Distinguish verified Warp-terminal invocations from external same-user invocations without trusting caller-declared labels.
5. **Credential issuance:** Issue scope-specific credentials with explicit grants and lifetimes only when local control is enabled and the user's granular permissions allow the requested category.
6. **Transport authentication:** Reject disabled or unauthenticated requests before reading or mutating app state.
7. **Safety and user-auth policy:** Enforce action tiers, target scopes, execution-context requirements, and authenticated-user requirements locally in the app bridge.
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
    CLI->>Enablement: Check local control enabled status
    Enablement-->>CLI: Enabled or disabled
    alt Disabled
        CLI-->>Invoker: local control disabled; enable in Warp settings
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
    Bridge->>Bridge: Check tier + context + authenticated-user + target scope
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
- When local control scripting is disabled, records must not publish actionable control endpoints or credential references. A minimal disabled-status record is acceptable only if it contains no authority.
- The CLI must prune or ignore stale records whose PID is gone or whose health/protocol check fails.
- If multiple compatible instances are ambiguous, the CLI must require explicit `--instance` selection.
- Discovery metadata must not expose terminal contents, environment variables, auth tokens for cloud services, raw local-control credentials, or mutating capability grants.
## Credential model
The full `warpctrl` catalog requires scoped credentials. A single shared full-power bearer token is not sufficient once automation, terminal data, command execution, and destructive actions are supported.
### Credential properties
A control credential should encode or reference:
- issuing Warp instance;
- protocol version or accepted version range;
- granted action tiers;
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
Warp should issue credentials through an app-owned local broker or equivalent trusted path. The broker decides which grants to issue based on the requested action tier, target scope, user configuration, execution context, and any explicit user approval.
Recommended defaults:
- Credential issuance is unavailable unless the protected in-app enablement state says local control scripting is enabled.
- Commands should start from least privilege and request only the grant needed for the requested action.
- External same-user invocations should default to the smaller logged-out-safe local action set unless policy or explicit approval grants more.
- Verified Warp-terminal invocations may receive broader local-control grants when the user's granular settings allow them.
- Authenticated-user grants are available only when the selected Warp app has a true logged-in Warp user and the requested execution context is allowed by local-control settings.
- Terminal data reads require an explicit `read_terminal_data` grant.
- Non-destructive mutations require an explicit `mutate_non_destructive` grant.
- Destructive operations, input injection, and command execution require explicit high-risk grants.
- User-authenticated data reads or mutations require an explicit `authenticated_user` grant and an allowed authenticated action family.
- Integrations should receive the narrowest grant needed for the configured workflow.
The broker must not issue broad authority merely because the request came from the signed `warpctrl` binary. It should evaluate the requested action tier, target scope, configured policy, execution context, and whether user approval is required. The CLI must not mint its own authority. It can request, load, and present credentials, but the app bridge remains the enforcement point for these safety grants.
### Safety grants, not strong access control
The tier system should be understood as a user-intent and accident-prevention mechanism:
- A user can ask an agent or script to operate with read-only metadata grants so it can inspect structure but cannot accidentally mutate state.
- A workflow can request terminal-data reads separately from structural metadata reads because terminal contents are more sensitive.
- A script can request non-destructive mutation without also receiving command-execution capability.
- Destructive actions and command execution can require an explicit approval or configured policy so surprising operations pause before they happen.
This model does not make untrusted same-user software safe. A malicious local process may invoke `warpctrl`, simulate user workflows, or use other OS-level capabilities outside `warpctrl`. The tier model is still valuable because it lets honest clients, agents, and scripts constrain themselves and gives Warp a structured point to prompt, deny, or audit risky actions.
### Credential storage
Credential storage should be platform-appropriate:
- Local discovery may store a credential reference rather than the credential itself.
- The authoritative local-control enablement state should use the same class of protected local storage as raw credential material, but it should be accessible to the Warp app for in-app settings UI and not writable by `warpctrl` or arbitrary external apps.
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
- Require explicit user approval or preconfigured policy for Tier 4 actions and other sensitive grants.
- Distinguish user-approved credential requests from ambient unattended invocations through explicit approval prompts, configured policy, terminal/session context, or narrow credential request flows.
- Bind issued credentials to the requested instance, action tier, optional action family, optional target scope, and short expiry.
- Let `warpctrl` preflight and request credentials, but require the local app bridge to enforce scopes because direct protocol clients can bypass the CLI.
- Make denials structured and non-fatal for automation so callers can request narrower or user-approved grants rather than falling back to unsafe behavior.
These mitigations are about routing high-risk operations through intentional `warpctrl` flows rather than exposing a reusable localhost token to any process. They should not be documented as a guarantee that arbitrary same-user applications cannot cause Warp-visible actions.
## Transport authentication
The default transport is an instance-local loopback listener bound to `127.0.0.1` on an ephemeral per-process port.
Transport requirements:
- Bind only to loopback for local control.
- Do not set permissive CORS headers.
- Reject control requests while local control scripting is disabled, even if the request presents an otherwise valid credential.
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
- Actions that can execute user-authored cloud-backed content, such as running Warp Drive workflows or inserting notebook commands, require both the authenticated-user grant and the appropriate high-risk action tier.
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
4. Determine granted action tiers, execution context, target scopes, and authenticated-user grants.
5. Map the requested action to a required tier, action family, execution-context requirement, and authenticated-user requirement.
6. Check optional target-family restrictions.
7. Reject requests that exceed the credential's grants with `insufficient_permissions`.
8. Reject authenticated-user actions without a logged-in user or authenticated-user grant with a structured authenticated-user error.
9. Only then resolve selectors and invoke the allowlisted handler.
The CLI frontend may provide helpful preflight errors, but those checks are advisory. Local app-side bridge enforcement is mandatory because other tools can bypass the official CLI and speak the protocol directly.
## Action risk tiers
Every action belongs to exactly one tier. These tiers describe risk and intended safety prompts; they are not a sandbox or a complete OS-level access-control model.
### Tier 1: read-only metadata
Returns app structure or configuration without terminal contents or user data from sessions.
Examples:
- `instance list`, `app active`, `app version`, `app ping`;
- `window list`, `tab list`, `pane list`, `session list`;
- `theme list`;
- allowlisted settings reads that expose configuration but not terminal contents.
Default unattended credentials may include this tier.
### Tier 2: read-only terminal data
Returns potentially sensitive terminal/session data without mutating state.
Examples:
- pane output or scrollback reads;
- current input buffer reads;
- command history reads;
- session replay or transcript reads.
This tier is separate from metadata because terminal content often contains secrets, file paths, command output, customer data, and other sensitive information.
### Tier 3: mutating non-destructive
Changes visible app state in reversible or low-risk ways without executing terminal content or destroying user state.
Examples:
- creating or activating tabs;
- moving, renaming, or coloring tabs;
- creating or focusing windows;
- splitting, focusing, navigating, maximizing, or resizing panes;
- theme, font, zoom, and allowlisted non-destructive settings changes;
- opening panels, palettes, and user-facing surfaces.
### Tier 4: mutating destructive or high-risk
Can destroy active work, inject terminal input, execute commands, or run user-authored content.
Examples:
- closing windows, tabs, panes, or sessions;
- clearing, replacing, or inserting terminal input;
- command execution in a session;
- switching input modes when it can change execution behavior;
- executing Warp Drive workflows or notebooks in a terminal session;
- broad Warp Drive object mutation.
This tier should require explicit user or policy approval for unattended automation and integrations.
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
- a documented risk tier;
- a documented `requires_authenticated_user` value;
- a documented allowed execution context, including whether external clients can run it or whether it is limited to verified Warp-terminal invocations;
- local app-side safety-grant checks;
- deterministic target resolution;
- a handler that reuses existing user-visible app behavior where possible;
- typed success and error responses.
Adding a new action should be additive and reviewable: extend the protocol enum, implement validation, map the action to a risk tier, declare whether it requires an authenticated user, declare its allowed execution contexts, add a handler, and add tests for authentication, safety-policy denial, authenticated-user denial, selector failure, and success behavior.
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
- action name and risk tier;
- target type and opaque target ID when safe;
- success or structured error code.
Avoid logging:
- bearer tokens or scoped credentials;
- terminal output;
- command text for command execution unless explicitly approved by policy;
- input buffer contents;
- Warp Drive object contents;
- environment variable values.
Error-level logs should be used only for conditions that need developer attention, not normal denied requests or user-caused selector failures.
## Security- and safety-relevant errors
Structured errors are part of the security contract.
Important errors include:
- `local_control_disabled` when the user has not enabled local control scripting in Warp settings or has disabled it after credentials were issued;
- `unauthorized_local_client` for missing, malformed, expired, revoked, or invalid credentials;
- `insufficient_permissions` for valid credentials that lack the requested safety tier or target scope;
- `authenticated_user_required` when an action requires a logged-in Warp user but the credential lacks an authenticated-user grant;
- `authenticated_user_unavailable` when the selected Warp app has no logged-in Warp user or cannot access the required authenticated user state;
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
- Local control scripting must be explicitly enabled in Warp's app UI before the action family can run.
- The authoritative enablement state is protected from external writes and is local-only rather than synced.
- The action has a documented tier.
- The action has a documented `requires_authenticated_user` value. New actions default to `true` unless explicitly reviewed as logged-out-safe.
- The action documents allowed execution contexts and whether external clients may run it.
- The bridge maps the action to that tier locally in the selected Warp app process.
- The credential model can express the required grant.
- The credential model can express authenticated-user grants and verified execution context requirements when needed.
- The handler checks optional target restrictions where relevant.
- Requests with invalid credentials or insufficient safety grants fail before selector resolution or mutation.
- Requests that require authenticated-user access fail unless the selected app has a true logged-in Warp user and the credential includes an authenticated-user grant.
- Ambiguous, missing, and stale targets return structured errors.
- Tests cover allowed, insufficient-permission, and denied credential paths.
- Logs and errors do not expose credentials, terminal contents, command text, or sensitive settings.
- Operator docs distinguish available commands from planned catalog entries.
## Platform requirements
### macOS and Linux
Discovery files must be stored in a per-user directory with owner-only permissions.
On macOS, raw credential material and the authoritative local-control enablement state should live in Keychain, not in the discovery record or an ordinary preferences file. Keychain access should be constrained to Warp-owned signed binaries or helpers using code-signing based access control. The enablement state should be writable by the Warp app's in-app settings flow and not writable by `warpctrl`. The discovery record should hold only metadata and a credential reference when local control is enabled.
On Linux, raw credentials and the authoritative enablement state should prefer platform-secure storage where available; otherwise short-lived scoped credentials may live in owner-only local state with strict file and directory permissions. If the enablement state falls back to owner-only local state, the weaker same-user protection should be documented.
Unix domain sockets with peer credential checks may be considered for stronger same-machine identity than bearer tokens alone.
### Windows
Discovery records and credential material must live under the current user's app data directory with ACLs restricted to the current user, Administrators, and SYSTEM.
The authoritative enablement state should use Credential Manager, DPAPI-backed protected storage, or an equivalent app-controlled protected store rather than a normal registry setting that arbitrary same-user processes can write.
Windows support for authenticated local control should not be considered complete until the implementation creates, validates, and tests those ACLs and the protected enablement-state behavior.
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
