# Context
`PRODUCT.md` defines a standalone local Warp control CLI binary, provisionally named `warpctrl`, with an allowlisted action catalog, deterministic addressing across multiple running Warp app processes, and an incremental implementation plan.
`SECURITY.md` is the normative security architecture for this feature. Implementation work must follow it for the top-level Settings > Scripting surface, protected enablement storage, granular permissions, discovery metadata, credential storage, scoped safety grants, verified execution context, authenticated-user requirements, localhost/browser protections, permission-category enforcement, deterministic target resolution, and local app-side validation. The long-term architecture includes separate verified inside-Warp and outside-Warp invocation contexts, but the current foundation implementation supports outside-Warp requests only and must reject `InvocationContext::InsideWarp` until the verified Warp-terminal proof broker lands. If this technical plan and `SECURITY.md` disagree, update the plan before implementing rather than treating the security architecture as optional follow-up work.
The existing app already has three relevant building blocks:
- `crates/http_server/src/lib.rs (7-61)` runs a native-only loopback Axum server on fixed port `9277`.
- `app/src/lib.rs (1993-2001)` registers that HTTP server in the native app and currently merges only installation-detection and profiling routers.
- `crates/app-installation-detection/src/lib.rs (15-60)` and `app/src/profiling.rs (208-242)` show the current local HTTP routes. They are narrow endpoints, not a general control plane.
Warp also already has the app-side behaviors the control API should reuse rather than reimplement:
- `app/src/terminal/view/action.rs (193-196)` defines split-pane terminal actions.
- `app/src/pane_group/mod.rs (4266-4360, 5377-5414)` shows pane creation/splitting semantics and how split events mutate pane layout.
- `app/src/workspace/action.rs (153-156)` defines the existing tab creation actions, including default and terminal-tab variants.
- `app/src/workspace/view.rs (21203-21244)` shows how user-visible default and terminal-tab actions are dispatched.
- `app/src/settings/theme.rs (9-82)` defines persisted theme settings.
- `app/src/themes/theme_chooser.rs (416-458)` shows persisted theme selection behavior.
- `app/src/workspace/action.rs (95-776)` is the largest existing inventory of user-visible workspace actions and informs the allowlist catalog.
- `app/src/workspace/util.rs (12-18)` defines `PaneViewLocator`, and `app/src/pane_group/pane/mod.rs (84-177)` defines serializable pane identifiers, both useful reference points for selector resolution.
- `app/src/uri/mod.rs (822-1093, 1166-1364)` demonstrates external intents being resolved into active windows/workspaces and dispatched into running app state.
The current Oz CLI build/distribution model is also directly relevant because the control CLI should follow the same standalone-artifact approach rather than relying on the Warp GUI executable to service ordinary shell invocations:
- `crates/warp_cli/src/lib.rs (88-188, 316-418)` defines the existing CLI/parser conventions and channel-specific command naming support.
- `app/src/lib.rs (631-746)` routes CLI invocations into CLI execution rather than GUI launch.
- `script/macos/bundle (353-735)` and `script/linux/bundle (157-294)` build standalone CLI artifacts with the `standalone` feature.
- `.github/workflows/create_release.yml (423-554, 660-858, 992-1276)` publishes macOS/Linux CLI artifacts.
- `script/windows/windows-installer.iss (235-263)` shows the current Windows helper-wrapper pattern for CLI access.
The most important constraint surfaced by this code is that the current fixed-port local HTTP server cannot be the entire solution for a multi-process control API. If multiple local Warp processes attempt to expose mutating routes through the same fixed port, only one can own it. The control design therefore needs explicit per-process discovery and addressing.
## Proposed changes
### 0. Security architecture dependency
Before implementing any local-control listener, CLI command, credential path, or action handler, the implementation must be checked against `SECURITY.md`.
Required security gates:
- Long term, local control scripting has separate inside-Warp and outside-Warp enablement states. Inside-Warp control for verified Warp-managed terminal sessions can default on only after the app-issued proof broker exists; outside-Warp control for external terminals, scripts, IDEs, launch agents, and other same-user processes defaults off.
- In the current foundation slice, only outside-Warp enablement and permissions are implemented. Inside-Warp credential requests must be rejected and inside-Warp settings must not be exposed in the UI.
- In the long-term model, both controls live under a new top-level Settings pane page named **Scripting**.
- The authoritative enablement states are local-only, not Settings Sync'd, and stored in protected local storage rather than ordinary user-editable settings.
- The current foundation branch must mark all implemented outside-Warp local-control settings as `private: true` and `sync_to_cloud: SyncToCloud::Never`. They must not appear in the user-visible `settings.toml` file, generated settings schema, Settings Sync, Warp Drive, server-backed preferences, or any future `warpctrl settings` surface.
- `warpctrl`, direct protocol requests, shell scripts, config files, registry/plist edits, defaults writes, and server-backed preferences must not be able to enable either setting.
- Discovery records do not publish actionable endpoints or credential references for disabled outside-Warp control.
- Credential issuance is unavailable when the request's invocation context is disabled.
- Raw credential material is kept out of plaintext discovery records and stored in platform secure storage where available.
- The broker distinguishes verified Warp-terminal invocations from external invocations using an app-issued execution-context proof, not a caller-declared label. Until that broker exists, `InsideWarp` is a reserved protocol concept that must not receive credentials.
- External invocations default to a smaller logged-out-safe action set that does not touch user-authenticated data.
- Verified Warp-terminal invocations may receive authenticated-user grants only when the selected app has a true logged-in Warp user and local-control settings allow authenticated-user actions from Warp terminals.
- The app rejects disabled, unauthenticated, expired, revoked, insufficient-scope, unsupported, malformed, ambiguous, missing-target, and stale-target requests with structured errors.
- Every action has a documented state/data category and the app bridge enforces the required permission category locally before selector resolution or handler dispatch.
- Every action has a documented `requires_authenticated_user` value and allowed execution contexts. New actions default to requiring an authenticated user unless explicitly reviewed as logged-out-safe.
- Granular local-control settings under Settings > Scripting gate the maximum grants for metadata reads, underlying data reads, app-state mutations, metadata/configuration mutations, underlying data mutations, authenticated-user actions from Warp terminals, and authenticated-user actions from external clients.
- Permission categories are treated as user-intent and accident-prevention guardrails, not as strong same-user malicious-app isolation.
- Remote control remains out of scope for the local same-machine credential model.
The first implementation slice should include the protected enablement gate, credential issuance checks, and app-side permission-category enforcement even if the only mutating action initially implemented is `tab.create`. Shipping `tab.create` without the enablement and validation architecture would create the wrong foundation for the full catalog.
### 1. Protocol crate and stable envelope
Create a small shared protocol crate or equivalent shared module used by both the app server and standalone CLI client. It should define:
- Protocol version metadata.
- Discovery/health response types.
- Execution-context proof/request types for verified Warp-terminal invocations versus external invocations.
- Action metadata describing state/data category, required permission grant, `requires_authenticated_user`, allowed execution contexts, and target families.
- Selector types:
  - `InstanceSelector`
  - `WindowSelector`
  - `TabSelector`
  - `PaneSelector`
  - `SessionSelector`
  - `BlockSelector`
  - `FileSelector`
  - `DriveObjectSelector`
