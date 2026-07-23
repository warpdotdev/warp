# Tech Spec: Select WSL distributions in tab configs

**Issue:** [warpdotdev/warp#12358](https://github.com/warpdotdev/warp/issues/12358)

**Product spec:** [`specs/GH12358/product.md`](product.md)

## Context

Tab configs already carry a per-leaf shell override into the shared pane-template
startup path, and Warp already has a native WSL launcher. The missing work is a
typed tab-config representation, strict WSL resolution, guest-directory handling,
and error propagation between those two systems.

Relevant current code:

- `app/src/tab_configs/tab_config.rs:95-134` defines pane types and
  `TabConfigPaneNode`. `shell` is currently `Option<String>` and pane tables deny
  unknown fields.
- `app/src/tab_configs/tab_config.rs:206-242` renders a config into
  `PaneTemplateType`. Tree-resolution errors currently log a warning and return a
  blank fallback pane rather than returning an error to the caller.
- `app/src/tab_configs/tab_config.rs:311-405` resolves leaves. It expands every
  directory with the host process's `~`, builds command queues, and copies the
  string shell into the pane template.
- `app/src/launch_configs/launch_config.rs:77-109` defines `PaneMode` and the
  shared `PaneTemplateType::PaneTemplate`, whose current shell field is also an
  `Option<String>`.
- `app/src/workspace/view.rs:6862-6885` renders a tab config and immediately calls
  `add_tab_with_pane_layout`; it has no fallible or asynchronous WSL preflight.
- `app/src/pane_group/mod.rs:253-278` resolves a tab-config shell string by local
  command/path. `AvailableShells::find_by_command_name` intentionally does not
  select WSL entries.
- `app/src/pane_group/mod.rs:1333-1415` recursively creates each leaf. It resolves
  the shell independently, accepts only host-existing `cwd` values, queues commands,
  and defers Agent Mode until the command queue completes. Because creation happens
  during the walk, validation inside this function alone could leave a partial
  multi-pane tab.
- `app/src/terminal/available_shells.rs:50-168` represents a WSL distribution as an
  `AvailableShell` and deliberately keeps distribution names out of telemetry.
- `app/src/terminal/available_shells.rs:200-271` converts a WSL selection to
  `ShellLaunchData::WSL` without revalidating the distribution.
- `app/src/terminal/available_shells.rs:442-495` populates available WSL shells from
  `WslInfo`; `app/src/terminal/available_shells.rs:518-579` resolves persisted WSL
  launch data with exact distribution-name equality.
- `app/src/terminal/available_shells.rs:918-948` implements command-name lookup and
  explicitly skips WSL entries, explaining why the current string field cannot
  select a distribution.
- `app/src/terminal/wsl/model.rs:14-116` reads installed distribution names and the
  Windows default distribution from the Lxss registry. This list includes stopped
  installed distributions and filters internal Docker/Rancher distributions; it
  does not track running state.
- `app/src/terminal/local_tty/shell.rs:66-184` converts an `AvailableShell` into a
  shell-starter request. WSL selections retain the distribution name.
- `app/src/terminal/local_tty/shell.rs:404-430` asynchronously initializes WSL but
  currently computes a host fallback shell when WSL initialization fails. That is
  incompatible with an explicit structured WSL request.
- `app/src/terminal/local_tty/shell.rs:500-596` queries the distribution's `$SHELL`,
  creates `WslShellStarter`, and resolves its Windows-host home path. The query may
  start a stopped distribution.
- `app/src/terminal/local_tty/shell.rs:722-747` builds native WSL launch arguments
  for the selected distribution and its supported default shell.
- `app/src/terminal/local_tty/terminal_manager.rs:295-323` chooses the requested or
  user-default `AvailableShell`; `app/src/terminal/local_tty/terminal_manager.rs:494-630`
  finishes asynchronous shell resolution, defaults WSL to its guest home, and
  reports PTY spawn errors.
- `app/src/terminal/local_tty/windows/mod.rs:159-210` passes the WSL starter to
  `CreateProcessW` with a Windows-native start directory;
  `app/src/terminal/local_tty/windows/mod.rs:237-252` builds the WSL process command.
- `crates/warp_util/src/path.rs:444-489` converts absolute WSL paths to either a
  Windows drive path (`/mnt/c/...`) or a distribution-specific `\\WSL$` path.
  Windows-only coverage is in `crates/warp_util/src/path_tests.rs:557-628`.
- `app/src/terminal/view.rs:9386-9409` installs a pending command queue.
  `app/src/terminal/view.rs:12267-12319` advances it only after successful blocks,
  and `app/src/terminal/view.rs:12821-12823` first submits it on
  `BootstrapPrecmdDone`.
- `app/src/user_config/native.rs:45-56,130-143,296-315` loads and hot-reloads tab
  configs, replacing the menu's valid configs and emitting errors after file saves.
  `app/src/workspace/view.rs:2655-2707` owns the persistent, file-linked error toasts.
- `resources/bundled/skills/tab-configs/SKILL.md:31-102` is the canonical user-facing
  schema reference and currently documents only string shell selectors and host
  `~` expansion.

## Proposed changes

### 1. Extend `shell` without breaking its string form

In `app/src/tab_configs/tab_config.rs`, replace `TabConfigPaneNode.shell` with a
tab-config-specific untagged value:

```rust
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum TabConfigShell {
    Command(String),
    Structured(StructuredTabConfigShell),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StructuredTabConfigShell {
    Wsl { distribution: String },
}
```

If Serde does not permit `deny_unknown_fields` on the internally tagged enum in the
pinned version, use a dedicated `WslTabConfigShell` struct for the table payload or
a small custom deserializer; do not weaken the extra-key invariant.

`TabConfigShell::Command(String)` preserves the existing TOML string encoding and
behavior. The structured variant validates that the distribution is nonempty and
has no leading/trailing whitespace. It remains literal: unlike `directory` and
`commands`, rendering does not apply Handlebars substitution to it.

Add tab-config semantic validation before a parsed config enters `WarpConfig`:

- structured WSL is allowed only on terminal/agent leaves;
- split/cloud usage is an error;
- invalid structured payloads become `TabConfigError` values associated with the
  source file.

Do not apply these new semantic rejections to the legacy command-string variant;
that would be an unrelated compatibility change.

The separate `environment = { ... }` shape suggested in the issue was considered
but is not selected. It would introduce two launch selectors on one pane and require
precedence rules when both `shell` and `environment` are present. A bare
`shell = "Ubuntu"` was also rejected because a distribution name can collide with
a host command. Extending `shell` with a tagged table keeps one selector, preserves
the string wire format, and makes the WSL namespace explicit.

### 2. Carry a typed selection through the pane template

Add an internal `PaneShellSelection` in
`app/src/launch_configs/launch_config.rs` with `Command(String)` and
`WslDistribution(String)` variants, and change the pane template's shell member to
`Option<PaneShellSelection>`. `TabConfigShell` converts exhaustively into this type
during rendering.

Keep the existing string serialization/deserialization for launch-config-created
`PaneTemplateType` values via a field adapter: a legacy serialized string maps to
`Command`, while `WslDistribution` is produced only by tab-config rendering. This
prevents this issue from accidentally publishing structured WSL as a separate
launch-config-file feature.

Also introduce an internal startup-directory value that distinguishes
`Host(PathBuf)` from `WslGuest(String)`. Local command/default panes keep the
current host `shellexpand::tilde` path. A structured WSL pane renders the unquoted
parameterized directory as `WslGuest` without applying the Windows process's home
directory. This removes the current ambiguity of putting a guest string in a
Windows `PathBuf`.

Change `render_tab_config` and its recursive helpers to return a `Result` instead of
manufacturing a blank pane when tree or new schema validation fails. The caller
must present the error and must not call `add_tab_with_pane_layout`.

### 3. Resolve distributions with exact installed identity

In `app/src/terminal/available_shells.rs`, add a narrowly named lookup such as
`find_wsl_distribution_exact(&str) -> Option<AvailableShell>`. It searches only
`Config::Wsl` entries and uses the same exact equality already used by
`get_from_shell_launch_data`. Keep `find_by_command_name` unchanged for legacy
command strings.

Before creating a tab, walk the complete rendered template and resolve every WSL
selection against a snapshot of `AvailableShells`. On non-Windows builds, return a
platform error. On Windows, return an unavailable-distribution error that contains
the requested name and remediation suitable for a local toast. Do not use the
registry's `is_default` bit for this schema: a named distribution is always
required.

This is a two-phase operation: all leaves are prepared first, and only a completely
prepared tree is handed to `add_tab_with_pane_layout`. This preserves atomicity for
multi-pane preflight failures.

### 4. Resolve WSL guest directories off the UI path

Add a helper near `WslShellStarter`/the WSL model that accepts the selected exact
distribution plus the rendered guest directory and returns a Windows-host
`PathBuf` or a typed error:

1. No directory remains `None`; `TerminalManager` already substitutes the WSL home.
2. `~` resolves to the selected distribution user's host-visible home path.
3. `~/rest` joins `rest` to that resolved home after rejecting parent/root escape
   forms that would change the meaning of `~`.
4. An absolute typed Unix path goes through
   `convert_wsl_to_windows_host_path`, preserving `/mnt/<drive>` conversion and
   distribution-specific `\\WSL$\<distribution>` conversion.
5. Relative, `~user`, Windows-drive, and UNC input returns a syntax error.
6. The resolved path must satisfy `is_dir`; otherwise return a missing-directory
   error rather than dropping the override.

Run WSL `$HOME` discovery and filesystem checks from the existing asynchronous
open/preparation path, not synchronously in `WorkspaceView`. A stopped installed
distribution may be started by this discovery, consistent with current WSL shell
initialization. Cache the resolved home per distribution for one tab preflight so
several panes in the same distro do not repeat the query.

After preparation, replace `WslGuest` with the resolved host path used by
`CreateProcessW`. Local panes continue to carry and validate host-native paths.

### 5. Make explicit WSL startup strict

Thread a fallback policy with the resolved shell selection into terminal creation:

- legacy omitted/string shell selection keeps today's fallback behavior;
- `WslDistribution` uses `Required`, which forbids fallback.

Refactor `ShellStarterSourceOrWslName::to_shell_starter_source` to return a typed
error for a required WSL starter that cannot determine a supported `$SHELL`, cannot
contact WSL, or otherwise cannot initialize. Route that error through the existing
`on_pty_spawn_failed` surface and exit reason. Do not call
`ShellStarter::compute_fallback_shell` for the required path.

Continue to construct `AvailableShell::Wsl` -> `ShellLaunchData::WSL` ->
`WslShellStarter`; do not model WSL as a command placed into the terminal input.
The preflight lookup prevents normal unavailable-distribution failures before tab
creation. The strict starter policy covers removal and operational races after
preflight without changing existing settings/session-restore fallback policy.

### 6. Preserve bootstrap and command-queue sequencing

Keep `TerminalView::set_pending_command_queue` and the existing
`BootstrapPrecmdDone` submission path. The only integration requirement is that the
queue is installed on the WSL `TerminalView` before bootstrap completes, just as it
is for host panes. Do not invoke commands from the WSL discovery/preflight command
or append them to `wsl.exe` arguments.

Keep Agent Mode's existing deferred-entry flag. A strict starter failure never
emits the bootstrap event, so the pending queue remains unsubmitted and the agent
view remains unentered.

### 7. Errors, reload, documentation, and privacy

In `app/src/workspace/view.rs`, make `open_tab_config_with_params` complete the
fallible preparation step before adding a tab. Present structural/render,
unsupported-platform, unavailable-distribution, and directory errors as tab-config
errors with config/pane context. Preflight errors add no tab; a post-preflight PTY
race uses the existing in-pane spawn-failure surface.

Keep the current watcher model. Structural errors are returned by load and use the
existing persistent file-linked toast. Runtime availability is intentionally not
cached as a parse error. A reload replaces the stored definition for future opens
and does not visit existing `PaneGroup`s.

Update `resources/bundled/skills/tab-configs/SKILL.md` to document:

- the legacy string and structured WSL forms;
- exact, required distribution identity;
- WSL-only directory syntax and home semantics;
- terminal/agent-only validation;
- a single-pane and mixed-pane example.

Retain the existing telemetry value `"WSL"`. Do not add distribution names,
directories, or full error strings to telemetry. Local user-facing errors may
include the configured name and pane context required to repair the file.

## End-to-end flow

1. `WarpConfig` parses and structurally validates the table as
   `TabConfigShell::Structured(Wsl { ... })`.
2. Parameter submission renders commands/directories and produces a typed pane
   tree; the distribution remains literal.
3. The open preflight walks the entire tree, rejects non-Windows, resolves each
   exact WSL `AvailableShell`, expands guest homes, converts guest paths, and checks
   directories.
4. Only a successful prepared tree is passed to `add_tab_with_pane_layout`.
5. Each WSL leaf passes its resolved `AvailableShell` with required/no-fallback
   policy into terminal creation.
6. `WslShellStarter` discovers the guest's supported default shell and the Windows
   PTY launches that native WSL session at the prepared host-visible directory.
7. Warp bootstraps the guest shell. `BootstrapPrecmdDone` submits the first pending
   command; subsequent successful blocks advance the queue.
8. Agent panes enter Agent Mode only after their setup queue completes.

## Testing and validation

### Automated coverage

| Product invariants | Automated verification |
| --- | --- |
| 1-5 | Add `app/src/tab_configs/tab_config_tests.rs` cases for the legacy string, valid WSL table, round trip, missing/blank/whitespace distribution, unknown type, extra key, split/cloud use, literal distribution, and omitted-shell behavior. |
| 7, 13 | Add `app/src/terminal/available_shells_tests.rs` coverage using synthetic WSL entries: exact match succeeds, case mismatch/unavailable names fail, and two named distributions resolve independently. Add a template preflight test proving one invalid leaf returns an error before pane construction. |
| 8 | Unit-test the platform gate through an injected `is_windows` capability so the non-Windows error is covered on every CI host. |
| 9-10 | Extend `crates/warp_util/src/path_tests.rs` for absolute guest and `/mnt/<drive>` conversion as needed; add platform-independent parser tests for `~`, `~/rest`, relative, `~user`, drive, and UNC rejection. On Windows, test `$HOME` expansion and missing-directory errors with a command/filesystem seam rather than requiring a developer distro in ordinary unit tests. |
| 6, 12, 14 | Add `app/src/terminal/local_tty/shell_tests.rs` coverage proving the structured selection reaches `ShellLaunchData::WSL`/`ShellStarter::Wsl`, plus fallback-policy coverage: legacy WSL initialization may retain its existing behavior, while a required WSL failure returns an error and never constructs a direct-shell fallback. Test the failure event leaves pending commands and Agent Mode unexecuted. |
| 11-12 | Extend terminal/pane-group tests around the pending queue to prove WSL uses the same `BootstrapPrecmdDone` gate, advances commands only after success, clears the remainder after failure, and defers Agent Mode until completion. |
| 13 | Add a pane-template test containing host `pwsh`, two distinct WSL selections, and cloud mode; assert independent selectors/directories survive rendering and preparation in layout order. |
| 15-16 | Extend `app/src/user_config/mod_tests.rs` with valid/invalid structured WSL files and reload replacement. Existing workspace toast subscription tests or a focused test should verify the stale file toast is dismissed after correction; no code path mutates already-created pane groups. |
| 17 | Assert WSL telemetry remains the constant `"WSL"` and that no new telemetry field carries a distribution or directory. |

Run the repository-required checks before an implementation PR update:

```bash
./script/format --check
cargo clippy --workspace --exclude warp_completer --all-targets --tests -- -D warnings
cargo clippy -p warp_completer --all-targets --tests -- -D warnings
cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2
cargo test --doc
```

### Manual Windows matrix

Use a Windows machine with two installed distributions (for example,
`Ubuntu-24.04` and `Debian`) and record the evidence required by `CONTRIBUTING.md`.

| Scenario | Expected result |
| --- | --- |
| Running distro, terminal pane, no directory | Native WSL pane opens in that distro's home; no visible host `wsl.exe` command. |
| Stopped distro | Opening starts it and completes normal Warp bootstrap. |
| `~`, `~/project`, `/home/...`, `/mnt/c/...` | Each starts in the corresponding guest-visible directory; `pwd` confirms the WSL path. |
| Relative, `C:\...`, UNC, missing directory | Clear error; no tab or startup command is created. |
| Missing and wrong-case distro | Clear exact-name error; no host/default-distro fallback and no partial layout. |
| Distro removed after preflight | Failed WSL pane shows shell-start failure and runs no commands; already-started siblings remain. |
| Sequential commands | First command begins only after bootstrap; success advances; failure prevents later commands. |
| WSL agent pane with setup commands | Commands finish in the selected distro/directory, then Agent Mode opens. |
| Mixed host + two WSL distros + cloud | Layout, focus, per-pane distro, directory, and mode all match the file. |
| Edit and save while tab is open | Next open uses the edit; existing panes and commands do not change. |
| Save malformed table, then fix it | Config disappears and file-linked toast appears; fixing restores it and dismisses the stale toast. |

On macOS or Linux, open the same structurally valid WSL config and verify the
Windows-only error with no pane/default-shell fallback.

## Risks and mitigations

- **WSL queries can be slow or start a stopped VM.** Perform preflight off the UI
  path and cache one home lookup per distribution for the tab open.
- **A distro can disappear between validation and process creation.** Required
  shell policy forbids fallback and turns the race into an explicit pane failure.
- **Host and guest path strings are easy to confuse.** Carry a typed startup
  directory until preflight conversion and reject Windows syntax for WSL panes.
- **Changing `shell` could break existing files.** Keep an untagged string variant,
  add direct compatibility tests, and scope new semantic validation to the table.
- **Multi-pane validation could create partial tabs.** Separate preparation from
  mutation and call `add_tab_with_pane_layout` only after the whole tree succeeds.

## Follow-ups

- Consider a separately specified default-distribution shorthand if explicit names
  prove cumbersome.
- Consider structured selectors for other native launch targets only after their
  product semantics and launch-config compatibility are defined.
