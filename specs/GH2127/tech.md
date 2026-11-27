# SSH Profiles Panel - Tech Spec
Product spec: `specs/GH2127/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/2127

## Context
Warp already has the core pieces needed for a dedicated SSH profile manager, but they are spread across the workspace panel system, settings, terminal command execution, SSH detection, and secure storage.

Relevant current code:

- `app/src/workspace/header_toolbar_item.rs:29` defines `HeaderToolbarItemKind` variants for toolbar items and maps them to labels/icons/availability.
- `app/src/workspace/view.rs:18966` renders configurable panels for toolbar items. Tabs, Tools, and Code Review already have panel branches; other toolbar items return `None`.
- `app/src/workspace/view.rs:19097` already has access to both `SshSettings` and `WarpifySettings` when assembling the settings template context.
- `app/src/workspace/view.rs:14421` exposes `TerminalView::execute_command_or_set_pending`, which is the right command path for a new tab whose shell may not be bootstrapped yet.
- `app/src/terminal/warpify/settings.rs:46` defines `EnableSshWarpification`; `app/src/terminal/warpify/settings.rs:56` defines `UseSshTmuxWrapper`. The profile flow should respect these existing settings rather than introducing a parallel SSH mode.
- `app/src/settings/ssh.rs` currently owns SSH-related settings through `define_settings_group!`; it is the right place for saved profile metadata.
- `app/src/terminal/ssh/util.rs:204` parses interactive SSH commands for Warpify detection. The same module is the natural home for profile command rendering and typed SSH login prompt helpers.
- `app/src/terminal/view.rs:10422` detects interactive SSH commands and starts the existing SSH login/Warpify monitoring path.
- `app/src/terminal/view.rs:23991` evaluates whether a completed SSH login should prompt for Warpification.
- Existing modals use `Modal::new(...)` and typed action views, for example `app/src/settings_view/mcp_servers_page.rs:93` and workspace modal construction sites in `app/src/workspace/view.rs`.
- Existing credential-bearing settings use `warpui_extras::secure_storage` rather than TOML-backed settings. SSH profile secrets should follow that pattern.

The proposed feature should add a new profile surface without changing the behavior of Warpify internals, `SshTmuxWrapper`, `SshRemoteServer`, or remote server installation. The implementation should treat profiles as a structured way to launch SSH commands and optionally provide one-shot secret input during login.

## Proposed changes

### 1. Profile data model and settings
Add `SshHostProfile` and `SshJumpHost` to `app/src/terminal/ssh/util.rs` or a sibling `profile.rs` under `terminal/ssh`. The model should include:

- `SshHostProfile { id: Uuid, name, host, user, port, identity_file, jump_hosts, tags }`
- `SshJumpHost { source_profile_id: Option<Uuid>, host, user, port, identity_file }`

The profile id is immutable after creation and is part of the local secure-storage namespace for profile secrets. Use serde defaults to migrate profiles created before ids or ports existed:

- missing/nil/duplicate ids get a new UUID
- zero/missing ports normalize to 22
- jump-host ports normalize the same way
- jump-host entries created before `source_profile_id` existed deserialize with `source_profile_id: None`; preserve them for connection rendering as legacy snapshots, but do not prune them by mutable host metadata when another profile is deleted

Add `saved_ssh_host_profiles: Vec<SshHostProfile>` to `SshSettings` in `app/src/settings/ssh.rs` with:

- `private: true`
- `sync_to_cloud: SyncToCloud::Never`
- a TOML path under `ssh.saved_profiles`

Do not add secret fields to the settings model.

### 2. Command rendering
Implement profile command rendering as argv assembly plus shell quoting, not string concatenation. Required helpers:

- `SshHostProfile::to_ssh_command()` for all profile connections. The rendered SSH command must not vary based on Warpify settings.
- `CredentialKind`, initially with `HostPassword` and optionally `KeyPassphrase { identity_file }` when key-passphrase auto-entry is implemented.
- `ssh_profile_secret_key(id: Uuid, kind: CredentialKind) -> String`, so different credential kinds cannot collide in secure storage.
- `parse_ssh_login_prompt(output: &str) -> Option<SshLoginPromptKind>`, where `SshLoginPromptKind` distinguishes account-password prompts from key-passphrase prompts and carries the identity-file path when OpenSSH prints one.

The renderer should:

- include `-i` only when identity file is non-empty
- include `-p` only when the port is not 22
- use `-J` for jump chains that do not need per-jump identity files
- use `ProxyCommand` chaining when any jump host needs an identity file, so per-jump `-i` and `-p` can be honored
- add `--` before the final target to prevent option injection
- quote every argument through the repo's existing shell quoting crate/pattern
- render nested `ProxyCommand` values from their own argv vectors before inserting them into the outer ssh argv; do not build the nested command through ad hoc string concatenation
- treat OpenSSH `%` expansion as a separate context from shell quoting: only renderer-owned tokens such as `%h:%p` may remain unescaped, and user-controlled values that can enter a ProxyCommand must either reject `%` tokens or escape them according to OpenSSH's `%%` literal rules before the outer argument is quoted

### 3. Toolbar and panel integration
Add an SSH Profiles item to `HeaderToolbarItemKind` with a globe/server-style icon and a label of "SSH Profiles". Mark it as a panel item and include it in the default left toolbar for new/default toolbar configurations. Users with configurable toolbar support can still remove or reposition it through the existing toolbar customization path.

In `Workspace`, add:

- `ssh_profiles_panel_view: ViewHandle<SshProfilesPanelView>`
- `ssh_profiles_panel_open: bool`
- `WorkspaceAction::ToggleSshProfilesPanel`
- `WorkspaceAction::ConnectSshProfile(Uuid)`

Update `render_config_panel` so the SSH Profiles branch returns the profile panel without mutating `PaneGroup::left_panel_open` or `right_panel_open`. This keeps the panel independent from Tabs, Tools, and Code Review.

### 4. SSH profiles panel UI
Create `app/src/workspace/view/ssh_profiles_panel.rs` for the panel and modal body. Reuse existing Warp UI primitives:

- `Hoverable`, `Container`, `Flex`, `Stack`, `ConstrainedBox`
- shared `icon_button` and `ActionButton` themes
- `EditorView::single_line` for form fields
- `Dropdown` for jump-host selection
- `Modal::new(...)` with typed events for Add/Edit

Panel behavior:

- header with title and add button
- empty state when no profiles exist
- profile rows styled like existing tab cards
- edit/remove hover controls anchored at the row's upper-right corner
- row click dispatches `ConnectSshProfile`
- edit/remove clicks defer to children and must not trigger row connect

Modal behavior:

- add/edit modes share the same body
- Save disabled while invalid
- Escape emits cancel
- Enter submits only when valid
- password field defaults to masked, supports reveal and explicit clear
- on close, clear password editor contents and reset masking state

Jump-host dropdown behavior:

- candidates come from other saved profiles
- exclude the current profile and already selected targets
- render selected hosts as removable chips
- when saving a newly selected jump profile, snapshot the selected profile's `id` as `source_profile_id: Some(id)` plus its direct host/user/port/identity metadata into `SshJumpHost`
- do not recursively copy the selected profile's own `jump_hosts` in v1; multi-hop chains are represented by selecting each hop explicitly

### 5. Profile save/delete and secure storage
On submit, update `SshSettings::saved_ssh_host_profiles` and separately handle `CredentialIntent`:

- `Keep { kind }`: no secure-storage write for that credential kind
- `Set { kind, secret: Zeroizing<String> }`: remove or durably disable the existing secure-storage entry for `ssh_profile_secret_key(profile.id, kind)` before writing the replacement secret to that same key
- `Clear { kind }`: remove the secure-storage entry at `ssh_profile_secret_key(profile.id, kind)`

On profile removal:

- remove the profile metadata
- remove all secure-storage secret keys for that profile id and all credential kinds that the implementation can derive or enumerate, ignoring `NotFound` per key
- remove jump-host references from remaining profiles by matching `SshJumpHost::source_profile_id == Some(removed_profile_id)`, not by comparing mutable host metadata; legacy `None` snapshots remain until the user edits/removes them

Store credential metadata separately from the secret value. At minimum, the secure-storage key/value path must distinguish host account passwords from private-key passphrases, for example by including a credential kind in the key or by storing a small typed secure-storage payload. Auto-entry must check the prompt type before reading or writing the secret: host account passwords are used only for strict OpenSSH account password prompts, and key passphrases only for prompts that identify the matching profile identity file.

If secure storage is unavailable or a secret write fails, keep the profile metadata save path independent: save the metadata, show a visible warning/toast or inline form error that the secret was not saved, clear the password editor buffer, and leave no in-memory pending credential intent. Subsequent connections should behave as if no compatible credential is saved.

On edit, a failed `Set` must not leave an older stored secret active for auto-entry. Before attempting to write a replacement credential, remove or durably mark disabled the existing key for the same `(profile_id, credential_kind)`. If the replacement write fails, keep the metadata update, show the warning, clear the password buffer, and ensure lookup for that credential kind returns no usable secret until a later save succeeds.

Use `Zeroizing<String>` for password values that pass through Rust-owned application memory where current APIs allow it. Do not present this as an end-to-end memory guarantee: editor buffers, secure-storage backends, and PTY write APIs may still create non-zeroized copies unless those APIs are changed. Do not log password contents.

### 6. Connect flow and Warpify behavior
`WorkspaceAction::ConnectSshProfile` should:

1. read the profile by id from `SshSettings`
2. inspect `WarpifySettings::enable_ssh_warpification`
3. render `profile.to_ssh_command()` regardless of Warpify setting
4. create a new tab
5. arm SSH profile state on the new terminal view
6. call `execute_command_or_set_pending(&command, ctx)`

Do not use a direct "run command now" path that can silently fail before bootstrap.

When `enable_ssh_warpification` is false, use scoped suppression state rather than changing the SSH command. The terminal view should arm an `SshProfileConnectionState` containing the rendered command, the target profile id, the target block id once known, and `warpification_enabled_at_arm = false`. When the matching profile command is observed for that block, call a terminal-model method equivalent to `ignore_bootstrapping_messages_until_command_finished()` so remote bootstrap hooks from that SSH session are ignored only until the next command-finished event. The suppression state expires after the same short TTL used for profile connection state and is cleared when the target block changes, the command no longer matches, or the SSH login flow completes. This prevents a plain SSH profile connection from displaying as Warpified when the setting is off without breaking password-state matching or normal SSH login tracking for unrelated commands.

When `enable_ssh_warpification` is true, the profile connection should use the same existing `evaluate_warpify_ssh_host` and SSH login completion flow as manual SSH.

`UseSshTmuxWrapper` should influence only the existing Warpify decision after SSH login; it must not change the rendered SSH command.

### 7. Password auto-entry state
Add a small `SshAutoInjectState` under `app/src/terminal/ssh/auto_inject.rs` with:

- profile id
- original command string
- target block id
- attempted flag
- login completed flag
- 30-second expiry

Activation:

- arm only for direct profiles with no jump hosts
- match the active block command against the profile command, allowing expected wrapper normalization
- bind to that block id before any password lookup
- carry the expected credential kind from the profile state so prompt matching and secret lookup use the same kind

Polling or output-triggered check:

- inspect only a bounded tail of the active block output
- require a strict SSH prompt parser that returns a prompt kind rather than a boolean
- set `attempted = true` before secure-storage lookup
- read the secret by profile id and credential kind only after the prompt kind matches
- write the matched secret plus carriage return directly to the PTY path, not through shared-session/user-input broadcasting paths
- immediately disarm after the write

Disarm when:

- the target block changes
- command no longer matches
- login completes
- the state expires
- the first attempt has already happened

This design intentionally does not auto-enter passwords for jump-host profiles in the first version.

### 8. Tests and validation
Map product invariants to tests:

- Command rendering unit tests in `app/src/terminal/ssh/util.rs`:
  - shell quoting with spaces/metacharacters
  - `--` before final target
  - default and non-default port rendering
  - `-J` rendering for simple jump chains
  - `ProxyCommand` rendering for jump chains with per-jump identity files
  - `ProxyCommand` percent-token escaping or validation for user-controlled values
  - secret key includes stable UUID and credential kind
  - normalization migrates missing/duplicate ids and zero ports
  - legacy jump-host snapshots deserialize with `source_profile_id: None`
  - strict prompt detection classifies OpenSSH account-password prompts separately from key-passphrase prompts and rejects sudo/generic prompts

- Auto-inject unit tests in `app/src/terminal/ssh/auto_inject.rs`:
  - command matching across wrapper normalization
  - rejects different host/port
  - target block/attempt/login-complete one-shot state transitions
  - account-password secrets are not used for key-passphrase prompts and key-passphrase secrets are not used for account-password prompts

- Terminal model/view tests:
  - Warpify-disabled profile command suppresses remote bootstrap hooks only until command completion
  - Warpify-enabled profile leaves existing SSH Warpify detection available

- UI/view tests:
  - `SshProfilesPanelView` and modal can layout in light/dark theme without panic
  - invalid form disables Save
  - jump-host candidates exclude self and selected profiles
  - removing a profile prunes stale jump references by `source_profile_id`
  - secure-storage write failure saves metadata only, shows a warning, and clears the password buffer
  - unavailable secure storage leaves the profile connectable without password auto-entry
  - deleting a profile ignores a missing secure-storage secret without blocking metadata deletion
  - saved settings never contain password material after add, edit, clear, or delete

- Integration tests under `crates/integration`:
  - add a profile, save it, reopen panel, and see it listed
  - edit profile password visibility/clear behavior without leaking to settings
  - click a key-auth profile and verify a new tab queues/runs the expected SSH command
  - with SSH Warpify enabled, profile connection enters the same Warpify prompt/success path as a manual SSH command
  - with SSH Warpify disabled, profile connection remains plain SSH and does not show the Warpified success state
  - jump-host dropdown lists other profiles, excludes self, and renders a command that chains through selected profiles

Manual validation should cover a real SSH host for password auto-entry, a wrong-password retry, a sudo prompt after login, a host-trust `yes/no` prompt, and a jump-host profile with identity-file authentication.

### 9. PR sequencing
This is a feature request and should follow the repository's contribution model:

1. Use issue `#2127` as the tracking issue; do not open a duplicate issue.
2. Ask maintainers to mark `#2127` as `ready-to-spec`.
3. Open a spec PR containing only `specs/GH2127/product.md` and `specs/GH2127/tech.md`.
4. After spec approval, open or update the implementation PR from a separate branch with code, tests, and a changelog entry.

