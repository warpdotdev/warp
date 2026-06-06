# APP-4365: Use Queued Query UI for Oz Cloud Mode Queries
## Summary
When a user submits an initial or follow-up Oz cloud mode query, Warp should immediately show the same queued-query UI used for third-party cloud agents instead of inserting a bespoke optimistic user-query block. Setup-command rich content should continue to appear unchanged, and the real user query from the cloud session transcript should render normally once it arrives.
## Problem
Oz cloud mode currently uses a different pending-query presentation than third-party cloud agents. The bespoke optimistic query block requires special handling to hide the later real user-query element, making cloud mode behavior harder to reason about and causing Oz and third-party agent startup states to feel inconsistent.
## Goals
- Use one queued-query visual pattern for cloud submissions that are waiting for the cloud session to produce the real transcript.
- Keep setup-command rich content behavior unchanged.
- Let the real user query element render normally when it arrives from the cloud session transcript.
- Apply the behavior to both initial Oz cloud runs and Oz cloud follow-up runs.
## Non-goals
- Redesigning the queued-query UI.
- Changing third-party cloud agent queued-query behavior.
- Changing setup-command grouping, ordering, expansion, or collapse behavior.
- Changing the cloud submission API, follow-up API, or agent execution lifecycle.
- Adding new user controls to the queued-query card.
## Figma
Figma: none provided
Use the existing in-app third-party cloud agent queued-query UI as the reference.
## Behavior
1. When a user submits an initial Oz cloud mode query and the submission is accepted by Warp, the terminal immediately shows a queued-query UI item for that submitted prompt.
2. When a user submits an Oz cloud mode follow-up query after a cloud execution has ended and the submission is accepted by Warp, the terminal immediately shows a queued-query UI item for that submitted follow-up prompt.
3. The queued-query item for Oz uses the same visual pattern as third-party cloud agents in cloud mode:
   - The submitted prompt is shown as the user-authored query.
   - The item communicates that the query is queued or waiting.
   - The item does not show dismiss or "send now" controls when those controls are absent from the third-party cloud queued-query pattern.
   - The item uses the same user identity/avatar treatment as the third-party cloud queued-query pattern.
4. The queued-query item preserves the displayed prompt text the user expects to see. If the user submitted a cloud query through a mode prefix such as `/plan` or `/orchestrate`, the queued-query item shows the user-facing prompt form consistently with the rest of Warp's query UI.
5. Warp does not insert the bespoke Oz optimistic user-query block for initial Oz cloud mode queries.
6. Warp does not insert the bespoke Oz optimistic user-query block for Oz cloud follow-up queries.
7. Setup-command rich content remains unchanged. Any setup-command intro text, setup-command blocks, setup-command ordering, visibility, collapse state, and transitions continue to behave as they did before this feature.
8. The queued-query item does not replace setup-command rich content. If setup commands are executed while the cloud run is starting, the user sees both the queued-query state and the normal setup-command rich content in the same relative flow where pending query and setup progress are shown today.
9. The queued-query item remains visible while Warp is waiting for the cloud execution or follow-up session to become ready and no real transcript item for that submitted prompt is available yet.
10. The queued-query item remains visible after the shared session attaches if the real shared-session transcript has not yet delivered the submitted user query. Session readiness, setup-command output, progress updates, agent status updates, or generic agent output are not sufficient reasons to remove the queued-query item.
11. When the real shared-session transcript delivers the actual submitted user query, the queued-query item is removed or otherwise replaced so the user sees the query exactly once in the final transcript.
12. The real user query element from the cloud session transcript is not hidden merely because Warp previously showed a queued-query item for the same submitted prompt.
13. If the replayed or attached cloud transcript includes the submitted user query, that real user query renders using the normal transcript user-query presentation for Oz conversations.
14. If cloud session attach or replay delivers the real user query before the queued-query item has visibly rendered, Warp may skip showing the queued-query item, but the user must not see both a queued item and a duplicate real user query for the same submitted prompt at rest.
15. If the cloud submission fails before Warp accepts it, Warp should not leave behind a queued-query item for a query that was not actually queued. The user's prompt should remain available for retry according to the existing failed-submission behavior.
16. If the cloud submission is accepted but the run later fails, is cancelled, requires authentication, or hits another startup error before a real transcript item appears, the queued-query item follows the same lifecycle as the third-party cloud queued-query UI for that state.
17. Authentication, cancellation, capacity, quota, and startup error UI remains unchanged except for the absence of the bespoke Oz optimistic user-query block.
18. Starting a new Oz cloud run from an empty cloud compose state shows at most one queued-query item for the accepted initial prompt.
19. Starting an Oz cloud follow-up from a tombstone or other follow-up entrypoint shows at most one queued-query item for the accepted follow-up prompt.
20. Repeated lifecycle updates while the cloud run is starting do not insert duplicate queued-query items for the same accepted prompt.
21. If a user leaves and re-enters the relevant agent view while the cloud query is still waiting, the queued-query item remains associated with the same conversation context and does not appear in unrelated conversations.
22. Exiting the agent view or changing panes does not convert the queued-query item into a bespoke optimistic query block.
23. Queued-query UI for Oz does not affect the content or visibility of prior terminal output, prior agent responses, existing tombstones, or already-rendered setup-command blocks.
24. When the cloud run becomes live and begins streaming agent output, the transition from queued state to transcript state should feel continuous: the prompt is not lost, duplicated, or visually reordered around the first agent response.
25. The behavior is consistent between initial and follow-up Oz cloud queries. A user should not need to learn one pending-query presentation for the first cloud prompt and another for subsequent cloud prompts.
26. The behavior is consistent between Oz and third-party cloud agents wherever both are waiting for a cloud session to produce the real transcript. Any intentional differences should be limited to agent identity, iconography, or existing agent-specific transcript rendering, not the pending-query pattern.
