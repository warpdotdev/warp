# TECH.md — External Terminal Command Execution (VS Code Debugging)

Issue: https://github.com/warpdotdev/warp/issues/10165
Product spec: `specs/GH10165/product.md`

## Context

Ordinary external terminal opening (`"Terminal: Open New External Terminal"` in VS Code) passes a directory URL (`open -a Warp.app <cwd>` on macOS, or CWD-inherited process launch on Linux/Windows) which enters [`app/src/uri/mod.rs`](../../app/src/uri/mod.rs#L1251-L1289) as a `file://` URL.

Debug external-terminal launching (`"console": "externalTerminal"`) uses DAP `runInTerminal` (`ExternalTerminalService.runInTerminal` in `microsoft/vscode`), which passes:
- A working directory (`dir`)
- A command and argument vector (`args`)
- Merged caller + `launch.json` environment variables (`envVars`)

Previously, Warp lacked:
1. CLI launch flags (`-e` / `--command`, `--working-directory`, `--wait-after-command`, `--new-window`, `-T` / `--title`, Windows `/c`) on [`AppArgs`](../../crates/warp_cli/src/lib.rs#L188-L205).
2. A mechanism to inject sanitized caller environment variables and queue an exact shell-quoted command in a new tab/window on [`NewTerminalOptions`](../../app/src/pane_group/mod.rs#L794-L807).
3. Single-instance IPC forwarding for command/env launches across macOS (which had no single-instance IPC), Linux (where [`DBusServiceHost`](../../app/src/app_services/linux/mod.rs#L188-L258) left `activate_action` unimplemented), and Windows (where [`UriService`](../../app/src/app_services/windows/service_impl.rs#L13-L18) only carried `Vec<Url>`).

Relevant existing code:
- CLI parser: [`crates/warp_cli/src/lib.rs`](../../crates/warp_cli/src/lib.rs)
- App startup & launch dispatch: [`app/src/lib.rs`](../../app/src/lib.rs#L1170-L1235), [`app/src/lib.rs`](../../app/src/lib.rs#L3250-L3300)
- Captured startup environment: [`app/src/process_environment.rs`](../../app/src/process_environment.rs)
- Window/tab creation & command queueing: [`app/src/workspace/view.rs`](../../app/src/workspace/view.rs#L13103-L13200), [`app/src/root_view.rs`](../../app/src/root_view.rs#L930-L958), [`app/src/terminal/view.rs`](../../app/src/terminal/view.rs#L10077-L10081)
- Shell escaping: [`crates/warp_terminal/src/shell/mod.rs`](../../crates/warp_terminal/src/shell/mod.rs#L1042-L1049)
- PTY env merging (caller `env_vars` applied last): [`crates/warp_terminal/src/local_tty/unix.rs`](../../crates/warp_terminal/src/local_tty/unix.rs#L440-L443), [`crates/warp_terminal/src/local_tty/windows/environment.rs`](../../crates/warp_terminal/src/local_tty/windows/environment.rs#L199-L208)

## Proposed changes

### 1. CLI Argument Parsing (`crates/warp_cli/src/lib.rs`)

- Added `ExternalCommandSpec` (`Argv(Vec<String>) | WindowsCmdLine(String)`) and `ExternalSessionArgs` flattened onto `AppArgs`:
  - `--working-directory` / `--cwd <DIR>`
  - `--new-window`
  - `-T` / `--title <TITLE>`
  - `--wait-after-command[=<BOOL>]`
  - `-e` / `--command <COMMAND>...`
- Added `Args::try_parse_app_from` / `Args::try_parse_from_iter_with_options` with `find_external_command_split`:
  - Scans tokens prior to `--` only when no Clap subcommand or hidden worker invocation is active.
  - Dynamically inspects Clap's `clap::Command` argument metadata to skip flags that take a separate value, so a title like `--title -e` does not trigger splitting prematurely.
  - Splits before `-e`, attached `-e<first>`, `--command`, `--command=<first>`, or (on Windows) `/c` / `/C`, parses only the prefix with Clap, and stores all trailing tokens verbatim in `ExternalCommandSpec`.
- On Windows, `current_win32_command_line` reads Win32 `GetCommandLineW()` so `extract_raw_windows_cmdline_after_token` extracts the verbatim tail after `/c` / `/C` (preserving internal quotes around spaced paths) and `unwrap_redundant_windows_cmdline_quotes` strips redundant outer `""..."` quotes from VS Code's non-Windows-Terminal fallback.
- External-session flags are hidden on `oz --help` and rejected when `oz` / `WARP_CLI_MODE` runs as a standalone CLI binary.

### 2. External Session Launch, Env Sanitization & Quoting (`app/src/external_session.rs`)

- `ExternalSessionLaunchRequest` holds `working_directory`, `command`, sanitized `env_vars`, `title`, `new_window`, and `wait_after_command`. On macOS, when `--working-directory` is omitted and `current_dir()` is `/` (as inherited by GUI/LaunchServices launches), `working_directory` is omitted so Warp uses its configured default startup directory.
- `sanitize_caller_env_vars` strips `WARP_*` and ambient terminal/launcher variables (`TERM`, `TERM_PROGRAM`, `TERM_PROGRAM_VERSION`, `COLORTERM`, `TERMINFO`, `WINDOWID`, `SHELL`, `SHLVL`, `PWD`, `OLDPWD`, `_`, `ZDOTDIR`, `__CFBundleIdentifier`, `__CF_USER_TEXT_ENCODING`, `XPC_SERVICE_NAME`, `XPC_FLAGS`, `DESKTOP_STARTUP_ID`, `GIO_LAUNCHED_DESKTOP_FILE`, `GIO_LAUNCHED_DESKTOP_FILE_PID`) while preserving `PATH`/`Path`, `VIRTUAL_ENV`, `PYTHONPATH`, debugger vars, and user `launch.json` `env` values.
- `format_external_command_for_shell` unwraps Linux VS Code's `bash -c "''...;''"` outer quote wrapper, uses `shell_quote_arg` per `ShellType` (`Bash`/`Zsh`, `Fish`, `PowerShell` with `&` call operator), wraps `WindowsCmdLine` as `cmd.exe /c '<raw>'`, and appends `; exit` when `wait_after_command` is `Some(false)`.
- `open_external_session` inspects the primary `Workspace` (`uri::get_primary_window`) via `Workspace::open_or_reuse_external_session_tab` when `!request.new_window`: if a `TabData` marked `opened_for_external_session` still holds a single unshared idle terminal pane (`is_idle_single_terminal_for_external_session_reuse` / `is_idle_for_external_session_reuse`: bootstrapped, no pending/executing command or alt-screen app, no AI conversation, and empty input buffer), it swaps in a fresh `PaneGroup` (`PanesLayout::SingleTerminal(Box::new(options))`), shuts down and detaches the previous `PaneGroup` without pushing to `UndoCloseStack`, and preserves single-tab windows; otherwise it opens a new tab via `add_tab_with_pane_layout` and marks `opened_for_external_session = true` (also set on the initial tab when opening a new window). On Windows, when launching a `/c` (`WindowsCmdLine`) command and the user's preferred shell is WSL or MSYS2, it falls back to a native Windows shell so `cmd.exe /c` runs in a compatible native environment.

### 3. Single-Instance IPC & Startup Wiring (`app/src/app_services/`, `app/src/lib.rs`)

- Added `StartupForwardRequest { Urls(Vec<Url>), ExternalSession(ExternalSessionLaunchRequest) }` and `should_run_single_instance_server(ctx)` (enabled only for GUI `ExecutionMode::App` and `Channel != Channel::Integration`, preventing integration test runners or CLI/TUI processes from colliding).
- **macOS (`app/src/app_services/mac.rs`)**: Added `MacSingleInstanceServer` over an owner-only Unix domain socket (`<config_local_dir>/run/app.sock` with UID-verified `/tmp/warp-<uid>/<hash>.sock` fallback for `SUN_LEN`), creating runtime directories atomically with `0700` via `DirBuilderExt` (`mode(0o700)`), enforcing `0600` socket permissions, using `safe_*` logging macros for diagnostics, plus `pass_startup_args_to_existing_instance` running in `run_internal` before AppKit starts.
- **Linux (`app/src/app_services/linux/mod.rs`)**: Implemented `org.freedesktop.Application.activate_action("open_external_session", ...)` and updated `pass_startup_args_to_existing_instance` to forward `ExternalSessionLaunchRequest` JSON in both dev and release builds.
- **Windows (`app/src/app_services/windows/`)**: Generalized `UriService::Request` to `StartupForwardRequest` and enabled `SingleInstanceManager` + external-session forwarding for non-`release_bundle` GUI builds as well as release bundles.
- **`app/src/lib.rs`**: Wired warm-instance forwarding in `run_internal()` and cold-start `open_external_session` handling in `launch()`.

## Upstream `microsoft/vscode` Follow-Up (macOS)

Stock VS Code's `MacExternalTerminalService.runInTerminal` (`src/vs/platform/externalTerminal/node/externalTerminalService.ts`) currently rejects any `terminal.external.osxExec` other than `Terminal.app`, `iTerm.app`, or `Ghostty.app`. Once this Warp support is shipped, a small upstream PR to `microsoft/vscode` mirroring the `Ghostty.app` branch enables `"terminal.external.osxExec": "Warp.app"` (and `"WarpPreview.app"`) out of the box:

```ts
} else if (terminalApp === 'Warp.app' || terminalApp === 'WarpPreview.app') {
    const env = Object.assign({}, getSanitizedEnvironment(process), envVars);
    const openArgs = ['-na', terminalApp, '--args'];
    openArgs.push('--working-directory=' + dir);
    openArgs.push('--wait-after-command=true');
    if (title) {
        openArgs.push('--title', title);
    }
    openArgs.push('-e', ...args);

    const cmd = cp.spawn('/usr/bin/open', openArgs, { env });
    setupSpawnErrorHandling(cmd, resolve, reject, terminalApp);
}
```

## Testing and validation

| Product invariant | Verification |
|---|---|
| 1–4 CLI flags, attached `-e<prog>`, dynamic flag inspection, `GetCommandLineW()` `/c` raw extraction, `oz` / `WARP_CLI_MODE` isolation, missing-arg error | `crates/warp_cli/src/lib_tests.rs` (`cargo nextest run -p warp_cli`) |
| 5–7 Tab vs. `--new-window`, idle external-tab reuse vs. busy/user-tab preservation, custom title, working directory, precmd readiness | `app/src/workspace/view_tests.rs` (`reuses_idle_external_session_tab_in_place`, etc.) + `crates/integration/src/test/external_session.rs` (`test_external_session_opens_tab_and_runs_command_with_env`, `test_external_session_new_window_runs_command`, `test_external_session_reuses_idle_debug_tab`, `test_external_session_opens_new_tab_when_debug_tab_is_busy`) |
| 8 Env sanitization & macOS `/` CWD omission | `app/src/external_session_tests.rs` + integration test `$VSCODE_EXT_DEBUG_VAR` check |
| 9 Shell quoting across Bash/Zsh/Fish/PowerShell, Linux `bash -c "''...;''"` unwrapping, Windows WSL/MSYS2 `/c` fallback | `app/src/external_session_tests.rs` (`cargo nextest run -p warp external_session`) |
| 10–12 Warm IPC handoff (atomic `0700` `DirBuilder` + `safe_*` logging on macOS), cold start, `--wait-after-command=false` (`; exit`) vs. interactive longevity | IPC serde round-trip & quoting unit tests + integration test follow-up `echo still_alive` + manual verification per OS |
