# GH10235: Keep a CLI agent pane's identity across shell job control (Ctrl+Z / fg)
Issue: https://github.com/warpdotdev/warp/issues/10235

## Summary
When a third-party CLI agent (Claude Code, Codex, and the other agents Warp detects) is running in a terminal pane and the user suspends it with Ctrl+Z and later resumes it with `fg`, Warp must keep treating it as the same CLI agent session. After resume, the pane shows up exactly once in the vertical tabs panel, with the same agent identity, status, and session context it had before the suspend.

## Problem
Today Warp ties a CLI agent session to the block that launched the agent. Suspending the job returns the shell to its prompt, so Warp completes that block and throws the session away. The command that brings the agent back (`fg`, `fg %1`, `%1`, ...) is not recognized as a CLI agent command, so Warp never re-attaches the session.

After `fg` the agent is back in the foreground, but its pane's row in the vertical tabs panel no longer looks like an agent. Before the suspend, the row showed the agent logo, the agent's title (for example `✳ Claude Code`), and its status. After `fg`, it shows the plain terminal icon and the shell title `claude`, the same as any other long-running command. This reproduces on Stable v0.2026.10.21 (macOS, zsh). The original report called it a duplicated entry. It is really the same pane's row losing its agent identity, which reads as a second, different entry for the same worker. The CLI agent footer, rich input, status pill, and notifications are also gone after resume. If the agent's plugin sends another event later, Warp starts a brand-new session instead of restoring the old one, and the rich-input draft and earlier session context are lost.

This reproduces with both Claude Code and Codex, so the problem is in Warp's session tracking, not in any one agent.

## Goals / Non-goals
Goals:
- One logical CLI agent session per pane survives a suspend/resume cycle.
- On resume, Warp restores the agent's identity, status, session context, plugin listener, and rich-input draft from before the suspend.
- Works for every way a shell can resume a stopped job (`fg`, `fg %N`, `%N`, `%name`, `%?text`, aliases or functions that call `fg`), and in every shell Warp supports with job control.

Non-goals:
- A dedicated UI for suspended jobs (a "suspended" badge, a jobs list).
- Supporting `bg` for TUI agents. A TUI agent sent to the background stops again on its next terminal read or write. That is shell behavior, not something Warp should paper over.
- Exact restoration for agents running inside a remote shell (SSH or warpified SSH). See behavior 11.
- Windows shells, which have no POSIX job control.
- Changes to the CLI agents themselves or to their Warp plugins.

## Behavior
1. Nothing changes while the agent runs in the foreground. The pane shows a single agent row with the agent icon, agent title or latest prompt, and status, as it does today.
2. When the foreground CLI agent is stopped by job control (Ctrl+Z / SIGTSTP, or SIGSTOP) and the shell returns to its prompt, the pane behaves like a plain shell. The CLI agent footer and rich input are hidden, and the shell input box gets focus.
3. While the agent is suspended, the pane still appears exactly once in the vertical tabs panel and in every other per-pane surface. It does not show a live status pill for an agent that is not running.
4. Suspending is not ending. Side effects that only make sense when the agent process actually exits do not fire on suspend. In particular, a session-sharing session that Warp auto-started for the CLI agent run is not auto-stopped.
5. The rich-input draft text present at suspend time is kept, and so is whether the rich input was open.
6. When the user brings the suspended agent back to the foreground, by any command that does so, the pane returns to the agent representation, still as exactly one row. Restored state includes:
   - the same agent and status as before the suspend
   - the same session context (cwd, project, agent session id, title, latest prompt)
   - the same plugin listener, so plugin events keep updating the same session
   - the rich-input draft, and the rich input reopened if it was open at suspend time.
7. A resume is not a new session. It does not count as a new session start for telemetry, and it does not auto-open rich input against the user's earlier choice.
8. The resumed agent's TUI redraw uses the same in-place redraw behavior Warp applies when the agent first starts, so frames do not stack in the resumed block.
9. If several CLI agents are suspended in the same pane, Warp restores the one the shell actually resumed, never a different one. This holds for `fg` with no argument, numeric job specs, `%name`, `%?text`, and `%-`.
10. If the resume fails (for example `fg: no current job`, or the job was killed while suspended), nothing is restored and no agent row appears. A suspended agent whose process no longer exists is discarded, with the normal session-ended side effects.
11. Remote shells: when Warp cannot see the agent's process (the agent runs on an SSH host), Warp falls back to the agent's plugin. If the first plugin event after resume carries the same agent session id as a suspended session in that pane, Warp restores that session instead of creating a new one. Without a plugin event, the pane stays a plain terminal row until the agent's next plugin event. That is today's behavior, but the old session is no longer silently replaced.
12. Closing the pane or exiting the shell discards any suspended sessions in that pane with the normal session-ended side effects.
13. After a resumed agent exits normally, it ends exactly like an agent that was never suspended.

## Open questions
- Should a suspended agent leave a subtle hint on its pane row (for example "Claude Code (suspended)")? This spec keeps the row a plain terminal row while suspended. A hint can be added later without changing the session model.
