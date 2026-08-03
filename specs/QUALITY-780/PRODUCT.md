# Client Awareness of `wait_for_events` Yields

## Summary
When an Oz cloud agent calls the server-side `wait_for_events` tool, the agent has not finished its work — it has yielded the turn and is waiting on an inbound message or lifecycle event. The Warp client must observe this yield, keep the run alive, suppress completion-shaped UI (notifications, success badges), and surface a distinct "waiting for events" presentation in the orchestration UI.

## Problem
Today the client cannot tell a `wait_for_events` yield apart from real completion. The model turn ends cleanly, the local conversation status flips to `Success`, and two things go wrong as a result:

1. The Oz CLI driver treats `Success` as run completion and schedules the process exit through its `--idle-on-complete` timer. For cloud agents, this means the worker can exit seconds after yielding, even though child events were expected. Server-side compensations (`shouldPreserveInProgressOnClientSuccess`, `ExtendTaskIdleTimeout`) preserve task state in the database but do not keep the running process alive, so the inbound event has no agent to wake.
2. The notifications mailbox fires `NotificationCategory::Complete` ("Task completed.") on every `Success` transition. Orchestrators that yield between turns produce spammy "Task completed" toasts that users have complained about.

A fix has to come from the client: server-side preservation cannot keep a local process alive and cannot suppress local notifications.

Note on orchestration badges: in current one-level orchestration the pill bar aggregator already returns `InProgress` whenever any child is still active, so a yielded orchestrator with children-in-flight does not display a green check today. The new state still drives a distinct badge in the narrower case where an orchestrator yields with no descendants (or with all descendants terminal) and in any future multi-level orchestration; see invariants 21–24.

## Goals
1. The client distinguishes a `wait_for_events` yield from real completion using a first-class state.
2. While yielded, the agent run stays alive, the conversation reads as non-terminal, completion notifications do not fire, and orchestration UI surfaces a distinct "waiting" badge.
3. The yield state clears automatically when the next user input or inbound event resumes the agent.

## Non-goals
1. Changing how `wait_for_events` is invoked by the model, or its tool contract from the agent's perspective.
2. Adding a new mechanism for the user to manually pause / resume a run.
3. Redesigning the orchestration pill bar visual language; the new state reuses existing badge primitives with a new color/icon.
4. Re-architecting `ConversationStatus` consumers beyond what is required to add the new variant exhaustively.

## Figma
Figma: none provided. The new badge should reuse the existing `render_avatar_with_status_overlay` / `status_icon_and_color` plumbing with a distinct color and icon for `WaitingForEvents` so it is visually distinguishable from `Success`, `Blocked`, and `InProgress`.

## Behavior

### Terms
1. A **`wait_for_events` yield** is the act of the agent calling the server-side `wait_for_events` tool. The yield ends the current model turn but does not end the run.
2. A **waiting run** is an agent run whose most recent turn ended via a `wait_for_events` yield and which has not yet received any resume input — user query or other conversation input, inbound message, inbound lifecycle event, cancellation, or watchdog timeout — that ends the waiting state.
3. A **terminal status** is one of `Success`, `Error`, `Cancelled`. These mean the run is finished.
4. A **quiescent status** is a status in which the agent is not actively streaming output. `Success`, `Error`, `Cancelled`, `Blocked`, and the new `WaitingForEvents` are all quiescent. `InProgress` is not quiescent.

### Conversation status
5. The client adds a new `ConversationStatus::WaitingForEvents` value alongside the existing `InProgress`, `Success`, `Blocked`, `Error`, and `Cancelled` values.
6. `WaitingForEvents` is **quiescent but not terminal**: the agent is not actively streaming, but the run is still alive and may resume.
7. When the current model turn ends via a `wait_for_events` yield, the conversation status transitions to `WaitingForEvents`, not `Success`.
8. When the agent resumes, the conversation status transitions back to `InProgress` and the waiting state is cleared. Any of the following triggers a resume: the user submits a new query or other conversation input; the user invokes a slash command or other action that adds a new exchange; the orchestration event stream delivers a message or lifecycle event the agent will consume on its next turn.
9. `WaitingForEvents` may only be reached from `InProgress`. It may transition to `InProgress`, `Cancelled`, `Error`, or `Success` (if a follow-up turn completes the run conventionally). It must not transition directly to another `WaitingForEvents` without re-entering `InProgress` first.
10. The waiting state is **not** durable across restart. Shutting the app down ends the wait; on the next start the conversation restores as whatever its last-exchange status implies (typically `Success`, since the yielding stream finished cleanly). The unresolved `wait_for_events` tool call stays in the transcript as an orphan, and the next outbound request triggers the server's existing supersede mechanism to synthesize the matching `Cancel`. The user can re-engage manually if they want to resume the conversation.