- Opaque protocol-facing ID newtypes for instance/window/tab/pane/session identifiers.
- Allowlisted `ControlAction` variants and typed parameter payloads.
- Success/error envelopes with stable machine-readable error codes.
The protocol should treat target IDs as opaque. The app may encode existing runtime identifiers internally, but the public wire contract should not require callers to understand `EntityId`, `PaneId`, or other implementation types.
Recommended selector variants:
- `InstanceSelector`: `Active`, `Id(InstanceId)`, `Pid(u32)`.
- `WindowSelector`: `Active`, `Id(WindowId)`, `Index(u32)`, `Title(String)`.
- `TabSelector`: `Active`, `Id(TabId)`, `Index(u32)`, `Title(String)`.
- `PaneSelector`: `Active`, `Id(PaneId)`, `Index(u32)`.
- `SessionSelector`: `Active`, `Id(SessionId)`, `Index(u32)`.
- `BlockSelector`: `Id(BlockId)`.
- `FileSelector`: `Path { path, line, column }`.
- `DriveObjectSelector`: `Id(DriveObjectId)` or `Lookup { object_type, name_or_path }`.
Index selectors are resolved only within their parent selector context, so tab index resolution requires a resolved window and pane/session index resolution requires a resolved tab or pane. Title and name/path lookup selectors are ergonomic helpers for interactive use and must fail on ambiguity rather than choosing the first match.
Recommended top-level request shape for `tab.create`:
```json
{
  "protocol_version": 1,
  "request_id": "client-generated-id",
  "action": "tab.create",
  "target": {
    "window": "active"
  },
  "params": {}
}
```
Recommended response shape:
```json
{
  "ok": true,
  "protocol_version": 1,
  "request_id": "client-generated-id",
  "instance_id": "opaque-instance-id",
  "resolved_target": {
    "window_id": "opaque-window-id",
    "tab_id": "opaque-tab-id"
  },
  "result": {}
}
```
Error payloads should include stable codes defined in `SECURITY.md`, including `local_control_disabled`, `unauthorized_local_client`, `insufficient_permissions`, `authenticated_user_required`, `authenticated_user_unavailable`, `execution_context_not_allowed`, `ambiguous_instance`, `ambiguous_target`, `stale_target`, `invalid_selector`, `unsupported_action`, `not_allowlisted`, `invalid_params`, `target_state_conflict`, `missing_target`, and `no_instance`.
### 2. Per-process discovery instead of fixed-port-only routing
Keep the existing fixed-port HTTP behavior intact for installation detection/profiling compatibility. Add a separate local-control listener that follows the same native Axum/Tokio pattern but supports multiple local Warp app processes.
Recommended design:
- Each participating Warp process creates a random opaque `instance_id` at startup.
- Each process binds a loopback control listener on an ephemeral port or an app-managed available port.
- Each process writes a discovery record into a secure per-user Warp state directory. The record should contain:
  - `instance_id`
  - PID
  - channel/build metadata
  - control-listener endpoint
  - protocol version
  - start timestamp
  - credential metadata or secure-storage references only when the relevant inside-Warp or outside-Warp context is enabled
- The CLI loads discovery records, removes or ignores stale records after health checks, and chooses an instance using the product selector rules.
- `warpctrl instance list` is a CLI-first projection of this discovery registry plus health responses.
When outside-Warp control is disabled, discovery must follow `SECURITY.md`: either publish no actionable local-control record for external clients or publish only a minimal disabled-status record with no endpoint authority or credential reference.
This design preserves the current `9277` behavior while avoiding cross-process port contention for the new control API.
### 3. Local authentication, enablement, and safety boundary
Mutating localhost routes should not copy the permissive CORS posture of `/install_detection`.
Recommended local trust model:
- No browser-readable CORS allowance on control endpoints.
- The relevant implemented Scripting setting must allow the request context before credentials are minted or sensitive control requests are accepted. In the current foundation branch that means outside-Warp only; future inside-Warp support must add its own verified setting gate.
- The authoritative enablement bit must live in protected local storage and must not be writable by `warpctrl` or ordinary same-user preference/config edits.
- Per-instance raw credential material must be kept out of plaintext discovery records and stored in platform secure storage where practical.
- The CLI may load or request scoped credentials through an app-owned broker/helper, but it must not mint authority itself.
- The broker verifies whether the invocation originated from a Warp-managed terminal session before issuing in-Warp-only grants.
- The broker issues authenticated-user grants only when the selected app has a true logged-in Warp user and the relevant local-control permission is enabled.
- The app rejects disabled-state, missing, malformed, invalid, expired, or revoked credentials before selector resolution or mutation.
- The app maps every action to a state/data category and rejects insufficient grants before selector resolution or mutation.
- The app maps every action to a `requires_authenticated_user` value and allowed execution contexts, rejecting mismatches before selector resolution or mutation.
- Health metadata exposed without credentials, if needed for stale-record pruning, must not reveal mutating capabilities, credentials, or sensitive target state.
This keeps the protocol local and scriptable without creating an ambient browser-to-localhost control surface.
Do not ship the first slice as a plaintext discovery bearer token, even for same-user human CLI use. The first slice is the foundation for underlying data reads, app-state mutations, metadata/configuration mutations, and underlying data mutations, so it must establish the protected enablement, credential storage, scoped grant, and app-side enforcement model from `SECURITY.md`.
### 4. Future verified Warp-terminal invocation context
The current foundation branch does not implement verified inside-Warp invocation. `InvocationContext::InsideWarp` and `ExecutionContextProof::VerifiedWarpTerminal` may remain in the shared protocol as reserved future concepts, but the credential broker must reject them until the proof broker described here exists.
Minimum implementable design:
- When Warp creates or Warpifies a terminal session, the app creates a high-entropy per-session capability and records verifier state in an app-owned terminal-session registry.
- The registry entry is bound to the selected app instance, terminal/session identifier, issuing process generation, expiry, and revocation state.
- The shell receives only proof material needed by `warpctrl`, such as an opaque handle plus a short-lived token or challenge-response input. Plain environment variables may carry handles or hints, but a caller-set variable must not be sufficient authority.
- `warpctrl` invoked from that terminal sends `InvocationContext::InsideWarp` and `ExecutionContextProof::VerifiedWarpTerminal` to `/v1/control/credentials` when it has proof material. Without proof material it must use `OutsideWarp`.
- The broker verifies the proof against the app-owned registry, including app instance, session liveness, expiry, revocation, and nonce or challenge binding before minting any inside-Warp scoped credential.
- The broker then checks Settings > Scripting and permission-category policy for the requested action. A valid proof raises the maximum eligible grant set; it does not bypass user settings, action metadata, authenticated-user requirements, target scopes, or bridge enforcement.
- The minted credential records `invocation_context: InsideWarp`, the granted permission category, expiry, instance, and any authenticated-user subject. The app bridge revalidates the grant and current app policy on every control request.
Hardened follow-ups can strengthen this minimum design by storing secret material in platform secure storage, exposing only opaque handles through the shell, using Unix-domain-socket or named-pipe peer-credential checks, binding proofs to broker challenges, and invalidating proofs on shell/session teardown, app logout, user switch, or Settings > Scripting changes. These hardening layers improve direct-token theft resistance, but they do not create a perfect security boundary against malicious same-user software with process, filesystem, Accessibility, or screen-observation access.
### 5. App-side request bridge onto the UI/application context
The HTTP handler runs on a Tokio runtime thread owned by the local-control server. It cannot directly access or mutate Warp's UI models, views, or app context because all WarpUI state is single-threaded and owned by the main app event loop. The bridge solves this by sending a closure from the Tokio handler thread to the main thread, executing it in the model's context, and returning the result to the waiting HTTP handler.
#### Thread model
- **Tokio runtime thread (HTTP handler):** Owns the Axum router, receives HTTP requests, authenticates, deserializes the `RequestEnvelope`. Cannot touch `AppContext`, views, or models.
- **Main app thread:** Owns all WarpUI entities (`App`, `AppContext`, views, models). All UI state reads and mutations must happen here.
- **Bridge:** Transfers a typed closure from the Tokio thread to the main thread, executes it with `&mut ModelContext`, and sends the return value back.
#### Implementation: `ModelSpawner`
The bridge uses WarpUI's `ModelSpawner<T>` mechanism, which is the standard way for background threads to schedule work on a model's main-thread context:
1. During app initialization, a `LocalControlBridge` singleton model is created. The model's `ModelContext::spawner()` method returns a `ModelSpawner<LocalControlBridge>` — a cloneable, `Send` handle that can enqueue closures from any thread.
2. The `ModelSpawner` is stored in the Axum router's shared state (`ControlServerState`), making it available to every HTTP handler.
3. When an HTTP request arrives, the handler calls `spawner.spawn(|bridge, ctx| { ... }).await`:
   - `spawn` sends a boxed `FnOnce(&mut LocalControlBridge, &mut ModelContext<LocalControlBridge>) -> R` closure through an `async_channel` to the main thread's task-callback loop.
   - The main thread dequeues the closure, constructs a fresh `ModelContext` for the bridge model, and calls the closure.
   - Inside the closure, the bridge has full access to `ModelContext`, which derefs to `AppContext`. This means it can call `ctx.windows()`, `ctx.views_of_type::<Workspace>(window_id)`, `workspace.update(ctx, ...)`, and any other main-thread API.
   - The closure returns a typed result (e.g., `ResponseEnvelope`), which is sent back to the Tokio thread via a `oneshot` channel.
