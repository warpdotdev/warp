# Warp Control CLI Verification: Settings, Appearance, Surfaces

- Repository: `warpdotdev/warp`
- Branch: `origin/zach/warp-cli-integration-fanin`
- Commit: `492fde34b397ec9ab89f1dee2f11087b03ce3b2b`
- Checkout: `/workspace/warp`

## Build and Launch
- Passed: `cargo build -p warp --bin warpctrl --features warp/warp_control_cli`
- Passed: `cargo build -p warp --bin warp --features warp/warp_control_cli`
- Passed for GUI launch on Linux: `cargo build -p warp --bin warp --features warp/warp_control_cli,warp/gui`
- Launch: Xvfb display `:99` with Fluxbox, then `/workspace/warp/target/debug/warp` from this checkout.

## Permission Setup
- Verified default-off: `appearance get` returned `local_control_disabled` with outside-Warp control OFF.
- Enabled in Settings > Scripting: outside-Warp control, metadata reads, app-state mutations, metadata/configuration mutations.
- Left OFF: underlying data reads and underlying data mutations.

## Findings
- Default-off outside-Warp behavior works: app is discoverable but brokered commands return local_control_disabled until the top-level outside-Warp setting is enabled.
- Read metadata commands passed: theme list, appearance get, setting get.
- Metadata/configuration mutations passed and were restored: theme set Light then setting set appearance.themes.theme Dark; terminal.input.syntax_highlighting toggled false then true.
- Surface commands passed with explicit --window-id 0 targeting and returned handled:true acknowledgements for settings, command palette, command search, Warp Drive open/toggle, resource center, AI assistant, code review, left panel, right panel, and vertical tabs.
- Review blocker: `setting list` fails with invalid_params because the CLI sends {"namespace": null}, while app/src/local_control/resolver.rs validates setting.list with validate_empty_action_params and rejects any non-empty object.
- Usability note: surface commands without an explicit target can fail with missing_target if another X11 window has focus; explicit --window-id works.

## Command Manifest
### 00-default-off-appearance-get
- Command: `./target/debug/warpctrl --output-format json appearance get`
- Context: outside Warp, top-level outside-Warp control OFF
- Required permission: `read_metadata`
- Expected: Denied with local_control_disabled because outside-Warp control is default-off
- Actual: failed: local_control_disabled: outside-Warp local control credential broker is disabled for this instance
- Terminal screenshot: `screenshots/terminal-00-default-off-appearance-get.png`
- UI screenshot: `screenshots/ui-settings-scripting-default-off.png`
- Raw output: `outputs/00-default-off-appearance-get.txt`

### 01-theme-list
- Command: `./target/debug/warpctrl --output-format ndjson theme list`
- Context: outside Warp, required Scripting permissions enabled
- Required permission: `read_metadata`
- Expected: JSON theme list result with available themes and current theme marker
- Actual: passed
- Terminal screenshot: `screenshots/terminal-01-theme-list.png`
- Raw output: `outputs/01-theme-list.txt`

### 02-appearance-get-before
- Command: `./target/debug/warpctrl --output-format ndjson appearance get`
- Context: outside Warp, required Scripting permissions enabled
- Required permission: `read_metadata`
- Expected: JSON appearance state result
- Actual: passed
- Terminal screenshot: `screenshots/terminal-02-appearance-get-before.png`
- Raw output: `outputs/02-appearance-get-before.txt`

### 03-theme-set-light
- Command: `./target/debug/warpctrl --output-format ndjson theme set Light`
- Context: outside Warp, required Scripting permissions enabled
- Required permission: `mutate_metadata_configuration`
- Expected: Acknowledgement JSON with changed flag and visible switch to light theme
- Actual: passed
- Terminal screenshot: `screenshots/terminal-03-theme-set-light.png`
- UI screenshot: `screenshots/ui-before-03-theme-set-light.png, screenshots/ui-after-03-theme-set-light.png`
- Raw output: `outputs/03-theme-set-light.txt`

### 04-setting-list
- Command: `./target/debug/warpctrl --output-format ndjson setting list`
- Context: outside Warp, required Scripting permissions enabled
- Required permission: `read_metadata`
- Expected: JSON allowlisted settings list
- Actual: failed: invalid_params: setting.list does not accept parameters; confirmed bug: CLI sends namespace:null params but app resolver validates setting.list as empty-params-only
- Terminal screenshot: `screenshots/terminal-04-setting-list.png`
- Raw output: `outputs/04-setting-list.txt`

