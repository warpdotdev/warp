# Spec: Consolidate agent-run and conversation terminology (APP-4945)

This document is the terminology survey and naming-consolidation contract for
APP-4945. It is intentionally a proposal only: the spec PR must not change
production source, persisted data, or public APIs. The implementation phase
will reuse this PR branch and add the smallest migration that satisfies every
criterion below.

## PRODUCT

### Summary

Warp currently uses overlapping names for harness, execution location, storage,
server identity, and access role. The overlap is user-visible in conversation
navigation, pane state, CLI output, icons, continuation, and task management.
The most dangerous shorthand is `task_id.is_some() == cloud`: local runs now
receive server task IDs, so that predicate misclassifies local Warp-agent runs
and local third-party harness runs.

This proposal gives each concept one axis, retires ambiguous “ambient agent
conversation” / “cloud conversation” uses as general predicates, and stages
the migration so persisted fields, API identifiers, environment variables, and
CLI compatibility are not broken.

### Key design choices

1. Treat harness, execution location, storage, identity, access/ownership, and
   launch source as independent dimensions; do not encode one dimension in
   another object's presence.
2. Use `LocalExecution` and `RemoteExecution` in code, reserving “runs in the
   cloud” for Warp-hosted user copy. A remote/self-hosted worker is still remote
   execution but is not necessarily Warp cloud infrastructure.
3. Keep `Harness::Oz` and wire slugs such as `oz` as compatibility identifiers
   while canonical display copy becomes “Warp agent”; introduce explicit
   location metadata/helpers before undertaking broad mechanical renames.

### Behavior invariants

1. A conversation's harness is one of `Warp agent`, `Claude Code`, `Codex`,
   `Gemini CLI`, `OpenCode`, or a future/unknown harness. “Oz agent” is not
   canonical user-facing copy, although `Oz` remains a compatibility spelling
   until all persisted and external consumers migrate.
2. A run has an explicit execution location: `LocalExecution` or
   `RemoteExecution`. A local run may be server-observable and may have a
   server task identity.
3. A conversation's storage capabilities are independent: it can be
   local-only, cloud-only, or both locally and in the cloud. Storage state never
   changes merely because execution moved or a viewer opened the transcript.
4. `task_id`/`run_id` identifies a server-backed task/run. Presence of that
   identity never, by itself, decides execution location, storage location,
   harness, or access role.
5. Shared-session viewers and remote-child placeholders do not claim ownership
   of task status updates. Local children execute locally and are not treated as
   remote merely because they have server task IDs.
6. A launch/source label such as `Interactive`, `CLI`, scheduled, integration,
   or `CloudMode` describes how the run was launched, not where its worker
   executes or where its transcript is stored.
7. User-facing UI, CLI, telemetry labels, and documentation use the canonical
   terms above. Existing API, persistence, environment, and command aliases
   continue to parse during the migration and are explicitly marked legacy.
8. A conversation may be resumed, forked, shared, or viewed without changing
   the meaning of its local conversation ID, server conversation token, or
   server task identity.

## TECH

### Context at the surveyed commit

The survey was performed at Warp commit
`88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`.

- `app/src/ai/agent/conversation.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  documents that a local conversation gets
  `task_id` from `StreamInit.run_id`, while a remote child placeholder gets it
  from `SpawnAgentResponse.task_id`; the conversation also carries
  `is_viewing_shared_session` and `is_remote_child`.
- `crates/persistence/src/model.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  persists `AgentConversationData` locally,
  including server conversation token, `run_id`, harness metadata, parent IDs,
  and `is_remote_child`. Local conversation/task persistence is distinct.
- `app/src/ai/blocklist/local_agent_task_sync_model.rs:with_local_conversation @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  deliberately accepts owned local conversations with task IDs, while excluding
  shared-session viewers and remote-child placeholders. Its tests cover local
  owned runs, missing IDs, viewers, remote children, and local third-party CLI
  mappings.
- `app/src/ai/agent_conversations_model/entry.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  currently combines
  `LocalInteractive`, `AmbientRun`, and `CloudSyncedConversation` provenance and
  implements `is_cloud_agent_run` using provenance, ambient-run flags, and task
  ID presence. The same entry already has independent
  `has_local_persisted_data` and `has_cloud_data` flags.