4. The HTTP handler awaits the oneshot result and serializes it as the HTTP response.
#### Concrete flow for `tab.create`
```
HTTP handler (Tokio thread)
  │
  ├─ verify inside-Warp or outside-Warp context is enabled
  ├─ verify credential, execution context, safety grant, and authenticated-user grant
  ├─ deserialize RequestEnvelope
  ├─ call bridge_spawner.spawn(move |bridge, ctx| {
  │      bridge.handle_request(request, ctx)  // runs on main thread
  │  }).await
  │
  └─ serialize ResponseEnvelope as JSON

LocalControlBridge::handle_request (main thread)
  │
  ├─ verify protected context-specific enablement state is still enabled
  ├─ map action to required permission category
  ├─ map action to authenticated-user and execution-context requirements
  ├─ verify presented credential grants that category, target family, execution context, and authenticated-user access
  ├─ match request.action.kind
  │   └─ ActionKind::TabCreate
  │       ├─ validate_tab_create_target(&request.target)
  │       ├─ ctx.windows().active_window()
  │       │   └─ if none: return invalid_selector / missing_target
  │       ├─ ctx.views_of_type::<Workspace>(window_id)
  │       └─ workspace.update(ctx, |workspace, ctx| {
  │             workspace.handle_action(
  │                 &WorkspaceAction::AddTerminalTab { hide_homepage: false },
  │                 ctx,
  │             )
  │           })
  │
  └─ return ResponseEnvelope::ok(request_id, json!({ ... }))
```
#### Why this pattern
- **Thread safety.** WarpUI's entity/view system is not `Send` or `Sync`. The only safe way to interact with it from a background thread is through `ModelSpawner`, which serializes access through the main event loop.
- **Synchronous result.** Unlike fire-and-forget patterns (e.g., URI intent dispatch in `app/src/uri/mod.rs`), the `spawn` call returns a concrete `Result<R, ModelDropped>`, so the HTTP handler can produce a structured success or error response.
- **Reuses existing infrastructure.** `ModelSpawner` is already used throughout the codebase for background-to-main-thread communication (e.g., async file I/O results, network responses). No new concurrency primitive is needed.
- **Action dispatch reuses existing app behavior.** The bridge calls `workspace.handle_action(&WorkspaceAction::AddTerminalTab { ... }, ctx)` — the exact same method the UI keybinding system uses. This ensures the control CLI produces identical behavior to the corresponding user action, including side effects like tab count updates, focus changes, and event emissions.
- **Deterministic targeting.** The bridge must not silently fall back from the active window to an arbitrary ordered window for mutating actions. If the caller relies on the default active selector and no active window exists, return a structured missing-target or invalid-selector error. If future command forms allow explicit window IDs, resolve the explicit ID exactly or return `stale_target`.
#### Adding new action handlers
To add a new action to the bridge:
1. Add a variant to `ActionKind` in `crates/local_control/src/protocol.rs`.
2. Document its `SECURITY.md` state/data category, required permission grant, `requires_authenticated_user` value, and allowed execution contexts.
3. Add a match arm in `LocalControlBridge::handle_request` in `app/src/local_control/mod.rs`.
4. Before selector resolution or dispatch, verify local control is enabled and the presented credential grants the action category, target family, execution context, and authenticated-user access if required.
5. Inside the match arm, use `ctx` (which is a `&mut ModelContext<LocalControlBridge>` that derefs to `&mut AppContext`) to resolve selectors and dispatch the action onto existing app types.
6. Return a `ResponseEnvelope::ok(...)` or `ResponseEnvelope::error(...)` with the result.
The bridge closure has access to the full `AppContext` API surface, including `ctx.windows()`, `ctx.window_ids()`, `ctx.views_of_type::<T>(window_id)`, `handle.update(ctx, ...)`, and `handle.read(ctx, ...)`. This makes it straightforward to wire new actions to existing UI behavior without introducing new concurrency concerns.
### 6. Target resolution model
Implement target resolution as a reusable component rather than scattering lookup logic across handlers.
Recommended resolution order:
1. Select instance in the CLI/discovery layer.
2. Resolve window inside the target process.
3. Resolve tab within the window.
4. Resolve pane within the tab/pane-group context.
5. Resolve session only for session-scoped commands.
6. Resolve block/file/Drive selectors only for commands whose action metadata declares that target family.
Selector behavior:
- `active` resolves from current app focus/selection state.
- Explicit opaque IDs must resolve exactly or return `stale_target`.
- Index selectors are allowed only for user-visible indexed concepts and should resolve to a concrete opaque ID before execution.
- Title, name, and path selectors are convenience selectors. They must be exact by default, document any future fuzzy behavior explicitly, and return `ambiguous_target` when more than one target matches.
- A session-scoped request against a non-terminal pane returns `target_state_conflict`.
Target resolution must happen after protected enablement, authentication, and safety-grant checks. This prevents denied requests from learning more target state than necessary and keeps enforcement centralized.
Implementation references:
- Window-level active selection already exists inside the app through `WindowManager`.
- Pane scoping can build on the conceptual model of `PaneViewLocator` in `app/src/workspace/util.rs (12-18)`.
- Existing URI intent routing in `app/src/uri/mod.rs (895-1093)` shows how to locate workspaces/windows and avoid silently acting in the wrong place.
#### CLI selector grammar
`crates/warp_cli/src/local_control.rs` should expose a shared selector argument group that is flattened into every command that accepts app targets. The parser must support:
- Instance selectors: `--instance <instance_id>` and `--pid <pid>`, with clap conflicts.
- Window selectors: `--window <active|id:<id>|index:<n>|title:<title>>`, `--window-id <id>`, `--window-index <n>`, and `--window-title <title>`, with one form allowed.
- Tab selectors: `--tab <active|id:<id>|index:<n>|title:<title>>`, `--tab-id <id>`, `--tab-index <n>`, and `--tab-title <title>`, with one form allowed.
- Pane selectors: `--pane <active|id:<id>|index:<n>>`, `--pane-id <id>`, and `--pane-index <n>`, with one form allowed.
- Session selectors: `--session <active|id:<id>|index:<n>>`, `--session-id <id>`, and `--session-index <n>`, with one form allowed.
- Block/file/Drive selectors only on commands that need them: `--block-id <id>`, path arguments or `--path <path>` plus `--line`/`--column`, and Drive object ID arguments or `--drive-id <id>`.
The CLI converts these flags into the protocol `TargetSelector` before sending the request. It must not rely on positional entity IDs for commands like `window close 1`; target entities are selected through the shared selector flags so command arguments remain reserved for action parameters.
### 7. Allowlisted handler families
Use one handler module per action family. The protocol layer owns parsing/validation; handler modules own target resolution and delegation to existing app logic.
Recommended modules/families:
- Discovery/state:
  - instances, version, active chain, windows/tabs/panes/sessions listings.
