---
name: modify-settings
description: View or modify Warp application settings, including common natural-language settings like spinner verbs, using the bundled JSON schema for guidance
---

# modify-settings

Use this skill when the user wants to view, change, or troubleshoot Warp application settings.

## Fast path: spinner verbs / warping verbs

When the user asks to change Warp/Oz spinner verbs, warping verbs, loading text, flavor text, or a spinner verb pack:

1. Do NOT search the codebase, read source files, or grep for anything.
2. Do NOT create, modify, or add built-in packs in source code. Packs are read-only.
3. ALWAYS write directly to the settings TOML — this is the ONLY correct action.

Every request to "change", "set", or "update" spinner verbs means **replace the entire list** with exactly what the user asked for.
If the user does not provide a list or built-in pack name (for example: "update spinner verbs"), ask which verbs or built-in pack they want. Do not search.

```toml
[agents.warp_agent]
custom_warping_verbs = ["Verb one", "Verb two"]
```

Rules:

- Use `agents.warp_agent.custom_warping_verbs`.
- Store raw phrases without trailing ellipses; Warp adds `...` at display time.
- Use sentence capitalization for custom phrases: capitalize only the first word, not every word in a phrase.
- Set `custom_warping_verbs = []` to restore the default `Warping...` display.
- If the user provides a custom list (comma-separated, numbered, or plain), write those phrases verbatim as a TOML string array. Replace the entire list every time — never append.
- If the user asks for a built-in pack by name, write the exact pack array below. Do NOT modify source code to add a new pack.
- If the user asks to "update spinner verbs" without giving values, ask for the list or pack name. Do NOT grep.

Built-in spinner verb packs:

```toml
# Medieval
custom_warping_verbs = ["At your service, my liege", "At once, my lord", "The scribes set to work", "Seeking wisdom from the realm", "Consulting the ancient tomes", "Dispatching riders across the kingdom", "Draining the flagons", "Interrogating the lesser lords", "Raising the drawbridge", "Rallying the bannermen"]

# Conspiracy
custom_warping_verbs = ["Questioning science", "Conspiring", "Speculating", "Melting steel beams", "Confirmation biasing", "Doing my own research", "Looking for alternative facts", "Waking up the sheep", "Internet deep diving", "Gathering evidence", "Proceeding with skepticism"]

# Cooking
custom_warping_verbs = ["Sautéing", "Caramelizing", "Slicing and dicing", "Bruleeing", "Flambéing", "Immersion blending", "Sous viding", "Emulsifying", "Fermenting", "Braising"]

# Warpy
custom_warping_verbs = ["Warping", "Going to infinity", "Gaining speed", "Morphing", "Wormhole-ing", "Orbiting", "Galaxy braining", "Shooting stars", "Nebulizing", "Constellating"]
```

## Settings Schema

A JSON schema describing all available settings is bundled at:

```
{{settings_schema_path}}
```

The schema follows JSON Schema draft 2020-12, with settings organized hierarchically under `properties`. Each setting includes:

- **`description`** — what the setting controls
- **`type`** — the value type (`string`, `boolean`, `integer`, etc.)
- **`default`** — the default value
- **`enum`** or **`oneOf`** — valid values, when the setting is constrained

### Finding a setting

Use `grep` to do an initial broad search for candidate key names:

```sh
grep -i "font" {{settings_schema_path}}
```

Once you have a candidate key name, run the bundled script to get the **full dotted path**, the setting's properties, and any parent context. This is critical — the schema has multiple sections with similar names (e.g. several `input` keys), so never assume the nesting from grep output alone.

```sh
python3 <skill_dir>/scripts/find_setting.py {{settings_schema_path}} <key_name>
```

The output gives you the unambiguous full path (e.g. `properties.appearance.properties.input.properties.input_mode`) and the setting's full definition including valid values.

## Settings File

The user's settings are stored in a TOML file at:

```
{{settings_file_path}}
```

Settings use dotted TOML section headers matching the schema hierarchy. Always trace the **full** nesting path from the schema to the TOML — each intermediate `properties` key becomes a section level. For example:

A property at `properties.appearance.properties.font_size` (one level deep) corresponds to:

```toml
[appearance]
font_size = 14
```

A property at `properties.appearance.properties.themes.properties.theme` (two levels deep) corresponds to:

```toml
[appearance.themes]
theme = "light"
```

A common mistake is to stop one level too early — always count the full depth before writing the TOML section header.

If the file does not exist yet, create it. Warp hot-reloads this file, so changes take effect immediately.

## Workflow

1. **Find the setting** — if a fast path above applies, use it directly. Otherwise, use `grep` to identify candidate key names, then run the Python path-tracing script to get the full dotted path and the setting's valid values. Never rely on grep output alone to infer nesting.
2. **Read current value** — check the settings file to see whether the setting is already configured.
3. **Apply the change** — add or update the setting in the TOML file with a valid value from the schema.
