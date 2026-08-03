---
name: modify-settings
description: View or modify Warp application settings using the bundled JSON schema for guidance
---

# modify-settings

Use this skill when the user wants to view, change, or troubleshoot Warp application settings.

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
python3 {{skill_dir}}/scripts/find_setting.py {{settings_schema_path}} <key_name>
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

1. **Find the setting** — use `grep` to identify candidate key names, then run the Python path-tracing script to get the full dotted path and the setting's valid values. Never rely on grep output alone to infer nesting.
2. **Read current value** — check the settings file to see whether the setting is already configured.
3. **Apply the change** — add or update the setting in the TOML file with a valid value from the schema.