- Window/tab:
  - new, focus, close, activate, move, rename, color, close variants.
- Pane:
  - split, focus, navigate, close, maximize, resize.
- Input/session:
  - insert, replace, clear, run command, cycle session, mode switch where supported.
- Appearance/settings:
  - theme list/set, system-theme controls, font/zoom actions, allowlisted settings reads/writes/toggles.
- Panels/surfaces:
  - settings/page/search, palettes, left/right panels, Drive, resource center, code review, vertical tabs, AI assistant.
Do not use a generic “dispatch action by string” endpoint. Every handler should be reachable only through an explicit `ControlAction` variant.
#### WarpCtrlBehavior review gate
The public `ControlAction` catalog remains the source of truth for the wire protocol, CLI parser, permission metadata, generated documentation, and app bridge handlers. Internal app actions such as `WorkspaceAction`, `TerminalAction`, `PaneGroupAction`, settings actions, and future user-visible action enums must not become the public protocol directly because they can contain transient view locators, indices, debug-only variants, implementation-specific payloads, and unstable internal semantics.
To prevent drift between user-visible Warp behavior and the `warpctrl` catalog, every user-visible app action enum should implement a `WarpCtrlBehavior` review mapping. The mapping is a code-level forcing function, not an automatic exposure mechanism. It answers whether each internal app action is:
- `Exposed` through a specific public `ControlAction` kind.
- `CoveredBy` an existing public `ControlAction` kind because several internal actions map to one stable CLI behavior.
- `Excluded` with an explicit reason such as debug-only, unsafe/privileged, internal implementation detail, not user-visible, no deterministic targeting model, no stable public semantics, or prohibited in the initial public version.
- `Deferred` with an explicit reason and tracking issue when the action might belong in `warpctrl` later but needs additional product, security, selector, or protocol design.
`WarpCtrlBehavior` implementations must use exhaustive matches without wildcard arms. Adding a new variant to a reviewed action enum should fail compilation until the developer or agent deliberately classifies its relationship to `warpctrl`. This mirrors the existing exhaustive-action-review style used by app-state saving decisions and makes “should this exist in Warp Control?” part of the ordinary code path for adding new user-visible actions.
Recommended shape:
- Define a shared `WarpCtrlBehavior` trait in the local-control integration layer or another app-visible module that does not force the core `warpui::Action` blanket implementation to change.
- Define review enums such as `WarpCtrlActionReview`, `WarpCtrlExclusionReason`, and `WarpCtrlDeferredReason`.
- Implement `WarpCtrlBehavior` for the major user-visible action enums, starting with `WorkspaceAction` and `TerminalAction`.
- Keep the mapping one-way from internal behavior to public catalog metadata. `WarpCtrlBehavior::Exposed(ControlActionKind::TabCreate)` means the action is represented by the public `tab.create` command; it does not mean raw `WorkspaceAction::AddTerminalTab` is serializable or dispatchable over the protocol.
- Add tests that collect reviewed action kinds and verify every `ControlActionKind` has protocol metadata, permission metadata, CLI parser coverage, generated-doc coverage, and an app-side handler before it can be advertised as supported.
The `warpui::Action` trait should not be extended for this purpose because it currently has a blanket implementation for any `Any + Debug + Send + Sync` type. The enforcement point is the concrete user-visible action enums and binding/action registration surfaces, where exhaustive review can be required without weakening the allowlisted protocol boundary.
### 8. First slice: prove discovery and `tab.create`
The first `warpctrl` implementation slice should land the minimum cross-cutting architecture plus a single representative tab mutation:
- Shared protocol types and error envelopes.
- `FeatureFlag::WarpControlCli` and Cargo feature `warp_control_cli`, with app-side runtime gating for settings, discovery, bridge registration, and local-control endpoints.
- New top-level Settings > Scripting page rendered only while `FeatureFlag::WarpControlCli` is enabled. The current foundation exposes outside-Warp local-control settings only; verified inside-Warp controls are deferred until the proof broker exists.
- Protected local-only enablement storage where outside-Warp control defaults off. Future inside-Warp enablement must use the same protected-storage class before it is exposed.
- As an interim foundation step, the outside-Warp top-level enablement and granular permission bits live in the typed `LocalControlSettings` group as private settings with `SyncToCloud::Never`, explicit private storage keys, and no `toml_path`. This keeps them out of the user-visible settings file and generated settings schema while leaving the protected-storage migration as a required pre-ship hardening step.
- Granular outside-Warp local-control permission storage under Settings > Scripting for metadata reads, underlying data reads, app-state mutations, metadata/configuration mutations, and underlying data mutations. Future inside-Warp permissions should be added only with verified terminal proof support.
- Discovery registry and CLI instance selection.
- A standalone `warpctrl` binary or artifact path that runs control commands without starting the GUI app runtime.
- Per-process authenticated local-control server that refuses sensitive work when outside-Warp control is disabled and rejects inside-Warp credential requests until verified terminal proof support is implemented.
- Scoped credential issuance/storage with no raw credentials in plaintext discovery records, including execution-context fields and authenticated-user grant fields.
- App-side request bridge and selector resolver.
- Action-category mapping and app-side safety-grant enforcement.
- Action metadata for `tab.create` that deliberately classifies it as a logged-out-safe app-state mutation only when the user's granular local-control settings allow app-state mutation.
- Read-only `ping/version` plus `warpctrl instance list` or equivalent minimal discovery command.
- End-to-end `warpctrl tab create` for the selected instance, reusing the same app behavior as the user-visible new-terminal-tab action.
Why `tab.create` first:
- It proves a UI/layout action can be targeted and executed against live app state.
- It exercises process discovery, local authentication, request bridging, selector defaults, app-context dispatch, and structured success/error output without introducing higher-risk terminal input execution.
- It exercises the protected enablement and scoped-grant model before higher-risk action families depend on it.
- It gives operators a concise end-to-end smoke test: discover a running instance, create a tab, and confirm the live app changed.
The PR should also introduce the shell-facing CLI command grammar that the remainder of the protocol will reuse and establish a lightweight CLI startup path distinct from GUI startup.
### 9. Follow-up slices: fill out the remaining protocol in parallel
After the first slice validates discovery, auth, selector resolution, CLI syntax, and server-to-app execution, follow-up slices can add the remaining allowlisted catalog in parallelized action-family groups. The baseline code should make new action additions mostly additive:
- Extend `ControlAction`.
- Update the relevant `WarpCtrlBehavior` mappings for the internal app actions that implement, overlap with, exclude, or defer the behavior.
- Add typed params/results.
- Add a handler.
- Add validation/tests.
- Add CLI surface/tests.
### 10. CLI parsing and output libraries
The `warpctrl` CLI must use the same argument parsing and output libraries as the existing Oz CLI so that conventions, derive patterns, and shell-completion generation remain consistent across both binaries.
- **clap** (with the `derive` feature) for argument parsing, subcommand trees, and help generation. Both binaries share the `warp_cli` crate, so parser types defined there are reused directly.
- **serde** / **serde_json** for JSON request/response serialization and for `--output-format json` output.
- **clap_complete** for shell completion generation, reusing the same infrastructure the Oz CLI uses.
- The `OutputFormat` enum (`Pretty`, `Json`, `Ndjson`, `Text`) is shared from `warp_cli::agent::OutputFormat` so human-readable vs. machine-readable output follows the same conventions.
- New subcommand types for `warpctrl` live in `warp_cli::local_control` and follow the same `#[derive(Parser)]` / `#[derive(Subcommand)]` / `#[derive(Args)]` patterns used by the Oz CLI's top-level `Args` and `CliCommand` types.
Do not introduce alternative parsing libraries (e.g., `structopt`, `argh`) or alternative serialization approaches. Keeping one set of libraries across both CLIs reduces dependency weight, ensures consistent `--help` formatting, and lets contributors move between the two surfaces without learning a different stack.
### 11. CLI packaging and release shape
The shipped product shape should be a separate bundled `warpctrl` CLI binary that reuses shared CLI/protocol crates but does not depend on launching the GUI binary in command mode. Follow the Oz CLI release model as closely as practical:
- macOS:
  - Add a standalone control CLI artifact path next to the existing Oz standalone CLI artifact flow.
  - If the app bundle also exposes a wrapper/install flow, keep channelized naming consistent with the final product name decision.