## Risks and mitigations

### Risk: password injection into the wrong prompt
The largest security risk is entering a saved password into a prompt that is not the intended SSH login prompt.

Mitigation: strict prompt matching, target block binding, one-shot attempted flag, login-complete disarm, short TTL, no auto-entry for jump-host chains in v1, and no secret input through user-input/shared-session broadcasting.

### Risk: profile connection ignores Warpify settings
A profile-launched SSH command can accidentally bypass or trigger Warpify differently from manual SSH.

Mitigation: render the same SSH command regardless of `enable_ssh_warpification`, branch only to arm scoped bootstrap suppression when SSH Warpification is disabled, keep `UseSshTmuxWrapper` inside the existing post-login Warpify decision, and add regression tests for both setting states.

### Risk: jump-host metadata drift
If jump hosts are stored only as raw strings, selected profile metadata such as port or identity file is lost.

Mitigation: snapshot structured jump-host metadata into `SshJumpHost`, and remove stale references when a profile is deleted.

### Risk: large UI surface without integration coverage
The feature spans settings, secure storage, panels, modals, terminal execution, and SSH login state.

Mitigation: keep the implementation split into small modules, add focused unit tests for logic, and add integration coverage for the user-facing add/edit/connect/Warpify flows before the implementation PR is marked ready.

## Follow-ups
- Import and periodically refresh profiles from `~/.ssh/config`.
- Folder/group organization and connect-to-all.
- Search/filter inside the SSH Profiles panel.
- Per-profile environment variables and startup directories.
- Safe multi-hop password handling if Warp can reliably associate prompts with each hop.
- Optional cloud sync for non-secret profile metadata if product/security agree.
