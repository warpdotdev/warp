---
name: warpctrl
description: Control and inspect the currently running local Warp application with the warpctrl CLI. Use this skill whenever the user asks the agent to manipulate Warp's own windows, tabs, panes, sessions, input buffer, themes, or UI surfaces; open a file in Warp; inspect local Warp state; or explain how to invoke Warp Control manually.
---

# Warp Control

Use `warpctrl` to inspect or control an already-running local Warp application. The built-in Warp agent can invoke it through shell commands, and users can run the same commands manually.

Prefer `warpctrl` when the requested action changes Warp itself rather than the user's project or operating system. Examples include creating a Warp tab, splitting a pane, staging text in Warp's input, opening Warp settings, or focusing a Warp window.

## How to invoke Warp Control

Warp Control is bundled into the Warp application. It is not a separate standalone binary; it is a hidden control mode that is served by the running Warp process. A small wrapper script is installed into the app bundle at `Contents/Resources/bin/warpctrl`, and the Warp UI can install a symlink for that wrapper to `/usr/local/bin` so it is available on `PATH`.

When invoking Warp Control, use the wrapper name that matches the currently running Warp channel:

- **Warp (Stable):** `warpctrl`
- **WarpDev:** `warpctrl-dev`
- **WarpPreview:** `warpctrl-preview`
- **WarpLocal:** `warpctrl-local`
- **WarpOss:** `warpctrl-oss`

If the wrapper is not on `PATH`, invoke it from the app bundle by running the matching wrapper path:

- **Warp (Stable):** `/Applications/Warp.app/Contents/Resources/bin/warpctrl`
- **WarpDev:** `/Applications/WarpDev.app/Contents/Resources/bin/warpctrl-dev`
- **WarpPreview:** `/Applications/WarpPreview.app/Contents/Resources/bin/warpctrl-preview`
- **WarpOss:** `/Applications/WarpOss.app/Contents/Resources/bin/warpctrl-oss`
- **WarpLocal:** `<bundle-path>/WarpLocal.app/Contents/Resources/bin/warpctrl-local`

If the wrapper is missing from the bundle (e.g., older builds), invoke the channel executable directly with the `--warpctrl` flag:

- **WarpDev:** `/Applications/WarpDev.app/Contents/MacOS/dev --warpctrl ...`
- **WarpPreview:** `/Applications/WarpPreview.app/Contents/MacOS/preview --warpctrl ...`
- **Warp (Stable):** `/Applications/Warp.app/Contents/MacOS/stable --warpctrl ...`
- **WarpOss:** `/Applications/WarpOss.app/Contents/MacOS/warp-oss --warpctrl ...`
- **WarpLocal:** `<bundle-path>/WarpLocal.app/Contents/MacOS/warp --warpctrl ...`

To install the wrapper on `PATH` from the Warp UI, use the command palette entry **Install Warp Control CLI command** (or `workspace:install_warpctrl`). To uninstall it, use **Uninstall Warp Control CLI command** (or `workspace:uninstall_warpctrl`).

For a local checkout, invoke the hidden control mode with `cargo run` using the channel-specific binary target. **Do not run a development build automatically. If the user wants to use the local checkout, ask which channel they want to build before running `cargo run`.**

> Ask the user which channel to build before running a development command.
> Example: "Do you want to run the dev, preview, or stable channel build?"
> For the dev channel, the command would be:
> ```sh
> cargo run -p warp --bin dev --features warp_control_cli -- --warpctrl instance list
> ```

The easiest way to verify a working invocation is to run the wrapper that matches the running Warp build, e.g.:

```sh
warpctrl-dev app version
```

which should print the channel, app ID, and protocol version.

## Workflow

Always prefer discovering commands from `warpctrl` itself rather than guessing or inventing them. The CLI provides full help and an action catalog that is the source of truth for what the installed build supports.

1. Discover running Warp instances:

   ```sh
   warpctrl instance list
   ```

2. If exactly one instance is running, commands select it automatically. If multiple instances are running, select one explicitly with `--instance <instance_id>` or `--pid <pid>`.

3. Discover the exact command and parameters instead of guessing. This is the preferred source of truth for the available command surface:

   ```sh
   warpctrl help
   warpctrl <group> help
   warpctrl <group> <command> --help
   warpctrl action list
   warpctrl action inspect <action.name>
   ```

