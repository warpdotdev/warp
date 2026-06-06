# warpctrl security architecture
`warpctrl` is a local-control CLI for an already-running Warp app instance. Its security architecture is designed to support the control catalog: discovery, structural metadata reads, underlying data reads, app-state mutations, metadata/configuration mutations, underlying data mutations, input-buffer staging, file/path app-state intents, Warp Drive operations, and explicitly authenticated execution-underlying actions. Accepted-command submission and agent-prompt submission remain future high-risk capabilities that require separate product/security review. Local file content operations are intentionally excluded from the public `warpctrl` catalog because native agent file tools are the preferred surface for file content reads and writes.
The correct architecture is not a single shared localhost bearer token with client-side conventions. The CLI, app bridge, and protocol must treat security as a local app-enforced capability system: discovery finds compatible instances, protected storage safeguards the authoritative mode and any future long-lived proof secrets, broker-issued credentials grant one exact action, the running Warp app's local-control bridge verifies that action before dispatch, and target resolution never silently retargets a request.
Exact-action credentials and action-specific approval policy are primarily safety and intent mechanisms, not a hard security boundary against malicious same-user software. They let a user, script, or agent request only the specific operation it intends to perform so authority for a harmless UI action cannot accidentally be reused to expose sensitive content, mutate durable data, or execute commands. They should not be described as strong access control against a process that can already run arbitrary commands as the user.
`warpctrl` has two distinct authorization dimensions: local-control authority and authenticated scripting authority. Local-control authority proves the request is allowed to control the local app. Authenticated scripting authority proves the logged-in Warp user that is allowed to act on user-authenticated data such as Warp Drive objects, AI conversation traces, synced settings, cloud-backed user state, and execution-underlying actions. Logged-out users should retain a smaller local-only control surface, but authenticated-user and high-risk underlying-data mutation actions require a verified Warp-terminal grant tied to the selected app's logged-in user.
## Current foundation status
The current foundation implementation stores a single local-control mode with three choices: disabled by default, enabled within Warp, and enabled everywhere including outside Warp. Verified inside-Warp invocation is specified as future work because the app-issued terminal-session proof broker, proof injection path, and session registry do not exist yet. Until those pieces land, `InvocationContext::InsideWarp` requests must be rejected with `execution_context_not_allowed`, implemented action metadata must not advertise inside-Warp support, and external requests must receive credentials only when the user selects the broadest mode. On Unix, the broker authenticates the connecting OS user through kernel peer credentials before decoding credential requests, then mints short-lived scoped credentials in memory without a stored or bootstrap local-control secret. The current broker therefore trusts the owning OS user rather than authenticating Warp-signed client code. Windows outside-Warp publication remains disabled until discovery-record ACL enforcement and an equivalent authenticated broker transport land.
## Security goals
- Allow trusted local users and approved automation to control a running Warp instance through a stable, scriptable interface.
- Prevent unauthenticated localhost clients from invoking read or mutating control actions.
- Prevent browser-origin JavaScript from becoming an ambient localhost control client.
- Support multiple running Warp processes without a shared global mutating port or global credential.
- Separate discovery metadata from control authority so enumerating an instance does not automatically grant full control.
- Require explicit in-app user enablement before local control scripting from outside Warp can issue credentials or accept control requests.
- Allow local control scripting from verified Warp-managed terminal sessions once proof verification exists and the user selects a mode that permits that context, subject to action policy.
- Store the authoritative local-control mode in protected local storage so external apps cannot enable outside-Warp control by editing ordinary settings.
- Keep credentials out of plaintext discovery records, mint the current short-lived local-control credentials in memory, and protect any future long-lived proof or bootstrap secrets with platform secure storage where available.
- Distinguish verified `warpctrl` invocations that originate from a Warp-managed terminal session from external same-user invocations.
- When the broadest mode enables outside-Warp control, allow external invocations only for the action set explicitly allowed by the action catalog and granted credential.
- Allow in-Warp invocations to receive authenticated-user grants when the selected Warp app has a true logged-in user and the user's local-control mode and action policy permit that grant.
- Support least-privilege exact-action grants for automation and interactive use without relying on an unenforceable identity label.
- Authorize every action by its exact typed identity in the local app bridge, not in the CLI frontend.
- Classify every action by whether it requires an authenticated Warp user. New actions should default to requiring an authenticated user unless they are deliberately reviewed as logged-out-safe and therefore eligible for external use.
- Prevent `warpctrl` from becoming an ambient full-power confused deputy that any same-user process can invoke for high-risk actions.
- Require authenticated scripting identity, an exact execution-action grant, explicit approval or configured policy, deterministic target resolution, and audit records before terminal command execution or typed workflow execution can ship; accepted-command submission and agent-prompt submission remain prohibited until separately reviewed.
- Preserve deterministic targeting so a request never silently mutates or reads the wrong window, tab, pane, session, file/path intent, or Warp Drive object.
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
- prevent ambient direct HTTP calls to the localhost control listener by requiring a just-in-time broker-issued scoped credential;
- use platform secure storage, such as macOS Keychain, for future long-lived proof or bootstrap secrets so they are accessible only to Warp-owned signed code where practical;
- make high-risk operations go through `warpctrl` or a Warp-owned helper where user approval, configured policy, and exact-action grants can be applied;
- avoid giving `warpctrl` ambient non-interactive full-control authority.
In other words, the security model can make ambient direct localhost protocol calls fail, and future protected secrets can make direct credential theft harder. The current Unix broker still allows any process running as the owning OS user to request eligible scoped credentials. It cannot make a same-user malicious app safe if that app can invoke `warpctrl`, connect to the broker, automate the user's desktop, read other local state, or wait for the user to approve prompts.
## Comparison with other local scripting models
Other developer tools expose local automation through a few recurring patterns. The `warpctrl` design should borrow the parts that match Warp's needs while avoiding designs that assume localhost or same-user access is enough by itself.
### VS Code
VS Code's `code` command is primarily a launch and routing CLI: it opens files, folders, diffs, merge views, chat sessions, extension-management commands, and remote/tunnel workflows. It is not a general unauthenticated localhost API for arbitrary UI control of an already-running desktop app.
VS Code's richer local automation runs through extension APIs and extension hosts. Extensions are installed into a trusted editor environment and run with broad access to the workspace or UI side depending on extension kind. Workspace Trust and remote extension placement help users reason about whether code should run locally, remotely, or in a browser sandbox, but they do not create a fine-grained same-user security boundary against arbitrary local software.
Lessons for `warpctrl`:
- a narrow, typed CLI command surface is safer to reason about than exposing arbitrary internal app commands;
- agent and script workflows should request exact actions instead of inheriting ambient full-control authority;
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
- do not rely only on platform automation permission if Warp needs cross-platform, exact-action grants.
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
The resulting architecture should not claim to solve arbitrary same-user malicious-app isolation. Its real value is preventing web-origin and other-user access, preventing unauthenticated direct localhost calls, keeping raw credentials out of plaintext discovery, giving honest scripts and agents narrow exact-action grants, and routing high-risk operations through local Warp app validation and user/policy approval.
## Authoritative enablement model
Warp control has one top-level mode setting based on invocation context:
- **Disabled:** default. No local-control invocation context can receive credentials.
- **Enabled within Warp:** controls `warpctrl` invocations from verified Warp-managed terminal sessions once proof verification exists.
- **Enabled everywhere, including outside Warp:** controls verified Warp-managed terminal invocations and external terminals, scripts, launch agents, IDEs, or other same-user processes.
The mode should live in a new top-level Settings pane page named **Scripting**. The Scripting page owns the user-facing controls for local scripting surfaces, including Warp control, and should explain the difference between commands run inside Warp and commands run from other apps.
The visible UI setting is not enough by itself. The authoritative mode must be stored in the most secure local storage provider available for the platform, with read/write access limited to the Warp application or Warp-owned trusted helper code where the platform supports that restriction. On macOS this means Keychain or an equivalent protected store constrained to Warp-signed code, not ordinary UserDefaults; on Windows this means Credential Manager, DPAPI-backed protected storage, or an equivalent app-controlled protected store; on Linux this means the platform secret service where available, with any owner-only file fallback explicitly documented as weaker. This avoids turning outside-Warp control into a feature that any process can silently enable before invoking `warpctrl`.
Current foundation implementation note: the mode is represented in the typed `LocalControlSettings` group and reads only from Warp's secure storage provider, never ordinary private preferences, `settings.toml`, SQLite, or a synced cloud preference. The implemented setting uses `SyncToCloud::Never` and remains absent from user-visible settings files, generated schemas, Settings Sync, Warp Drive, local-control settings read/write commands, and user-editable or server-backed settings surfaces. It does not migrate or fall back to an earlier private-preferences value; when no valid protected value is available, it fails closed to disabled. This is a tamper-resistant platform storage preference, not a claim that arbitrary same-user compromise is impossible.
Enablement requirements:
- The mode is local-only and must not sync through Settings Sync, Warp Drive, or server-backed user preferences.
- The implemented foundation setting must remain private and absent from user-visible settings files, generated schemas, local-control settings read/write commands, and any allowlisted settings mutation catalog.
- Only the running Warp app, through the Settings > Scripting UI, should be able to change the authoritative mode.
- `warpctrl`, shell scripts, config files, command-line flags, registry edits, defaults writes, and direct local-control protocol requests must not be able to enable or widen the mode.
- The enabled-within-Warp mode may allow verified Warp-terminal invocations once proof verification exists, but turning the mode to disabled should prevent verified Warp-terminal invocations from receiving local-control grants.
- Outside-Warp control requires an intentional user gesture to select the broadest mode; the UI should explain that it allows scripts and automation from other apps to control Warp.
- The mode should be easy to change from the same UI, and narrowing the mode should revoke or invalidate active local-control credentials for invocation contexts no longer allowed.
- If enterprise or managed-device policy is added later, policy may force-disable the mode or force a narrower default, but policy should be separate from user-editable local settings.
Local-control actions that open, focus, or view cloud-backed objects must not create unexpected cloud-synced durable side effects merely because the object was displayed through automation. If an action intentionally mutates synced state, that exact action must require authenticated-user authority plus user or policy approval where applicable.
Disabled-state behavior:
- Warp should not mint scoped local-control credentials for a request whose invocation context is disabled.
- The control listener should reject requests from disabled contexts with a structured disabled-state error before authentication, selector resolution, or handler dispatch.
- Discovery records should avoid publishing actionable endpoint or credential-reference metadata for disabled outside-Warp control. If a minimal record is needed for UX, it should expose only non-sensitive status such as `outside_warp_control_enabled: false`.
- `warpctrl` may detect a disabled context and print instructions to enable it in Settings > Scripting, but it must not offer a command that flips the setting.
- Previously issued credentials must become unusable when their invocation context is no longer allowed, even if their original expiry has not elapsed.
These enablement gates do not create perfect same-user malicious-app isolation. A hostile process with Accessibility or Screen Recording permission might still try to automate the Warp UI. The outside-Warp gate is still important because it closes the much easier paths where external apps silently edit local preferences, call a config CLI, or write synced settings to enable a powerful control surface.
### Exact-action grants and direct policy
The foundation stack should not expose separate per-risk toggles under Settings > Scripting. Once the selected mode allows a request context, the broker issues a short-lived credential for the one typed action requested, and the app bridge verifies that exact action. A credential for one action never authorizes another action, even if both actions read similar data or produce similar mutations.
Sensitive requirements attach directly to actions. Actions that expose terminal output or user content may require authenticated-user access or approval. Actions that execute code, mutate or share Warp Drive objects, change persistent configuration, or cause external effects require authenticated scripting identity, deterministic targets, action-specific approval or configured policy, and audit coverage as specified for that action. Opening or focusing Warp UI must never imply authority to execute commands or mutate user data.
## Trust boundaries
`warpctrl` has several distinct trust boundaries.
### Operating-system user boundary
The baseline local trust boundary is the OS user account. Discovery records and local credential material must be readable only by the owning user. This protects against other local users and network peers, but it does not protect against an already-compromised same-user process.
### Invocation boundary
Same-user does not mean same authority. Interactive use and unattended automation may both run commands under the same user account, but they should be able to intentionally request only the action they need. The protocol needs exact-action credentials that encode the granted action, target scopes, and lifetimes rather than an abstract caller type that the bridge cannot reliably verify.
These scoped credentials are guardrails for well-behaved clients. They prevent accidental overreach and make user intent explicit, but they are not a defense against malicious same-user code that can automate the CLI, inspect the user's environment, or wait for user approvals.
### Warp-terminal execution context boundary
`warpctrl` should be able to receive special grants when it is invoked from within a Warp-managed terminal session, but the bridge must not trust a caller-supplied string such as `caller_class=warp_terminal`. The app should issue a session-bound execution-context proof to Warp-managed terminal sessions and have the broker verify that proof before minting in-Warp-only grants.
Acceptable designs include a short-lived per-session capability, an app-owned broker handshake tied to the terminal session, or an equivalent proof that arbitrary external processes cannot mint by setting an environment variable. Plain environment variables may be used as handles or hints, but they must not be the sole authority for in-Warp privileges because external processes can spoof them.
Verified in-Warp context can raise the maximum eligible grant set, especially for authenticated-user actions. It does not by itself bypass the selected local-control mode, exact-action check, target scopes, or logged-in-user requirements.
### Authenticated scripting boundary
Actions that touch user-authenticated Warp data or perform high-risk underlying-data mutations require authenticated scripting authority. This includes Warp Drive object contents, object mutation, the v0 personal-to-team sharing path, AI conversation traces, cloud-backed user settings, team/account data, typed workflow execution, terminal command execution, and any other surface whose normal app access depends on the user's Warp account or can cause external side effects.
Authenticated scripting uses verified Warp-terminal mode: `warpctrl` presents an app-issued terminal-session proof. If the selected app is logged into Warp and Settings > Scripting mode plus action policy permit authenticated actions from verified Warp terminals, the broker may mint an authenticated-user grant tied to the selected app's current user subject. External API-key authenticated scripting is not part of the selected public contract and requires a separate product/security review before it can be allowlisted.
For app-backed authenticated actions, the app bridge should execute on behalf of the selected app's logged-in user through existing app auth state. If the selected app logs out, switches users, or no longer matches a grant that requires app-user identity, authenticated actions fail with structured errors rather than falling back to logged-out behavior.
Logged-out users may still use the smaller local-only action set explicitly marked as not requiring authenticated scripting authority.
### Authenticated scripting protocol
`warpctrl` should provide auth/status flows for interactive app login. The CLI must not collect Warp passwords.
Requirements:
- `warpctrl auth status [selectors]` reports whether the selected app instance is logged in and whether verified Warp-terminal authenticated grants are available. It may return stable, non-secret subject/scope metadata when the caller has the required grant.
- `warpctrl auth login [selectors]` focuses or opens the selected Warp app's normal sign-in UI and waits, or exits with actionable instructions, until the user signs in through Warp itself.
- The credential broker may mint an app-user authenticated grant only after confirming the selected app has a true logged-in Warp user and the selected mode plus action policy allow the verified invocation context.
- Authenticated credentials are bound to the selected instance, subject, grant mode, scopes, expiry, and optional target/resource restrictions. If the app logs out, switches users, loses authenticated state, or the presented credential subject no longer matches a grant that requires app-user identity, authenticated actions fail with `authenticated_user_mismatch` or another structured authenticated-user error.
- Raw Firebase, server, OAuth, and cloud API tokens must not be exported to `warpctrl` output, shell completions, generated docs, logs, discovery records, or local-control JSON responses.
Logged-out-safe actions continue to use local-control credentials without requiring authenticated scripting identity.
### Application identity boundary
On platforms with secure credential storage, especially macOS, future long-lived proof or bootstrap secrets should be readable only by Warp-owned, correctly signed code. On macOS this means storing that material in Keychain with access constrained by Warp's signing identity, designated requirement, Keychain access group, or equivalent platform mechanism. This narrows extraction from “any same-user process can read a file” to “only trusted Warp-signed code can unwrap the secret.”
This future boundary protects stored secrets from direct theft and can prevent arbitrary apps from using those secrets to make authenticated raw HTTP requests to the local-control listener. It also lets the authoritative mode be stored somewhere harder to modify than ordinary user preferences. The current Unix foundation does not implement this application-identity boundary for local-control credential issuance: it verifies the broker peer's OS user and mints short-lived credentials in memory. Neither model proves that the user personally intended the specific action. Any same-user process may still be able to invoke the trusted `warpctrl` binary or automate the Warp UI. That confused-deputy risk is reduced by explicit in-app enablement, exact-action credential issuance, action-specific policy, and local app-side bridge enforcement, but it is not eliminated as a hard same-user security boundary.
### Action boundary
Every credential grants one exact typed action. The bridge must compare the requested action to that granted action before selector resolution or handler dispatch. Actions with stronger identity, context, target, approval, or audit requirements declare and enforce those requirements directly.
### Target boundary
A valid credential for one instance or target must not imply authority over another. Credentials should be bound to the issuing Warp instance and may be further scoped to target families such as terminal sessions, files, or Warp Drive objects when those surfaces are exposed.
## Threat model
### In scope
- Other local OS users attempting to control a Warp instance owned by the current user.
- Browser-origin JavaScript attempting to call localhost control endpoints.
- Same-user automation attempting an action without a credential for that exact action.
- Same-user processes attempting to extract plaintext credentials from local state.
- Same-user processes invoking `warpctrl` as a confused deputy for actions the process does not hold exact-action authority for directly.
- External same-user processes attempting authenticated-user actions that should be limited to verified Warp-terminal invocations.
- Logged-out requests attempting actions that require a true logged-in Warp user.
- Stale discovery records from exited Warp processes.
- Multiple running Warp instances where ambiguous selection could target the wrong process.
- Malformed clients attempting unknown, unsupported, unallowlisted, or invalid action payloads.
- Valid clients attempting actions other than the exact action granted by their credential.
- Explicit target IDs that become stale between discovery and execution.
- Future handlers that expose terminal data, settings writes, input mutation, command execution, file intents, or Warp Drive object operations.
### Out of scope
- A malicious process that already has arbitrary same-user filesystem and process access, except that scoped credentials should still reduce accidental over-granting to ordinary automation.
- Kernel, hypervisor, or administrator-level compromise.
- Security semantics for remote URL control endpoints. Remote control requires a separate transport and identity design before it can ship.
## Architecture overview
The full security model has eight layers. The current foundation branch implements the single mode gate with disabled as the default, allows outside-Warp credentials only in the broadest mode, and keeps the inside-Warp execution-context layer as a rejected future protocol concept until proof verification exists.
The security model has eight layers:
1. **Protected enablement:** Use protected local storage for the single local-control mode, with all contexts disabled by default, inside-Warp allowed only when the user selects the within-Warp or broadest mode after proof support lands, and outside-Warp off unless the broadest mode is selected.
2. **Discovery:** Find compatible live Warp instances without granting broad authority.
3. **Secret handling:** Mint the current short-lived local-control credentials in memory, keep all secrets outside plaintext discovery records, and restrict future stored proof or bootstrap secrets to trusted Warp-owned code where the platform supports it.
4. **Execution context verification:** Distinguish verified Warp-terminal invocations from external same-user invocations without trusting caller-declared labels.
5. **Credential issuance:** Issue exact-action credentials with explicit lifetimes only when the selected mode allows the request's invocation context and policy allows the requested action.
6. **Transport authentication:** Reject disabled or unauthenticated requests before reading or mutating app state.
7. **Safety and user-auth policy:** Enforce the exact granted action, target scopes, execution-context requirements, authenticated-user requirements, and direct action policy locally in the app bridge.
8. **Deterministic dispatch:** Resolve targets exactly and invoke only allowlisted typed handlers.
```mermaid
sequenceDiagram
    participant Invoker as User / Automation
    participant CLI as warpctrl
    participant Registry as Per-user discovery registry
    participant Enablement as Protected enablement state
    participant Context as Execution context proof
    participant Broker as Credential broker
    participant Auth as App auth state
    participant HTTP as Warp control listener
    participant Bridge as App bridge + safety policy
    participant UI as Warp app state

    Invoker->>CLI: Invoke allowlisted command
    CLI->>Registry: Read instance metadata
    Registry-->>CLI: instance_id, endpoint, protocol version, broker reference
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
    Broker->>Broker: Mint short-lived scoped credential in memory
    Broker-->>CLI: Scoped credential with grants, context, user scope, expiry
    CLI->>HTTP: Authenticated typed request
    HTTP->>Bridge: Verify credential and protocol envelope
    Bridge->>Bridge: Check exact action + context + authenticated-user + target scope
    alt Denied
        Bridge-->>CLI: structured safety-policy error
    else Allowed
        Bridge->>UI: Resolve target exactly and run allowlisted handler
        UI-->>Bridge: typed result or structured target error
        Bridge-->>CLI: response envelope
    end
    end
```
## Current foundation discovery and request flow
The current Unix foundation deliberately uses three different mechanisms for three different jobs:
1. **Private filesystem discovery finds candidate instances.** `crates/local_control/src/discovery.rs` defines the shared registry format and validation rules. Each enabled Warp process publishes an owner-only JSON record that tells clients which instance exists, which actions it implements, its exact loopback HTTP endpoint, and the filename of its instance-specific credential-broker socket. The record contains routing metadata, not a bearer token or other control authority.
2. **The Unix-domain socket authenticates the OS user and issues authority.** The broker socket is the protected bootstrap path from discovery metadata to a short-lived exact-action credential. The Warp app obtains the connecting process's UID from kernel peer credentials before decoding its request, then evaluates current policy and, if allowed, returns an in-memory credential for the one requested action.
3. **The loopback HTTP endpoint carries the typed action.** The client presents the broker-issued credential to the selected instance's `/v1/control` endpoint. The app validates the endpoint headers and credential, then hands the typed request to the app bridge for current-policy, exact-action, target, and handler validation.

