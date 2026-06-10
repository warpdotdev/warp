# TECH: Orchestrated agents can read plans by explicit ID
## Context
Orchestrated child agents need to read parent-created plans when explicitly given a plan ID. Plan availability is backed by Warp Drive rather than inherited launch metadata or copied markdown.
## Architecture
### Publish parent plans before fan-out
Before `RunAgentsExecutor` dispatches children, it asks `AIDocumentModel` to publish every document owned by the parent conversation.
- `AIDocumentModel` keeps its cached backing identity synchronized with `CloudModel`: live `ObjectSynced` events update matching plans by stable `ai_document_id`, initial-load completion reconciles restored documents, and explicit sync/publication calls reconcile defensively before creating or waiting.
- Unbacked plans start Warp Drive creation.
- Plans already being created refresh their queued or client-backed notebook content before remaining in the wait set.
- Server-backed plans immediately queue their latest content for update instead of waiting for the normal two-second document save throttle.
- Newly created or saving plans receive concurrent, bounded waits for server backing.
- Publication failures and timeouts are logged per plan, and child launch always continues.
- Cancelling `run_agents` during publication removes the pending launch, so publication completion cannot fan out children. Cancellation after fan-out preserves the existing spawning aggregation lifecycle.
`RunAgentsRequest.plan_id` remains the orchestration-config key. It is not treated as inherited child context, and no plan IDs or plan content are added to child launch requests, prompts, server task metadata, or driver options.
### Read plans by explicit ID
`ReadDocumentsExecutor` first reads requested IDs from the local `AIDocumentModel`. If a requested plan is absent, it attempts Warp Drive hydration only when the acting conversation participates in orchestration:
- the conversation identifies itself as a child through a local parent conversation ID or remote parent run ID; or
- `BlocklistAIHistoryModel` knows child conversations for it.
Hydration uses `AIDocumentModel::hydrate_saved_plan_from_warp_drive`, then retries the complete read so result ordering and all-or-nothing behavior remain unchanged. Missing IDs in non-orchestrated conversations, or IDs still missing after hydration, return the existing clear `Document(s) not found` error.
Hydrated plans retain ordinary `EditDocumentsExecutor` behavior. No inherited-document write policy or stale-revision policy is introduced.
## Testing and validation
Focused Rust coverage verifies:
- `RunAgentsExecutor` starts publication for every plan owned by the parent conversation before dispatch, without publishing unrelated plans;
- saving-plan publication refreshes content edited after Warp Drive creation began;
- already server-backed plans do not wait when their local document has a stale client sync ID;
- cancellation during the publication wait prevents child dispatch;
- a remote child with only a parent run ID can lazily hydrate and read a requested saved plan;
- a non-orchestrated conversation retains the missing-document error;
Validation uses `cargo fmt` and targeted `cargo check`. Do not run the app, nextest, or presubmit for this change.
## Risks and mitigations
Warp Drive creation or update can be slow or unavailable.
- Publication uses bounded waits only for plans that are not yet server-backed.
- Failures and timeouts are logged per plan and never block child launch indefinitely.
Explicit-ID discovery depends on the child receiving a relevant plan ID in its task prompt.
- No implicit plan selection is attempted.
- A missing or unavailable ID produces a clear `read_plans` error.
