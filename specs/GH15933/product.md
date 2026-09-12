# PRODUCT.md — Notify for Codex permission requests only when a human decision is required

Issue: https://github.com/warpdotdev/warp/issues/15933

## Summary

When Codex is configured with an automatic approvals reviewer, a permission request is often answered without the user ever being asked.
Warp currently raises a needs-attention notification for every such request, so the notification stops meaning "you have to act".
This spec defines the behavior Warp should have instead: a needs-attention notification is raised for a Codex permission request only once Codex has determined that a human decision is required.

## Problem

Codex resolves a permission request through a configured reviewer:

```toml
approval_policy = "on-request"
approvals_reviewer = "auto_review"
```

With that configuration many requests are approved by the reviewer and the session keeps running, never blocking on the user.
Warp still notifies, because the only signal it receives is emitted when the request is raised rather than when it is resolved.

The cost is concentrated exactly where the notification is supposed to help.
A user running several Codex sessions cannot tell which one actually stopped for them, and turning needs-attention notifications off also silences the requests that genuinely wait for a human.
The same signal drives Warp's in-app session status, so the vertical tab and agent chip also claim attention is needed while nothing is waiting.

## Goals / Non-goals

**In scope**

- The condition under which Warp raises a needs-attention notification for a Codex permission request.
- The in-app session status shown for a request that is under automatic review rather than waiting on the user.
- The behavior required when Codex or the plugin is too old to report the distinction.
- Behavior across every value of `approvals_reviewer`, not only `auto_review`.

**Out of scope**

- Completion, failure, and idle notifications, which keep their current behavior.
- The notification surfaces themselves — desktop notification, banner, tab and footer chrome are unchanged in appearance and placement.
- Warp's own Agent Mode approvals, which do not go through the CLI agent session protocol.
- Claude Code and other CLI agents, whose plugins answer to a different upstream contract. Whether this should extend to them is left to the open questions below.
- Any change to how the user answers a permission request once they have been notified.

## Behavior

1. A Codex permission request that Codex resolves without asking the user raises no needs-attention notification, at any point in its lifetime. This covers a request approved by the automatic reviewer and a request allowed by policy without review.

2. A Codex permission request that reaches the user and waits for their decision raises exactly one needs-attention notification, subject to the existing gate in (3). The notification's title and description keep their current content: the session's query or summary, and the request's own summary text.

3. The existing suppression gate is unchanged: a needs-attention notification is raised only while the user is navigated away from the window holding that session. A request that waits for the user while its window is focused raises no notification, as today.

4. The notification is raised no earlier than the moment Codex has determined a human decision is required. A notification that is raised when the request is created, and would have to be withdrawn once the reviewer answers, does not satisfy this spec — a desktop notification cannot be withdrawn once delivered.

5. Suppression is never achieved by delaying or by guessing. A notification is withheld only on a signal that the request is being handled without the user; it is never withheld on a timer, a heuristic about the tool being requested, or on the mere presence of `approvals_reviewer` in the configuration.

6. When the reviewer declines a request and Codex then asks the user, that request raises a needs-attention notification under (2). A decline followed by Codex abandoning the action without asking the user raises no needs-attention notification.

7. When automatic review does not produce an answer — it times out, errors, or is interrupted — and Codex asks the user as a result, the request raises a needs-attention notification under (2). The user is never left silently waiting because a reviewer failed.

8. A request that waits a long time under review and only then reaches the user still raises its notification at that later moment. Suppression while a request is under review is never permanent.

9. While a request is under automatic review, the session's in-app status does not claim that the user must act. The vertical tab, agent icon, and footer do not show the needs-attention treatment they show for a request that is waiting on the user.
   **Open question:** whether a request under review presents as ordinary in-progress work, or as a distinct third state visible in those surfaces. A distinct state is more informative and is a wider change; in-progress is the smaller change and loses the information that a request was raised at all.

10. Rich input behavior follows the same distinction. Rich input closes for a request that waits on the user, because answering it needs the terminal, and stays open for a request under automatic review.

11. When several Codex sessions run at once, each session's notifications follow (1) through (10) from that session's own requests. A request under review in one session never suppresses or raises a notification for another.

12. When Codex or the plugin cannot report whether a request reached the user, Warp keeps the current behavior for that session and notifies on every permission request. A session that cannot be told apart notifies too often, exactly as today; it is never silenced. Trading a false notification for a missed one is a regression, not a partial fix.

13. A user who configures no automatic reviewer sees no change whatsoever. Every permission request reaches them, and every one of them notifies under (2) and (3).

14. The user's answer continues to clear the state a request created. Once a request is resolved by any route — the user answered, the reviewer answered, the tool ran, a new prompt was submitted, or the session ended — the session leaves the blocked presentation and its request summary stops appearing in the tab title and other surfaces that fall back to it.

15. Completion and failure notifications are untouched by this change. A session that finishes its turn notifies on completion, and one that fails notifies on failure, regardless of how any permission request within that turn was resolved.

## Open questions

- Whether Warp should surface a setting for this behavior, or whether notifying only on human-required requests is simply correct and needs no toggle. No setting is proposed here.
- Whether this treatment should extend to the other CLI agents that emit permission requests through the same protocol, once their upstreams can report the same distinction.
- The question in invariant (9), on how a request under automatic review presents in-app.
