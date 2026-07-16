# product.md — CLI agent working-directory changes update the block's directory metadata

Issue: https://github.com/warpdotdev/warp/issues/10031

## Summary

When a CLI agent (e.g. Claude Code via the `claude-code-warp` plugin) changes
its working directory mid-session, Warp does not update the block's displayed
directory metadata. The most common trigger is `claude --worktree`, which
relocates the agent into `<project>/.claude/worktrees/<name>` and checks out a
new branch — but the block's `WorkingDirectory` chip, tab subtitle, git branch
indicator, and diff/PR chips all continue to reflect the launch directory.

The plugin already reports the agent's current directory to Warp on every event
(in the OSC 777 `warp://cli-agent` payload). Warp stores it on the CLI-agent
session but never propagates it to the block that owns the displayed directory.
This spec defines the behavior that closes that gap.

## Goals / Non-goals

In scope:

- When a CLI-agent session reports a working directory that differs from the
  block's current directory, the block's directory and everything derived from
  it (working-directory chip, tab subtitle, git branch, git diff / PR chips)
  update to reflect the reported directory.

Out of scope:

- Changing how the plugin reports its directory (no plugin change is required;
  the `cwd` is already in the payload).
- Directory tracking for agents that do **not** report a `cwd` (nothing to
  propagate).
- Remote / SSH sessions: an agent running inside an SSH-wrapped block must not
  move that block's directory (see invariant 6).
- Reconstructing directory history for past blocks; only the block that owns
  the active CLI-agent session is updated.

## Testable behavior invariants

1. **Happy path — directory change is reflected.** Given a CLI-agent session
   whose block shows directory `A`, when the agent reports a new working
   directory `B` (B ≠ A, B non-empty), the block's displayed working directory
   becomes `B`.

2. **Derived metadata follows.** After invariant 1 fires, the working-directory
   chip, the tab subtitle, the git branch indicator, and the git diff / PR
   chips recompute against `B` (i.e. they reflect `B`'s branch and changes, not
   `A`'s). This is the observable fix for the `claude --worktree` report.

3. **No-op when unchanged.** If the reported directory equals the block's
   current directory, no update, event, or recomputation is triggered.

4. **Empty directory is ignored.** If the reported directory is absent or an
   empty string, the block's directory is left unchanged.

5. **Update timing.** The directory update occurs on the CLI-agent session
   events that already carry directory information — session start, prompt
   submitted, and tool completed — so the displayed directory converges to the
   agent's directory as the session progresses, without requiring the user to
   run a shell command.

6. **SSH blocks are protected.** If the CLI-agent session's block is an
   SSH-launching block, an agent-reported directory does **not** change the
   block's directory (the remote shell owns that block's directory; a
   locally-reported path must not clobber it). This matches the existing
   guard applied to shell-emitted OSC 7 directory updates.

7. **Non-agent blocks unaffected.** Blocks with no CLI-agent session continue
   to derive their directory solely from shell precmd / shell-emitted OSC 7, as
   before. This change adds no new behavior to non-agent blocks.

## Open questions

None. The reporting mechanism already exists; this spec only defines
propagating the reported directory to the block.
