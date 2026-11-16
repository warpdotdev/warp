# warpctrl operator README
`warpctrl` is the provisional CLI entrypoint for controlling an already-running local Warp app instance. It is intended for scripts, demos, agent workflows, and developer automation that need to perform allowlisted Warp UI actions through the installed channel-specific Warp binary without launching the GUI.
The implemented command surface contains exactly 84 typed, allowlisted actions. All 84 actions execute after exact-action credential validation. Close actions flow through normal Warp close behavior, so existing app warnings remain authoritative. The Block, Auth, Drive, and History families are absent, and `input.insert` plus `input.replace` stage text without submitting it.
## Packaging model
`warpctrl` should be packaged as an Oz-style wrapper script rather than a standalone Rust binary. The wrapper should resolve the installed channel-specific Warp executable and invoke it with the hidden `--warpctrl` control-mode flag:
- `crates/local_control` owns discovery records, local authentication material, client transport, protocol envelopes, action names, and error types.
- `crates/warp_cli` owns command parsing conventions for local-control subcommands.
- the channel-specific app binary owns the hidden `--warpctrl` dispatch path and exits before normal GUI startup.
- the app-side bridge owns the per-process loopback listener and dispatches supported actions onto the live Warp UI context.
The control-mode path should initialize only the work needed for CLI parsing, instance discovery, local authentication loading, request serialization, HTTP transport, and output formatting. It should not initialize GUI state, terminal models, rendering, workspaces, or main-app startup paths.
During the provisional naming period, release artifacts and helper names may be channelized, but operator docs and examples should use `warpctrl` unless an integration branch explicitly documents a channel-specific alias.
This branch wires the core hidden dispatch contract through the existing Warp binary. Platform packaging should create wrapper scripts that call the channel binary with `--warpctrl` instead of producing or selecting a separate `warpctrl` binary.
## Install and invocation guidance
### macOS
For local development checks, build the local Warp binary and invoke it with the hidden control-mode flag:
```bash
cargo run -p warp --bin warp -- --warpctrl instance list
```
For distributable checks, use the installed `warpctrl` wrapper. The wrapper execs the app bundle's channel-specific executable with `--warpctrl`.
### Linux
For local development checks, build the local Warp binary and invoke it with the hidden control-mode flag:
```bash
cargo run -p warp --bin warp -- --warpctrl instance list
```
For distributable checks, use the packaged `warpctrl` wrapper. The wrapper execs the packaged channel-specific Warp executable with `--warpctrl`.
The standalone `script/linux/bundle --artifact warpctrl` validation artifact includes that wrapper and compiles the forwarded channel binary with `warp_control_cli`. Installing the wrapper into the normal Linux app package remains a separate packaging follow-up.
Run `warpctrl --version` after installation to confirm the shell is resolving the expected build.
### Windows
Until the wrapper installer lands, build the local Warp binary and invoke it with the hidden control-mode flag for development checks:
```powershell
cargo run -p warp --bin warp -- --warpctrl instance list
```
Installer helper creation and release-artifact wiring still need a later packaging change before docs can promise an installer-provided `warpctrl` command.
## End-to-end local test flow
Use matching app and CLI bits from the same branch or release artifact so the protocol version and action catalog agree.
1. Start Warp and leave at least one window open.
2. Open **Settings > Scripting**. Local control is enabled by default on internal dogfood builds and disabled by default on public channels (Stable, Preview, OSS). Verify that the Scripting toggle is set to **Enabled**, and enable it if needed. Enabling scripting allows for programmatic and agentic control of Warp; refer to the docs for more info.
3. Confirm that the local-control server registered the running same-channel process:
   ```bash
   warpctrl instance list
   ```
4. Confirm app health and inspect protocol and build identity metadata:
   ```bash
   warpctrl app ping
   warpctrl app version
   warpctrl surface list
   ```
5. If exactly one compatible same-channel instance is listed, create a new terminal tab:
   ```bash
   warpctrl tab create
   ```
6. If multiple compatible same-channel instances are listed, copy the desired `instance_id` and target it explicitly:
   ```bash
   warpctrl app ping --instance <instance_id>
   warpctrl app version --instance <instance_id>
   warpctrl tab create --instance <instance_id>
   ```
7. Verify the running app receives focus for the selected instance and a new terminal tab appears according to Warp's normal new-tab placement behavior. The success response includes the created tab's opaque ID.
8. Inspect state before and after the mutation:
   ```bash
   warpctrl tab list --instance <instance_id>
   ```