### Run lifecycle
11. A waiting Oz cloud agent run does not trigger CLI driver process exit. The local CLI driver only schedules its `--idle-on-complete` exit timer on a true terminal status, not on `WaitingForEvents`.
12. A waiting run may still bound its lifetime: if **no resume input arrives** within an upper bound — no user query or other input, and no inbound message or lifecycle event — the client emits an empty `WaitForEventsResult` as a new tool-call-result input on the agent's behalf, closing the unresolved `wait_for_events` call. The agent's next turn observes the empty timeout result and decides how to proceed (commonly `finish_task`, but the agent may also re-yield via another `wait_for_events`, ask the user, or take other action). The run does **not** auto-cancel on timeout; the agent owns the post-timeout decision. The upper bound is read from the server-supplied `idle_timeout_seconds` on the `wait_for_events` tool call, falling back to a built-in client default if the server did not supply one.
13. The watchdog used in (12) is distinct from the completion idle timer. A `WaitingForEvents` run does not enter the completion-idle path even when the configured `--idle-on-complete` value is shorter than the waiting watchdog.
14. The user can cancel a waiting run through any existing cancel affordance. Cancellation transitions to `Cancelled` immediately.
15. The local `ai_tasks` row reported via `LocalAgentTaskSyncModel.update_agent_task` reports `IN_PROGRESS` while the conversation is `WaitingForEvents`. The client does not report `SUCCEEDED` for a yielded conversation.

### Notifications
16. A transition into `WaitingForEvents` does not produce any notification (no toast, no badge in the notification mailbox).
17. A pre-existing stale notification for the same conversation origin (e.g. a leftover "task in progress" item) is cleared on entry to `WaitingForEvents`, the same way `InProgress` clears it today.
18. A subsequent transition from `WaitingForEvents` back to `InProgress` (because the agent resumed) does not produce a notification on its own.
19. A subsequent transition from `WaitingForEvents` to a terminal status (`Success`, `Error`, `Cancelled`) produces the same notification that the same transition from `InProgress` would have produced.
20. An orchestrator's notifications on `Success`, `Cancelled`, and `Error` fire as they do today. If the orchestrator itself reaches `Success` (or another terminal status) it is treating itself as done; the mailbox does not second-guess that based on the state of descendants. The known "orchestrator notification spam" case is the one where the orchestrator yielded via `wait_for_events` between turns — that case is already handled by (16), because the orchestrator's status is `WaitingForEvents`, not `Success`, while it is waiting.

### Orchestration pill bar and avatar badges
21. A child pill whose conversation status is `WaitingForEvents` renders with the new "waiting" badge (icon + color distinct from `Success`, `Blocked`, and `InProgress`).
22. The orchestrator pill bar badge is driven by `aggregated_orchestrator_status` over the orchestration tree. The aggregator's precedence is `InProgress > Blocked > WaitingForEvents > Error > Cancelled > Success`, with one carve-out: when the orchestrator itself yielded into `WaitingForEvents`, its own waiting state outranks any descendant `InProgress`. Rationale:
    - `InProgress` wins when the orchestrator itself is active or when no node is yielded, because something is actively streaming.
    - When the orchestrator's own status is `WaitingForEvents` but a descendant is still running, the parent's waiting state is the more useful signal: the user sees that THIS conversation is paused waiting on inbound input even while work continues in the tree.
    - `Blocked` outranks `WaitingForEvents` because a blocked node needs user action and must not be masked by a quiescent parent.
    - `WaitingForEvents` outranks terminal statuses because the parent is explicitly still alive and listening; the orchestration is not done.
23. The hover details card and the orchestration breadcrumb avatars use the same badge mapping. There is no per-surface override for `WaitingForEvents`.
24. The pill sort order treats `WaitingForEvents` as part of the "active-ish" section of the bar, not the "done" bucket. A waiting child does not drift to the right of completed siblings.