### 05-setting-get-theme
- Command: `./target/debug/warpctrl --output-format ndjson setting get appearance.themes.theme`
- Context: outside Warp, required Scripting permissions enabled
- Required permission: `read_metadata`
- Expected: JSON setting value for appearance.themes.theme
- Actual: passed
- Terminal screenshot: `screenshots/terminal-05-setting-get-theme.png`
- Raw output: `outputs/05-setting-get-theme.txt`

### 06-setting-set-theme-dark
- Command: `./target/debug/warpctrl --output-format ndjson setting set appearance.themes.theme Dark`
- Context: outside Warp, required Scripting permissions enabled
- Required permission: `mutate_metadata_configuration`
- Expected: JSON setting mutation result and visible switch back to dark theme
- Actual: passed
- Terminal screenshot: `screenshots/terminal-06-setting-set-theme-dark.png`
- UI screenshot: `screenshots/ui-before-06-setting-set-theme-dark.png, screenshots/ui-after-06-setting-set-theme-dark.png`
- Raw output: `outputs/06-setting-set-theme-dark.txt`

### 07-setting-get-syntax-before
- Command: `./target/debug/warpctrl --output-format ndjson setting get terminal.input.syntax_highlighting`
- Context: outside Warp, required Scripting permissions enabled
- Required permission: `read_metadata`
- Expected: JSON setting value for terminal.input.syntax_highlighting before toggle
- Actual: passed
- Terminal screenshot: `screenshots/terminal-07-setting-get-syntax-before.png`
- Raw output: `outputs/07-setting-get-syntax-before.txt`

### 08-setting-toggle-syntax-off
- Command: `./target/debug/warpctrl --output-format ndjson setting toggle terminal.input.syntax_highlighting`
- Context: outside Warp, required Scripting permissions enabled
- Required permission: `mutate_metadata_configuration`
- Expected: JSON setting mutation result toggling syntax_highlighting to false
- Actual: passed
- Terminal screenshot: `screenshots/terminal-08-setting-toggle-syntax-off.png`
- Raw output: `outputs/08-setting-toggle-syntax-off.txt`

### 09-setting-toggle-syntax-restore
- Command: `./target/debug/warpctrl --output-format ndjson setting toggle terminal.input.syntax_highlighting`
- Context: outside Warp, required Scripting permissions enabled
- Required permission: `mutate_metadata_configuration`
- Expected: JSON setting mutation result restoring syntax_highlighting to true
- Actual: passed
- Terminal screenshot: `screenshots/terminal-09-setting-toggle-syntax-restore.png`
- Raw output: `outputs/09-setting-toggle-syntax-restore.txt`

### 10b-surface-settings-open-window-id
- Command: `./target/debug/warpctrl --output-format ndjson surface settings open --page Scripting --window-id 0`
- Context: outside Warp, required Scripting permissions enabled, explicit --window-id 0 target
- Required permission: `mutate_app_state`
- Expected: Settings/Scripting surface opened with explicit window target
- Actual: passed
- Terminal screenshot: `screenshots/terminal-10b-surface-settings-open-window-id.png`
- UI screenshot: `screenshots/ui-before-10b-surface-settings-open-window-id.png, screenshots/ui-after-10b-surface-settings-open-window-id.png`
- Raw output: `outputs/10b-surface-settings-open-window-id.txt`

### 11b-surface-command-palette-open-window-id
- Command: `./target/debug/warpctrl --output-format ndjson surface command-palette open --query theme --window-id 0`
- Context: outside Warp, required Scripting permissions enabled, explicit --window-id 0 target
- Required permission: `mutate_app_state`
- Expected: Command palette opened with query using explicit window target
- Actual: passed
- Terminal screenshot: `screenshots/terminal-11b-surface-command-palette-open-window-id.png`
- UI screenshot: `screenshots/ui-before-11b-surface-command-palette-open-window-id.png, screenshots/ui-after-11b-surface-command-palette-open-window-id.png`
- Raw output: `outputs/11b-surface-command-palette-open-window-id.txt`

### 12b-surface-command-search-open-window-id
- Command: `./target/debug/warpctrl --output-format ndjson surface command-search open --query ls --window-id 0`
- Context: outside Warp, required Scripting permissions enabled, explicit --window-id 0 target
- Required permission: `mutate_app_state`
- Expected: Command search opened with query using explicit window target
- Actual: passed
- Terminal screenshot: `screenshots/terminal-12b-surface-command-search-open-window-id.png`
- UI screenshot: `screenshots/ui-before-12b-surface-command-search-open-window-id.png, screenshots/ui-after-12b-surface-command-search-open-window-id.png`
- Raw output: `outputs/12b-surface-command-search-open-window-id.txt`