Expected failures:
- `warpctrl instance list` with no running compatible app: exits zero with an empty list;
- a command that needs a selected app when no compatible app is running: exits non-zero with a no-instance error;
- multiple ambiguous instances: exits non-zero and asks for `--instance`;
- an explicit instance or PID from another channel: exits non-zero with a no-instance error;
- unsupported app build or stale discovery record: exits non-zero with a protocol, stale-target, or transport error;
## Security model
The local-control protocol is designed for same-user scripting, not cross-user or network access. The trust boundary is the local user account.
- **Loopback-only listener.** Each Warp process binds its control server to `127.0.0.1` on an ephemeral port. The listener is not reachable from the network.
- **Brokered scoped credentials.** Discovery records contain instance metadata, loopback control-endpoint information, and an instance-bound Unix-domain-socket broker reference when Scripting is enabled. The broker authenticates the connecting OS user with kernel peer credentials before decoding the credential request or issuing an action-scoped credential. Records do not contain bearer tokens or reusable full-access credentials.
- **Short-lived grants.** `warpctrl` requests an action-scoped credential over the owner-authenticated broker socket for the selected instance, then presents that credential to `/v1/control`. Grants are instance-bound, expired entries are pruned, and the in-memory grant set is capped. Missing, invalid, expired, revoked, or wrong-instance credentials are rejected before request decoding. After decoding identifies the requested action, insufficient-scope credentials are rejected before selector resolution or handler dispatch.
- **Protected local state.** The authoritative Scripting setting uses platform secure storage, never imports a value from ordinary or private preferences, and defaults to enabled only on internal dogfood channels (disabled by default on public channels). On POSIX platforms, discovery records and broker sockets use owner-only permissions. On Windows, local-control publication remains disabled until equivalent ACL and broker protections are implemented.
- **Channel-scoped discovery.** Each channel-specific CLI considers only records from its own Warp channel. Listing, implicit selection, and explicit instance or PID selection cannot target another channel.
- **Stale-record pruning.** On each `instance list` or implicit discovery call, same-channel records whose PID is no longer alive are deleted automatically. Candidates are also health-probed and accepted only when the live app reports the expected instance identity.
- **No CORS.** The control endpoints do not set permissive CORS headers, so browser-origin JavaScript cannot read responses even if it guesses the port. The credential requirement provides a second layer since browsers cannot read the brokered credential material.
```mermaid
sequenceDiagram
    participant CLI as warpctrl
    participant FS as ~/.warp/local-control/
    participant Broker as Credential broker
    participant HTTP as Warp loopback server<br/>(127.0.0.1:ephemeral)
    participant Bridge as App bridge

    CLI->>FS: Read discovery records (user-only permissions / ACL)
    FS-->>CLI: instance_id, loopback endpoint, broker socket reference
    CLI->>CLI: Prune stale PIDs, select instance
    CLI->>Broker: Connect to Unix socket<br/>exact action + selected instance
    Broker->>Broker: Authenticate peer OS user before decode;<br/>check Scripting enabled + catalog action
    alt Disabled, invalid, or insufficient scope
        Broker-->>CLI: Structured denial
    else Grant allowed
        Broker-->>CLI: Short-lived scoped credential
        CLI->>HTTP: POST /v1/control<br/>Authorization: Bearer <scoped credential>
        HTTP->>HTTP: Verify credential expiry + instance binding before decode
        HTTP->>HTTP: Decode typed request; verify action scope
        HTTP->>Bridge: Dispatch action to app context
        Bridge-->>HTTP: Structured result or error
        HTTP-->>CLI: JSON response envelope
    end
```
**Known limitations and future hardening:**
- Windows local-control publication is disabled until discovery-record ACL creation and validation are implemented.
- Unexpired exact-action credentials may be reused for their granted action. Close actions flow through normal Warp close behavior and may trigger existing app warnings.
- Same-user malicious software can still invoke trusted wrappers or automate the desktop, so brokered credentials are least-privilege guardrails rather than a complete hostile same-user sandbox.
- Future catalog expansion should consider per-request nonces, stricter platform secure-storage constraints, and stronger approval or policy gates.
## Documentation review notes
- Treat `warpctrl` as provisional executable naming until packaging signs off on final artifact aliases.
- Keep examples scoped to the authoritative 84-action catalog and explicitly call out that close actions use normal Warp close behavior.
- Do not document excluded families or actions as usable just because internal app implementations exist.
- Windows packaging may initially follow the existing helper-wrapper pattern. Update this README when that decision is final.
