# Third-party harness threshold pricing
## Summary and scope
Improve provider-cost estimates by selecting pricing from each request’s input size, not a conversation’s accumulated input. Keep existing estimate surfaces and billing behavior unchanged. This is a presentational list-price estimate, not the user’s actual provider invoice.
Scope: Claude Code and Codex extraction, per-request `harness_metadata`, API parsing, server price selection, and preservation of existing token reporting. No new UI, raw-transcript backfill, or client pricing tables.
## Context
The client already reconciles Claude response identities and confirmed Codex request deltas before summing them. Request boundaries are lost during attribution grouping, not during capture. The server prices those groups using standard list prices only.
Grounded source references:
* Client [crates/warp_harness_usage/src/api.rs (86-119)](https://github.com/warpdotdev/warp/blob/1cc4edf69b02ec8f70b8692990c1080a1202c3d8/crates/warp_harness_usage/src/api.rs#L86-L119) and [crates/warp_harness_usage/src/counters.rs (145-211)](https://github.com/warpdotdev/warp/blob/1cc4edf69b02ec8f70b8692990c1080a1202c3d8/crates/warp_harness_usage/src/counters.rs#L145-L211): attribution types and grouping.
* Server [../warp-server/model/types/harness_usage.go (90-105)](https://github.com/warpdotdev/warp-server/blob/06d4783a30f1052ea722ddce22020c53e60386a0/model/types/harness_usage.go#L90-L105) and [../warp-server/router/handlers/public_api/harness_usage_conversion.go (95-122)](https://github.com/warpdotdev/warp-server/blob/06d4783a30f1052ea722ddce22020c53e60386a0/router/handlers/public_api/harness_usage_conversion.go#L95-L122): stored rows and explicit API conversion.
* Server [../warp-server/logic/ai/llm/llm.go (1800-1834)](https://github.com/warpdotdev/warp-server/blob/06d4783a30f1052ea722ddce22020c53e60386a0/logic/ai/llm/llm.go#L1800-L1834): standard-only list-price helper; threshold rates already exist in the same file.
For example, GPT-6.1 Sol has a configured threshold of 272,000 input tokens. Claude Opus 4.8 currently has no threshold configuration; do not invent a blanket Claude long-context surcharge. Model context capacity is not a pricing threshold.
## Behavior
1. Store one row per reconciled inference request, even when requests share a model or input size. Each row contains native usage counters and available model/tier/geo/speed metadata. Never turn multi-request cumulative usage into a synthetic request.
2. Derive total input separately for each request. Standard rates apply at or below the model’s configured threshold; threshold rates apply strictly above it, to the whole request’s input/cache/output categories. Many small requests cannot trigger a surcharge through their combined input.
3. Missing input components retain standard-rate fallback; do not infer size from conversation totals or context capacity. Missing counters remain unknown, not zero. Unknown models remain unpriced; preserve existing cache accounting and provider adjustments.
4. Version 2 has no duplicated aggregate `payload.usage`, no grouped `attribution`, and no `request_input_tokens`. Retain only usage not represented by request rows in optional `unattributed_usage`, without guessed model metadata. It contributes to token reporting, not cost. Derive measured totals from requests plus this non-overlapping remainder.
5. Preserve deduplication, captured scope, tool counts, transcript-upload-before-publication, and cumulative snapshot replacement/retry semantics. Version 1 snapshots retain their existing pricing path; never price both representations.
6. Bounded snapshots retain usable request rows and measured remainder, rather than failing solely because there are too many requests. Detail lost to bounds is explicitly partial and unpriced. Token/tool coverage is not a promise of complete cost estimation; add no new UI or completeness claim.
## New `harness_metadata` model
Version 2 payload:
* `requests`: required array, possibly empty. Each row has `usage` plus optional `model`, `service_tier`, `inference_geo`, and `speed`. The outer row shape is shared; usage counters remain harness-specific.
* `unattributed_usage`: optional native counters for measured usage not assigned to a retained request. Omit when absent; never duplicate request counters here.
* `toolCalls`: unchanged.
No persisted derived input size, request count, price, or aggregate total. Native identities remain client-local for deduplication and deterministic ordering. The server does not need request IDs because each publication replaces the entire retained snapshot, rather than appending rows.
Illustrative stored Codex snapshot: each array entry is exactly one request.
```json
{
  "metricsVersion": 2,
  "harness": "CODEX",
  "executionId": 123,
  "captureSequence": 7,
  "capturedAt": "2026-10-01T19:00:00Z",
  "snapshot": {
    "coverage": {"tokenStatus": "known", "toolStatus": "unavailable"},
    "payload": {
      "requests": [
        {
          "model": "gpt-6.1-sol",
          "usage": {
            "input_tokens": 200000,
            "cached_input_tokens": 100000,
            "output_tokens": 1000
          }
        },
        {
          "model": "gpt-6.1-sol",
          "usage": {
            "input_tokens": 300000,
            "cached_input_tokens": 200000,
            "output_tokens": 2000
          }
        }
      ]
    }
  }
}
```
The server derives 200K input for the first request and 300K for the second. With current GPT-6.1 Sol rates and no other adjustments, they cost $0.22 + $0.47 = $0.69. Their 500K combined input has no role in tier selection.
## Proposed changes
### Client extraction
* In `crates/warp_harness_usage/src/api.rs`, replace the producer’s aggregate/grouped payload with typed request rows and optional unattributed counters. Reuse native usage types and classification fields; remove classification-based request merging from `counters.rs`.
* In `claude.rs`, emit one row from each reconciled `(session, message.id)` response. Preserve streaming reconciliation and captured subagent scope. Partial response counters remain partial; do not sum streaming observations or merge distinct responses.
* In `codex.rs`, emit a request row only when checkpoint reconciliation establishes a single request: initial `last == total`, or a comparable cumulative delta confirmed by `last_token_usage`. Preserve the observed request counters, not the session total. Repeated checkpoints add nothing. Initial ambiguous history, skipped-request deltas, and unassignable field baselines go to `unattributed_usage`, without a guessed model. Preserve conservative handling of field changes, declining counters, and session boundaries.
* Track request-assigned and unassigned counts during reconciliation. Do not subtract unknown fields as though they were zero or add counters that overlap; totals must retain existing measured/unknown and checked-arithmetic semantics. Preserve optional Codex cache-write counters when integrating its separate client work.
### Snapshot bounds
Proposed limits: 4,096 retained request rows and the existing 1 MiB publication body cap. Other capture/identity/tool limits remain unchanged; the old 64 attribution-group limit does not apply to request rows.
Keep a deterministic prefix (Claude sorted native response identities; Codex checkpoint order). Fold excess rows’ measured counters into `unattributed_usage`, mark token coverage partial, and record resource-limit diagnostics. Apply the same policy if the encoded report exceeds the body cap, before freezing it for publication. Retry that exact representation. Excess requests are unpriced, so very long runs may have an understated estimate; do not silently revert to grouped pricing or store a truncated row as a request.
### Server parsing and consumers
* Extend `../warp-server/model/types/harness_usage.go`, `public_api/openapi.yaml`, and both directions of `router/handlers/public_api/harness_usage_conversion.go` with separate legacy and request-based payload shapes for both harnesses. Regenerate `public_api/types/types.gen.go` using the OpenAPI generator from `script/codegen`.
* Accept legacy reports as today. Identify new reports by their required `requests` array; the server assigns stored `metricsVersion: 2`. Reject null `requests`, mixed old/new payload fields, and stored version/payload mismatches. Producers still do not choose the storage version.
* Update `model/harness_usage.go` to preserve the selected format rather than stamping every envelope with version 1. Decode/read both versions, including batches. No change to identity ordering or exact-snapshot idempotency.
* In `model/harness_usage_validation.go`, count measured requests/remainder as token data. Validate nonnegative int64 counters, measured usage per row, provider counter relationships when present, coverage, tools, and request limits. Bound publication bodies to 1 MiB before reading/decoding them. An empty request array is usable only if remainder or tools provide measured data.
* Update [../warp-server/logic/run_scoring/metrics.go (183-221)](https://github.com/warpdotdev/warp-server/blob/06d4783a30f1052ea722ddce22020c53e60386a0/logic/run_scoring/metrics.go#L183-L221): version 2 output-token totals come from request counters plus remainder, using checked sums and preserving unknown versus measured zero. Keep legacy attribution/aggregate handling for version 1.
### Server pricing
* Derive request input with checked arithmetic: Claude = `input_tokens + cache_read_input_tokens + cache_creation_input_tokens`; Codex = `input_tokens` (already cache-inclusive). Do not add Claude TTL partitions, Codex cached/cache-write counters, or output tokens again. Missing components or overflow mean unknown tier input and standard-rate fallback.
* Add a threshold-aware list-price helper in `logic/ai/llm/llm.go` accepting optional request input. Read the existing `ThresholdPricing` table; select all tier rates together and derive Anthropic 1-hour cache-write pricing from the selected input rate. Keep existing `ListPriceForModel` standard-only and exclude Warp billing/promotion multipliers.
* In `logic/harness_cost/harness_cost.go`, version 2 prices each request independently using existing provider category accounting and adjustments, then sums costs. Ignore remainder. Version 1 continues pricing legacy attribution at standard rates. Models without thresholds remain unchanged.
## Compatibility and rollout
Use stored `metricsVersion: 2` because the payload meaning changes, not merely an optional classification. Preserve version 1 readers, writers, and pricing for historical rows and old producers. The existing JSONB column needs no database migration or backfill.
Deploy all server readers/writers first, then release client emission. Strict older readers cannot decode version 2, so do not roll back parser support after new rows exist. Retain both decoders during any estimator rollback. Keep existing authentication, capture identity, freshness, and transcript persistence lifecycle.
## Testing and validation
Use focused logical tests:
* **Behavior 1–3:** threshold−1/threshold/threshold+1; mixed requests; equal-size requests remain separate; many small requests whose sum exceeds the threshold; cache-inclusive tier selection; all category rates and existing adjustments; missing input, unknown model, and no-threshold model.
* **Behavior 1, 4–5:** Claude streaming duplicates/subagents; Codex confirmed/repeated/skipped checkpoints and field/session changes; non-overlapping remainder; derived output totals; unknown versus zero; optional cache-write preservation. Replay supplied transcripts as below-threshold controls, plus an above-threshold fixture.
* **Behavior 5–6:** both formats round-trip API → JSONB → read API; reject mixed/version-mismatched payloads; unchanged retry/ordering semantics; deterministic row/body bounds conserving measured counts without inventing requests or pricing remainder.
Run `cargo nextest run -p warp_harness_usage`, targeted client publication tests, and targeted Clippy. On the server, run affected tests in `logic/harness_cost`, `logic/ai/llm`, `logic/run_scoring`, `model`, `model/types`, and `router/handlers/public_api`, then targeted build/vet and applicable formatters. No full client presubmit or UI validation is needed.
## Execution approach
Implement directly after review: extraction/remainder semantics and the versioned wire contract are tightly coupled, so child agents are not proposed. Deliver one server change and one client change, with server deployment preceding client emission.
