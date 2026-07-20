# Local Automations — Slice A (Convention, Skill, Run Now)

## Summary
Slice A lets a user define a **local automation** as a TOML file under their Warp data directory, create and edit it via an agent skill from natural language, and **run it immediately** into a local agent tab (or shell). Cron scheduling, promote-to-cloud, and a full management UI ship in later slices; this slice proves the config format and the “run this job once” loop.

## Problem
Warp has strong interactive local agents and cloud scheduled agents, but no simple on-ramp for “run this on a schedule on my machine.” Competitors (Codex local schedules, Cursor `/loop`) make that moment easy. Slice A establishes the durable automation object format and a zero-friction create + run path so we can dogfood before building the scheduler.

## Goals / Non-goals
**Goals**
- A clear, versionable local automation TOML schema.
- An agent skill that creates/edits automations from a prompt.
- Manual **Run now** that executes the automation in a local tab using unattended agent permissions (Warp agent) or a non-interactive shell.
- A minimal way to list automations and open their config.

**Non-goals (this slice)**
- Cron / catch-up / missed-run handling (Slice B).
- Promote-to-cloud wizard (Slice C).
- Dedicated run-history route, daemon, repo-local configs, first-class third-party harness profiles beyond `shell`.
- Team-shared automations or server-backed objects.

## Figma
Figma: none provided

## Behavior

### What a local automation is
1. A local automation is one TOML file on disk under the user’s Warp data directory: `automations/` (channel-aware, same base as tab configs — e.g. `~/.warp/automations/` on stable).
2. One file = one automation. Filename is snake_case ending in `.toml`. Display identity may differ from filename via the `name` field.
3. The file is the source of truth for configuration in Slice A. Editing the file (manually or via the skill) is editing the automation.
4. Automations are **personal / user-scoped** only in this slice. There is no repo-local discovery path yet.

### Schema (user-visible fields)
5. Every automation file includes at least:
   - `name` (string) — display name.
   - `enabled` (bool, default true) — whether the automation is considered active for later scheduling; Run now may still run a disabled automation with an explicit warning (see 22).
   - `schedule` (string) — cron expression or documented preset string. **Slice A does not fire on schedule**; the field is required so files are forward-compatible with Slice B.
   - `runner` — either `warp_agent` or `shell` (see runners).
6. Working directory is specified as exactly one of:
   - `cwd` (string, supports `~`), or
   - a worktree table that describes creating/reusing a git worktree under the existing Warp worktree root (`~/.warp/worktrees/<repo>/<name>` convention), consistent with tab-config worktrees.
7. Optional fields allowed in Slice A: `timeout_seconds`, simple `env` key/value map, freeform notes/comments in TOML.
8. Unknown fields must not crash listing or Run now: ignore with a user-visible warning when opening/running, or reject with a clear parse error if the file is invalid TOML / missing required fields. Prefer fail-closed on required-field absence.

### Runners
9. **`runner = "warp_agent"`** (or equivalent TOML shape):
   - Requires a `prompt` string (the agent task).
   - Run now opens a **local agent tab/conversation** in the configured cwd or worktree and starts the agent with that prompt.
   - The run uses a **CLI-like unattended execution profile**: no interactive permission prompts; actions that would block unattended fail the run rather than hang. Default command denylist behavior still applies as for CLI unattended runs.
10. **`runner = "shell"`**:
    - Requires a `command` string (or ordered command list if the schema supports multiple).
    - Run now opens a local terminal tab in the cwd/worktree and runs the command non-interactively.
    - Non-zero exit is a failed run (visible in the tab / exit status).
11. No other runner types in Slice A. Third-party CLIs (Claude Code, Codex, etc.) are expressed as `shell` commands.
12. Shell automations are first-class for create/run; promote guidance (later) may differ, but Slice A does not block creating them.

### Creating and editing via skill
13. A bundled (or repo) skill teaches the agent the schema, path conventions, filename rules, and how to write valid TOML.
14. When the user asks to schedule/create a local automation in natural language, the agent:
    - Clarifies missing essentials if needed (what to do, roughly when, runner type, where on disk).
    - Writes a new file under `automations/` or updates an existing one.
    - Confirms path, name, schedule string, runner, and how to Run now.
15. The skill must not invent cron firing behavior that does not exist yet; it must state that **scheduling is not active until Slice B**, and that **Run now** is the supported execution path in Slice A.
16. The skill may offer to Run now immediately after create if the user wants a smoke test.
17. Overwrite vs new file: if a name/filename conflicts, the agent asks before overwriting.

### Listing and opening config
18. The user can see their local automations in a simple list surface (settings subsection, agent management adjacent page, or equivalent — exact placement open if needed, but there must be one discoverable list).
19. From the list, the user can:
    - See `name`, runner type, schedule string, enabled flag, and source path (home-relative).
    - Open the TOML config (in Warp’s editor or OS default).
    - Trigger **Run now**.
20. Empty state: if `automations/` is missing or empty, the list explains how to create one (skill / natural language / drop a TOML file) and does not error.
21. Parse errors for a single file: that row shows an error state and Open config; other valid automations still list and run.

### Run now
22. **Run now** always requires an explicit user action (button, command palette, or agent-invoked run after confirmation). It does not wait for cron.
23. Run now on a disabled automation: allowed with a clear warning that the automation is disabled for future scheduling.
24. Run now resolves cwd/worktree before starting:
    - If worktree setup fails (not a git repo, path missing, git error), the run does not start; user sees a concrete error.
    - If `cwd` does not exist, fail with a concrete error (do not silently fall back to `$HOME` without saying so).
25. Concurrent Run now on the same automation: allowed; each invocation opens its own tab. No single-flight requirement in Slice A.
26. While a run is starting, the UI shows a short pending state on the control; failure to spawn shows an error toast or inline error.
27. Success means a tab is open and the agent/shell has started — not that the task completed. Completion is observed in the tab like any other local agent/shell session.
28. Closing the tab cancels further interactive work in that session the same way closing any agent/terminal tab does; there is no separate automation-run supervisor in Slice A.

### Permissions, billing, and trust
29. Warp agent runs consume the user’s normal local agent usage/billing. Slice A does not introduce a separate automation meter.
30. Unattended profile means the user should treat automations as capable of writing files and running commands within denylist constraints; the skill and list UI copy must not imply “read-only by default” unless the automation’s prompt/command is read-only.
31. Slice A does not auto-promote, auto-handoff to cloud, or read cloud credits.

### What must not happen
32. Slice A must not silently start cron jobs in the background.
33. Slice A must not require a cloud environment or network beyond what a normal local agent/shell needs.
34. Slice A must not reuse full tab-config pane layout schemas; only the thin automation fields above (plus optional worktree/cwd setup).
35. Invalid TOML must never crash the app; bad files are isolated to error rows / run failures.

### Accessibility / discoverability
36. Run now and Open config are available from the list without memorizing filesystem paths.
37. Command palette entries (if added in this slice) use clear names: e.g. “Local Automations”, “Run Local Automation”.

## Open questions
- Exact navigation placement for the list (Settings vs Agent Management vs command palette only) can be decided in implementation if it does not block the skill + Run now path.
- Whether `enabled = false` blocks Run now hard vs warns: Behavior 23 chooses **warn + allow**; revisit if dogfood finds it confusing.