### Other client surfaces
25. The block status bar and any "is the agent thinking" indicator must not show a streaming spinner for `WaitingForEvents`. Waiting is quiescent: no spinner, no Stop button, no live-streaming affordances.
26. Conversation input is enabled while in `WaitingForEvents`: the user can submit a new query, run a slash command, or any other action that produces a new exchange. Doing so clears the waiting state and starts a new turn (transitioning to `InProgress`). The user does not need to wait for an inbound event before submitting input.
27. The block status bar may show an unobtrusive "waiting for events" affordance when the conversation is in `WaitingForEvents`. The exact copy is up to design; this spec only requires it not be styled as completion.
28. `ConversationStatus::is_done()` keeps its existing semantics — `Success | Error | Cancelled` — and so returns `false` for `WaitingForEvents`. No new helper is introduced.

### Resume and clearing the waiting state
29. The waiting state is cleared (i.e., the conversation leaves `WaitingForEvents`) when any of the following happens. The agent itself cannot self-resume from `WaitingForEvents`; resume always requires input from outside the agent's own decision-making — user input, the orchestration event stream, the client-side watchdog acting on the agent's behalf, or user cancellation.
    - The user submits a new query, runs a slash command, or otherwise adds a new exchange to the conversation. A user query is a first-class resume path — it does not require an inbound event to arrive first. The server's existing supersede mechanism emits a generic `Cancel` tool-call result for the unresolved `wait_for_events` call; the agent's next turn sees both the cancel and the new input.
    - The orchestration event stream delivers a message or lifecycle event that the agent will consume in its next turn. Same server-side supersede path as the user-input case.
    - The user cancels the run; the conversation transitions to `Cancelled`. Pending tool calls are not retroactively cancelled — the unresolved `wait_for_events` tool-call message stays in transcript history as an orphan.
    - The watchdog from (12) fires; the client emits an empty `WaitForEventsResult` on the agent's behalf, closing the unresolved call. The conversation transitions back to `InProgress` while the agent's next turn decides how to proceed (per (12)). The pending tool call is *not* orphaned by the watchdog path because the client-emitted result closes it.
30. Clearing the waiting state is observable: a transition out of `WaitingForEvents` must produce a status update event so subscribers (task sync, notifications, pill bar) re-evaluate.
31. The waiting state must not survive across distinct agent runs. Starting a new conversation never inherits `WaitingForEvents` from a prior conversation.

### Backwards compatibility and rollout
This fix requires a coordinated release across the proto contract (`warp-proto-apis`), the server (`warp-server`), and the client (`warp`). The user-visible behavior during the rollout is bounded by the following invariants.

32. The fix is gated by a server-side feature flag. When the flag is **off** for a `wait_for_events` call, the legacy behavior is unchanged: the client treats the conversation as `Success`, the CLI driver exits per `--idle-on-complete`, and the existing server-side preservation (`shouldPreserveInProgressOnClientSuccess`, `ExtendTaskIdleTimeout`) keeps the task `IN_PROGRESS` on the server. This is the pre-fix bug and is acceptable during rollout.
33. When the server flag is **on** and the client build includes the new `WaitingForEvents` support, all behavior in this spec applies. The flag flips per `wait_for_events` call (not per conversation), so an individual call's behavior is determined by the flag state at yield time.
34. The flag may flip mid-conversation. A single conversation can legitimately contain both legacy (pre-flag) and new (post-flag) `wait_for_events` yields. The new yields activate the `WaitingForEvents` flow; legacy yields stay on the existing path. The user-visible expectation is that new yields show the "waiting" badge and suppress completion notifications, while legacy yields look the same as today. Both produce the same final outcome (the run resumes correctly when the next inbound event arrives) because the server-side gates protect task state on both paths.
35. A new client receiving a transcript that contains only legacy `wait_for_events` yields (e.g. restored from a pre-flag persistence record) treats the conversation as it does today — no retroactive reclassification is attempted. The legacy server-handled tool call is opaque to the client and cannot be detected.
36. A new client receiving the new public `WaitForEvents` tool-call variant from any server with the flag on activates the full fix. There is no separate client-side feature flag.
