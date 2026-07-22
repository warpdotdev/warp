---
name: create-local-automation
description: Create or edit Warp local automation TOML files from natural-language requests. Use when the user asks for a local automation, wants to schedule or automate a recurring job on their own machine, or wants to create/update an automation TOML. Do NOT use Oz, cloud scheduled agents, or `warp agent schedule` for these requests — local automations are on-disk TOML files, not cloud objects.
---

# create-local-automation

Create or update a Warp local automation based on what the user wants.

## Local vs. cloud

A local automation is a plain TOML file on the user's machine. Creating one never involves the `oz` CLI, cloud scheduled agents, `warp agent schedule`, or cloud environments. Only suggest a cloud scheduled agent if the user explicitly asks for cloud/Oz or for runs while their machine is off — and confirm before switching approaches.

## Required context

- Use the `local-automations` skill as the canonical source of truth for:
  - schema details and path conventions
  - validation rules
  - runner semantics and current limitations

## Workflow

1. Understand what the user wants to automate.
2. If important details are missing, use the `ask_user_question` tool to clarify before writing anything. Essentials:
   - what the job should do (agent prompt vs shell command)
   - roughly when it should run (to fill the `schedule` string)
   - where it should run (`cwd` directory or a git worktree)
3. Pick the runner:
   - `warp_agent` for tasks described as an agent prompt (summaries, triage, code chores). Remember the run is unattended and can write files / run commands.
   - `shell` for plain commands or third-party CLIs (Claude Code, Codex, `gh`, ...).
4. Generate valid TOML that matches the `local-automations` schema. Keep the prompt/command self-contained: the run starts fresh with no conversation history.
5. Determine the correct automations directory for the user's Warp build (see the `local-automations` skill for how) and create the `automations/` subdirectory if needed. Write the file with a descriptive snake_case filename ending in `.toml`.
6. If the intended filename or automation name conflicts with an existing file, ask before overwriting (use the `ask_user_question` tool). Editing an existing automation the user referred to is fine without asking.
7. Confirm to the user: the file path, name, schedule string, runner, and how to run it. Be explicit that:
   - **schedules fire only while Warp is open** and the machine is awake (catch-up within ~6 hours after reopen; older gaps marked missed)
   - **Run now** is always available: Settings → Automations → Run now (or Command Palette → "Open Settings: Automations")
8. Offer to run it once now as a smoke test if the user wants (they trigger Run now from the list).