- Linux:
  - Extend bundle/release scripts to emit control CLI standalone artifacts and packages in the same broad pattern as the current Oz CLI tarball/deb/rpm/Arch package flow.
- Windows:
  - Mirror the existing installer-generated helper-wrapper pattern first if that remains the canonical Oz behavior on Windows.
  - If the product decision is to ship a true standalone Windows control CLI binary, add a dedicated release path in follow-up work rather than silently diverging from existing Oz precedent.
Startup and dependency expectations:
- The CLI process should initialize only command parsing, discovery, authentication material loading, protocol serialization, HTTP transport, and output formatting needed for the requested command.
- The CLI should not initialize GUI state, rendering, terminal session models, app workspaces, or other main-app-only subsystems.
- Startup cost should be treated as part of the product contract because control commands are expected to compose naturally in scripts and repeated interactive shell usage.
Naming decision:
- Product examples use provisional `warpctrl ...` command lines for the standalone local-control binary.
- Final artifact filenames, channelized aliases, and installer exposure should be chosen before broad rollout to avoid churn in bundle scripts, docs, shell completions, and release workflow files.
## Implementation Plan
### Branch stack
Use raw git for the stack; do not use Graphite for these branches.
The durable review stack should optimize for reviewability rather than mirroring only broad product phases. The bottom review branch now combines specs and the shared foundation so reviewers can see the product/security contract next to the protocol, settings, bridge, and CLI scaffolding that enforce it. The intended stack is:
1. `zach/warp-cli-core-foundation` — create this branch from `master`. It owns the specs in `specs/warp-control-cli/PRODUCT.md`, `TECH.md`, `SECURITY.md`, and supporting docs, plus the shared implementation foundation: protocol crate shape, discovery/auth scaffolding, selector and error types, action metadata/catalog structure, Scripting settings surface, protected local-control settings, local-control server/bridge skeleton, standalone `warpctrl` binary skeleton, packaging hooks, module split, `WarpCtrlBehavior` scaffolding, and the minimum `tab.create` smoke path if needed to prove the end-to-end architecture.
2. `zach/warp-cli-readonly-metadata` — create this branch from `zach/warp-cli-core-foundation`. It implements structural metadata reads: instance/app health and active-chain commands, windows, tabs, panes, sessions, capability/action metadata, opaque IDs, metadata target shapes, and metadata-read permission enforcement. It must not expose terminal output, input buffers, history, file contents, Drive object contents, or other underlying user data.
3. `zach/warp-cli-readonly-data-settings` — create this branch from `zach/warp-cli-readonly-metadata`. It implements underlying-data reads and read-only settings/appearance/docs: block listing/inspection/output, input-buffer reads, history reads, theme/settings/keybinding/action reads, project/file app-state reads, authenticated Drive metadata/content reads where present, read-only docs, and the read-only `warpctrl` Agent skill.
4. `zach/warp-cli-mutating-layout` — create this branch from `zach/warp-cli-readonly-data-settings`. It implements layout and app-state mutations for app/window/tab/pane behavior: window create/focus/close, tab create/activate/move/close, tab metadata that belongs with layout review if appropriate, pane split/focus/navigate/resize/maximize/close, app-state mutation permission checks, and layout mutation tests.
5. `zach/warp-cli-mutating-input-settings-surfaces` — create this branch from `zach/warp-cli-mutating-layout`. It implements the remaining approved mutating command families: session activation/cycling/reopen, input insert/replace/clear, input mode switching, theme/system-theme/font/zoom changes, allowlisted setting set/toggle, settings/palette/search/panel/surface commands, file/project/Drive open commands that are app-state-only, mutating docs, and the mutating-command Agent skill updates. It must preserve the initial public prohibition on terminal command execution, workflow execution, accepted-command submission, and agent-prompt submission.
The previous `zach/warp-cli-specs` branch is retained only as migration-source/history material. It is no longer a separate review PR or an authoritative branch in the active stack.
The goal is to keep durable review branches close to roughly 2,000 lines of incremental changes where practical while avoiding a one-branch-per-command maintenance burden. Product phases still matter, but they are not the primary PR boundary. The durable branches are the review spine; short-lived shard branches can feed into them during implementation.
Spec changes are an important part of the stacking strategy. All new spec changes must originate on `zach/warp-cli-core-foundation`, which is the authoritative source for `specs/warp-control-cli/PRODUCT.md`, `TECH.md`, `SECURITY.md`, and supporting docs. After a spec change lands there, propagate it upward through every stacked branch with raw git so the spec files stay synchronized across `zach/warp-cli-readonly-metadata`, `zach/warp-cli-readonly-data-settings`, `zach/warp-cli-mutating-layout`, and `zach/warp-cli-mutating-input-settings-surfaces`. Do not make independent spec edits directly on higher implementation branches except as part of resolving a propagation conflict in a way that preserves the authoritative bottom-branch content.
Recommended raw-git setup after `zach/warp-cli-core-foundation` is ready:
```bash
git fetch origin
git checkout -b zach/warp-cli-core-foundation origin/master
git checkout -b zach/warp-cli-readonly-metadata
git checkout -b zach/warp-cli-readonly-data-settings
git checkout -b zach/warp-cli-mutating-layout
git checkout -b zach/warp-cli-mutating-input-settings-surfaces
```
If a lower branch changes after higher branches exist, rebase each higher branch onto its immediate lower branch with raw git and resolve conflicts by preserving both the lower branch's stable API/permission model and the higher branch's owned behavior.
### Migrating from the earlier four-branch stack
The earlier four-branch stack (`zach/warp-cli-specs`, `zach/warp-cli`, `zach/warp-cli-readonly`, `zach/warp-cli-read-write`) should be treated as source material for the five-PR review stack, not as the final review structure.
Recommended migration:
1. Create backup refs before rewriting or replacing anything:
   - `backup/warp-cli-specs` from `zach/warp-cli-specs`.
   - `backup/warp-cli` from `zach/warp-cli`.
   - `backup/warp-cli-readonly` from `zach/warp-cli-readonly`.
   - `backup/warp-cli-read-write` from `zach/warp-cli-read-write`.
