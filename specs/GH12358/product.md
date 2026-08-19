# Product Spec: Select WSL distributions in tab configs

**Issue:** [warpdotdev/warp#12358](https://github.com/warpdotdev/warp/issues/12358)

**Figma:** none provided

## Summary

On Windows, a terminal or agent pane in a tab config can select an installed WSL
distribution and open as a native Warp WSL session. This supports mixed layouts in
which different panes use different distributions or host shells, without asking
users to run `wsl.exe` as a startup command.

The existing string form of `shell` remains valid. WSL uses a new structured form
of that same field:

```toml
name = "Ubuntu development"

[[panes]]
id = "app"
type = "terminal"
shell = { type = "wsl", distribution = "Ubuntu-24.04" }
directory = "~/code/app"
commands = ["cargo run"]
```

## Problem

Tab configs can currently select a host shell executable such as `pwsh`, `zsh`, or
`bash`, but they cannot identify a WSL distribution. Using `wsl.exe` as a command
does not create the same session as Warp's built-in WSL launcher and can run later
startup commands before the guest shell is ready. This is especially limiting for
multi-pane Windows layouts that span host shells and one or more WSL distributions.

## Goals

- Let each terminal or agent pane name one installed WSL distribution.
- Use Warp's native WSL session and bootstrap behavior.
- Give WSL startup directories deterministic guest-path semantics.
- Fail clearly instead of silently opening a host default shell.
- Preserve all existing `shell = "..."` tab configs.

## Non-goals

- Selecting a non-default shell inside a WSL distribution. The selected
  distribution's supported default shell remains authoritative.
- Adding WSL selection to cloud panes, split nodes, or the separate launch-config
  file format.
- Installing, importing, starting, stopping, renaming, or setting the default WSL
  distribution from Warp.
- Adding structured selectors for SSH, containers, MSYS2, or other environments.
- Accepting Windows paths as aliases for WSL pane directories.

## Behavior invariants

1. A `terminal` or `agent` leaf may select WSL with
   `shell = { type = "wsl", distribution = "<name>" }`. `type` and `distribution`
   are required, no additional keys are accepted, and WSL is the only structured
   `shell` type introduced by this feature.

2. Existing string values such as `shell = "pwsh"` retain their current parsing,
   shell discovery, fallback, rendering, and startup behavior. Omitting `shell`
   also retains the current user-preferred-shell behavior. The new table is an
   extension of `shell`, not a second `environment` field.

3. The WSL table is valid only on `terminal` and `agent` leaves. It is a structural
   config error on a `cloud` leaf or a split node. A missing, empty, all-whitespace,
   or leading/trailing-whitespace `distribution`, an unknown `type`, or an extra
   table key is also a structural config error.

4. `distribution` is a literal WSL identity, not a Handlebars parameter. It must
   exactly match the case-sensitive distribution name discovered by Warp (for
   example, `Ubuntu-24.04` does not match `ubuntu-24.04`). Warp does not trim,
   case-fold, match aliases, or use a distribution UUID.

5. The structured WSL form never selects the Windows default WSL distribution
   implicitly: `distribution` cannot be omitted. A pane with no `shell` field may
   still open WSL when WSL is already the user's Warp default, which is unchanged
   legacy behavior.

6. On Windows, opening a valid WSL pane creates the same native Warp WSL session as
   selecting that distribution from Warp's shell selector. Warp launches the
   distribution's supported default shell, performs normal Warp bootstrap, and
   does not enter `wsl.exe` as a visible command in a host-shell pane.

7. An installed but stopped distribution is available. Opening the tab may start
   that distribution as part of normal WSL initialization. If the exact configured
   name is not in Warp's discovered installed-distribution list, Warp shows an error
   naming the configured distribution and creates no part of the tab; it never
   substitutes a host shell or another WSL distribution.

8. On a non-Windows build, the new table remains structurally parseable so a shared
   config file is not corrupted or rewritten. Attempting to open it shows a clear
   "WSL tab config panes are only available on Windows" error and creates no part
   of the tab. It never falls back to the platform's default shell.

9. For a WSL pane, `directory` is a WSL/Linux path after parameter substitution.
   An absolute POSIX path such as `/home/me/code` addresses the selected
   distribution; `/mnt/c/code` addresses a mounted Windows drive. `~` and
   `~/...` resolve against that distribution user's `$HOME`, not the Windows host
   user's home. Relative paths other than `~`-prefixed paths, Windows drive paths
   such as `C:\code`, UNC paths, and `~otheruser` are rejected with a directory
   error that explains the accepted WSL syntax.

10. Omitting `directory` starts the WSL pane in the selected distribution user's
    home directory. Before creating any pane, Warp resolves each configured WSL
    directory to the Windows-host path required by the PTY and verifies that it is
    a directory. If resolution fails or the directory does not exist, Warp shows a
    clear error containing the pane/config context and creates no part of the tab.
    Warp never runs configured commands in a fallback directory.

11. A WSL pane's `commands` are queued in their declared order and the first command
    is submitted only after that pane completes Warp shell bootstrap and its first
    post-bootstrap prompt is ready. Each later command starts after the previous
    command succeeds; a failed command stops the remaining queue, matching existing
    tab-config command behavior.

12. An `agent` pane with WSL first completes WSL bootstrap and all configured setup
    commands, then enters Agent Mode. A `terminal` pane remains in terminal mode.
    If WSL initialization or bootstrap fails, no configured command runs and an
    agent pane does not enter Agent Mode.

13. A multi-pane tab may mix host-shell panes, WSL panes using the same or different
    distributions, and cloud panes. Each WSL leaf gets an independent native WSL
    session and uses its own distribution and directory. Warp validates every WSL
    selection and directory before creating the tab, so one preflight error cannot
    leave a partial layout behind.

14. If a distribution disappears or WSL fails after successful preflight but before
    a pane's process starts, that affected pane shows the normal shell-start failure
    state and runs no commands. Other panes that have already started remain open.
    Even in this race, the failed WSL pane must not fall back to a host shell or a
    different distribution.

15. Structural WSL schema errors follow existing tab-config reload behavior: after
    a save, the invalid file is excluded from the menu and its persistent load-error
    toast links to the file; fixing and saving it dismisses the stale toast and
    restores the config. Platform, installed-distribution, and directory checks run
    when the user opens the config because those conditions may change at runtime.

16. Reloading a valid edited config affects only future opens. Existing panes are
    not restarted, moved to another distribution, re-rooted to another directory,
    or sent newly edited commands.

17. User-visible errors may display locally configured distribution names to help
    the user correct the file. Telemetry continues to identify the launch only as
    WSL and does not record a distribution name or configured directory.

## Example: mixed Windows and WSL layout

```toml
name = "Windows + two distros"

[[panes]]
id = "root"
split = "horizontal"
children = ["windows", "ubuntu", "debian_agent"]

[[panes]]
id = "windows"
type = "terminal"
shell = "pwsh"
directory = "C:\\code\\app"

[[panes]]
id = "ubuntu"
type = "terminal"
shell = { type = "wsl", distribution = "Ubuntu-24.04" }
directory = "/mnt/c/code/app"
commands = ["npm test"]

[[panes]]
id = "debian_agent"
type = "agent"
shell = { type = "wsl", distribution = "Debian" }
directory = "~/code/app"
commands = ["mise install"]
is_focused = true
```

## Open questions

None. Additional structured shell types and a default-distribution shorthand require
separate product decisions.
