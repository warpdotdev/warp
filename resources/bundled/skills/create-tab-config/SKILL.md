---
name: create-tab-config
description: Create new Warp tab config TOML files from natural-language requests. Use when the user wants a new tab config, a new tab layout, or asks for a slash command to generate a tab config.
---

# create-tab-config

Create a new Warp tab config based on what the user wants.

## Required context

- Use the `tab-configs` skill as the canonical source of truth for:
  - schema details
  - validation rules
  - examples
  - common layout patterns

## Workflow

1. Understand what the user wants to create.
2. If important details are missing, use the `ask_user_question` tool to clarify them before writing anything. Do not guess about layout, commands, directories, parameters, or close-time behavior.
3. Generate valid TOML that matches the `tab-configs` schema.
4. Determine the correct tab config directory for the user's Warp build.
   Tab configs live under `~/.warp/tab_configs/` for standard builds. Non-stable builds use a channel-specific variant (e.g. `~/.warp-<channel>/tab_configs/`).
   To find the correct directory, run `ls -d ~/.warp*/` to list the available Warp data directories and pick the one that corresponds to the running build. When in doubt, ask the user which build they are using.
   Create the `tab_configs/` subdirectory if it does not exist.
   Write the file using a descriptive snake_case filename ending in `.toml`.
5. If the intended filename might conflict with an existing config and it is unclear whether to overwrite or create a new file, use the `ask_user_question` tool.
6. Briefly explain what you created, including the layout and any commands or parameters.