2. Create `zach/warp-cli-core-foundation` from latest `origin/master` and bring over both the specs from `zach/warp-cli-specs` and only the foundation pieces from `zach/warp-cli`. Prefer path-level checkout followed by selective editing or `git add -p`; do not preserve every old commit if that makes review boundaries worse.
3. Create `zach/warp-cli-readonly-metadata` from `zach/warp-cli-core-foundation` and bring over only metadata-read pieces from `zach/warp-cli-readonly`.
4. Create `zach/warp-cli-readonly-data-settings` from `zach/warp-cli-readonly-metadata` and bring over the remaining read-only underlying-data, settings, docs, and skill pieces from `zach/warp-cli-readonly`.
5. Create `zach/warp-cli-mutating-layout` from `zach/warp-cli-readonly-data-settings` and bring over only layout/app-state mutations from `zach/warp-cli-read-write`.
6. Create `zach/warp-cli-mutating-input-settings-surfaces` from `zach/warp-cli-mutating-layout` and bring over the remaining approved mutating input/session/settings/surface pieces from `zach/warp-cli-read-write`.
7. Recompute incremental diff sizes, validate compilation/tests for each branch, push the new stack, and retire or close the old broad branches once the new review stack is accepted.
Before redistributing feature work, prefer landing a mechanical module-split commit in `zach/warp-cli-core-foundation` so later branches do not all expand the same large files. The app-side target should be:
- `app/src/local_control/mod.rs` for registration and top-level exports.
- `app/src/local_control/bridge.rs` for the app request bridge.
- `app/src/local_control/resolver.rs` for target resolution.
- `app/src/local_control/permissions.rs` for app-side permission/auth checks.
- `app/src/local_control/handlers/metadata.rs`.
- `app/src/local_control/handlers/data.rs`.
- `app/src/local_control/handlers/layout.rs`.
- `app/src/local_control/handlers/input.rs`.
- `app/src/local_control/handlers/settings_surfaces.rs`.
Likewise, split CLI and protocol code if they become review bottlenecks:
- `crates/warp_cli/src/local_control/mod.rs`.
- `crates/warp_cli/src/local_control/selectors.rs`.
- `crates/warp_cli/src/local_control/output.rs`.
- `crates/warp_cli/src/local_control/commands/{metadata,data,layout,input,settings_surfaces}.rs`.
- `crates/local_control/src/{protocol,catalog,selectors}.rs`.
- `crates/local_control/src/actions/{metadata,data,layout,input,settings_surfaces}.rs`.
### Feature flag and rollout gate
The entire feature should be gated behind a Warp feature flag, proposed as `FeatureFlag::WarpControlCli` with Cargo feature `warp_control_cli`.
Implementation should follow the existing runtime feature-flag conventions:
- Add `warp_control_cli = []` under `[features]` in `app/Cargo.toml`, not under the default feature set until launch.
- Add `WarpControlCli` to the `FeatureFlag` enum in `crates/warp_features/src/lib.rs`.
- Add the `#[cfg(feature = "warp_control_cli")] FeatureFlag::WarpControlCli` entry in `app/src/features.rs` so the compile-time feature initializes the runtime flag.
- Enable the flag for dogfood or preview by adding it to `DOGFOOD_FLAGS` or `PREVIEW_FLAGS` only when the rollout plan calls for that exposure.
- Prefer runtime checks with `FeatureFlag::WarpControlCli.is_enabled()` over broad `#[cfg]` gates except where code cannot compile without the Cargo feature.
When `FeatureFlag::WarpControlCli` is disabled in the Warp app:
- the Scripting settings page should not expose Warp control settings;
- `LocalControlSettings` should not register user-visible controls for Warp control;
- the app should not create `LocalControlBridge` or `LocalControlServer`;
- no local-control discovery record should be written;
- no `/v1/control` or `/v1/control/credentials` local server endpoints should be exposed;
- command-palette/keybinding entries related specifically to installing, configuring, or using `warpctrl` should be hidden;
- tests should cover both enabled and disabled flag states with the repo's normal feature-flag override helpers.
The standalone `warpctrl` binary can still exist in a build where the app feature is disabled, but it should find no compatible enabled app instance and should return a structured no-instance or feature-disabled error rather than relying on hidden server state.
### Merge and review strategy
Keep PR boundaries aligned with the stack:
- PR1: `zach/warp-cli-core-foundation` into `master` for the combined specs, shared protocol, CLI, settings, bridge, and module scaffolding.
- PR2: `zach/warp-cli-readonly-metadata` into `zach/warp-cli-core-foundation` or its merged successor for metadata reads.
- PR3: `zach/warp-cli-readonly-data-settings` into `zach/warp-cli-readonly-metadata` or its merged successor for underlying-data reads, settings reads, docs, and skill updates.
- PR4: `zach/warp-cli-mutating-layout` into `zach/warp-cli-readonly-data-settings` or its merged successor for app/window/tab/pane layout mutations.
- PR5: `zach/warp-cli-mutating-input-settings-surfaces` into `zach/warp-cli-mutating-layout` or its merged successor for input/session/settings/surface mutations.
If a lower PR merges before higher branches are ready, rebase the next branch onto the merged successor of its base and continue upward through the stack. Use raw git for all rebases, conflict resolution, cherry-picks, and pushes.
## End-to-end flow
```mermaid
sequenceDiagram
    participant CLI as Warp control CLI
    participant REG as Local discovery registry
    participant PROC as Selected Warp process
    participant BROKER as Credential broker
    participant HTTP as Local control listener
    participant BRIDGE as App request bridge
    participant RES as Target resolver
    participant ACT as Allowlisted action handler
    participant UI as Live Warp app state

    CLI->>REG: Read local instance records
    CLI->>PROC: Health/protocol check for candidates
    PROC-->>CLI: Instance metadata + compatibility
    CLI->>CLI: Resolve instance selector
    CLI->>BROKER: Request scoped credential for action + execution context
    BROKER-->>CLI: Grant or structured denial
    CLI->>HTTP: Authenticated POST tab.create request
    HTTP->>HTTP: Verify context-specific enablement + credential + execution context
    HTTP->>BRIDGE: Typed request + response channel
    BRIDGE->>BRIDGE: Recheck enablement + permission + auth-user policy
    BRIDGE->>RES: Resolve window/tab/pane/session selectors
    RES-->>BRIDGE: Concrete target handles or typed error
    BRIDGE->>ACT: Execute allowlisted ControlAction
    ACT->>UI: Reuse existing tab creation behavior
    UI-->>ACT: Mutation/read result
    ACT-->>BRIDGE: Typed result
    BRIDGE-->>HTTP: Response envelope
    HTTP-->>CLI: JSON success/error response
    CLI-->>CLI: Pretty or JSON output
```
## Testing and validation
Map tests directly to `PRODUCT.md` behavior.
- Security architecture:
  - Protected enablement tests proving outside-Warp control defaults off, disabled outside-Warp context rejects credential issuance, sensitive discovery, and mutating requests with `local_control_disabled`, and current inside-Warp credential requests are rejected with `execution_context_not_allowed`.
  - Tests proving discovery in disabled state exposes no actionable endpoint authority or credential reference.
  - Credential-storage tests proving raw credentials are not written into plaintext discovery records.
  - Execution-context tests proving external clients cannot receive grants reserved for verified Warp-terminal invocations.
  - Permission-category enforcement tests proving insufficient grants fail with `insufficient_permissions` before selector resolution or handler dispatch, including separate denial cases for app-state mutation, metadata/configuration mutation, and underlying-data mutation.
  - Authenticated-user tests proving user-authenticated actions fail without a logged-in app user or authenticated-user grant.
  - Settings > Scripting tests proving both top-level toggles and granular disabled categories invalidate credentials and prevent new grants.
  - Structured-error tests for disabled, unauthenticated, expired, revoked, insufficient-scope, execution-context-denied, authenticated-user-required, authenticated-user-unavailable, unsupported, malformed, ambiguous, missing-target, stale-target, and invalid-selector requests.
