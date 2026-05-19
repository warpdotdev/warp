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
custom_warping_verbs = ["Verb one", "Verb two"]
```

The setting path is `agents.warp_agent.custom_warping_verbs`.

## How to handle requests

- If the user provides a custom list, replace the entire list with exactly those phrases.
- If the user asks for a built-in pack by name, replace the entire list with the exact pack phrases below.
- If the user asks to update spinner verbs but does not provide a list or pack name, ask which verbs or pack they want. Do not search.
- If the user asks to reset or restore the default, set `custom_warping_verbs = []`.
- Store raw phrases without trailing ellipses; Warp adds `...` at display time.
- Use sentence capitalization for custom phrases.

## Built-in pack values

Medieval:

```toml
custom_warping_verbs = ["At your service, my liege", "At once, my lord", "The scribes set to work", "Seeking wisdom from the realm", "Consulting the ancient tomes", "Dispatching riders across the kingdom", "Draining the flagons", "Interrogating the lesser lords", "Raising the drawbridge", "Rallying the bannermen"]
```

Conspiracy:

```toml
custom_warping_verbs = ["Questioning science", "Conspiring", "Speculating", "Melting steel beams", "Confirmation biasing", "Doing my own research", "Looking for alternative facts", "Waking up the sheep", "Internet deep diving", "Gathering evidence", "Proceeding with skepticism"]
```

Cooking:

```toml
custom_warping_verbs = ["Sautéing", "Caramelizing", "Slicing and dicing", "Bruleeing", "Flambéing", "Immersion blending", "Sous viding", "Emulsifying", "Fermenting", "Braising"]
```

Warpy:

```toml
custom_warping_verbs = ["Warping", "Going to infinity", "Gaining speed", "Morphing", "Wormhole-ing", "Orbiting", "Galaxy braining", "Shooting stars", "Nebulizing", "Constellating"]
```
