# PRODUCT.md — External Terminal Command Execution (VS Code Debugging)

GitHub issue: https://github.com/warpdotdev/warp/issues/10165

Figma: none required. External debug/command sessions open as standard Warp terminal tabs or windows with existing chrome.

## Summary

Allow IDEs, debuggers, and external tools — starting with VS Code's `"console": "externalTerminal"` debug workflow — to launch Warp with a working directory, caller environment overrides, optional tab title, and a command + arguments to run inside a new Warp terminal session across **macOS**, **Linux**, and **Windows**.

## Problem

Users can configure Warp as VS Code's external terminal (`"terminal.external.osxExec": "Warp.app"`, `"terminal.external.linuxExec"`, `"terminal.external.windowsExec"`) and open a directory via `"Open New External Terminal"`. However, starting a debug session with `"console": "externalTerminal"` fails because DAP `runInTerminal` needs the external terminal to:

1. Accept a working directory, command, and arguments via CLI flags without Warp's own CLI parser rejecting child flags (`--debug`, `--help`, `-m`, `--listen`, etc.).
2. Inherit the caller's environment overrides (such as `PYTHONPATH`, `VIRTUAL_ENV`, and `launch.json` `env` entries) into the spawned shell.
3. Forward that launch request to an already-running Warp GUI instance on macOS, Linux, and Windows — especially on macOS, where `open -na Warp.app --args ...` is required to pass `--args` and environment variables when Warp is already running.

## Goals / Non-goals

**In scope**

- Desktop CLI flags for launching an external terminal session: `-e` / `--command <PROGRAM> [ARGS...]` (including attached `-e<PROGRAM>` and `--command=<PROGRAM>`), `--working-directory <DIR>` (alias `--cwd`), `--new-window`, `-T` / `--title <TITLE>`, `--wait-after-command[=<BOOL>]`, and Windows `/c <CMDLINE>`.
- Verbatim forwarding of child arguments after `-e` / `--command` (and raw `/c` command lines on Windows via `GetCommandLineW()`, preserving spaced paths and unwrapping redundant outer quotes).
- Hiding desktop external-session flags from `oz --help` and rejecting them when `oz` / `WARP_CLI_MODE` runs as a standalone CLI binary.
- Sanitized caller environment forwarding into the spawned shell session.
- Single-instance IPC forwarding on macOS, Linux, and Windows so a secondary launcher delegates to the running Warp instance and exits `0` immediately.
- Opening a new tab in the primary Warp window when no idle external-session tab exists (or reusing an idle external-session tab in place so repeated debug runs do not accumulate empty tabs), or opening a new window when `--new-window` is specified or no Warp window exists.
- Running the command inside a bootstrapped interactive shell block so live `stdout`, `stderr`, and `stdin` work, keeping the shell open when `--wait-after-command` is `true` or omitted and exiting the shell after completion (`; exit`) when `--wait-after-command=false`.

**Out of scope**

- Modifying the upstream `microsoft/vscode` repository (stock VS Code on macOS currently maintains a hardcoded allowlist in `MacExternalTerminalService.runInTerminal`; adding `Warp.app` / `WarpPreview.app` there is a separate upstream follow-up).
- Returning the debuggee's PID from `runInTerminal` (matches VS Code's `Ghostty.app`, `Terminal.app`, and `iTerm.app` behavior, which resolve `undefined` and rely on DAP reverse-connect).
- Headless TUI (`warp-tui`) or Web (`wasm`) external terminal launching.
- Rewriting `AppArgs` into a remote/SSH session spawner.

## Behavior

1. **CLI flags**: Invoking the Warp desktop executable with any combination of `--working-directory <DIR>` (or `--cwd <DIR>`), `--new-window`, `-T <TITLE>` / `--title <TITLE>`, `--wait-after-command[=<BOOL>]`, `-e <PROGRAM> [ARGS...]` (including attached `-e<PROGRAM>`) / `--command <PROGRAM> [ARGS...]`, or (on Windows) `/c <CMDLINE>` launches an external terminal session instead of opening a blank default tab or failing CLI validation.

2. **Verbatim child arguments**: All tokens after `-e` (including `-e<PROGRAM>`) or `--command` (including `--command=<PROGRAM>`) are passed verbatim to the child command. Flags belonging to the child program — including `--debug`, `--help`, `-V`, `--version`, `-m`, and `--listen` — are never intercepted by Warp's CLI parser. On Windows, `/c <CMDLINE>` preserves the raw command tail from Win32 `GetCommandLineW()` (retaining quotes around spaced paths and unwrapping redundant outer quotes from VS Code's non-Windows-Terminal fallback).