- Behavior 1-6, 29-31:
  - Protocol version/unit tests.
  - Discovery-registry tests with zero, one, multiple, stale, and incompatible instance records.
  - Local-auth tests for missing, invalid, expired, revoked, and valid credentials.
- Behavior 7-13:
  - Selector-resolution unit tests for active, explicit ID, index, stale target, ambiguous target, and non-terminal session target.
  - Tests that no lower-level selector silently retargets after an explicit stale selector fails.
  - CLI selector parsing tests for every generic and explicit alias form: `--window`, `--window-id`, `--window-index`, `--window-title`, `--tab`, `--tab-id`, `--tab-index`, `--tab-title`, `--pane`, `--pane-id`, `--pane-index`, `--session`, `--session-id`, and `--session-index`.
  - CLI conflict tests proving only one selector form per entity family is accepted and that positional target IDs are rejected where the command expects selector flags.
- Behavior 15-28:
  - Parser/serde tests for every first-slice `ControlAction` variant.
  - Router tests proving unknown/unallowlisted actions are rejected.
  - CLI parse/output tests for pretty and JSON rendering.
- Behavior 18 and 33:
  - App-side tests for `tab.create` using existing workspace/tab helpers or a narrow extracted helper.
  - Manual local verification that `warpctrl tab create` creates a terminal tab in a running app.
- Behavior 30:
  - Multi-process integration-style coverage using two synthetic discovery records and mock health responders, plus manual testing with multiple channel builds where practical.
- Packaging:
  - `--artifact cli`-style bundle smoke tests or script-level checks for each supported platform path touched by the first slice.
  - Startup-path tests or focused checks confirming `warpctrl` dispatches commands without entering GUI-app launch code.
  - Shell completions/help output checks once final command naming is selected.
### Computer-use CLI verification
Before any stacked PR is considered ready for review, run an end-to-end computer-use verification pass against a cloud built validation artifact from that branch. The validation artifact is a Warp app build and standalone `warpctrl` binary built by the validation agent from the exact commit under review, with `warp_control_cli` compiled in and `FeatureFlag::WarpControlCli` enabled at runtime.
The verifier must launch the built Warp app, enable the Scripting settings needed for the command category under test, and capture screenshots or screen recordings that prove the user-visible result of each basic command family. Screenshots should be saved as durable artifacts and named by branch, invocation context, command family, and command name.
The verifier must exercise every invocation context implemented by the branch under review. For the current foundation branch, inside-Warp verification means proving `InsideWarp` requests are rejected and no inside-Warp Settings > Scripting controls are exposed. Once verified Warp-terminal support is implemented, the verifier must also run `warpctrl` from a terminal session inside the feature-flag-enabled Warp app and prove the app-issued execution-context proof is accepted and inside-Warp settings gate command categories. The outside-Warp path must run the same `warpctrl` binary from an external terminal or automation shell outside the built Warp app and prove outside-Warp control is default-off, then works only after enabling outside-Warp scripting and the required granular permissions in Settings > Scripting.
The verification matrix should cover every implemented command in `PRODUCT.md` at least once in JSON output mode and, where there is a visible UI effect, with a screenshot after the command runs. At minimum:
- read-only metadata commands show successful CLI output and, for active/focus/list commands, a visible target that matches the output;
- underlying data read commands show successful CLI output only when the underlying-data-read permission is enabled and a denial screenshot/output when it is disabled;
- app-state mutation commands show before/after screenshots proving the visible Warp UI changed;
- metadata/configuration mutation commands show before/after screenshots proving the persisted setting or label changed;
- underlying data mutation commands run only in a disposable test workspace/session with test files and test Warp Drive objects, show denial without the underlying-data-mutation permission, then show success with the permission enabled;
- authenticated-user commands show both the logged-out or missing-grant denial path and the enabled authenticated path when a test account/environment is available.
The verifier should produce a verification manifest checked into or attached to the PR artifacts. The manifest should list each command, branch under test, invocation context, required permission category, expected result, actual result, screenshot artifact path, and any skipped case with a reason. Missing screenshots for visible commands block review readiness.
## Parallelization
The durable review stack should remain linear, but implementation can still happen in parallel with Oz cloud agents. The pattern is contract-first fan-out: land the shared contracts and module boundaries in `zach/warp-cli-core-foundation`, then let cloud agents work on short-lived shard branches that feed the durable review branches.
Wave 0: foundation:
- Keep `zach/warp-cli-core-foundation` mostly sequential or use at most one or two tightly scoped agents because protocol envelope, discovery, authentication, feature-flag gating, selector resolution, module boundaries, and `tab.create` smoke behavior need one coherent architecture.
- Acceptable foundation shards are `core-protocol-cli` for shared protocol/CLI skeleton and `core-app-foundation` for settings, bridge, resolver, permissions, and handler skeletons. These shards should merge into the single durable `zach/warp-cli-core-foundation` branch before feature fan-out begins.
Wave 1: read-only fan-out:
- Launch short-lived Oz cloud shard branches from `zach/warp-cli-core-foundation` once the contracts compile.
- Suggested shards:
  - `zach/warp-cli-shard/readonly-metadata` owns structural metadata commands and feeds `zach/warp-cli-readonly-metadata`.
  - `zach/warp-cli-shard/readonly-data` owns block output, input-buffer reads, history reads, and other underlying-data reads, then feeds `zach/warp-cli-readonly-data-settings`.
  - `zach/warp-cli-shard/readonly-settings-docs` owns theme/settings/keybinding/action reads, docs, and read-only skill updates, then feeds `zach/warp-cli-readonly-data-settings`.