- `app/src/ai/harness_display.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  already maps `Harness::Oz` to user-facing
  `"Warp"`, providing the compatibility-first precedent for copy changes.
- `crates/warp_cli/src/agent.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  keeps the `Harness::Oz` enum/config spelling,
  `oz` config name, and `run-ambient` alias. These are compatibility surfaces,
  not evidence that “Oz” should remain canonical copy.
- `crates/ai/src/agent/orchestration_config.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  and `app/src/ai/blocklist/action_model/execute/run_agents.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  already model
  local/remote orchestration separately from harness type.
- `app/src/ai/agent_conversations_model/entry.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`,
  `app/src/terminal/view/shared_session/cloud_conversation_continuation.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`,
  and `app/src/pane_group/mod.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  use “cloud conversation”, “ambient run”, and
  continuation terms across execution, storage, and viewing contexts.
- `app/src/ui_components/agent_icon.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  is an existing correct pattern: it
  excludes local orchestration children and uses explicit cloud-session/restored
  metadata signals rather than mere task-ID presence.
- `app/src/ai/agent/api.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  currently derives `is_ambient_agent` from
  `conversation.ambient_agent_task_id.is_some()` when gating computer use.
- `app/src/ai/agent_sdk/mod.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  uses `args.task_id` for server-side prompt/task
  configuration, setup observability, and task creation/resume. Those are
  identity/task-source uses and must not be renamed as cloud predicates.
- `app/src/ai/agent_sdk/driver/harness/mod.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  exports task/run IDs and harness
  names to child processes. These environment variables are process identity
  and compatibility surfaces, not execution-location indicators.
- `app/src/ai/agent_sdk/ambient.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`,
  `app/src/ai/agent_events/message_hydrator.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`,
  and `app/src/ai/blocklist/orchestration_event_streamer.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  select task-scoped
  versus unscoped server APIs by parsing a run ID. That selection is request
  authorization/routing, not worker location.
