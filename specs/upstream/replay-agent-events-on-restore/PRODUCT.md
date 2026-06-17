# Orchestration Conversation Restore

## Summary
When Warp restarts — or when a parent conversation is re-attached via `warp agent run --conversation` — an orchestration session involving parent and child agent runs must resume as if the interruption had not occurred. Parents must receive pending events from children, event delivery must not duplicate messages already processed, and parent-child identity links must be re-established automatically.

## Problem
Orchestration conversations spanning a parent and one or more child agent runs are not durable across Warp restarts today. After a restart:
- The parent's event listeners are not restored, so no new events (lifecycle signals or messages) from children arrive.
- The event cursor is not persisted, so if event delivery is re-established, all events since the beginning of the conversation replay from sequence 0, causing duplicate delivery.
- In the driver case (`warp agent run --conversation`), the parent cannot discover children that ran on remote workers with no local DB record.

## Behavior

### Startup restoration (Warp GUI restart)
1. After Warp restarts and a previously active parent conversation is restored, the parent continues to receive lifecycle events (in-progress, succeeded, failed, blocked, cancelled, errored) and inbox messages from any children that were running at the time of the restart.
2. Any lifecycle events from children — including terminal events such as `succeeded` or `failed` — that arrived while Warp was not running are delivered to the parent once event delivery resumes. This is the primary scenario the feature addresses.
3. A parent that was in `Success` status at restart time with watched children resumes event delivery without requiring the user to take any action or for the conversation to transition back through `InProgress`.
4. A parent that was in `InProgress` at restart (i.e., Warp quit while the parent was actively running) resumes event delivery from children once the parent's current exchange completes and the parent becomes idle again.
5. A parent that has never spawned a child agent does not start any event polling after restart.
6. Child conversations whose records have a `parent_conversation_id` pointing at the restored parent are re-linked to that parent, so their status transitions continue to propagate correctly.
7. Event delivery resumes from the last event the parent had confirmed receiving; no event that the parent had already acted on before the restart is delivered again.
8. Under normal operation, each event is delivered to a parent agent at most once. A crash between the parent receiving a batch of events and acknowledging them may result in that batch being retransmitted once on restart; this is the worst-case behavior and does not cascade.
9. Transient network failures during event resumption after restart are retried automatically. The user does not need to restart Warp again to recover from a failed resume attempt.

### Driver restoration (`warp agent run --conversation`)
10. When a parent conversation is loaded from the server via `--conversation`, all children that were spawned by that conversation — including children that ran on remote workers with no record in the local database — are rediscovered and their events are delivered.
10a. If rediscovery of a specific child fails (e.g. a server error), the parent is still restored and events from any other children continue to be delivered. Partial child-rediscovery failure does not prevent the overall session from resuming.
11. After a driver restoration, event delivery from children behaves identically to invariants 1–9 above.

### V1 orchestration (local lifecycle dispatch)
12. When the client is running in legacy local lifecycle-dispatch mode, child status transitions (InProgress → Success, InProgress → Error, etc.) are still forwarded to the parent after restart without any action by the user.

### Invariants that must not regress
13. A conversation that was not part of an orchestration session (no parent, no children) is unaffected by this change — its restoration behavior is identical to before.
14. A conversation that is a shared-session viewer does not begin receiving events from a watched-event stream after restore. Shared-session viewers continue to receive updates through the session-sharing mechanism as before.
15. If a child's parent conversation no longer exists on the local machine (deleted or on another device), the child is restored as a standalone conversation with no parent; no error is surfaced to the user.
16. Removing or deleting a conversation tears down any associated event delivery state immediately — no further events are delivered after a conversation is removed.
