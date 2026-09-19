# PRODUCT.md — Windows symlink and junction traversal returns OS error 448 in Warp

Issue: https://github.com/warpdotdev/warp/issues/9044

## Summary

Windows users running local shells in Warp must be able to traverse NTFS symbolic links, file symbolic links, directory symbolic links, and junctions with the same behavior they get in Windows Terminal, standalone PowerShell, and standalone Git Bash. Today, common Windows development workflows fail inside Warp with `ERROR_UNTRUSTED_MOUNT_POINT` / WinError 448 when tools resolve or execute paths that contain reparse points. This issue blocks pytest cleanup, Node version manager shims, WinGet-installed command shims, and related toolchains.

The desired outcome is that Warp's Windows terminal sessions behave like a normal terminal host: shells and the command-line tools they spawn can follow user-created symlinks and junctions unless Windows itself would block the same operation outside Warp.

## Problem

The current Windows behavior makes Warp unusable for users whose workflows depend on common symlink or junction patterns:

- Python projects using pytest can complete their tests but exit with failure during cleanup when `Path.resolve()` traverses a symlink in a temporary directory.
- Node version managers can break when the active Node version is selected through a symlink or junction.
- WinGet `PortableCommand` packages install executable shims as NTFS file symlinks under `%LOCALAPPDATA%\Microsoft\WinGet\Links`; commands such as `jq`, `terraform`, `databricks`, and `copilot` can fail only in Warp.
- Related process-spawn failures can appear when a tool launches another executable through a path affected by the same Windows redirection trust behavior.

The same commands succeed in Windows Terminal and standalone shells, so users perceive this as a Warp-specific terminal compatibility regression rather than a general Windows filesystem limitation.

## Goals

- Local Windows Warp sessions can traverse symlinks and junctions in normal user-writable development locations.
- Executable file symlinks on `PATH`, including WinGet shims under `%LOCALAPPDATA%\Microsoft\WinGet\Links`, execute from Warp the same way they do in Windows Terminal.
- Directory symlinks and junctions used as intermediate path components continue to work.
- Toolchains that spawn child processes through symlinked or junction-backed paths, including pytest, Node version managers, WinGet packages, and cargo/rustc scenarios related to OS error 448, no longer fail because they are running inside Warp.
- The behavior applies consistently across supported local Windows shell types, especially PowerShell 7 and Git Bash/MSYS2.
- Warp does not introduce a user-visible setting, warning, or workaround for this compatibility behavior.
- Warp keeps its existing child-process lifetime management and terminal behavior where possible.

## Non-goals

- This spec does not change WSL filesystem semantics inside Linux distributions.
- This spec does not make Windows bypass OS-level blocks that also occur in Windows Terminal or standalone PowerShell.
- This spec does not require Warp to create, trust, or rewrite user symlinks, junctions, WinGet links, pytest temp directories, or version-manager layouts.
- This spec does not add UI for managing Windows process mitigation policies.
- This spec does not change non-Windows terminal behavior.
- This spec does not require supporting elevated administrator workflows beyond matching Windows Terminal for the same user, shell, command, and elevation state.

## Figma / design references

Figma: none provided. This is a terminal compatibility fix with no intended UI changes.

## User experience

1. A user opens a local Windows session in Warp and runs the issue's PowerShell repro:
   ```powershell
   $d = "$env:TEMP\symtest"
   mkdir "$d\real" -Force | Out-Null
   New-Item -ItemType SymbolicLink -Path "$d\link" -Target "$d\real"
   Get-ChildItem "$d\link"
   Remove-Item "$d" -Recurse -Force
   ```
   `Get-ChildItem "$d\link"` succeeds and lists the target directory contents instead of printing `The path cannot be traversed because it contains an untrusted mount point`.

2. Cleanup of directories containing symlinks or junctions succeeds when the same cleanup succeeds in standalone PowerShell. The repro's `Remove-Item "$d" -Recurse -Force` does not leave behind Warp-specific failures or locked reparse points.

