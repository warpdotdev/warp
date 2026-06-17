---
name: warpctrl
description: Control and inspect the currently running local Warp application with the warpctrl CLI. Use this skill whenever the user asks the agent to manipulate Warp's own windows, tabs, panes, sessions, input buffer, themes, or UI surfaces; open a file in Warp; inspect local Warp state; or explain how to invoke Warp Control manually.
---

# Warp Control

Use `warpctrl` to inspect or control an already-running local Warp application. The built-in Warp agent can invoke it through shell commands, and users can run the same commands manually.

Prefer `warpctrl` when the requested action changes Warp itself rather than the user's project or operating system. Examples include creating a Warp tab, splitting a pane, staging text in Warp's input, opening Warp settings, or focusing a Warp window.

## Workflow

1. Discover running Warp instances:

   ```sh
   warpctrl instance list
   ```

2. If exactly one instance is running, commands select it automatically. If multiple instances are running, select one explicitly with `--instance <instance_id>` or `--pid <pid>`.

3. Inspect the active target chain or list the relevant targets before changing them:

   ```sh
   warpctrl app active
   warpctrl window list
   warpctrl tab list
   warpctrl pane list
   warpctrl session list
   ```

4. Discover the exact command and parameters instead of guessing:

   ```sh
   warpctrl help
   warpctrl <group> help
   warpctrl <group> <command> --help
   warpctrl action list
   warpctrl action inspect <action.name>
   ```

5. Invoke the narrowest action that satisfies the request, then verify the result with the corresponding `list`, `inspect`, or `get` command when useful.

## Common actions

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

Warp Control must be enabled in **Settings > Scripting**, and the installed build must include the Warp Control feature. The installed `warpctrl` wrapper invokes the matching channel-specific Warp executable.

If `warpctrl instance list` is empty, confirm that a compatible Warp app is running and Scripting is enabled. If a command reports multiple instances, rerun it with `--instance <instance_id>`. If `warpctrl` is not installed while developing Warp locally, invoke the hidden control mode directly:

```sh
cargo run -p warp --bin warp --features warp_control_cli -- --warpctrl instance list
```