3. **Existing subcommands & standalone CLI isolation**: Subcommands (`agent`, `mcp`, `environment`, `run`, `login`, etc.) and hidden worker subcommands (`terminal-server`, `minidump-server`, etc.) continue to parse identically and are unaffected by `-e` / `--command` handling. Desktop external-session flags (`-e`, `--command`, `--working-directory`, `--cwd`, `--new-window`, `-T`, `--title`, `--wait-after-command`, `/c`) are hidden from `oz --help` and rejected when `oz` / `WARP_CLI_MODE` runs as a standalone CLI binary.

4. **Missing command rejected**: Passing `-e`, `--command`, or Windows `/c` with no trailing command exits with a non-zero CLI usage error.

5. **Tab vs. window placement & idle debug-tab reuse**: When Warp already has an open window and `--new-window` is not passed, Warp checks whether an existing tab opened for an external session is currently idle (a single unshared terminal pane sitting at the prompt with no running/pending command, no AI conversation, and an empty input buffer). If an idle external-session tab exists, Warp replaces its session in-place (with a fresh clean shell, working directory, caller environment, and title) without adding to undo-close history or closing a single-tab window. Otherwise (no external-session tab exists, or all of them are busy executing a command or modified), Warp opens a new focused tab in the primary window (honoring active quake mode or frontmost window selection) with the Code Mode homepage hidden. Ordinary user tabs are never reused. When `--new-window` is passed or no window is open, Warp opens and focuses a new window.

6. **Custom title**: When `-T` / `--title` is provided, the opened tab displays that title.

7. **Working directory**: When `--working-directory` / `--cwd` is provided (supporting leading `~` expansion and relative paths), the new shell starts in that directory if it exists; otherwise it falls back gracefully to Warp's default startup directory. When omitted, it uses the launcher process's current working directory, except on macOS when the inherited directory is `/` (from LaunchServices/`open`), where `/` is omitted so Warp uses its configured default startup directory.

8. **Environment inheritance and sanitization**: The spawned shell receives the launcher process's environment variables (preserving `PATH`, `VIRTUAL_ENV`, `CONDA_PREFIX`, `PYTHONPATH`, debugger variables, and `launch.json` `env` entries) while stripping ambient terminal/Warp-internal variables (`WARP_*`, `TERM`, `TERM_PROGRAM`, `TERM_PROGRAM_VERSION`, `COLORTERM`, `TERMINFO`, `WINDOWID`, `SHELL`, `SHLVL`, `PWD`, `OLDPWD`, `_`, `ZDOTDIR`, `__CFBundleIdentifier`, `__CF_USER_TEXT_ENCODING`, `XPC_*`, `DESKTOP_STARTUP_ID`, `GIO_LAUNCHED_*`) so Warp's own PTY bootstrap stays intact.

9. **Lossless shell quoting & platform unwrapping**: Arguments containing spaces, single/double quotes, `$` expansions, globs, semicolons, or newlines are escaped according to the target session's `ShellType` (`Bash`, `Zsh`, `Fish`, `PowerShell`) and queued as an exact pending command executed once shell bootstrap completes. On PowerShell, quoted command invocations use the `&` call operator. On Linux, VS Code's `bash -c "''...;''"` outer quote wrapper is unwrapped before execution. On Windows, `/c` command lines invoke `cmd.exe /c` inside a native Windows shell (falling back from WSL/MSYS2 user preferences so native Windows paths and environment work).

10. **Warm single-instance IPC handoff**: When a compatible Warp GUI instance is already running for the same OS user and channel on macOS, Linux, or Windows, a secondary invocation carrying external session options forwards the launch request (including working directory, command, and sanitized caller environment) over same-user IPC and exits `0` immediately without spawning a duplicate GUI window or macOS Dock icon.

11. **Cold start**: When no Warp GUI instance is running, Warp boots normally, restores or creates windows, opens the external session tab/window, and executes the requested command once bootstrapped.

12. **Interactive session longevity (`--wait-after-command`)**: Commands run inside a standard interactive Warp block with full TTY stdin/stdout/stderr and Ctrl+C support. When `--wait-after-command` is `true` or omitted, the shell session remains open and interactive after the command exits; when `--wait-after-command=false` is passed, the shell session exits after the command completes (`; exit`).
