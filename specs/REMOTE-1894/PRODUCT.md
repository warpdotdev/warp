# Resume cloud agent runs on transient transport failures

Linear: [REMOTE-1894](https://linear.app/warpdotdev/issue/REMOTE-1894/oz-cloud-runs-error-mid-execution-from-connectivitytransport-failures)

## Summary

Oz cloud runs (and local agent conversations) automatically recover from transient network/transport failures mid-turn instead of dying with a terminal error. While recovery is in flight the run visibly reports that it is reconnecting; recovery is bounded so a persistent outage still produces a clean terminal failure.

## Problem

Emily reported blank BDR outreach emails from the two-stage Oz pipeline (#feedback-platform): when either stage's cloud run errors, the email body is never written to HubSpot and the rep sees a blank template. Roland traced the blank emails to Oz runs on oz-dev/staging being killed mid-execution by transient transport failures, with broken recovery on the client:

- `ConnectionReset` mid-response killed runs with no retry at all ("MultiAgent request failed after 0 retries").
- A run that did log "retrying (attempt 1/3)" still failed immediately — the retry never actually recovered the run.
- `peer closed connection without sending TLS close_notify` (UnexpectedEof) killed a run the same way.

These are transient transport blips: a fresh request would almost certainly have succeeded, so the runs should have survived.

## Goals / Non-goals

Goals:

- A single transient transport failure mid-run never produces a failed run.
- Recovery state is visible (not silent) and bounded (no infinite retry loops).

Non-goals:

- Session-share initialization failures (bb8 pool timeouts) — tracked separately in [REMOTE-1878](https://linear.app/warpdotdev/issue/REMOTE-1878).
- Silent conversation death with no error emitted — tracked in [REMOTE-1881](https://linear.app/warpdotdev/issue/REMOTE-1881).
- Server-side LLM/turn retries (landed separately in warp-server #11754/#11755/#11770).

## Behavior

"User" below means both the operator observing a cloud run (Oz UI, run state API, pipeline webhooks) and the Warp user watching a local agent conversation.

### Recovery on transient failure

1. When the agent response stream fails mid-turn from a transient network/server failure (connection reset, TLS close_notify EOF, truncated response, 5xx, request timeout), the conversation automatically recovers and continues. A single such failure never produces a failed run.

2. If the failure happens before the agent has streamed any actions for the turn, recovery is invisible: the request is re-sent (up to 3 times) and, if an attempt succeeds, the user sees a normal uninterrupted turn.

3. If the failure happens after actions have streamed, the conversation resumes from the server's authoritative state. Work that already executed (commands, tool calls) is never re-executed by the recovery.

4. A run that recovers completes indistinguishably from one that never failed: final state SUCCEEDED (or the normal terminal state), full output present, downstream consumers (e.g. the BDR pipeline's webhook/HubSpot write) observe a normal completion.

### Visible recovery state

5. While recovery is pending, the conversation shows a non-terminal "Reconnecting" status with an in-progress treatment (spinner-style icon, not an error icon).

6. While recovery is pending, the cloud run's task state remains IN_PROGRESS with the status message "Connection lost while receiving the agent response; attempting to resume." The run state never flaps through a terminal ERROR that would tear down the execution mid-recovery.

7. Run lists, the orchestration pill bar, and parent-agent aggregations count a reconnecting conversation as working/in-progress, not failed.

8. No error notification, desktop notification, or conversation-ended tombstone is shown while recovery is pending; stale notifications for the conversation are cleared, as they are when a turn starts. A notification fires only on the eventual terminal outcome.

### Bounded failure

9. Recovery is bounded: at most 3 in-request retries before actions have streamed, and at most one automatic resume after actions have streamed. A resumed request does not auto-resume again.

10. If recovery is exhausted (the resume also hits a transient failure, or pre-action retries run out while online), the run ends with a terminal error and the message "Warp lost connection while receiving the agent response. This is usually temporary." There is no retry storm: a persistent outage produces exactly one resume attempt before the terminal failure.

11. A cloud run held open for recovery waits at most 120 seconds; if recovery has not restored progress by then, the run ends with the last recorded error.

12. Application-level failures are never auto-recovered: out-of-credits and server-overload failures end the turn immediately with their specific messages (a recovery attempt would fail identically or add load the server shed). Non-transient errors (4xx, malformed responses) likewise fail immediately.

### Offline behavior

13. If the client is offline when a pre-action failure occurs, the retry waits for connectivity to return instead of failing, showing the "Reconnecting" state while parked. The retry fires automatically when the client comes back online.

14. An automatic resume likewise waits for connectivity before sending.

### Cancellation and interaction during recovery

15. Cancelling a conversation while recovery is pending takes effect immediately: the conversation shows Cancelled, and no recovery attempt fires afterward.

16. Sending a new message to a conversation with a pending resume replaces the recovery: the pending resume is dropped and the new request proceeds normally. (While a retry is parked the original request is still logically active, so new messages queue as usual.)

17. Passive background requests (e.g. automatic code-diff suggestions) never auto-resume; their failures are silent and terminal as before.

### Limits and adjacent surfaces

18. Recovery does not survive an app restart: a conversation restored from disk mid-recovery restores with a terminal Error status.

19. Recovering conversations are treated as in-progress for queued prompts and follow-up gating; when a recovering conversation reaches a terminal state, the same finished-conversation handling runs as for any in-progress conversation (queued prompts fail/clear appropriately).