- `app/src/lib.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  enables managed IAP minting when an ambient task ID is
  present. The implementation must verify an explicit server-managed/sandbox
  runtime signal rather than make local-versus-remote behavior depend on ID
  presence alone.
- `app/src/terminal/view/ambient_agent/model.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`,
  `app/src/terminal/view/ambient_agent/view_impl.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`,
  `app/src/terminal/view/shared_session/view_impl.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`, and
  `app/src/terminal/view/shared_session/conversation_ended_tombstone_view.rs @ 88fe7be3b50326a1bd9c3fdf7d9dfc52475dea5e`
  use ambient/cloud terminology for setup, sharing, continuation, and failure
  UI. These paths need explicit execution/location metadata when they mean
  worker location.

### Canonical terminology and axis mapping

Every term in this list has exactly one primary axis:

- **Harness axis**
  - `Warp agent`: canonical product/display name for the first-party harness.
  - `Claude Code`, `Codex`, `Gemini CLI`, `OpenCode`: canonical third-party
    harness names.
  - `Harness::Oz`, `AIAgentHarness::Oz`, `HarnessKind::Oz`, serialized `oz`,
    `OZ_*` environment variables, telemetry/event names, and legacy CLI/API
    fields: compatibility identifiers. Do not break them in this migration.
  - “Oz agent”, “ambient agent” when used as a harness name, and generic
    “cloud agent” when the harness is unknown: non-canonical copy.
- **Execution-location axis**
  - `LocalExecution`: the worker/process executes on the user's/local or
    customer-managed host.
  - `RemoteExecution`: the worker executes on a remote worker. Product copy
    may say “runs in the cloud” only when the worker is Warp-hosted.
  - `RunAgentsExecutionMode::{Local,Remote}` and
    `OrchestrationExecutionMode::{Local,Remote{...}}`: existing location
    primitives to reuse or normalize.
  - “Cloud execution” may be a Warp-hosted display label, but must not replace
    the code-facing `RemoteExecution` type where self-hosted workers exist.
- **Storage axis**
  - `LocallyStoredConversation`: local transcript/state is persisted.
  - `CloudStoredConversation`: server transcript/state is available.
  - Prefer a capability set (`has_local_persisted_data`,
    `has_cloud_data`) over an exclusive storage enum; a conversation can have
    both capabilities.
  - “Cloud conversation” is prohibited as a storage predicate because it is
    routinely confused with remote execution.
- **Identity axis**
  - `LocalConversationId` (or the existing `AIConversationId`): client-local
    conversation identity.
  - `ServerConversationToken`: server conversation identity.
  - `AgentTaskId` as the canonical code-facing name for the server task/run
    identity, with `AmbientAgentTaskId`, `task_id`, `run_id`, and wire/API names
    retained as aliases until migration is complete.
  - IDs identify records and authorization scope; they do not imply location.
- **Access/ownership axis**
  - `ConversationOwner` / owned local run.
  - `SharedSessionViewer`: observes another owner's session and does not report
    task status.
  - `RemoteChildPlaceholder`: local placeholder for a child executing elsewhere.
  - `LocalChild`: child executing in this client.
  - `is_remote_child`, `is_viewing_shared_session`, and parent IDs belong here,
    not in storage or location.
- **Launch/source axis**
  - `Interactive`, `CLI`, scheduled, integration, and `CloudMode` describe
    launch/source or product workflow.
  - `AgentSource::{Interactive,CloudMode}` must not be used as a physical
    execution-location enum without an explicit mapping.
- **Process/runtime axis**
  - `ExecutionMode::{App,Tui,Sdk,RemoteServerDaemon}` describes the client
    process/runtime surface. It is not agent worker location and must retain its
    independent name.
- **Routing/attachment axis**
  - `AIQueryRouting::{Local,LiveRemoteVm,NewCloudVm,UnconnectedReadOnly}`
    describes query routing and attachment state. It is not storage state and
    must not be used as a generalized location predicate.

### Task-ID/cloud predicate migration matrix

The implementation must audit every `task_id`, `run_id`, and
`ambient_agent_task_id` location decision. At minimum, classify and handle these
call sites:

- `app/src/ai/agent_conversations_model/entry.rs:is_cloud_agent_run`: replace
  task-ID/provenance inference with an explicit execution-location field/helper.
  Keep `has_local_persisted_data` and `has_cloud_data` as independent storage
  capabilities.
- `app/src/ai/agent/api.rs` computer-use gating: retain the separate
  `LocalComputerUse` capability path, and use an explicit location/capability
  predicate for any remote-only behavior. A task ID alone is insufficient.
- `app/src/ai/blocklist/history_model.rs:is_ambient_agent_conversation` and related
  metadata helpers: distinguish server task metadata, harness, and execution
  location; do not call all task-backed conversations ambient/cloud.
- `app/src/ui_components/agent_icon.rs`: preserve the existing
  cloud-session/restored-metadata plus local-child exclusions, and make it the
  shared helper used by conversation cards, pane chrome, tabs, and notifications.
  Add regression coverage for a local task-backed run.
- `app/src/terminal/view/shared_session/conversation_ended_tombstone_view.rs`
  and the ambient/shared-session continuation paths: use explicit location and
  access role for “cloud failed”, “continue in cloud”, and viewer behavior.
  A task ID should only identify the task to continue or fetch.
- `app/src/pane_group/pane/terminal_pane.rs` and
  `app/src/pane_group/mod.rs`: retain task IDs for snapshot/restore and server
  conversation lookup; use explicit view/access state to choose ambient pane
  restoration versus a transcript viewer.
- `app/src/ai/agent_conversations_model.rs`: retain `parent_run_id` for
  parent/child navigation grouping; do not use it as a location or storage
  predicate.
- `app/src/ai/blocklist/local_agent_task_sync_model.rs`: retain task identity
  for status updates, but preserve the explicit owner/viewer/remote-child
  guards. Add a named ownership helper if needed rather than a cloud helper.
- `app/src/ai/agent_sdk/mod.rs`, `app/src/ai/agent_sdk/ambient.rs`,
  `app/src/ai/agent_sdk/common.rs`, and
  `app/src/ai/agent_events/message_hydrator.rs`: retain task ID parsing for
  server-side prompt/config resolution, authorization-scoped API calls,
  event hydration, and observability. Rename local variables/comments only when
  they incorrectly say “cloud”.
- `app/src/ai/agent_sdk/driver/harness/mod.rs`: retain `OZ_RUN_ID`,
  `OZ_PARENT_RUN_ID`, `OZ_HARNESS`, and listener aliases as compatibility
  process identity. Do not change these names merely to rename display copy.
- `app/src/lib.rs` managed-IAP initialization: require an explicit
  server-managed sandbox/runner mode in addition to a valid task identity; do
  not infer that mode from a task ID that a local run can also possess.
- Any telemetry, error, icon, or tab code that checks `task_id.is_some()` must
  be assigned to one of: identity/task-source (retain), explicit execution
  location (replace), access/ownership (replace), or storage capability
  (retain). A mechanical global replacement is not acceptable.

### Proposed staged migration

1. Add or normalize one explicit execution-location representation on the
   conversation/run entry boundary. Populate it from launch/orchestration
   configuration, restored server metadata, or a deliberate local default.
   Reject “unknown” only where a caller genuinely requires a location; do not
   silently treat unknown as remote.
2. Add centralized helpers with names that reveal the axis, for example
   `execution_location()`, `is_remote_execution()`,
   `storage_capabilities()`, `is_shared_session_viewer()`, and
   `is_remote_child_placeholder()`. Helpers must document that task identity is
   orthogonal.
3. Migrate high-signal UI and navigation call sites first: conversation entry
   classification, icons/tabs, pane restoration, continuation/tombstones,
   computer-use gating, and task synchronization ownership.
4. Change user-facing labels and docs to “Warp agent”, “runs locally”, “runs
   remotely”/“runs in the cloud” where appropriate, “locally stored”, and
   “cloud stored”. Keep internal `Oz`/`ambient` strings only where they are
   wire, persisted, telemetry, environment, or compatibility identifiers.
5. Retain `run-ambient` as a deprecated CLI alias for the existing
   `run-cloud` command during the compatibility window. New help and docs use
   the canonical command/name selected by the CLI owner; removing the alias is
   a separate deprecation task.
6. Preserve serialized fields, database columns, API request/response names,
   environment variables, and server task/run URLs. If a Rust type is renamed,
   use serde/API aliases and migration tests; do not rewrite existing records.
7. After the high-signal migration, search for ambiguous terms and
   task-ID-derived location predicates. Any remaining occurrence must be
   documented as compatibility, wire protocol, or an explicitly chosen
   axis-specific use.

### Design alternatives

- **Rename `Harness::Oz` everywhere immediately** vs. **staged compatibility**.
  An immediate rename gives cleaner source but risks persisted data, API
  clients, environment variables, telemetry, and CLI scripts. Select staged
  compatibility: display/copy first, aliases and wire names retained.
- **Call the location axis `CloudExecution`** vs. **`RemoteExecution`**.
  `CloudExecution` is familiar in Warp-hosted UI but is wrong for self-hosted
  remote workers. Select `RemoteExecution` in code and allow “cloud” only as
  precise Warp-hosted product copy.
- **Model storage as one enum** vs. **independent capabilities**. An enum makes
  mutually-exclusive states easy but cannot represent local and cloud copies
  simultaneously. Select independent local/cloud capabilities.
- **Infer location from task ID, pane type, `is_remote_child`, or source** vs.
  **carry explicit execution metadata**. Inference is already disproven by
  local task-backed runs and conflates access/source with location. Select
  explicit metadata; retain child/viewer/source fields for their own axes.
- **Mechanically rename every occurrence** vs. **centralize helpers and migrate
  high-signal call sites**. Mechanical churn obscures behavioral changes and
  increases compatibility risk. Select the staged, high-clarity migration.

### Open questions resolved

- **Does a local run receive a server task ID?** Yes. `StreamInit.run_id`,
  local task creation, and local third-party harness registration prove it.
  Therefore task identity is not location.
- **Is `task_id` the same as a local conversation ID or server conversation
  token?** No. They remain separate identity fields.
- **Can a conversation be both locally and cloud stored?** Yes. Existing
  backing-data flags already represent this; the proposal makes that contract
  explicit.
- **Should the product immediately remove “Oz” from APIs and persisted data?**
  No. The compatibility spelling remains until a separately versioned
  deprecation/migration is approved.
- **Should “cloud” be forbidden everywhere?** No. It remains valid for
  Warp-hosted worker copy and existing product/API concepts, but cannot be a
  generic predicate or substitute for remote execution.
- **What happens when location metadata is absent during restore or migration?**
  Treat it as `Unknown` at the boundary and select the conservative behavior
  for that caller; never infer remote from a task ID. The implementation must
  log/measure unknown cases so they can be eliminated.
- **Does this proposal include a broad API or CLI breaking change?** No. It
  specifies compatibility aliases and display-first migration; breaking
  removals require a separate deprecation decision.

## Validation & verification criteria

All criteria must pass before the implementation PR is marked ready. The
implementation phase must record exact test names/commands and any skipped
environment checks.

1. **Terminology inventory:** A repository search (for example,
   `rg -n "Oz agent|ambient agent|cloud conversation|task_id|run_id|ambient_agent_task_id"`
   across `app/`, `crates/`, and CLI code) produces a reviewed classification
   for every location-sensitive occurrence: harness, execution, storage,
   identity, access/ownership, launch/source, process/runtime, routing, or
   compatibility. No unclassified occurrence remains.
2. **Local first-party regression:** A unit/model test constructs a local Warp
   agent conversation with a non-empty server task ID and asserts
   `LocalExecution`, local owner status reporting, and non-cloud icon/navigation
   treatment. The test must fail against the pre-migration task-ID predicate.
3. **Local third-party regression:** A test covers a local Claude Code or Codex
   session that registers `terminal_view_id -> task_id`; it remains local while
   status synchronization reports the server task as `IN_PROGRESS`.
4. **Remote child regression:** A remote-child placeholder with a task ID is
   classified as `RemoteExecution` (or explicit remote-child role as
   appropriate), excluded from local status ownership, and not double-reported.
5. **Shared viewer regression:** A shared-session viewer with a task ID can
   render/fetch the transcript but does not claim task ownership, send local
   status transitions, or become a local execution entry.
6. **Storage matrix:** Tests cover local-only, cloud-only, and both
   `LocallyStoredConversation`/`CloudStoredConversation` capability combinations,
   including restore and fork. Storage classification is unchanged when a
   conversation's execution location or access role changes.
7. **Identity separation:** Tests assert that local conversation ID, server
   conversation token, and server task ID remain distinct through new run,
   local continuation, cloud restore, and fork. Fork clears/creates identities
   according to existing `RestorationMode::{Continue,Fork}` semantics.
8. **UI/capability predicates:** Tests cover computer-use gating, conversation
   entry classification, icon/tab state, pane restoration, continuation/
   tombstone copy, and task synchronization. None treats `task_id.is_some()` as
   a cloud/location decision; legitimate identity and API-scope checks remain.
9. **Parent/child and source separation:** Tests prove `parent_run_id`,
   `is_remote_child`, shared-viewer state, `AgentSource`, and
   `ExecutionMode::{App,Tui,Sdk,RemoteServerDaemon}` retain their independent
   semantics and cannot silently change execution-location classification.
10. **Compatibility checks:** Existing persisted records with `Oz`/`oz`,
    `run_id`, and `is_remote_child`; API payloads; `OZ_*` environment variables;
    task/run URLs; and the `run-ambient` CLI alias continue to parse and behave
    as before. New display/help copy uses canonical names.
11. **Existing high-signal tests:** The implementation runs and passes the
    local task-sync tests, conversation-entry/icon tests, restore/continuation
    tests, third-party harness tests, and any newly added tests corresponding to
    criteria 2–9.
12. **Repository checks:** Run `./script/format`, the repository-prescribed
    `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`,
    and the relevant `cargo nextest`/`cargo test` targets. Run `./script/presubmit`
    when the environment supports the full suite.
13. **Focused Cargo dependency blocker:** The previously attempted focused test
    (`cargo test --manifest-path app/Cargo.toml validate_cli_installed --lib`)
    was blocked before compilation because Cargo could not write its
    `/usr/local/cargo/git/db` cache while fetching `core-foundation`
    (`Permission denied`). Re-run after fixing the cache permissions or record
    the same environmental mismatch with the exact command and error; do not
    call the test green based only on source inspection.
14. **Runtime UI proof:** Because terminology and classification are
    user-facing, exercise a built Warp GUI with computer use and capture
    screenshots showing at least: a local task-backed Warp-agent conversation,
    a local third-party harness conversation, a remote/cloud run, a shared
    viewer, local/cloud/both storage where exposed, and continuation/fork
    actions. Verify visible labels use canonical copy and do not claim “cloud”
    solely because a task ID exists.
15. **CLI proof:** Run the relevant CLI help/validation and compatibility
    cases, including canonical harness labels, deprecated `run-ambient` alias,
    `--task-id` on a local run, and explicit local/remote execution options.
    Confirm `--task-id` still selects server task configuration without changing
    location semantics.
16. **Diff scope:** The spec commit contains only this markdown file. The
    implementation PR reuses this branch, includes no unrelated mechanical
    terminology churn, and links each changed call site to one of the
    axis-specific invariants above.

## Implementation handoff

The implementation child must read this committed file from the draft PR before
editing. It should update the PR description from “spec” to the shipped change
only after implementation and verification are complete. Any compatibility
breaking removal of `Oz`, `ambient`, `run-ambient`, or `OZ_*` identifiers is out
of scope unless a separately approved migration plan is added.