### 13b-surface-warp-drive-open-window-id
- Command: `./target/debug/warpctrl --output-format ndjson surface warp-drive open --window-id 0`
- Context: outside Warp, required Scripting permissions enabled, explicit --window-id 0 target
- Required permission: `mutate_app_state`
- Expected: Warp Drive surface opened using explicit window target
- Actual: passed
- Terminal screenshot: `screenshots/terminal-13b-surface-warp-drive-open-window-id.png`
- UI screenshot: `screenshots/ui-before-13b-surface-warp-drive-open-window-id.png, screenshots/ui-after-13b-surface-warp-drive-open-window-id.png`
- Raw output: `outputs/13b-surface-warp-drive-open-window-id.txt`

### 14b-surface-warp-drive-toggle-window-id
- Command: `./target/debug/warpctrl --output-format ndjson surface warp-drive toggle --window-id 0`
- Context: outside Warp, required Scripting permissions enabled, explicit --window-id 0 target
- Required permission: `mutate_app_state`
- Expected: Warp Drive surface toggled using explicit window target
- Actual: passed
- Terminal screenshot: `screenshots/terminal-14b-surface-warp-drive-toggle-window-id.png`
- UI screenshot: `screenshots/ui-before-14b-surface-warp-drive-toggle-window-id.png, screenshots/ui-after-14b-surface-warp-drive-toggle-window-id.png`
- Raw output: `outputs/14b-surface-warp-drive-toggle-window-id.txt`

### 15b-surface-resource-center-toggle-window-id
- Command: `./target/debug/warpctrl --output-format ndjson surface resource-center toggle --window-id 0`
- Context: outside Warp, required Scripting permissions enabled, explicit --window-id 0 target
- Required permission: `mutate_app_state`
- Expected: Resource center toggled using explicit window target
- Actual: passed
- Terminal screenshot: `screenshots/terminal-15b-surface-resource-center-toggle-window-id.png`
- UI screenshot: `screenshots/ui-before-15b-surface-resource-center-toggle-window-id.png, screenshots/ui-after-15b-surface-resource-center-toggle-window-id.png`
- Raw output: `outputs/15b-surface-resource-center-toggle-window-id.txt`

### 16b-surface-ai-assistant-toggle-window-id
- Command: `./target/debug/warpctrl --output-format ndjson surface ai-assistant toggle --window-id 0`
- Context: outside Warp, required Scripting permissions enabled, explicit --window-id 0 target
- Required permission: `mutate_app_state`
- Expected: AI assistant toggled using explicit window target
- Actual: passed
- Terminal screenshot: `screenshots/terminal-16b-surface-ai-assistant-toggle-window-id.png`
- UI screenshot: `screenshots/ui-before-16b-surface-ai-assistant-toggle-window-id.png, screenshots/ui-after-16b-surface-ai-assistant-toggle-window-id.png`
- Raw output: `outputs/16b-surface-ai-assistant-toggle-window-id.txt`

### 17b-surface-code-review-toggle-window-id
- Command: `./target/debug/warpctrl --output-format ndjson surface code-review toggle --window-id 0`
- Context: outside Warp, required Scripting permissions enabled, explicit --window-id 0 target
- Required permission: `mutate_app_state`
- Expected: Code review/right panel toggled using explicit window target
- Actual: passed
- Terminal screenshot: `screenshots/terminal-17b-surface-code-review-toggle-window-id.png`
- UI screenshot: `screenshots/ui-before-17b-surface-code-review-toggle-window-id.png, screenshots/ui-after-17b-surface-code-review-toggle-window-id.png`
- Raw output: `outputs/17b-surface-code-review-toggle-window-id.txt`

### 18b-surface-left-panel-toggle-window-id
- Command: `./target/debug/warpctrl --output-format ndjson surface left-panel toggle --window-id 0`
- Context: outside Warp, required Scripting permissions enabled, explicit --window-id 0 target
- Required permission: `mutate_app_state`
- Expected: Left panel toggled using explicit window target
- Actual: passed
- Terminal screenshot: `screenshots/terminal-18b-surface-left-panel-toggle-window-id.png`
- UI screenshot: `screenshots/ui-before-18b-surface-left-panel-toggle-window-id.png, screenshots/ui-after-18b-surface-left-panel-toggle-window-id.png`
- Raw output: `outputs/18b-surface-left-panel-toggle-window-id.txt`

