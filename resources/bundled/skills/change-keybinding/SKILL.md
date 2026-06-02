---
name: change-keybinding
description: Customize Warp keyboard shortcuts (keybindings, keymappings) by editing the user's keybindings.yaml file. Use when the user asks to remap a key combination, rebind an action, change a shortcut, or remove a default keybinding (e.g. "change ctrl+space to ctrl+s", "rebind the command palette to cmd+p", "remove the default for X").
---

# change-keybinding

Use this skill when the user wants to remap, rebind, or remove a Warp keyboard shortcut.

## Keybindings file

User customizations live at:

```
{{keybindings_file_path}}
```

This is the exact path Warp reads at launch — it is platform- and channel-specific (e.g. under `~/.warp*/` on macOS, under XDG config dirs like `~/.config/warp-terminal/` on Linux, and under `%LocalAppData%` on Windows). Use this path verbatim — do not infer a different one from the user's home directory layout. Create the file (and any missing parent directories) if it does not exist.

## File format

A flat YAML map of `action_name` → `key_trigger`. Action names contain a colon, so they **must be quoted**:

```yaml
"workspace:toggle_ai_assistant": ctrl-s
"editor_view:delete_all_left": cmd-shift-A
"workspace:toggle_command_palette": none
```

## Keystroke encoding rules

Triggers use Warp's normalized form — get this exactly right or the binding silently fails to load.

- **Modifiers** (in this order when combined): `ctrl-alt-shift-cmd-meta-`. Cross-platform alias: `cmdorctrl-` (becomes `cmd` on macOS, `ctrl` elsewhere).
- **Letter casing**: applies only to single-letter keys. Without `shift`, the letter is lowercase (`ctrl-s`). With `shift`, the letter is **uppercase** (`shift-A`, never `shift-a`). Mixing them is invalid.
- **Special keys**: `space`, `enter`, `escape`, `tab`, `backspace`, `delete`, `insert`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `f1`–`f20`, `numpadenter`. Always lowercase, even with `shift` (`ctrl-shift-space`, `shift-tab` — never `ctrl-shift-SPACE`). Use the literal word `space` — not `" "`.
- **Punctuation** is the bare character: `cmd-=`, `cmd-,`, `cmdorctrl-/`.
- **Remove a default binding**: set the value to the literal string `none`. The action becomes unbound.

Translate user phrasing into this form: `Ctrl+S` → `ctrl-s`, `Cmd+Shift+P` → `cmd-shift-P`, `Ctrl+Space` → `ctrl-space`.

## Identifying the action

Defaults are compiled into Warp and are **not** discoverable from the keybindings file on disk. There is no catalog the agent can consult to map a description or current shortcut to an action name. Pick the right strategy based on how the user described the change:

1. **By action name** ("set workspace:toggle_command_palette to cmd-p"): the user already gave you the name — write it directly.

2. **By description or current key combo** ("rebind the command palette to cmd-p", "change ctrl+space to ctrl+s"): you don't have the action name and cannot reliably guess it. Do not invent one. Direct the user to the **keybindings editor** (`workspace:show_keybinding_settings`, default `cmd-ctrl-k` on macOS; **Settings → Keyboard Shortcuts** on other platforms) — they can search by description or current shortcut there and either edit the binding in place or share the canonical `namespace:action_name` so you can write it.

## Workflow

1. Determine which action to remap and the new trigger (see "Identifying the action").
2. Read the existing keybindings file at `{{keybindings_file_path}}` if present. **Preserve every existing entry** — only add or update the one you're changing.
3. Write the file (creating parent directories if necessary). Make sure the action key is quoted and the value is normalized (see encoding rules).
4. Tell the user that **Warp must be restarted** for the change to take effect — the keybindings file is loaded only at app launch, unlike `settings.toml` which hot-reloads. They can quit with `cmd-Q` (macOS) or the equivalent on their platform and reopen Warp.

## Examples

Remap an existing custom binding by old trigger:

```yaml
# before
"workspace:toggle_ai_assistant": ctrl-space
# after
"workspace:toggle_ai_assistant": ctrl-s
```

Remove a default shortcut:

```yaml
"workspace:toggle_keybindings_page": none
```

Shift combined with a special key (note the special key stays lowercase):

```yaml
"workspace:toggle_ai_assistant": ctrl-shift-space
```

Cross-platform binding using the `cmdorctrl-` alias (resolves to `cmd` on macOS, `ctrl` elsewhere):

```yaml
"workspace:toggle_command_palette": cmdorctrl-shift-P
```
