# APP-4060: Allow selection of specific CLI agent for custom toolbar commands

## Summary

Allow users to specify which CLI agent (Claude Code, Gemini, Codex, etc.) a custom toolbar command maps to. Today, user-configured regex patterns in "Commands that enable the toolbar" always produce a generic `Unknown` agent session, which means they lack agent-specific features like the correct icon, rich input submit strategy, plugin support, and brand styling. This change adds a per-pattern agent selector dropdown to the settings UI so that custom commands can receive the full agent experience.

## Problem

Users at companies like Uber wrap CLI agents behind custom scripts (e.g. `aifx agent run claude` or a custom alias that doesn't start with `claude`). The existing command detection logic doesn't recognise these commands as a specific CLI agent, so:

1. The toolbar shows a generic appearance instead of the agent's icon and branding.
2. The session is tagged as `CLIAgent::Unknown`, which means:
   - No plugin listener is created (no rich status, no auto-show/hide of Rich Input).
   - Rich input submit strategy defaults to `Inline` instead of the agent-appropriate strategy.
   - Agent-specific skill providers are not available in the slash menu.
   - Notifications lack the agent's display name and icon.

Users can already add regex patterns to enable the toolbar, but there is no way to tell Warp which agent a pattern represents.

## Goals

1. Let users assign a specific CLI agent to each custom toolbar command pattern.
2. When a pattern matches a command, the resulting session should behave identically to a natively-detected agent session (icons, branding, plugin listener eligibility, rich input strategy, skills, notifications).
3. Preserve the existing `aifx agent run claude` → Claude detection for Uber team members (no change needed — already works).

## Non-goals

- Changing the auto-detection logic for commands that already resolve to a known agent.
- Supporting multiple agent assignments per pattern (one pattern = one agent).
- Adding new CLI agent types.
- Changing the plugin install/update flow for custom commands (the existing install chip logic already handles per-agent plugin checks).

## Figma / design references

Figma: none provided.

## User experience

### Settings UI: "Commands that enable the toolbar"

Each item in the command list gains a dropdown to its right. The dropdown shows all known CLI agents plus a default "Any CLI Agent" option.

#### Dropdown items

Each item in the dropdown displays:
- The agent's icon (from `CLIAgent::icon()`) at the standard menu item icon size.
- The agent's display name (from `CLIAgent::display_name()`).
- "CLI Agent" appears first, with no icon. This is the default selection for new patterns.

Agents with `CLIAgent::Unknown` are not shown as a selectable option (it's represented by "CLI Agent").

#### Layout of each command list row

```
[command regex text]     [Agent Dropdown ▼]  [× remove]
```

The command text is left-aligned and shrinkable. The dropdown is fixed-width, grouped with the command text on the left side. The remove button is right-aligned. The dropdown uses the standard `Dropdown` component. When collapsed, the top bar shows the selected agent's name.

#### Adding a new command

When a user submits a new command via the text input, it is added with the default "CLI Agent" selection. The user can then change the agent via the dropdown.

#### Removing a command

When a command is removed, its agent mapping is also cleaned up from the backing setting.

#### Changing the agent

Selecting an agent from the dropdown immediately persists the choice. The setting syncs to cloud like other AI settings.

### Session behavior when a custom pattern matches

When a user-configured regex matches a running command AND the pattern has a specific agent assigned:

1. `detect_cli_agent_from_model` returns the assigned `CLIAgent` variant (e.g. `CLIAgent::Claude`) instead of `CLIAgent::Unknown`.
2. The session is created with that agent, giving it:
   - The correct icon, display name, and brand color in the toolbar.
   - The correct rich input submit strategy (`BracketedPaste` for Codex, `DelayedEnter` for Gemini/OpenCode/Copilot/Auggie, `Inline` for Claude/Amp/Droid).
   - Eligibility for plugin listener registration (if `is_agent_supported` returns true for that agent).
   - The correct skill providers in the rich input slash menu.
   - The correct plugin install/update chip (if a plugin manager exists for that agent).
3. If the pattern has no agent assigned ("CLI Agent"), behavior is unchanged — the session uses `CLIAgent::Unknown` as today.

### `aifx agent run claude`

`aifx agent run claude` is already detected as `CLIAgent::Claude` for Uber team members via the `is_aifx_agent_run_claude` special case. No changes to this behavior — the Uber-team gate stays in place. Non-Uber users who want the same behavior can add a custom pattern and assign it to Claude.

## Success criteria

1. **Dropdown renders for each command**: Every item in the "Commands that enable the toolbar" list shows a dropdown between the command text and the remove button.
2. **Dropdown lists all agents with icons**: The dropdown contains "Any CLI Agent" (no icon) followed by each `CLIAgent` variant (except `Unknown`) with the correct icon and display name.
3. **Default is "CLI Agent"**: Newly added commands default to "CLI Agent".
4. **Selection persists**: Changing the dropdown updates the backing setting and survives app restart.
5. **Selection syncs to cloud**: The agent mapping setting syncs via the same mechanism as other AI settings.
6. **Correct agent in session**: When a custom pattern with an assigned agent matches, `CLIAgentSessionsModel` receives the specified `CLIAgent`, not `Unknown`.
7. **Full agent experience**: A custom pattern assigned to e.g. Claude shows the Claude icon in the toolbar, uses the Claude rich input submit strategy, shows Claude-specific skills in the slash menu, and is eligible for the Claude plugin listener.
8. **Plugin listener created**: If a pattern is assigned to an agent that supports plugin listeners (Claude, Codex, OpenCode), the plugin lifecycle (install chip, update chip, listener registration) works as if the command was natively detected.
9. **Removal cleans up**: Removing a command from the list also removes its agent mapping from the backing store.
10. **aifx unchanged**: `aifx agent run claude` continues to be detected as `CLIAgent::Claude` for Uber team members.
11. **Backward compatible**: Existing custom commands (with no agent assigned) continue to work as `CLIAgent::Unknown`.

## Validation

1. **Manual testing**: Add a custom command pattern (e.g. `my-claude-wrapper`), assign it to Claude, run `my-claude-wrapper` in the terminal, and verify:
   - The toolbar shows the Claude icon and branding.
   - Rich input uses Claude's submit strategy.
   - The plugin install chip appears (if the Claude plugin isn't installed).
   - Notifications show "Claude Code" as the agent name.
2. **aifx test**: Verify that `aifx agent run claude` continues to be detected as Claude for Uber team members (no regression).
3. **Default behavior**: Add a command pattern with "CLI Agent" and verify it shows the generic toolbar (same as today).
4. **Persistence**: Change an agent selection, restart Warp, and verify the selection is preserved.
5. **Removal**: Remove a command and verify its agent mapping is gone (re-add the same command and confirm it defaults to "CLI Agent").