### 19b-surface-right-panel-toggle-window-id
- Command: `./target/debug/warpctrl --output-format ndjson surface right-panel toggle --window-id 0`
- Context: outside Warp, required Scripting permissions enabled, explicit --window-id 0 target
- Required permission: `mutate_app_state`
- Expected: Right panel toggled using explicit window target
- Actual: passed
- Terminal screenshot: `screenshots/terminal-19b-surface-right-panel-toggle-window-id.png`
- UI screenshot: `screenshots/ui-before-19b-surface-right-panel-toggle-window-id.png, screenshots/ui-after-19b-surface-right-panel-toggle-window-id.png`
- Raw output: `outputs/19b-surface-right-panel-toggle-window-id.txt`

### 20b-surface-vertical-tabs-toggle-window-id
- Command: `./target/debug/warpctrl --output-format ndjson surface vertical-tabs toggle --window-id 0`
- Context: outside Warp, required Scripting permissions enabled, explicit --window-id 0 target
- Required permission: `mutate_app_state`
- Expected: Vertical tabs panel toggled using explicit window target
- Actual: passed
- Terminal screenshot: `screenshots/terminal-20b-surface-vertical-tabs-toggle-window-id.png`
- UI screenshot: `screenshots/ui-before-20b-surface-vertical-tabs-toggle-window-id.png, screenshots/ui-after-20b-surface-vertical-tabs-toggle-window-id.png`
- Raw output: `outputs/20b-surface-vertical-tabs-toggle-window-id.txt`

### 10-surface-settings-open
- Command: `./target/debug/warpctrl --output-format ndjson surface settings open --page Scripting`
- Context: outside Warp, required Scripting permissions enabled, no explicit target while xterm focused
- Required permission: `mutate_app_state`
- Expected: Acknowledgement JSON and Settings/Scripting surface visible
- Actual: failed: missing_target: surface.settings.open requires an active Warp window; diagnostic focus artifact: xterm stole active window; explicit --window-id retry passed
- Terminal screenshot: `screenshots/terminal-10-surface-settings-open.png`
- Raw output: `outputs/10-surface-settings-open.txt`

### 11-surface-command-palette-open
- Command: `./target/debug/warpctrl --output-format ndjson surface command-palette open --query theme`
- Context: outside Warp, required Scripting permissions enabled, no explicit target while xterm focused
- Required permission: `mutate_app_state`
- Expected: Acknowledgement JSON and command palette visible with query
- Actual: failed: missing_target: surface.command_palette.open requires an active Warp window; diagnostic focus artifact: xterm stole active window; explicit --window-id retry passed
- Terminal screenshot: `screenshots/terminal-11-surface-command-palette-open.png`
- Raw output: `outputs/11-surface-command-palette-open.txt`

### 12-surface-command-search-open
- Command: `./target/debug/warpctrl --output-format ndjson surface command-search open --query ls`
- Context: outside Warp, required Scripting permissions enabled, no explicit target while xterm focused
- Required permission: `mutate_app_state`
- Expected: Acknowledgement JSON and command search visible with query
- Actual: failed: missing_target: surface.command_search.open requires an active Warp window; diagnostic focus artifact: xterm stole active window; explicit --window-id retry passed
- Terminal screenshot: `screenshots/terminal-12-surface-command-search-open.png`
- Raw output: `outputs/12-surface-command-search-open.txt`

### 13-surface-warp-drive-open
- Command: `./target/debug/warpctrl --output-format ndjson surface warp-drive open`
- Context: outside Warp, required Scripting permissions enabled, no explicit target while xterm focused
- Required permission: `mutate_app_state`
- Expected: Acknowledgement JSON and Warp Drive surface visible or login-gated surface opened
- Actual: failed: missing_target: surface.warp_drive.open requires an active Warp window; diagnostic focus artifact: xterm stole active window; explicit --window-id retry passed
- Terminal screenshot: `screenshots/terminal-13-surface-warp-drive-open.png`
- Raw output: `outputs/13-surface-warp-drive-open.txt`

### 14-surface-warp-drive-toggle
- Command: `./target/debug/warpctrl --output-format ndjson surface warp-drive toggle`
- Context: outside Warp, required Scripting permissions enabled, no explicit target while xterm focused
- Required permission: `mutate_app_state`
- Expected: Acknowledgement JSON and Warp Drive surface toggled
- Actual: failed: missing_target: surface.warp_drive.toggle requires an active Warp window; diagnostic focus artifact: xterm stole active window; explicit --window-id retry passed
- Terminal screenshot: `screenshots/terminal-14-surface-warp-drive-toggle.png`
- Raw output: `outputs/14-surface-warp-drive-toggle.txt`