3. A Python project whose tests create symlinks under pytest `tmp_path` exits with code 0 when all tests pass. Warp must not cause pytest to exit with code 1 only because cleanup called `Path.resolve()` on a symlink.

4. Commands installed by WinGet as file symlinks under `%LOCALAPPDATA%\Microsoft\WinGet\Links` execute normally from Warp. For example, if `jq`, `terraform`, `databricks`, or `copilot` works from Windows Terminal, the same command and arguments work from Warp.

5. Tools using version-manager shims continue to work. If an active Node, Rust, Python, or similar executable is selected through a symlink or junction and works outside Warp, invoking it and invoking tools that spawn it also work inside Warp.

6. The fix is inherited by child processes launched from the shell. A shell process created by Warp must not be corrected only for direct shell built-ins while leaving processes such as `pytest`, `cargo`, `node`, `npm`, `jq`, or `terraform` with the same OS error 448 failure.

7. Behavior is shell-agnostic for local Windows sessions. PowerShell 7 and Git Bash/MSYS2 should both be able to traverse the same reparse points. If Warp later supports additional local Windows shell types, they should inherit the same behavior by default.

8. Existing Warp terminal UX remains unchanged. Block creation, command rendering, prompt integration, environment injection, startup directory handling, and tab/session lifecycle should look and feel the same before and after this fix.

9. If Windows blocks a traversal outside Warp for the same user and command, Warp can surface the same OS error. The required behavior is parity with normal terminal hosts, not weakening Windows for cases that are blocked system-wide.

10. If Warp cannot remove the problematic inherited process state on a specific unsupported Windows build, session startup should fail gracefully or continue with diagnostic logging rather than crashing the app.

## Success criteria

- The issue's symlink traversal repro succeeds in Warp on an affected Windows 11 build.
- The same repro succeeds in both PowerShell 7 and Git Bash/MSYS2 sessions launched by Warp.
- WinGet symlinked command shims on `PATH` execute in Warp when the target executable works in Windows Terminal.
- pytest suites that previously failed only during symlink cleanup exit successfully when tests pass.
- A child process launched from the Warp-hosted shell inherits the corrected behavior; a direct shell operation and a subprocess operation both traverse or execute through reparse points successfully.
- The fix does not regress shell startup, ConPTY creation, resize handling, process exit detection, or Warp's ability to terminate shell sessions.
- The fix does not require users to recreate symlinks or junctions, run Warp elevated, change Windows Defender or Exploit Protection settings, or bypass WinGet/version-manager defaults.
- Non-Windows platforms remain unchanged.

## Validation

- Manual validation on an affected Windows 11 machine:
  - Run the issue's PowerShell symlink repro in Warp and Windows Terminal and verify matching success.
  - Run an equivalent Git Bash/MSYS2 repro that creates and traverses a directory symlink or junction-compatible path.
  - Install or use a WinGet package that creates a file symlink in `%LOCALAPPDATA%\Microsoft\WinGet\Links`, then run `<tool> --version` from Warp.
  - Run a pytest suite or minimal pytest fixture that creates a symlink under `tmp_path` and resolves it during cleanup.
  - Run a nested child-process check where a script launched from the shell attempts to traverse or execute through a symlink.
- Automated validation should cover any new Windows-specific process policy helper with unit tests where possible, and the manual Windows repro should be documented in the implementation PR.
- Regression validation should include starting and closing local PowerShell and Git Bash sessions to ensure ConPTY and child exit watcher behavior still works.

## Open questions

- Which Windows release or Warp packaging/startup change first caused affected users to inherit the blocking redirection trust state?
- Does the same underlying fix also fully cover the related cargo/rustc process-spawn reports, or is a second CreateProcess-specific bug needed after symlink traversal parity is restored?
- Can CI provide a reliable Windows test environment with RedirectionGuard enforcement enabled, or will this remain primarily a manual/release-validation scenario?
