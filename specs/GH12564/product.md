# Memory: AI conversation restoration and passive suggestions re-materialize large payloads — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/12564
Linear context: APP-4525, APP-4650, APP-4589
Sentry: https://warpdotdev.sentry.io/issues/7259255054/
Figma: none provided; this is a performance and data-lifecycle fix with no new visual design.

## Summary
Warp should keep startup, AI conversation restore, passive suggestions, and normal terminal typing responsive even when persisted AI conversations contain very large task histories, file contexts, or tool-result payloads. Large payloads should remain available when the user explicitly needs the full conversation or file content, but background restore and suggestion flows must not eagerly re-materialize multi-GB histories.

## Problem
Large multi-agent conversations can persist FileContext and tool-result payloads that are valid conversation data but expensive to deserialize, clone, render, and serialize. Today those payloads can be rebuilt during app startup restoration, included again in passive suggestion requests after agent responses, and accumulated in long-running AI history stacks. Users experience multi-GB memory spikes and beachballs during startup or ordinary typing, even when they are not actively viewing or continuing the large conversation.

## Goals
1. Restore Warp sessions containing large AI conversations without multi-GB memory spikes or startup beachballs.
2. Keep passive prompt/code suggestions opportunistic and bounded; they should never serialize the full task history for a large conversation.
3. Preserve conversation continuity and user trust: users should not lose saved conversations, child-agent history, files referenced by prior tool results, or the ability to continue a conversation because Warp optimized memory use.
4. Keep normal terminal interactions, especially typing after long AI sessions, responsive and independent from background AI history serialization.
5. Make memory behavior observable enough that regressions can be detected before reaching users.

## Non-goals
- Deleting existing AI history or reducing the retention window for conversations as the primary fix.
- Changing the AI model behavior for normal user-initiated follow-up prompts beyond bounded context handling needed for large histories.
- Adding new visible controls, settings, or preference UI.
- Changing passive suggestion product semantics for small/normal conversations.
- Solving every possible source of high memory usage in Warp; this scope is limited to AI conversation restore, passive suggestions, and AI history stack growth involving large persisted payloads.

## Behavior
1. When the user launches Warp with session restoration enabled, terminal panes, AI blocks, agent-view state, orchestration children, and conversation-history metadata restore as they do today for ordinary-sized conversations.

2. When a restored AI conversation contains large file contents, search results, read-file results, edited-file snapshots, skill contents, or other tool payloads, Warp does not eagerly load all raw payload bytes into long-lived in-memory conversation objects during app startup.

3. Startup restore is progressive for large AI histories:
   - The user can interact with Warp while historical AI content is still being prepared or lazily hydrated.
   - Visible panes and metadata appear without requiring every hidden or off-screen AI payload to be materialized first.
   - A single oversized conversation cannot block restoration of unrelated terminal panes or unrelated conversations.

4. Restored conversation history remains understandable even when raw payload content is deferred or summarized. The user should still see the conversation title, status, relevant agent/child-agent identity, timestamps, and enough transcript structure to understand that previous file reads or tool results happened.

5. If the user explicitly opens, scrolls to, copies from, shares, or continues a part of a conversation whose full payload was deferred, Warp may hydrate the needed content then. Hydration should be scoped to the requested conversation or transcript section rather than all historical conversations.

6. If a deferred payload can no longer be loaded or exceeds the safe display/request budget, Warp degrades gracefully:
   - The transcript shows the file name, tool name, or result summary that is still available.
   - The user does not see corrupted conversation ordering, missing agent/child-agent identities, or a crash.
   - Any error or fallback is local to that payload and does not make the rest of the conversation unusable.

7. Passive suggestions triggered after an agent response completes are best-effort and bounded. The suggestion request may use recent conversation context, server conversation tokens, summaries, active task structure, and the current trigger, but it must not serialize the full task history or large raw FileContext/tool-result payloads.

8. Passive suggestions triggered by shell commands or file-change detection remain bounded by both per-file and aggregate payload limits. If relevant files or command output exceed those limits, Warp should trim, summarize, or skip passive suggestion generation instead of reading and sending unbounded content.

9. Skipping a passive suggestion because context is too large is acceptable and should be silent in the product UI. The user should not see an error toast, warning banner, or stale suggestion chip solely because Warp chose not to spend memory on a background suggestion.

10. Passive suggestions must never reintroduce stale or invalid context after trimming. If the current command, latest agent response, working directory, or conversation state changes while a bounded suggestion request is being prepared, Warp cancels or discards that request as it does for other invalidated passive suggestions.

11. User-initiated follow-up prompts remain higher priority than passive suggestions. If Warp must choose between preserving enough context for an explicit user follow-up and issuing a background passive suggestion, the explicit follow-up wins and the passive suggestion is skipped or reduced.

12. Long AI sessions should not make normal terminal typing beachball-prone. Typing in the terminal input, editing prompts, and navigating ordinary panes must not synchronously deserialize, clone, or serialize large historical AI payloads.

13. Existing small and medium conversations should continue to behave normally:
   - Startup restore should not visibly regress for ordinary sessions.
   - Passive suggestions should continue to appear when enabled and when context is within budget.
   - Conversation history should look the same when no payload is trimmed or deferred.

14. Multi-agent and child-agent conversations preserve their orchestration relationships after restart. Optimizations must not cause child agents to appear as unknown agents, detach children from parents, or lose pinned/status state.

15. The fix must be safe for corrupted, partially persisted, legacy, or cross-version conversation rows. Bad rows may be skipped or shown in a degraded state, but they must not force unbounded memory allocation during startup.

16. Memory bounding should be observable. Internal logs or telemetry should indicate when Warp trims, defers, skips, or hydrates large AI payloads, including enough reason information to debug regressions without recording raw file contents.

## Success Criteria
1. A reproduction fixture with persisted large multi-agent task/FileContext payloads no longer produces multi-GB memory spikes during app startup.
2. A passive suggestion after a large agent response does not include the full active task history or raw large file/tool-result payloads in the request.
3. Typing and prompt editing remain responsive after loading or creating a long AI history that previously caused beachballs.
4. Restored AI conversations, including orchestration children, still appear in the expected history/agent surfaces with correct metadata and relationship state.
5. Users can continue or inspect a restored large conversation without data loss; any deferred payload fallback is explicit and localized.

## Validation
1. Build a local persisted-conversation fixture containing large ReadFiles/SearchCodebase/RequestFileEdits/ReadSkill payloads and verify startup restores the session without materializing all raw payload bytes.
2. Trigger an agent-response-completed passive suggestion on a large conversation and verify the generated request stays within the configured context budget.
3. Trigger shell-command passive suggestions that reference multiple large files and verify aggregate file-reading limits are enforced.
4. Exercise a small conversation and verify passive suggestions, restore rendering, and conversation continuation are unchanged.
5. Exercise a multi-agent conversation with child agents after restart and verify pill bar/history identity state still resolves correctly.
6. Profile typing after a long AI session and verify the input path does not synchronously rebuild large AI histories.

## Open Questions
1. What exact byte budgets should ship for startup eager materialization, passive suggestion context, and on-demand hydration? The tech spec proposes defaults, but final values should be validated against real Sentry/local repro payloads.
2. Should user-initiated follow-up prompts use the same trimmed task payload path as passive suggestions when a conversation exceeds budget, or should they first attempt a server-side continuation using only the server conversation token and latest user input?
3. Which telemetry event names and sampling policy should be used for payload trimming and passive-suggestion skip reasons?