### 15-surface-resource-center-toggle
- Command: `./target/debug/warpctrl --output-format ndjson surface resource-center toggle`
- Context: outside Warp, required Scripting permissions enabled, no explicit target while xterm focused
- Required permission: `mutate_app_state`
- Expected: Acknowledgement JSON and resource center toggled
- Actual: failed: missing_target: surface.resource_center.toggle requires an active Warp window; diagnostic focus artifact: xterm stole active window; explicit --window-id retry passed
- Terminal screenshot: `screenshots/terminal-15-surface-resource-center-toggle.png`
- Raw output: `outputs/15-surface-resource-center-toggle.txt`

### 16-surface-ai-assistant-toggle
- Command: `./target/debug/warpctrl --output-format ndjson surface ai-assistant toggle`
- Context: outside Warp, required Scripting permissions enabled, no explicit target while xterm focused
- Required permission: `mutate_app_state`
- Expected: Acknowledgement JSON and AI assistant panel toggled
- Actual: failed: missing_target: surface.ai_assistant.toggle requires an active Warp window; diagnostic focus artifact: xterm stole active window; explicit --window-id retry passed
- Terminal screenshot: `screenshots/terminal-16-surface-ai-assistant-toggle.png`
- Raw output: `outputs/16-surface-ai-assistant-toggle.txt`

### 17-surface-code-review-toggle
- Command: `./target/debug/warpctrl --output-format ndjson surface code-review toggle`
- Context: outside Warp, required Scripting permissions enabled, no explicit target while xterm focused
- Required permission: `mutate_app_state`
- Expected: Acknowledgement JSON and right/code-review panel toggled
- Actual: failed: missing_target: surface.code_review.toggle requires an active Warp window; diagnostic focus artifact: xterm stole active window; explicit --window-id retry passed
- Terminal screenshot: `screenshots/terminal-17-surface-code-review-toggle.png`
- Raw output: `outputs/17-surface-code-review-toggle.txt`

### 18-surface-left-panel-toggle
- Command: `./target/debug/warpctrl --output-format ndjson surface left-panel toggle`
- Context: outside Warp, required Scripting permissions enabled, no explicit target while xterm focused
- Required permission: `mutate_app_state`
- Expected: Acknowledgement JSON and left panel toggled
- Actual: failed: missing_target: surface.left_panel.toggle requires an active Warp window; diagnostic focus artifact: xterm stole active window; explicit --window-id retry passed
- Terminal screenshot: `screenshots/terminal-18-surface-left-panel-toggle.png`
- Raw output: `outputs/18-surface-left-panel-toggle.txt`

### 19-surface-right-panel-toggle
- Command: `./target/debug/warpctrl --output-format ndjson surface right-panel toggle`
- Context: outside Warp, required Scripting permissions enabled, no explicit target while xterm focused
- Required permission: `mutate_app_state`
- Expected: Acknowledgement JSON and right panel toggled
- Actual: failed: missing_target: surface.right_panel.toggle requires an active Warp window; diagnostic focus artifact: xterm stole active window; explicit --window-id retry passed
- Terminal screenshot: `screenshots/terminal-19-surface-right-panel-toggle.png`
- Raw output: `outputs/19-surface-right-panel-toggle.txt`

### 20-surface-vertical-tabs-toggle
- Command: `./target/debug/warpctrl --output-format ndjson surface vertical-tabs toggle`
- Context: outside Warp, required Scripting permissions enabled, no explicit target while xterm focused
- Required permission: `mutate_app_state`
- Expected: Acknowledgement JSON and vertical tabs panel toggled
- Actual: failed: missing_target: surface.vertical_tabs.toggle requires an active Warp window; diagnostic focus artifact: xterm stole active window; explicit --window-id retry passed
- Terminal screenshot: `screenshots/terminal-20-surface-vertical-tabs-toggle.png`
- Raw output: `outputs/20-surface-vertical-tabs-toggle.txt`

## Unsupported / Not Run
- `theme.get`: Public CLI subcommand exists but source maps it to unsupported_action("theme.get") in crates/warp_cli/src/local_control/commands.rs.
- `appearance font-size/zoom mutations`: Public CLI subcommands exist but source maps font-size and zoom commands to unsupported_action in this shard.

## Restoration
- Theme restored to Dark via setting set appearance.themes.theme Dark.
- terminal.input.syntax_highlighting restored to true by toggling it back.
- Outside-Warp Scripting permissions were left enabled for verification rather than disabled afterward, so reviewers can rerun commands against the live app state in this sandbox.