The filesystem record and Unix socket are therefore complementary, not alternative discovery mechanisms. The JSON record is how a client learns that an instance and broker exist. The socket path in that record is how the client asks that selected instance for temporary authority. HTTP is how the client uses that authority. A client does not discover instances by enumerating or querying the Unix socket, and it cannot control an instance merely by reading its JSON record.

### Server publication lifecycle
`app/src/local_control/mod.rs` owns the running app side of all three mechanisms. When the feature, platform support, and protected Settings > Scripting mode allow outside-Warp control, the app:
1. binds an ephemeral TCP port on exactly `127.0.0.1`;
2. creates an `InstanceRecord` containing that endpoint and an instance-derived broker socket filename;
3. publishes the record through `RegisteredInstance::register`;
4. binds the referenced Unix-domain socket inside the same owner-only discovery directory;
5. starts the credential broker on the Unix socket and the typed control handler on `/v1/control`.

When outside-Warp control is disabled or the server stops, the app drops the registration and runtime. Graceful drop removes the JSON record and broker socket. Discovery scans provide the crash-recovery path by rejecting and pruning records whose PID is no longer alive.

The default discovery directory is `~/.warp/local-control/`. `WARP_LOCAL_CONTROL_DISCOVERY_DIR` overrides it, and `$XDG_RUNTIME_DIR/warp/local-control` is preferred when `XDG_RUNTIME_DIR` is present. On Unix, the directory is restricted to `0700`, while discovery records and broker sockets are restricted to `0600`. An enabled instance publishes files shaped like `inst_<id>.json` and `inst_<id>.broker.sock`. These permissions meaningfully protect the registry and broker from other OS users, but they do not distinguish among processes running as the owning user.

