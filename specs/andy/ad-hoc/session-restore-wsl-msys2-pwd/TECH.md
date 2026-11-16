# Session Restore PWD for WSL and Git Bash — Tech Spec

See `PRODUCT.md` for user-visible behavior.

## Context

When Warp saves a session snapshot it records the terminal's current working directory via `TerminalView::active_session_path_if_local`, which calls `ShellLaunchData::maybe_convert_absolute_path` on the raw Unix-style `$PWD` string the shell reports:

- For WSL, `/home/user/projects` → `\\WSL$\<distro>\home\user\projects` (Windows UNC path).
- For MSYS2/Git Bash, `/c/Users/user/projects` → `C:\Users\user\projects` (native drive path).

So by the time the path is written into `TerminalSnapshot::cwd`, it is already a Windows-native path.

**Relevant files:**

- `crates/warp_terminal/src/shell/mod.rs (768–790)` — `ShellLaunchData::maybe_convert_absolute_path`, which performs the Unix → Windows conversion at snapshot time.
- `app/src/terminal/view.rs (6506–6528)` — `active_session_path_if_local`, which calls `maybe_convert_absolute_path` and is the write path into the snapshot.
- `app/src/pane_group/mod.rs (1533–1570)` — session restore logic that reads `TerminalSnapshot::cwd` and computes `startup_directory`.

## Why we store host-native paths in sqlite

The snapshot stores `cwd` as a Windows-native path rather than the guest-native Unix path for three reasons:

1. **`CreateProcessW` requires it.** `lpCurrentDirectory` must be a Windows path. Storing it host-native means no conversion is needed at restore time.
2. **`is_dir()` works natively.** Windows can stat `\\WSL$\<distro>\...` paths directly, letting the restore code verify the directory still exists without any extra logic.
3. **Avoids per-shell branching at restore time.** Storing the guest-native path and re-converting at restore time would require extracting the distro or MSYS2 executable from `shell_launch_data` again — exactly the logic that caused the original bug.

## Root Cause

The restore code in `pane_group/mod.rs` was re-running the Unix→Windows conversion on `cwd`, passing the already-converted Windows path back into `convert_wsl_to_windows_host_path` / `convert_msys2_to_windows_native_path`. Both functions expect a Unix-style input; given a Windows path they fail and return `None`, so `startup_directory` was always `None` for WSL and MSYS2 sessions, causing the restored terminal to open in the shell's default directory instead of the saved one.

The `TODO(CORE-3130)` comment in the old WSL branch also noted that the resulting path was being ignored downstream — a sign the whole conversion was unnecessary.

## Proposed Changes

**`app/src/pane_group/mod.rs`**

Replace the `shell_launch_data`-aware path conversion block with a direct `PathBuf::from(cwd)`:

```rust
let startup_directory = terminal_snapshot
    .cwd
    .map(PathBuf::from)
    .filter(|path| path.is_dir());
```

`CreateProcessW`'s `lpCurrentDirectory` accepts both forms:
- `\\WSL$\<distro>\...` UNC paths — `wsl.exe` translates these back to Linux paths on startup.
- Native `C:\...` drive paths — MSYS2's `bash.exe` maps them to the corresponding MSYS2 path (e.g. `/c/...`) via its own mount table on startup.

The `chosen_shell` / `wsl_distro` / `msys2_executable` locals derived from `shell_launch_data` are no longer needed for path conversion. `chosen_shell` (used only for `AvailableShells::get_from_shell_launch_data`) is retained in a simplified form; the other two are removed. The `convert_msys2_to_windows_native_path`, `msys2_exe_to_root`, and `WindowsPath` imports that were used solely for the now-deleted conversion are also removed.

## Testing and Validation

- **Behavior 2 (WSL):** Open a WSL terminal, `cd` to a non-default directory (e.g. `~/projects`), quit Warp, relaunch. Confirm the restored WSL tab opens in `~/projects`.
- **Behavior 3 (MSYS2/Git Bash):** Open a Git Bash terminal, `cd /c/Users/<user>/projects`, quit Warp, relaunch. Confirm the restored tab opens in `/c/Users/<user>/projects`.
- **Behavior 4 (missing directory):** Delete the saved directory before relaunching. Confirm the tab opens without error, falling back to the shell default.
- **Behavior 5 (unaffected shells):** Verify PowerShell and Cmd session restore continues to work as before.
