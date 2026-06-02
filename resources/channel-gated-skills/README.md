# Channel-Gated Skills

Skills in this directory are bundled only for specific release channels.
During the build, `script/copy_conditional_skills` copies skills from the
gate directory that matches the current channel into the app bundle alongside
the always-bundled skills in `resources/bundled/skills/`.

## Directory structure

Each subdirectory is a **gate label**. Place skill directories inside the
gate that corresponds to the **earliest** channel where the skill should ship:

```
channel-gated-skills/
├── dogfood/              ← dogfood-only skills
│   └── my-skill/
└── preview/              ← preview and later
    └── another-skill/
```

> **Stable-ready skills** do not belong here. Place them in the always-bundled
> `resources/bundled/skills/` directory instead. The build script will error if a
> `stable/` directory exists under `channel-gated-skills/`.

## Progressive gating

Gating is **progressive**: earlier gates include all skills from later gates.

| Channel   | Gate      | Includes skills from          |
|-----------|-----------|-------------------------------|
| `local`   | `dogfood` | `dogfood/` + `preview/`       |
| `dev`     | `dogfood` | `dogfood/` + `preview/`       |
| `preview` | `preview` | `preview/`                    |
| `stable`  | —         | *(none — use resources/bundled/skills/)* |

A skill placed in `preview/` is bundled on **all** non-stable builds
(dogfood, preview). A skill placed in `dogfood/` is bundled on dogfood
builds only.

## Adding a new gated skill

1. Create a directory under the appropriate gate (e.g. `dogfood/my-skill/`).
2. Add a `SKILL.md` with the standard skill frontmatter and instructions.
3. Place any supporting scripts or files alongside it.
