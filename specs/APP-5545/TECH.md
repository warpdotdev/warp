# Third-party harness metrics: client implementation

## Context
Implement [PRODUCT.md](PRODUCT.md) in the Warp Rust producer. These specs guide the client stack; server
storage/authentication/API work remains owned by the separate server threads. The broader issue is
[APP-5545](https://linear.app/warpdotdev/issue/APP-5545/support-byollm-and-third-party-metrics).

Inspected Warp revision: `6f575836c02bd80a4b2de2755e952bec1793d3df`.
- [Claude save and upload, `claude_code.rs:555-651`](https://github.com/warpdotdev/warp/blob/6f575836c02bd80a4b2de2755e952bec1793d3df/app/src/ai/agent_sdk/driver/harness/claude_code.rs#L555-L651)
  reads a native envelope on a blocking worker and uploads it, joined with a block snapshot.
- [Codex save and upload, `codex.rs:426-521`](https://github.com/warpdotdev/warp/blob/6f575836c02bd80a4b2de2755e952bec1793d3df/app/src/ai/agent_sdk/driver/harness/codex.rs#L426-L521)
  does the equivalent for one rollout, skipping upload if its session/path is still unknown.
- [JSONL reader, `claude_transcript.rs:432-467`](https://github.com/warpdotdev/warp/blob/6f575836c02bd80a4b2de2755e952bec1793d3df/app/src/ai/agent_sdk/driver/harness/claude_transcript.rs#L432-L467)
  treats missing files as empty and skips malformed lines. Both harnesses use it.
- [Driver save lifecycle, `driver.rs:3173-3292`](https://github.com/warpdotdev/warp/blob/6f575836c02bd80a4b2de2755e952bec1793d3df/app/src/ai/agent_sdk/driver.rs#L3173-L3292)
  awaits periodic/final saves; [session updates, `driver.rs:4116-4145`](https://github.com/warpdotdev/warp/blob/6f575836c02bd80a4b2de2755e952bec1793d3df/app/src/ai/agent_sdk/driver.rs#L4116-L4145)
  spawn saves independently. The cadence is 30 seconds. Final-save success also influences cleanup.
- [`HarnessSupportClient` and startup types, `harness_support.rs:236-371`](https://github.com/warpdotdev/warp/blob/6f575836c02bd80a4b2de2755e952bec1793d3df/app/src/server/server_api/harness_support.rs#L236-L371)
  provide authenticated task APIs, but no metrics call or startup reporting context.

Server references are the published [storage PR #17056](https://github.com/warpdotdev/warp-server/pull/17056)
at `47513a4e089bc1ef7312b708f9f7706c0dea96d3` and
[API PR #17057](https://github.com/warpdotdev/warp-server/pull/17057) at
`48f345143bf0364334fb2260b926b857241956f4`. Their
[successful-snapshot refactor](https://github.com/warpdotdev/warp-server/blob/47513a4e089bc1ef7312b708f9f7706c0dea96d3/specs/APP-5545/TECH.md)
supersedes the original spec's writer-generation, immutable-source, and failure-report protocol.
The original [native counting requirements](https://github.com/warpdotdev/warp-server/blob/02b1f4ff96977153f8e13ca58c4acde4cc552647/specs/3p-run-metrics/TECH.md#L35-L67)
still inform extraction. Reconcile final server changes before integrating, rather than copying
uncommitted work from another thread.

## Proposed changes
### Phase 1: Execution-bound startup and transport
Add Rust transport types and a publication method to `app/src/server/server_api/harness_support.rs`.
Reuse the existing task authentication, run headers, workload token, and HTTP client. Do not add a
secret, decode execution identity from an unverified token, or modify generic shutdown authentication.

The [current HTTP schema](https://github.com/warpdotdev/warp-server/blob/48f345143bf0364334fb2260b926b857241956f4/public_api/openapi.yaml)
accepts `POST /api/v1/harness-support/harness-usage` with:
- `metrics_version` (1), `harness` (`CLAUDE_CODE` or `CODEX`), `execution_id`,
  positive `capture_sequence`, and read-start `captured_at`.
- `snapshot`: an object `payload`; `coverage` with independent `token_status` and `tool_status`,
  and no other fields. Payload contains only native metrics and necessary metric breakdowns.
- Status `accepted`, `ignored_older_capture`, or `idempotent`, plus retained execution/capture identity.

Metrics version covers both native counting and wire format. Scope is fixed per harness/version;
missing expected inputs degrade coverage rather than change scope. Reason counts and native
session/root/subagent/captured-scope identifiers stay producer-local, never in a publication request
or retained metrics at any nesting level. A thin wire adapter omits these local extraction fields.

The authorized read is `GET /api/v1/agent/runs/{runId}/harness-usage`. Its `usage` object contains the
stored envelope, whose field naming differs from the snake-case publication DTO. Use explicit
transport types and wire tests, not serialization of the stored envelope as a request. The read
endpoint is for inspection, not a per-save prerequisite.

**Server handoff required before live enablement:** add optional reporting context to the existing
authenticated startup/resolve-prompt exchange. Proposed shape: `harness_usage` containing
`metrics_version` and `execution_id`.
Missing/null means unsupported or disabled. Exact naming and error codes must be agreed with the
API owner; these fields do not exist in the inspected published response. The response must validate
credentials against that execution, not merely return the newest execution after generic auth.
The client binds resolve-prompt and reporting to the same explicit task identity. Current server
startup does not advertise the capability, so publication remains disabled until this handoff.
The server must also expose a stable nonretryable status for same-identity conflicts: the client
disables on HTTP 409/412, never parses error prose, and can only apply bounded retries to a generic
HTTP 500 response from an unmapped precondition failure.

Consume the context on fresh and resumed managed runs. Paths that do not perform authenticated
server startup remain reporting-disabled. A new execution starts at 1; a process retains its counter
across follow-ups. Replacing that reporting process requires a new execution; same-execution recovery
is disabled in v1. The startup/lifecycle owner must enforce this boundary rather than issue reporting
context to competing processes. A saved sequence alone cannot prove an old request has finished, so
do not request or use `last_capture_sequence`. Do not infer a current ID from retained old metrics or
add a writer registry. Missing safe context disables reporting with a bounded diagnostic, not runs.

### Phase 2: Capture diagnostics and pure extractors
Introduce the focused `crates/warp_harness_usage` library with shared typed results and Claude/Codex
extractors. Extraction borrows captured records and diagnostics and returns usable native metrics or
an unavailable outcome; it does no I/O and cannot call the server. The library also provides JSONL
reader diagnostics. Filesystem capture adapters remain app-owned and are wired in the publication
layer without changing native envelope types. Capture time is taken at read start. This is one frozen
in-memory observation, not an atomic snapshot of every file while the native CLI keeps appending.

Extend the JSONL capture path to distinguish missing/unreadable files, an incomplete trailing record,
malformed interior records, and successfully read records. Preserve the existing tolerant interface
for unrelated callers. Pass diagnostics separately; do not change the raw envelope format merely to
carry metrics. Do not reread files for extraction after serialization.

Claude uses the existing root/subagent envelope. Record incomplete subagent discovery/read failures
instead of claiming full coverage; preserve successfully captured records where raw-save compatibility
permits. TODO files are not usage input. Codex version 1 uses its existing root rollout; it does not
recursively search for child histories. Unknown Codex
session/path is unavailable and retryable, never evidence of zero activity.

**Claude accounting**
- Key response usage by native session/response identity, independently of content-block identity.
  For compatible evolving duplicates, choose the last valid complete usage observation in source
  order. Do not sum copies or construct a vector from independent field maxima.
- Exclude synthetic/non-provider records. Missing response IDs and conflicting duplicates reduce
  token coverage; do not invent an identity or exact aggregate for ambiguous records.
- Sum observed `input_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens`,
  `output_tokens`, and the available 5-minute/1-hour `cache_creation` partitions separately.
  Partitions describe aggregate cache writes; they are not additional writes.
- Preserve attributable model, service-tier, inference-geo, and speed classifications where present.
  Never attribute unknown history using only the final configured model.
- Count `tool_use` blocks by session/tool-use ID across all captured records. Multiple distinct
  blocks remain distinct even when their response's usage was deduplicated. Results add no calls.

**Codex accounting**
- Recognize `token_count` usage events, `turn_context`, and `response_item` invocations of type
  `function_call` or `custom_tool_call`.
- Within a continuous cumulative-counter segment, use the last valid `total_token_usage`, not the
  sum of checkpoints. Identical repeated checkpoints add nothing. Do not deduplicate distinct
  requests merely because their `last_token_usage` values happen to be equal.
- Reconcile successive totals with `last_token_usage` before deriving attributable deltas. Use the
  applicable turn context. Earlier cumulative baseline usage stays unattributed when necessary.
- Split segments only at an explicitly supported native reset/session transition. A decrease alone
  can be replay, compaction, or corruption: mark unresolved usage partial rather than claiming an
  exact lifetime total. Do not sum unexplained segments or substitute per-field maxima.
- Preserve `input_tokens`, `cached_input_tokens`, `output_tokens`, `reasoning_output_tokens`, and
  `total_tokens`. Cached input and reasoning output are subsets, not additional total tokens;
  do not manufacture cache writes.
- Deduplicate both tool variants by session/call ID. Missing IDs, conflicting names for one ID, and
  unsupported invocation structures degrade tool coverage; outputs do not increment it.

Both adapters return separate token/tool coverage and bounded producer-local reason codes. Publish only
when at least one category has reliable observed data, including an established empty tool count.
Partial totals cover observed valid subsets; absent native fields remain optional. Preserve exact
integers above 2^53, use checked arithmetic within the server's signed-64-bit bounds, and omit unusable
values with degraded coverage rather than clamping. `toolCalls.total` and `toolCalls.byName` describe
the same deduplicated invocation set.

The server treats `payload` as an object, not a provider-specific generated schema. Before landing
extractors, fix compact v1 fixtures for native totals, model/unattributed attribution, and `toolCalls`
with the server owners. Preserve provider field names and avoid an unbounded per-request ledger.
Publication body cap: 1 MiB, or a smaller final API limit. Bound local scope/collection/identifier
state; an oversized report is a diagnosed non-publication, not silent truncation of
counts or coverage. Do not apply this metrics-body cap to existing raw transcript uploads.

### Phase 3: One ordered save helper
Use one helper per Claude/Codex runner, with one owned background save operation, one coalesced
fresh-save request, and a closing flag. It owns sequence allocation and pending retry state.
It is not the workspace checkpoint coordinator and must not change handoff checkpoint semantics.
Keep Gemini behavior unchanged if adapting shared runner interfaces.

The helper's flow is:
1. Acquire the single save slot; capture once on the existing blocking worker.
2. Derive serialized raw bytes and metrics from that captured envelope. Discard parsed records once
   they are no longer needed. Retain only the current retry bytes/report, not a history of captures.
3. Upload raw bytes using the existing signed target path.
4. After successful raw upload, publish a usable metrics result with this capture's identity/time.
   Extraction failure still allows raw upload; no usable result means no POST, not a failure report.

Block snapshots may run alongside this flow, but await independent outcomes instead of fail-fast
`try_join!`. Keep transcript-persistence success distinct from metrics success: the latter must not
change the driver's cleanup disposition or erase resumable state.
The final coordinator freezes the raw/block result before awaiting publication under the residual
deadline. A metrics timeout cannot convert successful persistence into failed cleanup. Incomplete
reads can trigger bounded fresh captures; retain the most recent successful capture if a later read
fails. Missing required or wholly unreadable roots cannot replace raw history with an empty capture.

Allocate a positive sequence before each new capture; gaps are allowed. A retry retains the exact
report, sequence, and timestamp. Retrying an upload retains its bytes, even if a new signed target
is necessary. A metrics-only retry must not re-upload an older raw capture after a newer one.
Never add stored server totals to newly extracted totals.

Use a bounded retry policy for network failures, 429s, and retryable server errors, respecting
`Retry-After` within the remaining budget. Proposed defaults for review: at most 3 attempts per
operation, 1s/2s backoff with jitter, and a 10s metrics-request timeout. A Retry-After exceeding
10 seconds ends that idle retry instead of prolonging the operation indefinitely. Raw uploads keep existing
transport behavior within the enclosing save/shutdown budget. Avoid nested retries multiplying
attempts; the helper owns retry scheduling.

Retries can run while idle even though ordinary periodic saves skip idle sessions. After attempts
are exhausted, release that capture; do not re-arm it indefinitely on every timer tick. A later
event/periodic request can take a fresh capture. Retry an incomplete read by taking a new capture,
not changing an existing report. Coalesced fresh requests proceed after the bounded current attempt.

Treat accepted/identical-retry responses as completed. An `ignored_older_capture` response finishes that attempt without
overwriting anything; never copy a returned replacement execution ID into the producer. Conflicting
same-identity content or loss of execution authorization stops metrics publication for that producer
and surfaces a bounded diagnostic. Validation errors are not transient retries. Unsupported/disabled
reporting disables further reports; ordinary authentication refresh still uses the existing client.
Keep HTTP status/error classification available to the helper instead of parsing error strings.

### Phase 4: Driver lifecycle and rollout
Route periodic and post-turn saves through the helper, maintaining session-update work such as
Codex ID discovery and Claude bridge acknowledgements before the associated capture. Request a
coalesced save from the existing completion/failure/cancellation handler as well; do not rely solely
on `SessionUpdated`. These events only trigger saving; all counting uses the captured transcript.
Do not add plugin hooks or a separate notification-based extractor. Periodic requests must not await
the network inside the driver's event-selection loop, delaying follow-ups, exit escalation, or
runtime-error handling.

After process termination, mark the helper closing, settle/cancel bounded pending work, then attempt
one final fresh capture before cleanup and execution-shutdown reporting. Reject late ordinary save
requests after closing. Proposed final-save budget: 30s total, including pending work and final
capture/upload/report, capped by any earlier sandbox deadline. There is no extra full retry budget
after this deadline. Structure cancellation so detached work cannot begin a new publication after
the helper returns; already-issued requests may still finish remotely and remain subject to server
execution checks. A blocking read that outlives cancellation must not schedule subsequent I/O.

No stability probes certify that native writing has stopped. Post-turn capture can be partial; final
saving is best-effort even when the task is already marked successful. Metrics failures do not
override the harness result or prevent status/shutdown delivery.

Ship server support first. Use server capability rather than a staging-only code check; missing
capability preserves the existing save path. Disabling reporting stops new metrics requests without
changing raw upload/download/resume or deleting retained metrics. Update the Warp/Oz executable or
sidecar containing the producer, not a separate `oz-agent-worker` extraction job.

Keep capture/extraction/publication timing, byte size, and bounded outcomes observable. Follow the
repository's logging guidance; do not log raw records, tool arguments/results, credentials, or signed
URLs. Never use tool names/session IDs as metric labels. Initial extraction is O(records + tool
blocks), added to the existing O(transcript bytes) read; measure it before considering incremental
parsing or promising negligible latency.

## Testing and validation
The following are implementation acceptance criteria, not tests claimed to have run. Use compact
synthetic fixtures, separate Rust test files, and controlled time/
mocked transport for sequencing rather than broad end-to-end tests for every permutation.

- **PRODUCT 3-8:** Claude fixtures cover evolving/conflicting duplicates, multiple tool blocks, native
  categories/partitions, attribution, missing IDs, and root/subagent boundaries. Codex fixtures cover
  repeated totals, equal-sized distinct requests, attributable deltas, explicit reset versus unexplained
  decrease, model changes, and both invocation variants. Cover zero versus missing, integers above 2^53,
  overflow, partial files, and malformed/unknown usage records. Do not commit private transcript exports.
- **PRODUCT 2, 9-14:** Helper tests cover coalescing, a slow save with responsive driver events, failure
  independence, upload-success/report-failure retry, late appends, idle retry exhaustion, final draining/
  timeout, and no publication after closing. Assert capture bytes and metrics share input and retry
  identity/time is stable. Metrics errors must not change cleanup/resume disposition.
- **PRODUCT 2, 10, 13:** Transport tests cover wire field names, exact integers, response statuses,
  disabled/old startup responses, schema mismatch, limits, rejected superseded credentials, and a
  counter retained across same-process follow-ups but reset only for a new execution. There is no
  same-execution recovery path. Reuse server auth tests rather than reimplementing token minting in Rust.
- **Build:** run repository formatting and the Clippy configurations in `script/presubmit`, focused
  `cargo nextest` tests, and build the producer. Cover native macOS development plus Linux worker
  compilation/smoke behavior before rollout; retain existing non-native cfg/trait compatibility.

### Local end-to-end acceptance
1. Use a server checkout containing the final storage/API/startup changes, apply its normal migration
   procedure, and enable `third_party_harness_usage_reporting` locally. Do not reset existing databases.
2. Build the local producer (`cargo build -p warp --bin warp`) and point the server's
   `./script/oz-local up --detach --wait --worker-backend direct --oz-path <absolute-built-binary>`
   at that build. Resolve its actual output path if Cargo uses a shared target directory. Follow the
   server's `docs/oz-local/README.md` for prerequisites, credentials, and supported run submission.
3. Launch managed Claude and Codex runs through the local run API, selecting `harness.type` of
   `claude` or `codex`. Exercise several tools and a run longer than the 30s cadence. Use a managed
   execution, not an arbitrary standalone CLI session.
4. Read `/api/v1/agent/runs/{runId}/harness-usage` during activity and after final saving. Verify
   `ai_conversation_metadata.harness_metadata` for the linked conversation if needed. Confirm the
   selected producer binary is actually running; starting a new server with an old binary is not
   end-to-end collection.
5. Compare with a controlled, preserved capture and confirm normal transcript viewing/resume still
   works. The latest downloaded transcript is not guaranteed to reproduce an older retained snapshot.
   Local raw uploads may use configured cloud storage, not a local directory; `local_fs` does not
   change that. Record capture size and added extraction/report latency without exporting private data.
6. Exercise a same-process follow-up, new-execution resume, and one failed upload/report. Verify no
   double-addition, no failure-to-zero conversion, unchanged retry time, and eventual newer reporting.
   Verify the configured final-save timeout without changing task outcome.
7. Default `oz-local` bypasses workload-token verification. Test that boundary separately with the
   bypass disabled; do not claim default local collection validates execution-token security.
   Repeat a smoke test on a Linux worker with the updated producer before production enablement.

No UI/browser recording or benchmark trial is required. Tests requiring services, credentials, or
another platform must report blockers rather than claiming synthetic tests cover the live path.

## Parallelization
Use three local agents in independent worktrees, based initially on the updated spec branch. Keep
the existing spec PR #15926 at the bottom and publish three implementation layers above it with
`gh stack`. Tests ship with their logical change, not in a fourth validation layer.

1. **metrics-extractors:** native counting, reusable read diagnostics, bounded coverage, and fixtures.
   Branch `varoon/harness-usage-extractors`, worktree `../warp.varoon-harness-usage-extractors`.
   Own `crates/warp_harness_usage` and its workspace registration. Keep the domain library independently
   consumable without exposing app internals or suppressing dead-code checks. No live reporting or
   changes to existing save behavior.
2. **metrics-saves:** ordered/coalesced save ownership, independent raw/block results, driver triggers,
   and bounded closing/final saves. Branch `varoon/harness-usage-saves`, worktree
   `../warp.varoon-harness-usage-saves`. Own the shared save helper, driver lifecycle, and runner save
   interfaces, but not extraction or HTTP transport. Preserve cleanup/resume and Gemini behavior.
3. **metrics-publish:** authenticated startup/transport, counting from the saved capture, report
   retries, and end-to-end integration. Branch `varoon/harness-usage-publish`, worktree
   `../warp.varoon-harness-usage-publish`. Own `server_api/harness_support.rs` and app-owned native
   capture adapters that preserve the tolerant readers and raw envelope formats; edit driver/runner
   integration only after the save agent hands it off. Do not edit the separately owned server code.

Local execution lets agents exchange committed branches without uploading intermediate code. The
extraction and save agents work independently and send interface decisions early; publication first
prepares transport against the documented server contract. The lead rebases saves onto extraction,
then publication onto saves, resolves interface seams, and reviews the combined behavior. The lead
alone manages shared `gh stack` state and PR descriptions. Agents return commits, changed paths, and
validation results before their worktrees are cleaned.

Serialize heavy Cargo validation using one existing build cache rather than three parallel caches.
Each code layer must pass required checks before publication. Live local validation additionally
requires the server startup handoff; missing capability must not be bypassed to claim success.

## Decisions to confirm in review
- The optional startup response shape and enforcement that a replacement producer gets a new execution.
- Native payload/coverage fixtures, published collection/body limits, and status/error mapping before
  wiring the client to the final API.
- The proposed retry/request/final-save budgets, which are new defaults rather than measured SLAs.

Storage ordering, auth enforcement, and schema changes are not implemented in this client PR. If the
server contract changes during review, update this spec before implementation rather than adding a
second competing provenance protocol.