### Client discovery and invocation
A client invocation follows this sequence:
1. Read JSON records from the per-user discovery directory.
2. Parse compatible records and reject records with a mismatched protocol version, malformed authority, or dead PID.
3. Require the HTTP host to be exactly `127.0.0.1` and the broker reference to be the filename derived from the selected `instance_id`. This prevents a record from redirecting the client to an arbitrary network host or arbitrary socket path.
4. Probe a candidate by connecting to its broker, requesting an exact-action `app.ping` credential, making an authenticated HTTP ping, and verifying the returned `instance_id`. This removes records that name an unresponsive or inconsistent instance.
5. Select one compatible instance. If selection is ambiguous, require the user to identify the intended instance rather than silently targeting one.
6. Connect to the selected instance's Unix broker socket and request a credential for the exact action about to be invoked.
7. Present that credential only to the exact loopback endpoint from the validated record and send the typed action request.

The probe is intentionally authenticated. Merely binding the stale record's old TCP port is insufficient to impersonate a live Warp instance because a port squatter cannot issue a credential through the instance-derived broker socket or satisfy the selected instance's in-memory credential lookup.

### What the Unix broker contributes
The key security property of the Unix-domain socket is kernel-authenticated peer identity. Before the broker reads or decodes a credential request, it calls the platform peer-credential API and verifies that the connecting process's UID equals Warp's effective UID. The caller cannot forge this kernel-reported UID through request data, environment variables, a claimed PID, or a username string.

