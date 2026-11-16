---
name: update-tab-config
description: Update existing Warp tab config TOML files from natural-language edit requests. Use when the user wants to modify a tab config that already exists or when editing a tab config file already open in Warp.
---

# update-tab-config

Update an existing Warp tab config in place.

## Required context

- Use the `tab-configs` skill as the canonical source of truth for:
  - schema details
  - validation rules
  - examples
  - common layout patterns

## Workflow

1. Read the existing tab config file before making changes.
2. Understand the requested edit.
3. If important details are missing or ambiguous, use the `ask_user_question` tool before editing. Do not guess about layout changes, command changes, parameters, or `on_close` behavior.
4. Make sure you are editing the tab config that belongs to the user's current Warp build/channel rather than assuming a single hardcoded base directory, then update it so it remains valid according to the `tab-configs` schema.
5. Preserve the user's existing structure and naming where possible unless the requested change requires restructuring.
6. Briefly explain what changed.
