---
name: modify-spinner-verbs
description: Change Warp/Oz spinner verbs, warping verbs, loading text, flavor text, or spinner verb packs such as Medieval, Conspiracy, Cooking, or Warpy. Use whenever the user asks to change, set, update, customize, or reset spinner verbs so the agent writes settings directly instead of searching code.
---

# modify-spinner-verbs

Use this skill when the user asks to change Warp/Oz spinner verbs, warping verbs, loading text, flavor text, or a spinner verb pack.

## Non-negotiable behavior

Do not search the codebase. Do not read source files. Do not grep. Do not create, modify, or add built-in packs in source code.

This is always a settings change. Write directly to the current app channel's settings TOML file:

```
{{settings_file_path}}
```

```toml
[agents.warp_agent]
spinner_verbs = "custom"
custom_spinner_verbs = ["Verb one", "Verb two"]
```

The source setting path is `agents.warp_agent.spinner_verbs`. The custom list path is `agents.warp_agent.custom_spinner_verbs`.

## How to handle requests

- If the user provides a custom list, set `spinner_verbs = "custom"` and replace `custom_spinner_verbs` with exactly those phrases.
- If the user asks for a built-in pack by name, set `spinner_verbs` to that pack identifier and leave any existing `custom_spinner_verbs` list in place unless the user explicitly asks to clear it.
- If the user asks to update spinner verbs but does not provide a list or pack name, ask which verbs or pack they want. Do not search.
- If the user asks to reset or restore the default, set `spinner_verbs = "default"` and leave any existing `custom_spinner_verbs` list in place unless the user explicitly asks to clear it.
- Store raw phrases without trailing ellipses; Warp adds `...` at display time.

## Built-in pack values

Medieval:

```toml
spinner_verbs = "medieval"
```

Conspiracy:

```toml
spinner_verbs = "conspiracy"
```

Cooking:

```toml
spinner_verbs = "cooking"
```

Warpy:

```toml
spinner_verbs = "warpy"
```
