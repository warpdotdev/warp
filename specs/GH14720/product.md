# Product Spec: Resume CLI-agent conversations per project after restart

**Issue:** [warpdotdev/warp#14720](https://github.com/warpdotdev/warp/issues/14720)

**Figma:** none provided

**Provenance:** written for the implementation originally contributed in
[#14734](https://github.com/warpdotdev/warp/pull/14734) by @samithaj, which was closed by the
stale-PR automation before code review. This spec documents that design so the revived PR can go
through the normal spec + review flow.

## Summary

Make CLI-agent conversations (Claude Code, Codex, Cursor CLI, OpenCode) **durable and resumable
per project**. Warp remembers which agent sessions ran, in which directory, under which
conversation name — surviving agent exit and full app restart — and lists them as task rows in a
Projects × Tasks sidebar layout. Clicking a dormant task opens a tab at the session's directory
with the agent's own resume command (e.g. `claude --resume <id>`) prefilled, never executed.

Sessions Warp never witnessed (started in another terminal, or imported) are discovered by
reading Claude Code's own transcripts on disk, so the rail reflects the user's real body of work,
not just what happened inside Warp.

## Problem

After a restart — or just closing a window — there is no way back to a specific coding-agent
conversation from Warp. `CLIAgentSessionsModel` drops a session when the agent exits, taking
both the conversation's name and its session id with it: a restored tab that was running
`claude` shows a truncated path, and resuming means digging the id out of `~/.claude` by hand
and typing `claude --resume <id>`.

For developers who run several agents across several repositories, the terminal has no answer to
"what was I working on, and how do I get back to it?"

## Goals

- An agent conversation keeps its identity — name, directory, session id — after the agent
  exits and after Warp restarts.
- One click resumes a dormant conversation, prefilled with the correct agent resume command;
  the user always confirms by pressing Enter.
- Projects (git repositories, with all worktrees of a repo collapsed into one project) group
  the sidebar, with agent conversations as task rows under each.
- Sessions run outside Warp appear too, discovered from the agent's own on-disk state, with
  their real conversation names — including names set by `/rename`.
- No behavior change while the feature flags are off.

## Non-goals

- Resuming agents that have no verified resume-by-id verb (Gemini, Amp, Copilot, etc.): their
  rows render as non-resumable rather than guessing at syntax.
- Transcript discovery for agents other than Claude Code (the handle store covers the rest;
  discovery for other agents is a follow-up once their on-disk formats are verified).
- Restoring the agent's process state. "Resume" means starting the agent with its own resume
  command; conversation continuity is the agent's responsibility.
- Cross-device durability. The store is local (SQLite) and rebuildable; the agent's transcript
  directory remains the source of truth.

## Behavior invariants

### Recording and durability

1. When a CLI agent starts in a pane, Warp records `(pane, cwd, agent)` immediately and adds the
   `session_id` as soon as the agent's plugin reports it. The record survives agent exit and app
   restart (SQLite).
2. One row per upstream session: resuming a session — from any pane — updates the same task
   identity `(agent, session_id)` rather than creating a duplicate.
3. At most one un-identified ("in-flight") launch is tracked per pane per agent; an in-flight
   row always belongs to a live pane.
4. The table is a rebuildable index. Deleting it loses no user data; the agent's transcript
   directory can re-derive discovery.

### The Projects × Tasks rail

5. With the project layout enabled (Settings → Appearance → "Group sessions by project"), the
   sidebar groups open tabs into projects. A project is a git repository keyed by its shared
   `.git` directory, so every worktree of a repo lands in the same project; tabs with no
   detectable repo group under "Other".
6. With "List tasks in the project rail" enabled, each project lists its agent conversations:
   live ones (running now), dormant ones (agent exited or Warp restarted), and discovered ones
   (found on disk, never witnessed by Warp).
7. Task rows are named by their **conversation**, not their directory. "Task rows show" selects
   what the row displays (e.g. agent session name).
8. A conversation renamed with `/rename` shows its new name — including for dormant and
   discovered sessions, resolved from the transcript.

### Resume

9. Clicking a dormant task whose pane still exists **and** whose recorded cwd still matches
   resumes in place: that pane gets the agent's resume command prefilled. Otherwise a fresh tab
   opens at the recorded directory with the command prefilled.
10. A discovered (never-witnessed) session can never resume "in place": Warp has no pane for
    it, so it always opens a fresh tab at the scanned directory. This is structural — a scanned
    session has no pane binding to look up.
11. The prefilled command is never executed automatically. The user reviews and presses Enter.
12. Resume commands exist only for agents with a verified resume verb: `claude --resume <id>`,
    `codex resume <id>`, `cursor --resume <id>`, `opencode --session <id>`. All other agents
    render non-resumable rows.
13. Session ids are validated before being embedded in a command; an id that fails validation
    produces a non-resumable row (never a malformed shell command).

### Discovery and naming

14. Claude Code sessions are discovered from `~/.claude/projects/<encoded-cwd>/*.jsonl` for the
    directories of open projects; a transcript's existence *is* the session.
15. Names resolve with a uniqueness rule, not recency alone: `/rename` in one session broadcasts
    the new title into other live sessions' transcripts, so a title claimed by multiple sessions
    is treated as contamination and skipped in favor of that transcript's own unique title.
16. Transcript reads are seek-bounded; a transcript is never read whole, regardless of size.

### Restart

17. Restored tabs do not spawn their shells until the tab is opened (`LazyShellStartup`), so
    restoring a window full of tabs does not start every shell at once.

### Gating

18. Everything above sits behind `FeatureFlag::ResumeProjectTasks`, which requires
    `FeatureFlag::Projects` (the Projects × Tasks layout). Both are dogfood flags. With them
    off, no schema is touched at runtime beyond the migration, no scan runs, and the sidebar is
    unchanged.

## Open questions

1. Should discovery extend to other agents' on-disk state (Codex, Cursor) once their formats
   are verified, or stay Claude-only until then?
2. Retention: should very old handles/scanned sessions age out of the rail, and after how long?
3. The original PR is large (~6,700 lines). If maintainers prefer, it can split into
   (a) durable handle store + resume, (b) transcript discovery + naming, and (c) the Projects ×
   Tasks rail UI, in that dependency order.
