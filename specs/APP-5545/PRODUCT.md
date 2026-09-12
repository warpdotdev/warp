# Third-party harness metrics: client collection

## Summary
Make managed Claude Code and Codex runs publish native token and tool usage alongside their existing
transcript saves. Developers can inspect the latest usable counts through the run metrics API,
including when running against a local server with `oz-local`.

This is the client collection slice of
[APP-5545](https://linear.app/warpdotdev/issue/APP-5545/support-byollm-and-third-party-metrics).
The server storage and publication APIs are separate work. Collection does not enable a production
rollout without authenticated server capability.

## Behavior
1. **Supported runs.** Collection applies to Claude Code and Codex running through the managed
   Warp/Oz execution path, including self-hosted workers and local development of that path.
   Standalone terminal sessions, unmanaged local CLI runs, Gemini, and Warp's own model usage are
   unchanged. The feature does not require a benchmark, UI, or separate extraction job.

2. **Enablement and compatibility.** Collection is enabled only when the server supports and enables
   reporting for this execution. An older or disabled server leaves transcript saving, viewing, and
   resume functional without repeated unsupported-report requests. Production enablement is a
   separate decision; local development must be able to opt in without enabling production.

3. **What a reader sees.** A run can have no usable metrics, or a retained snapshot with native counts,
   capture time, execution identity, and coverage. No metrics means unavailable, not zero. A snapshot
   may describe a still-running session; it is not certification that a turn or task has finished.
   This feature does not add a pending/failure-attempt history to the metrics API.

4. **Native categories.** Claude's input, output, cache-read, cache-write, and available cache-write
   duration breakdowns retain their native meanings. Codex's input, cached-input, output, reasoning,
   and total token counts retain theirs. Inclusive categories and their subsets must not be
   double-added. Missing categories stay missing; in particular, Codex cache writes are not inferred.
   Available model and other native usage attribution is retained without guessing absent values.

5. **Cumulative observations, not increments.** Every report describes the captured history, not an
   amount to add to the previously stored report. Repeated records, network retries, follow-ups, and
   resumed histories must not duplicate usage. Two cumulative observations of 10 and 15 tokens do not
   mean 25 tokens. Independently occurring requests with equal usage are still distinct requests.

6. **Tool counts.** Report invocation totals and counts by original native tool name. A tool result is
   not another invocation; duplicate observations of one invocation count once. Multiple distinct
   invocations in one assistant response all count, even when response usage appears repeatedly.
   Do not translate native tool names into Warp billing or benchmark categories.

7. **Scope and completeness.** Token and tool coverage are independent: each is known, partial, or
   unavailable for the fixed scope defined by harness and metrics version. Include captured Claude
   subagent histories without claiming every possible descendant was discovered. Codex v1 covers its root rollout;
   child histories are outside that version's fixed scope. Missing files, unknown session IDs,
   denied reads, malformed records, and ambiguous identities must not silently become complete zero
   totals. A supported, readable history with no tool invocations can establish a zero tool count;
   absent token categories still remain absent.

8. **Partial observations.** A snapshot containing useful observed counts may be published with
   partial coverage. Concise diagnostic reasons remain producer-local, not in publication payloads.
   Unreliable values are omitted rather than fabricated.
   If neither token nor tool data is usable, retain the older snapshot. A newer usable partial
   snapshot may contain lower observed counts than an older one without changing its defined scope;
   do not merge the two or take per-field maxima to disguise that change.

9. **Relationship to transcript saving.** Metrics describe the same captured input used by a
   successfully uploaded raw transcript. Failed extraction does not prevent raw upload. Failed raw
   upload does not publish new metrics from that capture. Block/display snapshot failure does not
   prevent raw transcript saving or metrics reporting. Existing transcript formats and resume
   behavior remain compatible.

10. **Freshness.** Capture time describes when the retained data was observed, not when a request
    arrived or an upload completed. Retrying a capture must not make it look newer. An existing
    snapshot remains readable after a failed save, failed report, or execution replacement.
    The current transcript download may be newer than retained metrics: reproducing historical
    counts from immutable historical transcript bytes is not a guarantee of this feature.

11. **Save timing.** During activity, use the existing periodic save cadence. Session updates and
    existing completion/failure/cancellation notifications only request the same best-effort save,
    with redundant requests coalesced. Metrics are extracted from native transcripts in that save
    path, never from notification payloads; no new native notification infrastructure is required.
    Native files can still be changing at these points; incomplete observations are not labeled
    final or settled. A bounded retry can recover while the harness is idle, without requiring a
    new user message or extending the configured idle lifetime indefinitely.

12. **Exit and cancellation.** On orderly exit, failure, or cancellation, attempt a final save before
    reporting execution shutdown. This wait is bounded and respects any earlier sandbox deadline.
    A crash, forced termination, lost network, or expired deadline can leave older metrics in place.
    Metrics failure alone must not change task success/failure, block future input, or discard
    otherwise usable resume state.

13. **Execution ownership.** Only the specific authenticated execution may report its observations.
    A superseded producer must not adopt its replacement's identity to get a report accepted.
    Follow-ups within the same process continue its ordering; a new execution can start a new
    sequence while reporting the resumed cumulative history. Replacing the reporting process must
    use a new execution. Same-execution process replacement/recovery and concurrent producers within
    one execution are not supported.

14. **Operational boundaries.** Reporting must not introduce unbounded save queues, indefinite
    retries, or unlimited shutdown delays. Invalid, oversized, or overflowing counts are not
    silently clamped or relabeled complete. Metrics diagnostics must not expose transcripts,
    tool arguments/results, workload credentials, or signed storage URLs.

## Non-goals
- Costs, billed usage, credits, benchmark/scoring integration, or a metrics UI.
- Historical backfills, an unbounded per-request usage ledger, or server-side transcript parsing.
- Immutable transcript retention, new upload acknowledgements, writer generations, or a claim that
  the latest downloaded raw transcript always corresponds to retained metrics.
- Incremental file parsing, universal discovery of native subagents, or changed worker credentials.

## Review dependency
The startup contract must identify and authorize this producer's execution and communicate reporting
support. It must not enable a replacement reporting process under an existing execution ID; reading
a retained sequence cannot prove that old requests have finished. The proposed handoff is in
[TECH.md](TECH.md#phase-1-execution-bound-startup-and-transport).
