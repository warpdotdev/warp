# TECH.md — Windows symlink and junction traversal returns OS error 448 in Warp

Issue: https://github.com/warpdotdev/warp/issues/9044
Product spec: `specs/GH9044/product.md`

## Problem

Local Windows shells launched by Warp can fail to traverse NTFS symlinks, file symlinks, and junctions with `ERROR_UNTRUSTED_MOUNT_POINT` / WinError 448 even when the same commands work in Windows Terminal. The reported failures line up with Windows RedirectionGuard / redirection trust behavior: a process with enforced redirection trust can be blocked when following reparse points created by a non-admin user. Warp's Windows terminal spawn path creates the shell process directly with ConPTY, a Windows job object, and an extended startup info attribute list, but the current code does not explicitly inspect or control the shell process's redirection trust mitigation state.

The implementation needs to make the hosted shell process, and the child processes it spawns, match normal terminal-host behavior for user development workflows without weakening unrelated Warp app behavior.

## Relevant code

- `app/src/terminal/local_tty/windows/mod.rs:27` — imports `CreateProcessW`, `CREATE_BREAKAWAY_FROM_JOB`, `EXTENDED_STARTUPINFO_PRESENT`, `STARTUPINFOEXW`, and related Windows process-startup types.
- `app/src/terminal/local_tty/windows/mod.rs (122-218)` — Windows PTY `spawn()` loads ConPTY, creates pipes, creates the pseudoconsole, builds `STARTUPINFOEXW`, sets the pseudoconsole thread attribute, and calls `CreateProcessW` for the shell.
- `app/src/terminal/local_tty/windows/mod.rs (229-287)` — builds the direct shell command line passed to `CreateProcessW`.
- `app/src/terminal/local_tty/windows/proc_thread_attribute_list.rs (14-60)` — wraps `InitializeProcThreadAttributeList` / `UpdateProcThreadAttribute` and currently allocates room for exactly one attribute, `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`.
- `app/src/terminal/local_tty/windows/environment.rs (34-191)` — constructs the environment block inherited by Windows shells, including PATH merging and Warp-specific environment variables.
- `app/src/terminal/local_tty/spawner.rs (119-239)` — owns direct PTY spawning and notes that Windows uses direct spawning rather than the Unix terminal-server helper path.
- `app/src/lib.rs (882-889)` — initializes the Windows job object before the PTY spawner is created.
- `crates/command/src/windows.rs (49-118)` — creates the Warp process job object, enables kill-on-close, and allows breakaway.
- `crates/command/src/async.rs (38-115)` — shared async process wrapper sets `CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB` for Windows child processes created by Warp internals.
- `crates/command/src/blocking.rs (63-103)` — blocking process wrapper sets the same Windows creation flags.
- `crates/lsp/src/command_builder.rs (31-45)` — Windows LSP commands are wrapped in `cmd.exe /c` for script resolution; this is a separate CreateProcess caller that can be used as a regression check for shared command behavior.

External API context:

- Windows `PROCESS_MITIGATION_REDIRECTION_TRUST_POLICY` controls RedirectionGuard with `EnforceRedirectionTrust` and `AuditRedirectionTrust`.
- Windows `GetProcessMitigationPolicy` / `SetProcessMitigationPolicy` can inspect or set `ProcessRedirectionTrustPolicy` for a process.
- Windows `PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY`, when available for a creation-time mitigation, is the preferred way to create a child process with an explicit process mitigation policy because the policy is applied before the child starts.

## Current state

Warp starts a Windows shell with this high-level flow:

1. `PtySpawner::new()` creates a Windows direct spawner; there is no separate terminal-server process on Windows.
2. `windows::spawn()` loads `conpty.dll`, builds a ConPTY handle, creates the pipe pair, and prepares `STARTUPINFOEXW`.
3. `ProcThreadAttributeList::new()` allocates an attribute list for one attribute and `set_pty_connection()` adds `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`.
4. `CreateProcessW` is called with `EXTENDED_STARTUPINFO_PRESENT`, `CREATE_UNICODE_ENVIRONMENT`, and `CREATE_BREAKAWAY_FROM_JOB`.
5. The shell inherits the environment block and then launches user commands and toolchain subprocesses itself.

There are two important limitations in the current code:

- Warp does not query, log, or set RedirectionGuard state on the app process or shell process.
- `ProcThreadAttributeList` cannot add a second startup attribute today because it hardcodes `num_attributes = 1`.

The existing job-object setup is probably not the direct cause of OS error 448. The job object controls process lifetime and breakaway behavior, while RedirectionGuard is a process mitigation policy that affects filesystem redirection traversal. However, job-object behavior should be regression-tested because any process-spawn change must preserve shared-fate cleanup.