Wave 2: mutating fan-out:
- Launch mutating shards only after read-only target resolution and result shapes are stable.
- Suggested shards:
  - `zach/warp-cli-shard/mutating-window-tab-pane` owns window/tab/pane layout mutations and feeds `zach/warp-cli-mutating-layout`.
  - `zach/warp-cli-shard/mutating-input-session` owns session activation/cycling/reopen, input insert/replace/clear, and input mode switching, then feeds `zach/warp-cli-mutating-input-settings-surfaces`.
  - `zach/warp-cli-shard/mutating-settings-surfaces` owns theme/font/zoom/setting mutations and settings/palette/panel/surface commands, then feeds `zach/warp-cli-mutating-input-settings-surfaces`.
  - `zach/warp-cli-shard/mutating-files-drive` is optional and should be deferred unless the approved scope includes file/Drive app-state opens or future underlying-data mutations.
Each cloud shard prompt should include:
- The exact base branch and shard branch name.
- Owned command families.
- Owned files/modules.
- Files/modules the shard must not edit without calling out the need for integration.
- Required permission categories and authenticated-user behavior.
- Selector resolution requirements.
- Validation commands and expected tests.
- A handoff requirement: report branch name, changed files, implemented commands, permission decisions, validation results, and any conflicts or follow-ups.
Default file ownership for shards:
- Metadata shards own metadata handler/protocol/CLI modules and metadata tests.
- Data shards own data handler/protocol/CLI modules and underlying-data permission tests.
- Layout shards own layout handler/protocol/CLI modules and app-state mutation tests.
- Input/session shards own input/session handler/protocol/CLI modules and tests proving staging does not submit or execute.
- Settings/surface shards own settings/surface handler/protocol/CLI modules and metadata/configuration mutation tests.
The lead integrator merges or cherry-picks accepted shard work into the durable stack with raw git, in review order. Shard branches should not become independent long-lived PRs unless the lead intentionally splits review further; their default purpose is to feed the durable stack while preserving parallel implementation and focused context windows.
```mermaid
flowchart LR
    Core["zach/warp-cli-core-foundation<br/>specs + contracts + bridge"] --> ROMeta["zach/warp-cli-readonly-metadata<br/>structural reads"]
    ROMeta --> ROData["zach/warp-cli-readonly-data-settings<br/>data/settings reads"]
    ROData --> MutLayout["zach/warp-cli-mutating-layout<br/>layout mutations"]
    MutLayout --> MutInput["zach/warp-cli-mutating-input-settings-surfaces<br/>input/settings/surfaces"]
    ROMetaShard["shard/readonly-metadata"] --> ROMeta
    RODataShard["shard/readonly-data"] --> ROData
    ROSettingsShard["shard/readonly-settings-docs"] --> ROData
    MutLayoutShard["shard/mutating-window-tab-pane"] --> MutLayout
    MutInputShard["shard/mutating-input-session"] --> MutInput
    MutSettingsShard["shard/mutating-settings-surfaces"] --> MutInput
```
## Risks and mitigations
- Fixed-port server assumptions:
  - Mitigation: leave current `9277` endpoints undisturbed and use a per-process control listener plus discovery registry.
- Browser-to-localhost abuse:
  - Mitigation: no permissive CORS, protected in-app enablement, explicit local auth, scoped grants, and mutating routes gated before selector resolution.
- External apps silently enabling outside-Warp local control:
  - Mitigation: the outside-Warp enablement state defaults off, lives in protected local storage behind Settings > Scripting, is local-only, is not Settings Sync'd, and is not writable through `warpctrl`, config files, registry/plist preference edits, defaults writes, or server-backed settings.
- External apps obtaining in-Warp authenticated-user grants:
  - Mitigation: require an app-issued execution-context proof for Warp-terminal-only grants, do not trust caller-declared labels or plain environment variables as sole authority, and keep external authenticated-user grants behind a separate default-off permission.
- Logged-out requests touching user-authenticated data:
  - Mitigation: every action declares `requires_authenticated_user`, new actions default to true, and the bridge returns authenticated-user errors before selector resolution or dispatch.
- Implementation drift from `SECURITY.md`:
  - Mitigation: treat `SECURITY.md` as normative for security behavior; update this technical plan before implementation when there is disagreement, and include tests for the security architecture in the first slice.
- Action catalog drift from real UI behavior:
  - Mitigation: each control action reuses or factors existing UI action paths rather than duplicating behavior, and user-visible app action enums implement exhaustive `WarpCtrlBehavior` mappings so new internal actions cannot be added without an explicit expose/cover/exclude/defer decision.
- Leaking internal unstable identifiers:
  - Mitigation: public protocol exposes opaque IDs and selectors; internal runtime IDs stay implementation details.
- Over-broad settings mutation:
  - Mitigation: allowlisted setting keys only, with private/debug/derived settings rejected.
- Command execution risk:
  - Mitigation: keep `input.run`/session execution in the catalog but require explicit follow-up product/review decision before broad rollout.
- Packaging churn due to provisional executable naming:
  - Mitigation: document `warpctrl` as provisional and settle final aliases before broad release workflow rollout.
- Heavyweight CLI startup caused by sharing the GUI binary's launch path:
  - Mitigation: ship a separate control CLI artifact with a narrow initialization path and keep GUI-only subsystems out of ordinary CLI command execution.
## Follow-ups
- Decide the final artifact filename/channel alias scheme around the provisional `warpctrl ...` public command surface.
- Decide whether Windows should follow the current Oz wrapper pattern indefinitely or gain standalone control CLI artifacts.
- Decide whether a future subscription/watch protocol is useful for scripts that want live state changes, rather than single request/response calls only.