The broker also provides a protected just-in-time credential bootstrap path:
- bearer credentials are never written into discovery records;
- there is no reusable bootstrap token for a client to read from disk and send to a stale or squatted TCP endpoint;
- the running Warp app can evaluate the protected Scripting mode and direct action policy at issuance time;
- every issued credential is bound to the selected instance, one exact action, an invocation context, and a short expiry;
- issued credentials exist only in the running app's process-local credential map and the requesting client's memory.

The instance-derived socket filename and owner-only discovery directory bind the broker reference to the selected record and make arbitrary socket-path injection fail validation. Socket permissions provide an additional owner-only filesystem check, while peer credentials provide the authoritative same-UID check after connection.

### What the HTTP and app bridge contribute
The HTTP listener is a transport for the final typed action, not the discovery or credential-issuance mechanism. Knowing or guessing its loopback port is insufficient. Before dispatch, the app:
- rejects requests carrying a browser-style `Origin` header;
- requires the `Host` header to exactly match the selected `127.0.0.1:<port>` endpoint;
- requires a bearer credential present in the selected instance's process-local credential map;
- rejects missing, malformed, expired, revoked, or wrong-instance credentials;
- decodes the typed request only after transport authentication;
- has the app bridge re-check current Scripting mode, invocation context, exact granted action, authenticated-user requirements, target restrictions, and allowlisted handler dispatch.