4. Inspect the active target chain or list the relevant targets before changing them:

   ```sh
   warpctrl app active
   warpctrl window list
   warpctrl tab list
   warpctrl pane list
   warpctrl session list
   ```

5. Invoke the narrowest action that satisfies the request, then verify the result with the corresponding `list`, `inspect`, or `get` command when useful.

## Common actions

These are frequently used commands that are safe to invoke directly. For less common commands, use `warpctrl help`, `warpctrl action list`, or `warpctrl <group> <command> --help` to discover the exact syntax supported by the running build.

```sh
# Create and manage tabs and panes
warpctrl tab create
warpctrl tab create --type agent
warpctrl tab rename "server logs"
warpctrl pane split --direction right
warpctrl pane navigate --direction next

# Stage text in Warp's input without submitting it
warpctrl input insert "git status"
warpctrl input replace "cargo test"

# Open Warp UI surfaces
warpctrl surface list
warpctrl surface settings open
warpctrl surface command-palette open --query "theme"
warpctrl surface theme-picker open
warpctrl surface keybindings open
warpctrl surface project-explorer open
warpctrl surface global-search open
warpctrl surface conversation-list open
warpctrl surface code-review open
warpctrl surface vertical-tabs open
warpctrl surface agent-management open

# Open a file in Warp
warpctrl file open ./src/main.rs --line 42

# Inspect and update supported state
warpctrl theme get
warpctrl theme set "Dracula"
warpctrl appearance get
warpctrl setting list
warpctrl keybinding list
```

Add `--output-format json` when structured output is easier to consume:

```sh
warpctrl --output-format json tab list
```

## Targeting

Target selectors can be combined when the action supports their scope:

- Instance: `--instance <instance_id>` or `--pid <pid>`
- Window: `--window <id>`, `--window-index <n>`, or `--window-title <exact-title>`
- Tab: `--tab <id>`, `--tab-index <n>`, or `--tab-title <exact-title>`
- Pane: `--pane <id>` or `--pane-index <n>`
- Session: `--session <id>`

Use IDs returned by `list`, `inspect`, or `app active` when exact targeting matters. If a selector is omitted, most scoped actions operate on the active target. Prefer explicit selectors when more than one target could reasonably match the user's request.

Use `surface list` before a walkthrough or multi-step UI workflow. It reports both available and unavailable destinations with stable names and reasons. The direct `surface ... open` commands are idempotent; use them instead of toggle commands when the final open state matters. `surface list` accepts `--instance` or `--pid` for process selection but rejects window, tab, pane, and session selectors.

## Safety and limitations

- Invoke close actions only when the user explicitly asks to close something. Close actions flow through normal Warp close behavior and may trigger existing app warnings.
- `input insert` and `input replace` only stage text. Warp Control intentionally does not provide an action that submits or runs the input.
- Do not invent unsupported commands. Use `help`, `action list`, or `action inspect` to discover the allowlisted surface.
- Warp Control affects only a running local Warp application owned by the same user. It does not control remote or cloud Warp instances.
- On Windows, local-control publication is disabled until authenticated broker transport is supported.

## Manual setup and troubleshooting

Warp Control availability depends on the build channel and the **Settings > Scripting** toggle. The local-control mode defaults to enabled on internal dogfood builds (e.g., WarpDev) and disabled on public channels (Stable, Preview, OSS). On any channel, the final gate is the **Settings > Scripting** toggle. The installed `warpctrl` wrapper invokes the matching channel-specific Warp executable.

If `warpctrl instance list` is empty, confirm that a compatible Warp app is running and Scripting is enabled. If a command reports multiple instances, rerun it with `--instance <instance_id>`.

If the wrapper is not present in the bundle (e.g., older builds), invoke the channel executable directly with the `--warpctrl` flag:

```sh
/Applications/WarpDev.app/Contents/MacOS/dev --warpctrl instance list
```

If the wrapper is present but not on `PATH`, use the full bundle path to the channel-specific wrapper:

```sh
/Applications/WarpDev.app/Contents/Resources/bin/warpctrl-dev instance list
```
