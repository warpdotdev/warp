# Product Spec: Keep local shells running across Warp relaunches
GitHub issue: https://github.com/warpdotdev/warp/issues/9416
Figma: none provided

## Summary
Local shells keep running in the background when the user quits Warp, and each restored pane reattaches to its own shell when Warp is opened again, showing the output produced while Warp was closed. Closing a tab or pane still ends its process. The behavior is controlled by a new setting, on by default, on macOS and Linux.

## Problem
Quitting or relaunching Warp (including relaunching to apply an update) kills every local shell and whatever is running in it. Session restoration brings back the layout and the finished blocks, but with a fresh shell: long-running builds, servers, REPLs, `ssh` connections and TUIs are lost. Users postpone updates or keep a separate terminal multiplexer just to avoid this.

## Goals
- A running command survives quitting Warp and keeps producing output while Warp is closed.
- On relaunch, each restored terminal pane is connected to the same shell process, with its prompt, the running block and the output produced while Warp was closed, and accepts input as before.
- Closing a tab or pane keeps its current meaning: the process ends.
- The user can turn the behavior off, and turning it off never leaves processes running after quit.
- Failures (the background service cannot start, a version mismatch) degrade to today's behavior instead of blocking the user.

## Non-goals
- Windows. The feature is unix-only (macOS and Linux) in this iteration.
- Agent conversations or other in-app state. Only local shell processes (and anything they run) are kept alive; in-app agent state follows existing session restoration.
- Remote sessions, shared sessions, or reattaching from another machine or another user.
- Rebuilding Warp blocks for warpified subshells or SSH sessions on reattach (their output is shown, but as plain text).
- Keeping sessions across a reboot or a logout that kills user processes.
- Reattaching sessions started by a Warp version whose background protocol differs from the running one.

## Behavior

1. **Quitting keeps processes alive.** With the feature active (see 6–8), quitting Warp (menu, Cmd+Q / Ctrl+Shift+Q, or relaunch for an update) leaves every shell that was started by the background service running, together with its child processes. A command that was running keeps running and its output is not lost while Warp is closed.

2. **Relaunch reattaches.** When Warp is reopened and restores windows, tabs and panes, each terminal pane whose shell is still alive is connected to that same process (same pid). The pane shows its earlier prompt, the block that was running, and the output produced while Warp was closed, followed by live output. Typing, resizing, and running new commands work as in a fresh session. Full-screen programs (`vim`, `top`) redraw at the pane's current size after reattaching.

3. **Output while closed is bounded.** Up to 10 MiB of output per shell is kept while Warp is closed. If more was produced, the oldest output is dropped and an `[output truncated]` marker is shown before the retained output; the running command and the prompt state are still correct. A process producing output while Warp is closed is never blocked by Warp being closed.

4. **A shell that exits while Warp is closed** is shown on relaunch with its final output and exit, as if Warp had observed the exit. Exited shells nobody reattaches to are discarded after 24 hours.

5. **Closing a tab or pane kills its process**, exactly as today. A closed pane is not restored and its shell does not survive a later quit.

6. **Setting.** Settings > Features shows "Keep sessions running after quitting Warp", right after the session restoration setting, with the description "Shells keep running in the background when you quit Warp and reattach when you reopen it. Closing a tab or pane still ends its process. Requires restoring windows, tabs, and panes on startup." It is on by default and is stored as `general.persistent_sessions`. Changing it takes effect without restarting Warp.

7. **Requires session restoration.** When "Restore windows, tabs, and panes on startup" is off, the setting renders disabled and the feature behaves as off.

8. **Effective state.** The feature is active only when all of these hold: the platform is macOS or Linux, the setting is on, session restoration is on, and Warp is not running from an AppImage. In every other case Warp behaves exactly as today.

9. **Turning the setting OFF while Warp is running.** New tabs and panes get ordinary shells. Panes already backed by the background service keep working until they are closed. On the next quit, every shell is killed, including those already backed by the background service; nothing survives.

10. **Turning the setting ON while Warp is running.** Only tabs and panes created from then on are kept alive on quit; panes that already had ordinary shells end on quit as today.

11. **Stale sessions are cleaned up.** On launch, after restoring windows, background shells that no restored pane refers to are killed. If the feature is off at launch and a background service from an earlier run still holds shells, they are all killed and the restored panes get fresh shells. Launching Warp never starts the background service just to clean up.

12. **Quit warning wording.** When the whole app quits with the feature active and every long-running command belongs to a background-backed shell, the existing quit warning keeps its "You have N process(es) running…" line and appends "They will keep running in the background." In every other case (closing a window, tab or pane; the feature off; any running command in an ordinary shell) the warning is unchanged.

13. **A Warp crash is a detach.** If Warp crashes or is force-killed, background shells keep running and reattach on the next launch, as after a normal quit. Blocks that had not been persisted before the crash may be missing from the restored history; output is never duplicated.

14. **Background service unavailable.** If the background service cannot be reached or started within about 3 seconds, Warp reports the error once, uses ordinary shells for the rest of the run (restored panes get fresh shells), and quitting behaves as today. If the service dies while Warp runs, the panes it served end as if their shells exited, and the next new pane retries the service once.

15. **Version mismatch.** If a background service from a Warp version with a different background protocol is running, the new Warp does not reattach to its shells and falls back as in 14. The old service and its shells are left running until they exit on their own (a documented limitation). App updates that keep the same protocol version reattach normally.

16. **Isolation.** Shells and their output are reachable only by processes of the same OS user. Output kept while Warp is closed lives only in memory, never on disk.

17. **Idle exit.** With no shells left and Warp closed, the background service exits by itself after about 60 seconds.

18. **Shell environment.** A shell started by the background service gets the same environment, initial directory and resource limits it would get from Warp directly; it does not inherit stale state from the background service.

## Open questions
- Should the setting default to on for all users, or ship behind a feature flag first?
- Should a mismatched old background service be shut down (killing its shells) instead of left running?
