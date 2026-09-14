# Oz Lifecycle Hooks — Technical Specification

Linear: [APP-4344](https://linear.app/warpdotdev/issue/APP-4344/add-claude-codecodex-style-lifecycle-hooks-to-the-oz-warp-agent)

Product spec: `specs/APP-4344/PRODUCT.md`

External references:
- [Claude Code hooks](https://code.claude.com/docs/en/hooks)
- [Codex hooks](https://learn.chatgpt.com/docs/hooks)

## Summary

Implement a first-party Oz hook runtime in the Warp client and embedded cloud Oz process. Add protocol gates so the same runtime can execute hooks for server-owned tool and compaction boundaries. Keep Warp permissions authoritative. Apply redaction before data crosses a process or network boundary. Do not modify third-party harness setup or native hooks.

## Relevant code

### Warp

- `app/src/ai/agent_sdk/driver/harness/mod.rs (191-273)` — `HarnessKind::Oz` is separate from `ThirdPartyHarness`.
- `app/src/ai/agent_sdk/driver.rs (2070-2319)` — Oz setup, MCP startup, environment preparation, and skill loading.
- `app/src/ai/agent_sdk/driver.rs (3416-3615)` — `AgentDriver::execute_run` and conversation lifecycle subscriptions.
- `app/src/ai/blocklist/action_model/execute.rs (593-760)` — `BlocklistAIActionExecutor::try_to_execute_action` computes native permission behavior and dispatches client actions.
- `app/src/ai/blocklist/permissions.rs (1-132)` — typed command, read, and write permission results.
- `app/src/ai/blocklist/permissions.rs (352-486)` — execution-profile command permissions and deny lists.
- `app/src/ai/blocklist/permissions.rs (1217-1248)` — system-protected executable configuration paths.
- `app/src/ai/mcp/mod.rs (482-599)` — Warp user and project config-path conventions.
- `app/src/ai/mcp/file_based_manager.rs (246-444)` — content hashing, source scope, and project auto-start restrictions for executable configuration.
- `crates/warp_cli/src/agent.rs (211-279)` — first-party and third-party harness enum.
- `crates/warp_cli/src/lib.rs (111-299)` — shared Oz/Warp CLI argument tree.

### Multi-agent server

- `logic/ai/multi_agent/utils/output/tool_call_processor.go (86-286)` — native tool-call parsing and client-action production.
- `logic/ai/multi_agent/compression/summarize/summarize.go (48-78)` — summarization lifecycle callback interface.
- `logic/ai/multi_agent/compression/summarize/summarize.go (199-219)` — entry to server-owned context-window summarization.

### Protocol

- `apis/multi_agent/v1/request.proto (21-124)` — request input and user-input variants.
- `apis/multi_agent/v1/request.proto (460-701)` — client capability settings.
- `apis/multi_agent/v1/response.proto (14-59)` — streamed response envelope.
- `apis/multi_agent/v1/response.proto (352-454)` — server-to-client actions.
- `apis/multi_agent/v1/task.proto (24-53)` — task state and opaque server data.

### Cloud worker

- `internal/worker/backend.go (125-181)` — backend-neutral task parameters and task execution interface.
- `internal/worker/direct.go (58-72)` — minimal host environment inherited by direct tasks.
- `internal/worker/direct.go (152-284)` — direct workspace setup and embedded Oz execution.
- `internal/worker/docker.go (118-199)` — container task command, environment, and `/workspace` working directory.
- `internal/worker/dispatch_payload.go (9-55)` — versioned command-backend task payload.

## Current state

### Harness boundaries

`HarnessKind` routes Oz through Warp's MAA-backed runtime. Claude Code, Codex, and Gemini use `ThirdPartyHarness` implementations and their own CLI configuration. The new hook runtime must be constructed only for `HarnessKind::Oz`.

### Local client actions

MAA emits `ClientAction` values. Warp converts them to `AIAgentAction` values. `BlocklistAIActionExecutor::try_to_execute_action` computes whether Warp can auto-execute the action, whether it needs user confirmation, and which action executor runs it.

This is the final common client-side dispatch boundary. It is synchronous at entry and returns an async execution variant for actions that need one. Adding a blocking hook requires a staged asynchronous preflight before the existing confirmation and execution branch.

### Server-owned boundaries

The server parses native model tool calls in `nativeToolCallProcessor.ProduceActions`. Most calls become client actions, but the tool abstraction also permits server-owned processing. Context compaction is explicitly server-owned in `SummarizeMessagesForContextWindow`.

A client-only hook runtime cannot guarantee `PreToolUse`, `PostToolUse`, or `PreCompact` at these boundaries. The protocol must let MAA request a hook invocation and await a correlated result before it continues.

### Cloud execution

The worker launches the same Warp/Oz binary inside the task execution environment:
- Direct uses the task workspace as `cmd.Dir`.
- Docker uses `/workspace` as the container working directory.
- Kubernetes and command-dispatched backends also launch the task runtime outside the worker control plane.

The embedded Oz process, not the worker daemon, must own hook discovery and execution. The worker only needs to preserve required task metadata, trust material, cancellation, and sandbox placement.

## Technical design

### 1. Add a shared Oz hook module

Add an Oz-only module under `app/src/ai/agent_sdk/hooks/` with these responsibilities:
- `config`: discover, parse, validate, hash, and merge hook files.
- `trust`: evaluate exact project-definition trust.
- `matcher`: compile matchers and select handlers.
- `payload`: define the versioned event envelope.
- `redaction`: convert internal prompt, action, result, and compaction data to safe payloads.
- `runtime`: queue events, spawn commands, enforce limits, parse `PreToolUse` output, and aggregate outcomes.
- `telemetry`: emit metadata-only execution events.

Expose a narrow runtime interface:

```rust
pub(crate) trait OzHookRuntime {
    async fn observe(&self, event: OzHookEvent) -> OzHookObservation;
    async fn pre_tool_use(&self, event: OzPreToolUseEvent) -> OzPreToolUseDecision;
    fn cancel(&self, scope: OzHookCancellationScope);
}
```

`observe` never returns control effects. `pre_tool_use` returns only `Continue` or `Deny { reason, source }`.

Do not add hook methods to `ThirdPartyHarness`. This runtime is a first-party Oz service, not a generalized wrapper around native third-party hook systems.

### 2. Configuration model

Deserialize with strict unknown-field rejection.

The in-memory model should distinguish:
- `HookConfigFile`
- `HookEventName`
- `MatcherGroup`
- `CommandHandler`
- `HookConfigSource::User`
- `HookConfigSource::Project`
- `FailureMode::Continue`
- `FailureMode::Deny`

Validation happens before the file contributes handlers:
1. Enforce the 256 KiB byte limit.
2. Parse JSON.
3. Validate `schema_version`.
4. Reject unknown fields and events.
5. Validate handler counts.
6. Compile regular expressions.
7. Validate timeout bounds.
8. Reject `on_failure: "deny"` outside `PreToolUse`.
9. Compute SHA-256 over the exact validated file bytes.
10. Apply trust to the project file.
11. Merge user then project handlers without deduplication.

Do not canonicalize JSON before hashing. An exact byte change must require a new project trust decision. This matches the product contract and avoids ambiguity about semantically equivalent but differently audited files.

Snapshot the merged config once per conversation. Store the source path and definition hash on every configured handler for diagnostics.

### 3. Trust material

Add a `HookTrustStore` abstraction. A trust key contains:
- canonical Git root
- canonical config path
- SHA-256 file hash

The store must not accept trust data from `.warp/hooks.json` or another project file.

Local runs use private user state. Cloud runs receive signed or server-authenticated trust records in task metadata. The embedded runtime verifies that the record matches the canonical repository identity, config path, and hash it discovers inside the sandbox.

When a cloud run has no matching record:
- Skip the project file.
- Emit an `untrusted_project_hooks` setup diagnostic.
- Continue the run with valid user hooks.

The run-launch or environment-management surface that records cloud trust must show the exact validated handler definitions described in the product spec. The trust transport must contain hashes and identity only. It must not contain command output, hook payloads, or secrets.

Add `.warp/hooks.json` and the host trust store to the same system-protected write classification used for MCP configuration in `app/src/ai/blocklist/permissions.rs`. An Oz action must not auto-write either path. A user-confirmed write still creates a new project hash that remains untrusted.

### 4. Runtime ownership and lifecycle

Create one `OzHookRuntimeHandle` per Oz conversation.

For local Oz:
- Construct it when the conversation has an initial working directory and run identifiers.
- Load configuration before the first prompt.
- Store the handle with conversation-scoped state used by prompt and action execution.

For cloud Oz:
- Construct it in `AgentDriver` after terminal bootstrap and environment preparation.
- Construct it after repositories and setup commands are complete so project files exist.
- Construct it before the initial prompt enters `execute_run`.
- Execute commands through the embedded Warp process in the task sandbox.

Fire `SessionStart` after configuration and trust evaluation. Register graceful teardown so `SessionEnd` receives the final reason and a hard 3-second cap.

`SessionEnd` cannot be guaranteed after SIGKILL, container loss, worker loss, or host loss. Do not report a synthetic successful `SessionEnd` in those cases.

### 5. Event queue and ordering

Each runtime owns one FIFO queue keyed to the conversation.

Processing rules:
- Dequeue one lifecycle event at a time.
- Resolve matching handlers from the immutable config snapshot.
- Run handlers sequentially.
- Preserve source, group, and handler declaration order.
- Stop a `PreToolUse` chain after the first explicit deny or fail-closed failure.
- Continue after fail-open failures.
- Let different conversation runtimes process independently.

Every event has an opaque invocation ID. Tool events also carry the stable tool-use ID. Protocol results must match both identifiers before the runtime applies them.

Hook or session cancellation:
- Cancel the event future.
- Kill the active process group.
- Drop queued events scoped to the cancelled operation.
- Mark the invocation cancelled.
- Reject late local and protocol results.

Tool execution cancellation is a tool result, not automatic hook-runtime cancellation. Emit `PostToolUse` with terminal status `cancelled` when the conversation runtime remains active.

### 6. Command runner

Use a dedicated subprocess runner. Do not reuse the normal agent shell action executor because hook commands must not create recursive `PreToolUse` events.

Runner behavior:
- Select `command_windows` on Windows when present.
- Otherwise use `command`.
- Start the command through the active session shell.
- Set the command working directory to the event `cwd`.
- Create a process group on Unix or a Job Object on Windows.
- Build a new environment from the explicit allowlist in the product spec.
- Write one serialized payload to stdin and close stdin.
- Capture stdout and stderr independently.
- Enforce 64 KiB per stream while reading.
- Kill the process group or Job Object on timeout, cancellation, or output overflow.
- Decode output as UTF-8.
- Record duration and exit status.
- Never log the command's stdin or raw output.

The cloud Direct backend already starts the task from a minimal host environment in `internal/worker/direct.go (58-72, 256-264)`. The hook runner must still rebuild its own environment because task-level environment variables include resolved secrets.

### 7. Payload and redaction

Define typed Rust payload structs with a common envelope and event-specific flattened fields. Define equivalent typed Go payload templates for server-owned events. Serialize only after redaction.

Common fields:
- `schema_version`
- `hook_event_name`
- `session_id`
- `run_id`
- `conversation_id`
- `cwd`
- `hook_source`
- `model`
- `permission_mode`

Event fields:
- `SessionStart`: `source`
- `SessionEnd`: `reason`
- `UserPromptSubmit`: `prompt`
- `PreToolUse`: `tool_name`, `tool_use_id`, `tool_input`
- `PostToolUse`: `tool_name`, `tool_use_id`, `tool_input`, `tool_response`
- `PreCompact`: `trigger`
- `Stop`: `turn_status`

Add a Rust redaction adapter for every `AIAgentActionTypeDiscriminants` value. Add a Go redaction adapter for every server-owned tool category. Do not generically serialize an internal action or tool-call object as the hook payload. Generic serialization can expose fields that have not received a redaction review.

The adapters should retain:
- stable tool name
- paths
- argument keys
- non-sensitive enum values
- command text after secret replacement
- risk category
- status and numeric metadata
- bounded safe previews

Replace omitted values with explicit objects such as:

```json
{
  "content": {
    "redacted": true,
    "reason": "file_content",
    "byte_count": 18432
  }
}
```

Redaction order:
1. Convert the internal event to an allowlisted intermediate representation.
2. Replace known secret values and credential patterns.
3. Remove prohibited fields.
4. Truncate bounded leaf values.
5. Add omitted and truncation metadata.
6. Enforce the event-specific size budget.
7. Serialize.
8. Enforce the final 256 KiB limit.

Use explicit allowlisted intermediate types for local execution and protocol transport. Test the Rust and Go types against the same canonical JSON golden fixtures.

### 8. `PreToolUse` output parser

Accept:
- Exit 0 with empty stdout.
- Exit 0 with `{}`.
- Exit 0 with the exact compatible deny subset.
- Exit 2 with non-empty stderr.

The structured subset is:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "bounded reason"
  }
}
```

Use strict parsing:
- Reject unknown top-level fields.
- Reject unknown `hookSpecificOutput` fields.
- Require `hookEventName` to equal `PreToolUse`.
- Accept only `permissionDecision: "deny"`.
- Require a non-empty reason for structured denial.
- Reject mutation and context fields.
- Treat multiple JSON values or non-whitespace trailing data as malformed.

Exit precedence:
- Timeout, cancellation, spawn failure, and output overflow are execution failures.
- Exit 2 with non-empty stderr is denial.
- Exit 2 with empty stderr is failure.
- Exit 0 parses stdout.
- Every other exit code is failure.

Apply the handler's `on_failure` only to failures. Explicit denial always denies.

### 9. Warp permission composition

Refactor the client action path into stages:
1. Compute the existing Warp permission classification without side effects.
2. Reject an existing Warp deny.
3. Build the redacted hook event.
4. Await `PreToolUse`.
5. Reject a hook deny.
6. Show the existing Warp confirmation when the native classification requires it.
7. Execute the original action object without modification.
8. Build and await `PostToolUse`.
9. Send the original tool result to MAA.

`BlocklistAIActionExecutor::try_to_execute_action` currently combines permission decisions and dispatch. Introduce an async preflight state rather than blocking the model thread. Preserve the existing `NotExecutedReason::NeedsConfirmation` behavior after hook continuation.

The hook runtime must never return an action object. This type boundary prevents input mutation by construction.

When Warp denies before the hook stage:
- Do not execute the tool.
- Do not emit `PreToolUse`.
- Do not emit `PostToolUse`.
- Return the existing Warp denial result.

When a hook denies:
- Produce a tool result that clearly attributes the denial to a trusted Oz hook.
- Include the bounded reason.
- Let the model continue and choose another tool.
- Do not change the native permission profile.

### 10. Protocol contract

Extend `warp-proto-apis` with first-party Oz hook messages.

Add `apis/multi_agent/v1/oz_hooks.proto`. Import it from `request.proto` and `response.proto`. Define the shared event enum with an unspecified zero value and one value for each v1 event.

Add a client capability:
- Add `bool supports_oz_lifecycle_hooks = 34` to `Request.Settings`.

Add `repeated string supported_oz_hook_payload_schema_versions = 4` to `ResponseEvent.StreamInit`. A hook-enabled client must receive `warp.oz_hook.v1` in this field before it applies any action from the stream.

Add `OzHookContext oz_hook_context = 7` to `Request`:

```proto
message OzHookContext {
  repeated OzHookEvent enabled_events = 1;
  repeated string supported_payload_schema_versions = 2;
}
```

Add `OzHookResult oz_hook_result = 9` to `Request.Input.UserInputs.UserInput.input`.

Do not send hook commands, matchers, failure modes, or trust records to MAA. The execution host owns configuration and matching. When an event name is enabled, MAA emits a gate for every server-owned occurrence of that event. The execution host returns continue without spawning a command when no local matcher selects a handler.

Add `RunOzHook run_oz_hook = 15` to `ClientAction.action`:

```proto
message RunOzHook {
  string invocation_id = 1;
  string tool_use_id = 2;
  OzHookEvent event = 3;
  string schema_version = 4;
  google.protobuf.Struct redacted_payload = 5 [(sensitive) = true];
}
```

Define the client-to-server input:

```proto
message OzHookResult {
  string invocation_id = 1;
  string tool_use_id = 2;
  oneof outcome {
    Continue continue = 3;
    Deny deny = 4;
    Failed failed = 5;
    Cancelled cancelled = 6;
  }
}
```

`Deny` contains only a bounded reason and source identity. `Failed` contains a category and an explicit resolved action of continue or deny. It does not contain raw stdout or stderr.

Use explicit enum values for event and outcome names in protobuf. Keep the command-facing PascalCase event name in JSON.

Protocol rules:
- MAA assigns `invocation_id`.
- The client must echo it unchanged.
- MAA rejects missing, duplicate-with-different-content, stale, or mismatched results.
- Replaying an identical result is idempotent.
- A pending gate is scoped to the conversation, request, event, and tool-use ID.
- Pending state must survive the request boundary in server-owned task state.
- MAA must not execute or release the gated operation before a valid result.
- Cancellation clears pending gates.
- Old clients that do not advertise the capability never receive hook actions.
- A new server that receives an enabled hook context but cannot select a mutually supported payload schema finishes the stream before inference with an explicit incompatibility error.
- A hook-enabled client cancels a stream whose `StreamInit` omits its requested schema version.
- A hook-enabled runtime must reject a server that cannot provide required server-owned event coverage. It must not silently downgrade to client-only coverage.

Apply redaction before constructing `RunOzHook`. The protocol payload is a source-neutral template because MAA does not know which user or project handlers will match. The execution host adds `hook_source` for each selected handler before it serializes command stdin. It reapplies the final payload limit after adding that field. A field marked sensitive prevents accidental logging, but it is not a substitute for redaction.

### 11. Server-owned tool gate

Wrap server-owned tool execution in this state machine:
1. Parse and validate the complete tool call.
2. Determine the canonical tool name and stable tool-use ID.
3. Apply existing server policy.
4. Check whether the request hook context enables `PreToolUse`.
5. If it is not enabled, execute normally.
6. If it is enabled, persist a pending gate and emit `RunOzHook(PreToolUse)`.
7. End the current response at a resumable boundary.
8. On the next request, validate `OzHookResult`.
9. On continue, execute the exact stored tool input.
10. On deny, do not execute. Persist a synthetic denied tool result for the model.
11. After execution reaches a terminal result, repeat the gate when the request context enables `PostToolUse`.
12. Resume inference only after `PostToolUse` continues or fails according to the resolved client outcome.

Integrate the gate before a server tool implementation can produce side effects. Do not add it only after `nativeToolCallProcessor.ProduceActions`; that is too late for tool implementations that execute during action production.

Store a hash of the original canonical tool input with the pending gate. Validate the hash before execution after resume. This proves the executed input is the input that the hook observed.

Client-executed actions do not need the server round trip for pre/post execution. They use the local runtime stages in section 9. The server protocol is reserved for boundaries the execution host cannot otherwise intercept.

### 12. `PreCompact` gate

Compaction is server-owned. Add a resumable hook boundary immediately before `SummarizeMessagesForContextWindow`.

Flow:
1. MAA decides compaction is required.
2. MAA builds a redacted `PreCompact` payload with `trigger`.
3. MAA persists a pending compaction gate.
4. MAA emits `RunOzHook(PreCompact)`.
5. The client or embedded cloud Oz runtime executes matching observational handlers.
6. The runtime returns continue or failed-open.
7. MAA validates the invocation and begins compaction.

`PreCompact` cannot deny. A deny-shaped or malformed result is a failure and compaction continues after diagnostics. MAA must never wait indefinitely for this event.

Keep the existing `SummarizationEventHandler.Start` callback for summarization output. The hook gate is an earlier control-plane boundary, not a replacement for current telemetry callbacks.

### 13. Prompt, stop, and session events

`UserPromptSubmit`:
- Run after attachment and skill resolution has produced the prompt representation.
- Redact attachments and secrets.
- Await observation before sending the prompt to MAA.
- Continue on every failure.

`Stop`:
- Subscribe to the existing conversation status and final-output lifecycle.
- Emit once per user turn.
- Use a turn-generation token to prevent duplicate events from retries or repeated status notifications.
- Await observation before publishing the final terminal turn status.

`SessionStart`:
- Emit once per runtime after configuration and trust resolution.
- Do not inject stdout into the model.

`SessionEnd`:
- Emit from graceful shutdown and cancellation paths.
- Use a 1-second default and 3-second total teardown cap.
- Never extend worker or app shutdown beyond that cap.

### 14. Cloud worker integration

Do not add a second hook executor to `oz-agent-worker`.

Required worker changes are limited to:
- carry hook capability and cloud trust material into `TaskParams`
- preserve the metadata through Direct, Docker, Kubernetes, and command dispatch
- ensure the embedded Oz process can read the task sandbox's home and project hook files
- keep hook processes inside the task cancellation context
- reject a hook-enabled task when a backend cannot preserve the embedded runtime contract
- add backend tests that prove worker-control-plane credentials are not inherited

The assignment carries an optional, strict `oz_lifecycle_hooks` object:

```json
{
  "required": true,
  "supported_payload_schema_versions": ["warp.oz_hook.v1"],
  "project_trust": [
    {
      "git_root": "/workspace/repository",
      "config_path": "/workspace/repository/.warp/hooks.json",
      "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
    }
  ]
}
```

Unknown fields, `required: false`, empty or unsupported schema versions, more than 64 trust
records, and serialized objects over 64 KiB are rejected. The object is rejected for non-Oz
harnesses. The 64 KiB transport bound keeps the single argument below Linux `MAX_ARG_STRLEN`; the
256 KiB config and hook-stdin limits are separate.

The worker passes this object to embedded Oz as one non-secret
`--oz-lifecycle-hooks-context <JSON>` argument pair. It is never inherited through the process
environment. The argument uses a strict `OzLifecycleHooksContext` CLI type and is validated before
the session starts. Backends that cannot preserve the argument, task cancellation, or sandbox
placement reject the hook-enabled task.

Direct:
- The embedded runtime runs under the task process and task workspace.
- It inherits only task environment into Oz.
- The Oz hook runner clears that environment again before hook spawn.

Docker and Kubernetes:
- Hook commands run in the task container or pod.
- They use the task working directory.
- They never use Docker daemon or Kubernetes controller credentials unless those credentials are already intentionally present inside the task sandbox.

Command backend:
- Bump `DispatchPayloadVersion` if trust/capability metadata changes the stable dispatch JSON.
- Include non-secret hook capability and trust identifiers.
- Do not include hook payloads or command outputs in dispatch metadata.

### 15. Observability

Define structured hook telemetry with allowlisted fields:
- event
- source
- definition hash
- matched
- duration bucket
- result
- exit code category
- failure category
- timeout
- truncation flags
- execution mode
- worker backend when available

Never attach:
- command string
- payload JSON
- prompt
- tool input
- tool response
- stdout
- stderr
- denial reason
- environment

User-facing diagnostics can include the configured command during trust review because that is required for informed approval. Routine execution logs should identify the source and hash instead of repeating the command.

Add correlation fields to client and server tracing:
- conversation ID
- run ID
- hook invocation ID
- tool-use ID

### 16. Feature gating and rollout

Gate the behavior independently in Warp and MAA.

Rollout order:
1. Land protocol definitions and generated bindings.
2. Deploy MAA support with the server flag off.
3. Land client and embedded cloud runtime support with the client flag off.
4. Land worker metadata propagation.
5. Enable internal local Oz runs.
6. Enable internal cloud Oz runs.
7. Verify denial, timeout, and redaction telemetry.
8. Expand availability.

The server must handle capability negotiation throughout rollout. Third-party harness requests must never enter the Oz hook state machine.

## End-to-end flows

### Local client tool

1. Local Oz loads host-local configuration.
2. The user submits a prompt.
3. Oz runs `UserPromptSubmit`.
4. MAA returns a client action.
5. Warp computes native permission classification.
6. Warp runs local `PreToolUse`.
7. Warp applies any existing permission prompt.
8. Warp executes the original action.
9. Warp runs local `PostToolUse`.
10. Warp returns the result to MAA.
11. The turn ends and Oz runs `Stop`.

### Cloud client tool

1. The worker starts embedded Oz in the task sandbox.
2. Embedded Oz loads sandbox-local configuration and trust.
3. The same client-tool flow runs inside the sandbox.
4. Hook commands run in the task workspace.
5. The worker daemon does not execute the commands.

### Server-owned tool

1. MAA receives a complete model tool call.
2. MAA persists a `PreToolUse` gate.
3. The execution host receives `RunOzHook`.
4. The host runs the configured command chain.
5. The host returns a correlated continue or deny.
6. MAA executes only on continue.
7. MAA persists and emits a `PostToolUse` gate after execution.
8. MAA resumes inference after the observational result.

### Compaction

1. MAA decides to compact.
2. MAA emits a resumable `PreCompact` gate.
3. The execution host runs matching hooks.
4. MAA receives a correlated continue or failure.
5. MAA starts summarization.

## Decisions and trade-offs

### Reuse event names, not full third-party semantics

The command-facing event and field names follow the shared Claude Code and Codex shape. Oz rejects unsupported control fields instead of pretending they work.

Trade-off:
- Existing scripts that only observe the compatible subset are portable.
- Scripts that grant, mutate, inject context, or rely on third-party tool aliases need an Oz adapter.

### Add protocol gates

The implementation adds resumable MAA state and extra request latency for server-owned events.

Trade-off:
- More protocol and state-machine complexity.
- Complete coverage and reliable deny semantics.

### Sequential handlers

The runtime does not match Codex concurrency.

Trade-off:
- Higher worst-case latency when many hooks match.
- Stable ordering, simple fail-closed semantics, and reproducible side effects.

### Redacted structural payloads

Tool payloads omit file contents, attachments, full transcripts, and raw output.

Trade-off:
- Some third-party hooks cannot inspect every byte.
- The hook boundary does not become a general data-exfiltration channel.

### Fail-open default

Operational failures continue by default.

Trade-off:
- Observability automation cannot break normal work.
- Policy authors must explicitly select `on_failure: "deny"` when availability of the policy is mandatory.

## Assumptions

- The execution host can persist or receive project trust material before a session starts.
- Existing secret-redaction utilities can supply known secret values to the hook redactor without exposing them in logs.
- MAA can persist pending hook gates in request/task state without changing the external conversation model.
- Every server-owned tool has a stable tool-use ID before side effects.
- The first release may emit only `trigger: "auto"` for `PreCompact`.

## Out of scope

- Full Claude Code or Codex configuration-file compatibility.
- Managed organization hooks.
- Hook-distributed credentials.
- Hook output persistence.
- Server-side execution of user command hooks.
- A remote hook service.
- Live trust prompts inside unattended cloud tasks.
- Tool-input mutation and allow decisions.
- Third-party harness regression fixes.

## Validation criteria

### Warp unit tests

Run:

```bash
cargo test -p warp oz_hooks_config
cargo test -p warp oz_hooks_trust
cargo test -p warp oz_hooks_ordering
cargo test -p warp oz_hooks_payload
cargo test -p warp oz_hooks_redaction
cargo test -p warp oz_hooks_runtime
cargo test -p warp oz_hooks_permissions
```

Required coverage:
- strict config parsing and limits
- user/project merge order
- exact-byte hash trust
- all matcher subjects
- all seven payload goldens
- every redaction and size limit
- sequential execution
- explicit deny
- exit-2 deny
- fail-open and fail-closed
- timeout and process-group kill
- cancellation and late-result rejection
- no mutation or allow output
- native permission composition

### Warp integration tests

Add local Oz integration tests that:
- emit the seven events in lifecycle order
- verify one event per documented boundary
- deny a shell, file, MCP, and orchestration tool before side effects
- verify denied tools produce no `PostToolUse`
- verify failed tools produce `PostToolUse` with terminal status
- prove third-party harness startup does not construct the Oz hook runtime

Run the focused integration target, then run:

```bash
./script/presubmit
```

### Protocol tests

In `warp-proto-apis`, run:

```bash
./script/generate -a multi_agent -v v1
git diff --exit-code
```

Add compatibility tests for:
- old clients without the capability
- unknown future enum values
- duplicate identical results
- duplicate conflicting results
- stale and mismatched invocation IDs
- cancellation

### MAA tests

In `warp-server`, run:

```bash
go test ./logic/ai/multi_agent/...
```

Add focused tests that:
- pause a server tool before side effects
- preserve and verify the original input hash
- synthesize a denied tool result
- wait for `PostToolUse` before the next inference
- pause before compaction
- continue compaction after observational hook failure
- resume idempotently after request replay
- clear pending gates on cancellation
- never gate third-party harnesses

### Worker tests

In `oz-agent-worker`, run:

```bash
go test ./internal/worker/...
```

Add backend tests that:
- run a hook in the Direct task workspace
- run a hook in Docker and Kubernetes task workspaces
- preserve hook metadata through command dispatch
- reject an incompatible backend
- cancel hook subprocesses with the task
- prove worker API keys and control-plane credentials are absent from the hook environment

### End-to-end acceptance

Run one local Oz session and one cloud Oz session with a fixture hook set. Both must:
- emit all seven events
- produce payloads matching the same versioned golden schema
- preserve user-before-project order
- deny the same tool without side effects
- continue after the same observational failure
- deny after the same fail-closed `PreToolUse` failure
- record metadata-only diagnostics

No visual or computer-use validation is required.