These checks defend against browser-origin clients, network clients, unauthenticated clients that discover or guess the TCP port, stale or malformed records, wrong-instance credentials, and accidental action overreach. Loopback binding and header checks harden the endpoint, but the broker-issued credential and app-side checks remain the authority.

### Current boundary and limitation
The current broker authenticates the **OS account**, not the identity of the application running under that account. It does not prove that the caller is the official `warpctrl` binary, Warp-signed code, a process launched from a Warp terminal, or a human-approved invocation. Once the user enables outside-Warp control, any process running as that OS user can connect to the broker and request credentials for actions that current policy allows from the external invocation context.

The current architecture therefore provides a meaningful hard boundary against other OS users, browsers, network peers, and unauthenticated direct HTTP clients. For same-user software, protected enablement, short expiry, exact-action grants, app-side revalidation, deterministic targeting, and future approval policy are least-privilege and intent guardrails rather than strong isolation. A stronger same-user boundary would require an additional application-identity or user-intent mechanism, such as platform code-signature validation, verified Warp-terminal session proof, or per-action user approval.
## Discovery registry
Each participating Warp process writes a discovery record in a secure per-user local-control directory. Discovery records are metadata, not a full control-authority model.
A discovery record should contain:
- opaque `instance_id`;
- PID and process start timestamp;
- channel and build metadata;
- protocol version and supported capability summary;
- loopback endpoint for the instance-local control listener;
- credential broker reference that can mint a just-in-time scoped credential for a requested action, not a bearer token or reusable control credential.
Discovery rules:
- Records must be readable only by the owning user.
- POSIX records must use owner-only permissions such as `0600` for files and a non-world-readable directory.
- Windows records must live under the current user's app data directory with ACLs limited to the current user, Administrators, and SYSTEM.
- When outside-Warp control is disabled, records must not publish actionable control endpoints or credential references for external clients. A minimal disabled-status record is acceptable only if it contains no authority.
- The CLI must prune or ignore stale records whose PID is gone or whose health/protocol check fails.
- If multiple compatible instances are ambiguous, the CLI must require explicit `--instance` selection.
- Discovery metadata must not expose terminal contents, environment variables, auth tokens for cloud services, raw local-control credentials, or mutating capability grants.
- Discovery must not publish actionable endpoints or credential broker references for an invocation context unless the protected mode currently enables that context. Future UI should support temporary or session-scoped enablement and a quick path back to disabled so one-off control use does not leave an unexpectedly durable passive discovery surface.
## Credential model
The full `warpctrl` catalog requires scoped credentials. A single shared full-power bearer token is not sufficient once automation, underlying data reads, app-state mutations, metadata/configuration mutations, and underlying data mutations are supported.
### Credential properties
Current foundation implementation note: `warpctrl` discovers a loopback control endpoint and an instance-bound Unix-domain-socket broker reference, then requests a short-lived credential over that socket for the specific action it is about to invoke. The broker authenticates the connecting peer's OS user before decoding the request. The discovery record does not contain bearer tokens, raw credential material, or a stored credential that the CLI unwraps and sends to the discovered port.
A control credential should encode or reference:
- issuing Warp instance;
- protocol version or accepted version range;
- the one granted `ActionKind`;
- verified execution context, such as external client or Warp-managed terminal session;
- whether the credential may act on behalf of an authenticated Warp user;
- authenticated Warp user subject or stable user reference when an authenticated-user grant is present;
- optional target restrictions, such as one session, one workspace, one file path, or one Warp Drive object type;
- issued-at time;
- expiry time or process-lifetime binding;
- unique credential ID for revocation and auditing;
- integrity protection so callers cannot forge or widen grants.
### Credential issuance
Warp should issue credentials through an app-owned local broker or equivalent trusted path. The broker decides whether to issue a credential based on the requested exact action, target scope, user configuration, execution context, authenticated-user requirements, and any explicit user approval.
Recommended defaults:
- Credential issuance is unavailable unless the protected enablement state allows the request's invocation context.
- Commands request only the exact action they are about to invoke.
- External same-user invocations are limited to the smaller logged-out-safe local action set and cannot receive authenticated-user authority.
- Verified Warp-terminal invocations may request broader sets of actions over time, but each credential remains scoped to one action.
- App-user authenticated grants are available only when the selected Warp app has a true logged-in Warp user and a verified Warp-terminal execution context allowed by local-control settings.
- Actions that expose sensitive content, mutate durable data, execute code, or cause external effects require the direct identity, approval, target, and audit conditions specified for that action.
Callers do not manage low-level scope strings. They request a typed action, and the app-owned broker evaluates that action's configured policy, execution context, target restrictions, authenticated-user requirements, and any approval or consent prompt. If the action is denied, the broker or bridge returns a structured error. The broker must not issue broader authority merely because the request came from the signed `warpctrl` binary. The CLI must not mint its own authority, and the app bridge remains the enforcement point because direct protocol clients can bypass the CLI.
### Exact-action grants, not strong access control
Exact-action credentials prevent an honest client from accidentally reusing authority for a different operation and give Warp a structured point to prompt, deny, or audit sensitive actions. They do not make untrusted same-user software safe. A malicious local process may invoke `warpctrl`, simulate user workflows, or use other OS-level capabilities outside `warpctrl`.
### Credential storage
The current Unix foundation stores no bootstrap or long-lived local-control secret; it mints short-lived scoped credentials in memory. Future credential and proof storage should be platform-appropriate:
- Local discovery may store a credential reference rather than the credential itself.
- The authoritative local-control mode should use the same class of protected local storage as raw credential material, but it should be accessible to the Warp app for the Settings > Scripting UI and not writable by `warpctrl` or arbitrary external apps.
- Raw long-lived credentials should prefer platform-secure storage such as macOS Keychain or Windows Credential Manager when practical.
- On macOS, raw control secrets should be stored in Keychain and restricted to trusted Warp-signed code using a designated requirement, Keychain access group, trusted-application ACL, or equivalent code-signing based mechanism. Restricting by filesystem path alone is insufficient because paths can be replaced or wrapped.
- Keychain item access should include the Warp app, the signed `warpctrl` binary, and any signed Warp-owned local broker/helper that needs to unwrap raw secrets. It should exclude arbitrary same-user applications.
- Short-lived credentials may be stored in owner-only local state if their lifetime and scope are narrow.
- Credentials must never be printed in human-readable output, JSON output, logs, errors, or shell completion data.
### Confused-deputy mitigation
When future secrets use application-identity-constrained secure storage, it can prevent arbitrary apps from reading the token; it does not prevent arbitrary apps from asking trusted Warp code to use the token on their behalf. The current owner-authenticated Unix broker provides no Warp-signed-code boundary against same-user clients.
For example, if `warpctrl` can silently unwrap a full-power credential and execute any action, another same-user process can invoke `warpctrl input run ...` without reading the credential directly. That makes `warpctrl` a confused deputy.
Mitigations:
- Do not give `warpctrl` ambient non-interactive access to an unrestricted full-control credential.
- Prefer action-scoped or session-scoped credentials minted just in time by the broker.
- Require explicit user approval or preconfigured policy for underlying data mutations and other sensitive grants.
- Distinguish user-approved credential requests from ambient unattended invocations through explicit approval prompts, configured policy, terminal/session context, or narrow credential request flows.
- Bind issued credentials to the requested instance, exact action, optional target scope, and short expiry.
- Prune expired grants and cap the process-local active-grant set. The low-risk foundation slice may reuse an unexpired scoped grant, but a replay policy is required before broader or higher-risk action families ship.
- Let `warpctrl` preflight and request credentials, but require the local app bridge to enforce scopes because direct protocol clients can bypass the CLI.
- Make denials structured and non-fatal for automation so callers can request narrower or user-approved grants rather than falling back to unsafe behavior.
These mitigations are about routing high-risk operations through intentional `warpctrl` flows rather than exposing a reusable localhost token to any process. They should not be documented as a guarantee that arbitrary same-user applications cannot cause Warp-visible actions.
## Transport authentication
The default transport is an instance-local loopback listener bound to `127.0.0.1` on an ephemeral per-process port.
The current just-in-time credential broker avoids the specific stale-record bearer-token phishing failure mode where `warpctrl` unwraps a long-lived Warp-held credential and sends it to a port squatter. It uses an instance-bound Unix-domain socket inside the owner-only discovery directory and checks the peer OS user before reading the credential request. If future designs add stored bootstrap credentials, server-held secrets, or reusable credential references that must be presented to the discovered endpoint, the client must verify the server's identity before sending that material.
Transport requirements:
- Bind only to loopback for local control.
- Do not set permissive CORS headers.
- Reject any request carrying an `Origin` header.
- Reject any request whose `Host` header is not exactly `127.0.0.1:<selected-port>` for the selected discovery record.
- Reject discovery records unless the published control endpoint uses exactly `127.0.0.1` and the broker socket reference is the selected instance's expected filename inside the owner-only discovery directory.
- Reject control requests when their inside-Warp or outside-Warp invocation context is disabled, even if the request presents an otherwise valid credential.
- Authenticate every control request locally in the selected Warp app process before selector resolution or action dispatch.
- Reject missing, malformed, expired, revoked, or invalid credentials with structured authentication errors.
- Keep unauthenticated health metadata minimal and non-sensitive.
- Preserve structured error envelopes so the CLI does not collapse security failures into generic transport errors.
Remote URL support is a separate future transport mode. It should not reuse the local same-user credential model without additional identity, encryption, replay protection, and remote approval/policy design.
## Logged-in user requirements
Local-control validation always begins with local protocol state: discovery records, secure local credential references, exact-action grants, execution-context proof, protocol version, request shape, allowlisted actions, typed parameters, and deterministic target selectors.
Some actions additionally require authenticated scripting authority from a true logged-in Warp user in the selected app and a verified Warp-terminal invocation. The action allowlist must declare this explicitly with a `requires_authenticated_user` or equivalent authenticated-scripting requirement field.
Default rule for new actions:
- New actions require an authenticated Warp user unless the implementer deliberately classifies them as logged-out-safe.
- The logged-out-safe set should remain meaningfully smaller and limited to local app structure, local appearance metadata, and other surfaces that do not depend on the user's cloud-backed Warp identity.
- Actions that read or mutate Warp Drive, AI conversation traces, synced settings, team/account data, or other user-authenticated state must require an authenticated user.
- Actions that execute user-authored cloud-backed content, such as running typed Warp Drive workflows, require authenticated scripting authority plus the direct approval, targeting, and audit requirements for that exact action. Agent-prompt submission remains excluded until separately reviewed.
When an authenticated-user or authenticated-scripting action is requested:
- app-user mode requires the selected app to have an active logged-in Warp user;
- the presented local-control credential must include an authenticated grant for that user;
- the selected mode, action policy, and authenticated-scripting policy must allow authenticated actions for the verified Warp-terminal execution context;
- the app bridge should execute app-user actions through the app's existing authenticated state rather than exporting raw cloud auth credentials to `warpctrl`.
If these conditions are not met, the app returns a structured error. It must not fall back to logged-out behavior or silently omit user-authenticated data from a result that claims success.
## Exact-action policy model
Exact-action grants are enforced in the app bridge after transport authentication and before target resolution or handler dispatch. This provides consistent “do not accidentally do more than requested” behavior for honest clients, not a sandbox for hostile same-user code.
The bridge path must:
1. Authenticate the transport credential before decoding the typed request envelope.
2. Parse the typed request envelope.
3. Verify protocol version compatibility.
4. Determine the exact granted action, execution context, target scopes, and authenticated-user grant.
5. Compare the requested action to the granted action and load that action's direct policy requirements.
6. Check optional target-family restrictions, authenticated-user requirements, and action-specific approval or audit prerequisites.
7. Reject a request for any different action with `insufficient_permissions`.
8. Reject authenticated-user actions without the required app-user login or authenticated grant with a structured authenticated-user error.
9. Only then resolve selectors and invoke the allowlisted handler.
The CLI frontend may provide helpful preflight errors, but those checks are advisory. Local app-side bridge enforcement is mandatory because other tools can bypass the official CLI and speak the protocol directly.
## Direct action requirements
Action authorization is defined per typed action rather than by assigning actions to permission buckets. Similar actions remain independently authorized.
- Structural metadata actions such as `instance.list`, `window.list`, or `theme.list` each require their own grant and must not expose terminal content or other user data.
- Content-bearing reads such as block output, input buffers, command history, Warp Drive content, or AI conversations each require their own grant and any direct authenticated-user or approval policy specified for that action.
- UI actions such as creating tabs, focusing panes, opening files, or staging input each require their own grant and must not imply authority to execute commands, change persistent settings, or mutate user data.
- Persistent settings and metadata changes each require their own grant and allowlist validation.
- Actions that execute code, mutate or share Warp Drive objects, mutate AI content, or cause external effects each require their own grant plus authenticated scripting identity, explicit approval or configured policy, deterministic targeting, and audit coverage where specified.
Accepted-command submission and agent-prompt submission remain unavailable until separately reviewed, regardless of what other actions a credential grants.
## Target scoping and deterministic resolution
Targeting is part of security. The protocol must not convert ambiguous or stale selectors into best-effort mutations.
Rules:
- Instance selection happens before request dispatch and must be explicit when ambiguous.
- `active` selectors may be ergonomic defaults only when the resolved target is unambiguous. For window-scoped mutations, the resolver first uses the active window and may fall back to the sole existing window when exactly one window exists.
- If no active target exists for a mutating request and no action-specific deterministic fallback applies, return `missing_target` or `invalid_selector`; if multiple fallback candidates exist, return `ambiguous_target`.
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
- documented authenticated-user, invocation-context, target, approval, and audit requirements;
- a documented `requires_authenticated_user` value;
- a documented allowed execution context, including whether external clients can run it or whether it is limited to verified Warp-terminal invocations;
- local app-side exact-action grant checks;
- deterministic target resolution;
- a handler that reuses existing user-visible app behavior where possible;
- typed success and error responses.
Adding a new action should be additive and reviewable: extend the protocol enum, implement validation, declare whether it requires an authenticated user, declare its allowed execution contexts and direct policy requirements, add a handler, and add tests for authentication, exact-action denial, authenticated-user denial, selector failure, and success behavior.
## Browser and localhost protections
Loopback is not sufficient by itself because browsers can send requests to localhost.
This section is not a browser-only defense and must not rely on CORS as the primary control. Non-browser local clients can also send HTTP requests, so the local app must enforce credentials, invocation-context gating, app-side authorization, and endpoint hardening for every request.
Required protections:
- No permissive CORS on control endpoints.
- Reject any request that includes an `Origin` header.
- Reject any request whose `Host` header is not exactly the selected `127.0.0.1:<port>` endpoint.
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
- action name and applicable direct policy requirements;
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
- `insufficient_permissions` for valid credentials that grant a different action or do not include the requested target scope;
- `authenticated_user_required` when an action requires authenticated scripting authority but the credential lacks an authenticated-user grant;
- `authenticated_user_unavailable` when the selected Warp app has no logged-in Warp user or cannot access the required authenticated user state;
- `authenticated_user_mismatch` when an authenticated-user credential is bound to a different user subject than the user currently logged in to the selected Warp app;
- `execution_context_not_allowed` when the action or requested grant is not allowed from the verified invocation context, such as an external client attempting an in-Warp-only authenticated-user action;
- `ambiguous_instance` when multiple compatible instances cannot be resolved safely;
- `invalid_selector` for malformed or unsupported selector syntax;
- `missing_target` when an active/default target does not exist and no deterministic fallback target exists;
- `stale_target` when an explicit target ID no longer exists;
- `unsupported_action` for actions not implemented by the selected instance;
- `not_allowlisted` for actions intentionally excluded from the public control surface;
- `invalid_params` for malformed parameters;
- `target_state_conflict` when the target exists but cannot support the requested action.
The app must not downgrade these failures into broader default actions, and the CLI must preserve structured server errors in both human-readable and JSON output.
## Required controls before full catalog expansion
Before shipping each action family, verify that these controls are implemented for that family:
- Local control scripting must be enabled for the request's invocation context before the action family can run; disabled mode blocks all contexts, the within-Warp mode allows inside-Warp only once proof verification exists, and outside-Warp control requires the broadest mode.
- The authoritative mode lives under Settings > Scripting, is protected from external writes, and is local-only rather than synced.
- The action has documented authenticated-user, invocation-context, target, approval, and audit requirements.
- The action has a documented `requires_authenticated_user` value. New actions default to `true` unless explicitly reviewed as logged-out-safe.
- The action documents allowed execution contexts and whether external clients may run it.
- The bridge verifies the credential grants that exact action locally in the selected Warp app process.
- The credential model grants the exact requested action.
- The credential model can express authenticated-user grants and verified execution context requirements when needed.
- The handler checks optional target restrictions where relevant.
- Requests with invalid credentials or credentials for a different action fail before selector resolution or mutation.
- Requests that require authenticated-user access fail unless the selected app has a true logged-in Warp user and the credential includes an authenticated-user grant.
- Ambiguous, missing, and stale targets return structured errors.
- Tests cover the allowed path, use of a different action credential, and denied credential paths.
- Logs and errors do not expose credentials, terminal contents, command text, or sensitive settings.
- Operator docs distinguish available commands from planned catalog entries.
- Initial public action-family docs and tests prove terminal command execution, workflow execution, accepted-command submission, and agent-prompt submission are not allowlisted; input-buffer staging never submits the buffer.
- Initial public action-family docs and tests prove local file content reads, writes, appends, deletes, and filesystem-content mutations are not allowlisted; file/path support is limited to opening visible Warp UI surfaces and listing files already open in Warp.
## Platform requirements
### macOS and Linux
Discovery files must be stored in a per-user directory with owner-only permissions.
On macOS, the authoritative local-control mode and any future long-lived proof or bootstrap secrets should live in Keychain, not in the discovery record or an ordinary preferences file. Keychain access should be constrained to Warp-owned signed binaries or helpers using code-signing based access control. The mode should be writable by the Warp app's Settings > Scripting flow and not writable by `warpctrl`. The discovery record should hold only metadata and a credential reference when the selected mode allows the relevant invocation context.
On Linux, the authoritative mode and any future long-lived proof or bootstrap secrets should prefer platform-secure storage where available; otherwise short-lived scoped credentials may live in owner-only local state with strict file and directory permissions. If the mode falls back to owner-only local state, the weaker same-user protection should be documented.
The current Unix foundation uses an instance-bound Unix-domain-socket credential broker with peer credential checks before request decoding.
### Windows
Discovery records and credential material must live under the current user's app data directory with ACLs restricted to the current user, Administrators, and SYSTEM.
The authoritative mode should use Credential Manager, DPAPI-backed protected storage, or an equivalent app-controlled protected store rather than normal registry settings that arbitrary same-user processes can write.
Windows support for authenticated local control should not be considered complete until the implementation creates, validates, and tests those ACLs and protected mode behavior.
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