## Proposed changes

### 1. Add a Windows redirection trust helper

Add a small Windows-only helper module near the process creation code. The most focused location is `app/src/terminal/local_tty/windows/process_mitigation.rs`; if other Warp process wrappers need it after investigation, move the shared pieces into `crates/command/src/windows.rs`.

The helper should expose:

- A lightweight representation of RedirectionGuard state, including whether `EnforceRedirectionTrust` and `AuditRedirectionTrust` are enabled.
- `current_process_redirection_trust_policy()` for diagnostic logging around Warp startup or shell spawn.
- `process_redirection_trust_policy(process_handle)` for optional validation of a spawned shell process.
- A shell-spawn policy application function that ensures the shell process is not created with enforced redirection trust when Warp is creating an ordinary user terminal session.

Implementation notes:

- Prefer using the `windows` crate bindings for `GetProcessMitigationPolicy`, `SetProcessMitigationPolicy`, `PROCESS_MITIGATION_REDIRECTION_TRUST_POLICY`, and `ProcessRedirectionTrustPolicy` if they are exposed by the current dependency feature set.
- If bindings are missing, add the narrow missing imports/features in the workspace `windows` dependency rather than hand-rolling broad FFI. If the specific struct is not generated by `windows = 0.62.2`, use a tiny local `#[repr(C)]` flags struct only for this policy and keep it Windows-only.
- Log the policy state at debug level during shell spawn on Windows. Avoid logging user paths or environment variables.

### 2. Apply the fix at shell creation, not per command

The primary fix should apply to the shell process created in `app/src/terminal/local_tty/windows/mod.rs`, before or during `CreateProcessW`, so all shell descendants inherit normal traversal behavior.

Preferred approach:

- Extend `ProcThreadAttributeList` so callers can request more than one attribute.
- Keep `set_pty_connection()` unchanged for the pseudoconsole attribute.
- Add a method for `PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY` if Windows exposes a creation-time mitigation value that disables or avoids enforced RedirectionGuard for the child shell.
- In `windows::spawn()`, create the attribute list with enough capacity for the pseudoconsole attribute plus the mitigation attribute, then set both before calling `CreateProcessW`.

Fallback approach if Windows does not expose or honor a creation-time RedirectionGuard override:

- Before spawning a local terminal shell, call `SetProcessMitigationPolicy(ProcessRedirectionTrustPolicy, flags = 0)` from a narrowly-scoped Windows helper and verify with `GetProcessMitigationPolicy` that the current process is not enforcing redirection trust for subsequently-created shell processes.
- If the policy cannot be relaxed because Windows reports access denied or a permanently enforced policy, leave session startup functional and log a warning with the Windows error code. The product behavior is not complete until affected Windows builds are verified, so this fallback must be paired with manual validation.

Do not attempt to resolve or rewrite user symlinks in Warp. Rewriting PATH entries, WinGet links, pytest temp paths, or version-manager shims would only address specific surfaces and would miss arbitrary toolchain subprocesses.

### 3. Preserve ConPTY and process lifetime behavior

Keep these existing behaviors intact:

- `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` remains present on shell startup.
- `EXTENDED_STARTUPINFO_PRESENT` remains set when calling `CreateProcessW`.
- `CREATE_UNICODE_ENVIRONMENT` and the existing environment block remain unchanged.
- `CREATE_BREAKAWAY_FROM_JOB` remains set so the shell can break away from Warp's app-level job when appropriate.
- `ChildExitWatcher` continues to watch the shell process handle.
- `ConptyApi::release()` still runs after successful process creation.

If `ProcThreadAttributeList::new()` changes to accept a capacity, update callers so the default remains safe and explicit. For example, `ProcThreadAttributeList::new(attribute_count)` should replace the hardcoded `1`, and `windows::spawn()` should pass `2` only when it will set the mitigation attribute.

### 4. Add diagnostics for verification and support

Add targeted debug logs around Windows shell spawn:

- Redirection trust state on the Warp process before applying any fix.
- Whether Warp applied a creation-time child policy or current-process policy fallback.
- Redirection trust state observed on the spawned shell process when querying it is possible.
- Windows error code if applying or querying the policy fails.

These logs should not be surfaced in the terminal UI. They are for Warp logs and support/debugging only.

### 5. Keep shared command wrappers unchanged unless validation shows they inherit the same bug

The issue is user shell traversal, so the first implementation should focus on the PTY shell spawn path. If validation shows that Warp-internal Windows commands also fail with OS error 448 when using symlinked executables, then add the same helper to the shared `command` crate wrappers:

- `crates/command/src/async.rs` for async commands.
- `crates/command/src/blocking.rs` for blocking commands.

This should be a follow-up or same-PR extension only if needed. Avoid broad process-policy changes for all internal commands unless there is a concrete failing surface.

## End-to-end flow

1. User opens a new local Windows tab in Warp.
2. Warp initializes the Windows job object as it does today.
3. `PtySpawner` calls `windows::spawn()` for the selected local shell.
4. `windows::spawn()` prepares ConPTY and an extended startup attribute list.
5. Warp records or applies the redirection trust policy needed for ordinary terminal-host behavior.
6. Warp calls `CreateProcessW` with the pseudoconsole attribute and the policy behavior in place.
7. PowerShell or Git Bash starts in the ConPTY session.
8. The user runs a command that traverses a symlink or launches a WinGet file-symlink shim.
9. Windows permits the traversal when the same traversal is permitted in Windows Terminal.
10. Child processes launched by the shell inherit the corrected behavior and do not fail with Warp-specific OS error 448.

## Risks and mitigations

- Risk: Disabling RedirectionGuard too broadly could reduce protection for the Warp app process itself.
  - Mitigation: Prefer a shell-child-scoped creation policy over changing the Warp app process policy. If fallback requires changing the current process policy, keep the helper Windows-only, document why terminal compatibility needs it, and verify no unrelated app code depends on enforced redirection trust.

- Risk: Windows may not allow RedirectionGuard policy relaxation on some builds or when policy is enforced by enterprise configuration.
  - Mitigation: Log failures with error codes, keep the app from crashing, and document that parity cannot be guaranteed when the OS or enterprise policy also blocks standalone shells.

- Risk: Increasing `ProcThreadAttributeList` capacity or adding another startup attribute could break ConPTY startup.
  - Mitigation: Keep `set_pty_connection()` behavior unchanged, add a Windows unit test for multi-attribute initialization where feasible, and manually validate PowerShell/Git Bash startup, resizing, and exit.

- Risk: The issue might be caused by a different Windows process attribute than RedirectionGuard.
  - Mitigation: Start implementation with `GetProcessMitigationPolicy` instrumentation and compare Warp, Warp-hosted shell, Windows Terminal, and standalone PowerShell on an affected machine before finalizing the fix. If RedirectionGuard is not the difference, update this tech spec with the confirmed attribute before implementing the production change.

- Risk: Fixing only the initial shell process may not fix toolchain descendants if the policy is not inheritable.
  - Mitigation: Validation must include subprocesses such as pytest, WinGet shims, and a nested script-created child process, not only direct shell cmdlets.

## Testing and validation

Automated tests:

- Add unit coverage for any new policy flag packing/parsing helper so `EnforceRedirectionTrust` and `AuditRedirectionTrust` are decoded correctly.
- Add unit coverage for `ProcThreadAttributeList` capacity handling if it becomes parameterized.
- Keep existing Windows child-exit watcher coverage in `app/src/terminal/local_tty/windows/child_tests.rs` passing.
- Run formatting and the smallest relevant Rust test subset available for the changed crates. At minimum, run `cargo fmt --check` and targeted tests for `warp_app` / `command` modules touched by the implementation.

Manual validation on affected Windows 11:

- Compare `GetProcessMitigationPolicy(ProcessRedirectionTrustPolicy)` for Warp, Warp-hosted PowerShell, standalone PowerShell, and Windows Terminal-hosted PowerShell before and after the fix.
- Run the issue's PowerShell symlink repro in Warp.
- Run an equivalent Git Bash/MSYS2 symlink traversal repro in Warp.
- Run a WinGet file-symlink shim such as `jq --version` or `terraform --version` from Warp.
- Run a minimal pytest project that creates a symlink under `tmp_path`, resolves it, and exits successfully.
- Run a nested child-process repro, for example PowerShell launching `pwsh -NoProfile -Command "Get-ChildItem <symlink>"`, to verify descendants inherit the behavior.
- Start and close multiple PowerShell and Git Bash tabs to verify ConPTY release, child exit detection, and Warp session cleanup still work.

## Follow-ups

- If cargo/rustc failures from related issue reports persist after this fix, write a narrower follow-up spec for process creation through non-reparse paths.
- If internal Warp commands or LSP launchers fail on symlinked executables, extend the Windows process-policy helper into the shared `command` crate.
- Consider adding a Windows support diagnostic command or log bundle field that records shell process mitigation state when OS error 448 appears in a terminal session.
