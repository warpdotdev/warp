# Queue terminal commands alongside prompts

GitHub: [warpdotdev/warp#11912](https://github.com/warpdotdev/warp/issues/11912). Builds on the v2 queued-prompts panel from [`specs/REMOTE-1543/PRODUCT.md`](../REMOTE-1543/PRODUCT.md) and [`specs/APP-4562/PRODUCT.md`](../APP-4562/PRODUCT.md).

Figma: none provided (the only new visual is a shell-mode `!` prefix on command rows).

## Summary
Extend the v2 prompt-queuing model so that a shell command submitted while prompt queuing is active is queued in the same panel as prompts, instead of interrupting the running agent. Queued prompts and commands fire in strict FIFO order: a command runs after the item before it finishes, and the item after it fires once the command's terminal block completes.

## Problem
Today, when auto-queue is enabled and an agent run is in progress, submitting a shell command from shell mode (e.g. `! ls` or `cargo run`) interrupts/cancels the in-progress agent work and runs the command immediately, rather than queuing it. Only agent prompts can be queued. Users want to line up terminal commands behind in-progress agent work the same way they line up prompts.

## Goals
- Let users queue shell commands while an agent run is in progress, interleaved in FIFO order with queued prompts.
- Execute each queued command when it reaches the head of the queue, and advance to the next item only when the command finishes.
- Reuse the existing queued-prompts panel, interactions, and draining semantics so commands behave like prompts everywhere it makes sense.
- Work in both local Agent Mode and cloud-mode panes.

## Behavior

### Gating
1. All behavior below is gated on the `QueuedPromptsV2` feature flag. When it is off, shell-command submissions behave exactly as today: they run immediately and interrupt any in-progress agent run.

### Queuing a command
2. A shell-mode submission is queued (instead of run immediately) under exactly the same conditions that queue an agent prompt today: an agent conversation in this pane is in progress (or summarizing), and prompt queuing is active for it (the auto-queue/queue-next toggle is on, or — under V2 — the conversation is summarizing). The only change from prompt queuing is that the input being in shell mode no longer disqualifies the submission.
3. A queued command appears as a new row at the tail of the same queued-prompts panel, in FIFO order with any queued prompts. Its preview text is the command exactly as typed (including a leading `!` the user typed for shell mode is normalized away the same way the shell editor handles it).
4. A queued command row is visually distinguished from a prompt row by a leading `!` rendered in the same blue used for the shell-mode `!` prefix in the input. Aside from the prefix, the row renders like a prompt row.
5. Queued command rows are not locked: they support the same interactions as queued prompt rows — drag-to-reorder, hover-revealed edit (pencil) and delete (trash), and they participate in collapse/expand exactly like prompt rows. Editing a command row edits its command text. If the in-progress agent turn finishes while a command row is being edited, the row's text is moved into the input with the input forced into shell mode (so it stays a command), mirroring how an edited prompt row is moved into the input on a clean finish.
6. Submitting an empty shell input does not append a row (existing trim-and-skip behavior).
7. If prompt queuing is not active (no in-progress agent run and auto-queue off), a shell submission runs immediately and is not queued (unchanged from today).

### Draining and execution
8. When the in-progress agent conversation finishes cleanly, the queue drains from the head, one item per finish, exactly as queued prompts do today. If the head is a prompt, it is submitted to the agent. If the head is a command, it is executed.
9. A drained command executes through the same terminal routing a manually submitted shell command uses for that pane: in a local pane it runs in the local terminal; in a cloud-mode / shared-session pane it is submitted via the shared session to run in the remote environment. Because the command came from the queue rather than the current editor buffer, any draft the user is composing in the input is preserved while the command starts and completes.
10. When a command fires, its row is removed from the panel immediately (the same moment a fired prompt row disappears). The running command is visible as its own terminal block; the panel does not keep a duplicate "running" row.
11. The item after a command fires only after that command finishes — i.e. its terminal block completes / the process exits. A command that does not terminate (e.g. a long-running dev server) holds the queue until it exits or is killed; no later item fires in the meantime.
12. A command's exit status does not affect draining: a non-zero exit still counts as finished, so the next item fires regardless of whether the command succeeded.
13. Strict FIFO holds across mixed item types. For a queue of [prompt A, command B, prompt C]: A runs first; B runs only after A's agent turn finishes; C is submitted only after B's terminal block completes. (This is the issue's motivating example.)

### Keeping order while a command runs
14. While a queued command is running, prompt queuing stays active even though the agent itself is idle: any input the user submits during that window (prompt or command) is appended to the back of the queue, preserving strict FIFO — the same as submitting while a prompt is running. Nothing the user submits jumps ahead of, or runs concurrently with, the in-flight command.

### Long-running agent commands (LRC)
15. If a fired queued prompt causes the agent to start an agent-controlled long-running command (managed by a subagent), the queue must not advance while that LRC is still in flight. Intermediate subagent responses produced while the LRC runs do not drain the queue; the next item fires only once the whole agent turn — including the LRC — has finished. (This preserves today's queued-prompt behavior.)
16. Agent-executed command blocks (including LRC snapshots) never count as the completion of a queued command. Only the queued command the panel actually dispatched advances the queue.

### Pause on error / cancel
17. When the in-progress conversation finishes for a non-clean reason (error, cancellation, cancellation during requested command execution), draining pauses immediately, exactly as for queued prompts. If the input editor is empty, the head item is removed from the panel and its text is placed in the input editor (a command's text is restored with the input forced into shell mode, so it stays a command rather than an agent prompt); if the editor is non-empty, nothing is removed or modified. Remaining rows stay intact, and draining resumes the next time the conversation completes an exchange cleanly.

### Lifecycle
18. The queue (including command rows and any in-flight-command tracking) is scoped to its conversation. It persists across agent-view exit and conversation switches so a background conversation can continue draining, and is cleared when the conversation is removed or its owning terminal view is cleared — identical to queued-prompt lifecycle today.

### Cloud mode
19. All of the above applies in cloud-mode panes. Commands queue while the cloud agent run is in progress and, when drained, are submitted via the shared session to the remote environment; the queue advances when the remote command's block completes. Draining off the cloud conversation's completion uses the same conversation-status path that cloud-mode queued prompts already use.
