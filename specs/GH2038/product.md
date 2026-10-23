# Product Spec: Nushell support

**Issue:** [warpdotdev/warp#2038](https://github.com/warpdotdev/warp/issues/2038)
**Figma:** none provided

## Summary

Warp should support [Nushell](https://www.nushell.sh/) as a first-class local shell option. Users who have `nu`/`nu.exe` installed should be able to select it, launch an interactive Warp terminal session, and keep core Warp terminal features such as blocks, prompt integration, history discovery, aliases, environment-variable export, and command execution working with Nushell syntax.

## Problem

Nushell users currently cannot use Warp as their daily terminal because Warp's shell integration does not recognize or bootstrap `nu`. The lack of support blocks users who rely on Nushell's structured pipelines, scripting language, and cross-platform shell experience.

## Goals

- Allow users to launch local Warp sessions backed by Nushell.
- Preserve Warp's core shell-integration experience in Nushell sessions.
- Avoid treating Nushell as Bash when syntax or control-flow semantics differ.
- Keep the first iteration focused on local shell support that can be tested reliably.

## Non-goals

- Implementing a Nushell plugin, MCP server integration, LSP integration, or a full custom-completion engine backed by Nushell internals.
- Supporting remote SSH/session warpification for Nushell in the first iteration.
- Supporting Nushell subshell bootstrap flows in the first iteration.
- Changing Nushell itself or requiring users to modify their Nushell configuration manually.

## Behavior

1. Warp recognizes Nushell executables named `nu`, login-shell aliases named `-nu`, Unix paths ending in `/nu`, and Windows executable basenames case-insensitively equal to `nu.exe`.

2. Warp must not misclassify unrelated executables as Nushell merely because their path or name contains the substring `nu`. For example, `menu.exe`, `NuGet.exe`, and `/usr/bin/menu.exe` are not Nushell.

3. When the user has Nushell installed in a discoverable location, Warp lists it as a selectable shell named "Nushell" in the same shell-selection surfaces used for Bash, Zsh, Fish, and PowerShell.

4. If Nushell is the selected/default shell, opening a new local Warp session starts an interactive login Nushell session and automatically runs Warp's Nushell bootstrap without requiring the user to edit `config.nu` or `env.nu`. The user's normal Nushell startup files still run before Warp's bootstrap.

5. WSL default-shell detection treats a WSL default shell whose basename is `nu` as Nushell. Paths that merely contain `nu` in a parent directory or unrelated executable name do not count.

6. Once a Nushell session is bootstrapped, Warp receives the same core lifecycle information it depends on for other supported shells: session id, shell name, current working directory, command start, command finish, prompt boundary, clear-screen events, and input-buffer reporting.

7. Blocks, command boundaries, exit codes, and prompt redraws continue to work in Nushell sessions. Running a command that exits non-zero should mark the corresponding command result as failed without corrupting subsequent prompt/block state.

8. Warp discovers Nushell metadata that is useful to existing terminal features: shell version, home directory, PATH, history file when Nushell uses plaintext history, aliases, custom commands, built-ins, keywords, environment-variable names, editor, OS category, Linux distribution, WSL distro name, and shell path.

9. Warp reads Nushell history and configuration locations from Nushell's platform-derived runtime paths rather than assuming Unix-only defaults. This includes `$nu.history-path`, `$nu.env-path`, `$nu.config-path`, and `$nu.loginshell-path` when Nushell provides them, with conventional platform defaults used only as a fallback. Windows `nu.exe`, macOS, Linux, WSL, and MSYS2 local sessions should all use the paths reported by the running Nushell instance when available.

10. Warp's prompt-mode toggles work in Nushell: the user can switch between Warp-managed prompt integration and honoring the user's Nushell prompt where Warp exposes those controls. Existing Nushell prompt configuration, hooks, and keybindings should continue to run unless they directly conflict with Warp's integration.

11. Warp's environment-variable initialization and export features emit valid Nushell syntax. Constant values are assigned with `$env.NAME = ...`, command-backed values use Nushell command substitution, and variable names that are not valid bare Nushell identifiers are quoted safely.

12. Warp Drive environment-variable collection export/copy uses the active session's actual shell type. In a Nushell session it emits Nushell syntax, not Bash syntax inherited from the broader POSIX shell family.

13. Warp-generated or Warp-executed commands that need Nushell control-flow semantics do not assume POSIX `&&`. Success-gated flows, such as Linux package update completion, must not report success after an earlier package-manager command failed.

14. The existing Bash, Zsh, Fish, and PowerShell behaviors remain unchanged except for shared UI or metadata surfaces that now include Nushell as another supported shell.

15. When a feature is not yet supported for Nushell, Warp fails gracefully or uses the existing unsupported-shell path rather than silently running Bash/POSIX syntax in a Nushell session.

16. The minimum supported Nushell version is `0.109.0`. Older Nushell versions are best-effort and should fail through the normal shell startup/bootstrap failure path rather than falling back to Bash syntax.
