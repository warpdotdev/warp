---
name: local-automations
description: Reference the Warp local automation TOML schema, path conventions, validation rules, and examples. Use when creating or updating local automation TOML files (on-disk automations, not Oz/cloud scheduled agents) or when another automation skill needs the canonical schema details.
---

# local-automations

Canonical reference for Warp local automations: personal jobs defined as TOML files on the user's machine.

## Current limitations (important)

- **Schedules do not fire yet.** The `schedule` field is required and stored for forward compatibility, but Warp does not run automations on a timer in this version. The only supported execution path is **Run now** (from Settings → Automations, also opened via Command Palette "Open Settings: Automations").
- Never tell the user their automation "will run" at the scheduled time. Be explicit that scheduling activates in a future release and that Run now is how to execute it today.

## Where automations live

- One automation = one TOML file in the `automations/` subdirectory of the user's Warp data directory.
- Standard builds use `~/.warp/automations/`. Non-stable builds use a channel-specific variant (e.g. `~/.warp-dev/automations/`). Run `ls -d ~/.warp*/` to list the available Warp data directories and pick the one that corresponds to the running build; ask the user if unsure.
- Filenames are descriptive snake_case ending in `.toml` (e.g. `morning_repo_brief.toml`).
- Automations are personal and user-scoped. There is no repo-local discovery path.

## Schema

Unknown fields are rejected — only use the fields documented here.

```toml
# Required: display name shown in the Local Automations list.
name = "Morning repo brief"

# Optional (default true): whether the automation is active for future
# scheduling. Run now still works on disabled automations (with a warning).
enabled = true

# Required: cron expression or preset string (e.g. "@daily"). Stored as an
# opaque string; NOT fired on a timer yet.
schedule = "0 9 * * 1-5"

# Exactly ONE of `cwd` or `[worktree]` is required.

# Option A: run in an existing directory (supports ~).
cwd = "~/code/warp"

# Option B: run in a git worktree created (or reused) under Warp's worktree
# root: ~/.warp/worktrees/<repo-name>/<name>. A branch named after the
# worktree is created from `base_branch` (or the repo's HEAD when omitted).
# [worktree]
# repo = "~/code/warp"
# name = "automation-morning-brief"
# base_branch = "main"   # optional

# Required: how the automation executes.
[runner]
type = "warp_agent"   # or "shell"
prompt = "Summarize commits on main from the last 24h."
# command = "gh pr list --author @me"   # required instead of prompt when type = "shell"

# Optional: stored now, enforced/applied in a future release.
# timeout_seconds = 1800
# [env]
# FOO = "bar"
```

### Runners

- `warp_agent` — requires `prompt`. Run now opens a new local agent tab in the resolved directory and starts the agent with the prompt under a CLI-like **unattended** execution profile: no interactive permission prompts, and the agent may read/write files and execute commands (the default command denylist still applies). Do not describe agent automations as read-only unless the prompt itself is read-only.
- `shell` — requires `command`. Run now opens a terminal tab at the resolved directory and runs the command. Third-party CLIs (Claude Code, Codex, `gh`, etc.) are expressed as shell commands. Non-zero exit means the run failed.

### Validation rules

- `name` and `schedule` must be non-empty strings.
- Exactly one of `cwd` or `[worktree]` must be set (never both, never neither).
- `runner.prompt` (warp_agent) / `runner.command` (shell) must be non-empty.
- Invalid files never crash Warp; they show as error rows in the Local Automations list, and other valid automations still load.
- A `cwd` that does not exist fails the run with a concrete error at Run now time (no silent fallback).

## Billing and trust

- Warp agent runs consume the user's normal local agent usage/billing; there is no separate automation meter and no cloud schedule is created.
- Running requires the Warp app to be open; nothing runs in the background or while the machine is asleep.
