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

2. If the failure happens before the agent has streamed any actions for the failing request, recovery is invisible: the request is re-sent and, if an attempt succeeds, the user sees a normal uninterrupted turn. This covers a failure at any point ahead of the first streamed action, including one that arrives before the response starts at all — an initial connection error or a 5xx before headers.

3. If the failure happens after actions have streamed, the conversation resumes from the server's authoritative state. Work that already executed (commands, tool calls) is never re-executed by the recovery.

4. A run that recovers completes indistinguishably from one that never failed: final state SUCCEEDED (or the normal terminal state), full output present, downstream consumers (e.g. the BDR pipeline's webhook/HubSpot write) observe a normal completion.

### Visible recovery state

5. While recovery is pending, the conversation shows a non-terminal "Reconnecting" status with an in-progress treatment (spinner-style icon, not an error icon).

6. While recovery is pending, the cloud run's task state remains IN_PROGRESS with the status message "Connection lost while receiving the agent response; attempting to resume." The run state never flaps through a terminal ERROR that would tear down the execution mid-recovery.

7. Run lists, the orchestration pill bar, and parent-agent aggregations count a reconnecting conversation as working/in-progress, not failed.

8. No error notification, desktop notification, or conversation-ended tombstone is shown while recovery is pending; stale notifications for the conversation are cleared, as they are when a turn starts. A notification fires only on the eventual terminal outcome.

### Bounded failure

9. Recovery is bounded by one budget of 3 attempts per request, shared between in-request retries and automatic resumes. A resumed request may itself be recovered, but only out of what is left of that budget — so a failing request is recovered at most 3 times, however those attempts are split between retries and resumes. The budget is per request, not per turn: a turn spans many requests (every tool-result round trip is its own) and each starts with a full budget, as it did before retries and resumes were unified. ([REMOTE-2269](https://linear.app/warpdotdev/issue/REMOTE-2269/allow-multiple-resume-attempts) raised this from "at most one automatic resume", which left a post-action failure with an effective budget of one attempt.)

10. If recovery is exhausted, the run ends with a terminal error and the message "Warp lost connection while receiving the agent response. This is usually temporary." There is no retry storm: each attempt waits a jittered exponential backoff first (~0.5s, ~1s, ~2s), so a persistent outage produces at most 3 spaced attempts before the terminal failure rather than an immediate re-send into the same failure window.

11. A cloud run held open for recovery waits at most 120 seconds per recovery attempt: the deadline is armed when an attempt fails and cancelled when the next one lands, so a request that recovers repeatedly is not killed by the cumulative wait. If a single attempt does not restore progress within that window, the run ends with the last recorded error.

12. Application-level failures are never auto-recovered: out-of-credits and server-overload failures end the turn immediately with their specific messages (a recovery attempt would fail identically or add load the server shed). Non-transient errors (4xx, malformed responses) likewise fail immediately.

### Offline behavior

13. If the client is offline when a pre-action failure occurs, the retry waits for connectivity to return instead of failing, showing the "Reconnecting" state while parked. The retry fires automatically when the client comes back online. A parked retry waits for connectivity rather than the backoff, since the backoff exists to space out attempts against a struggling server.

14. An automatic resume likewise waits for connectivity before sending, after its backoff.

### Cancellation and interaction during recovery

15. Cancelling a conversation while recovery is pending takes effect immediately: the conversation shows Cancelled, and no recovery attempt fires afterward.

16. Sending a new message to a conversation with a pending resume replaces the recovery: the pending resume is dropped and the new request proceeds normally. (While a retry is parked the original request is still logically active, so new messages queue as usual.)

17. Passive background requests (e.g. automatic code-diff suggestions) never auto-resume; their failures are silent and terminal as before.

### Limits and adjacent surfaces

18. Recovery does not survive an app restart: a conversation restored from disk mid-recovery restores with a terminal Error status.

19. Recovering conversations are treated as in-progress for queued prompts and follow-up gating; when a recovering conversation reaches a terminal state, the same finished-conversation handling runs as for any in-progress conversation (queued prompts fail/clear appropriately).
