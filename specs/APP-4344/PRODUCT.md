# Oz Lifecycle Hooks

Linear: [APP-4344](https://linear.app/warpdotdev/issue/APP-4344/add-claude-codecodex-style-lifecycle-hooks-to-the-oz-warp-agent)

Origin: [Slack request](https://warpdev.slack.com/archives/C0BDQDW8V5E/p1788608487182989?thread_ts=1788608487.182989&cid=C0BDQDW8V5E)

References:
- [Claude Code hooks](https://code.claude.com/docs/en/hooks)
- [Codex hooks](https://learn.chatgpt.com/docs/hooks)

## Summary

Add deterministic lifecycle hooks to Warp's first-party Oz harness. Users can run trusted commands when an Oz session starts or ends, when a prompt is submitted, around every tool call, before compaction, and when a turn stops. The first version supports command handlers only. `PreToolUse` can deny one tool call. No hook can grant permission, change tool input, stop the agent, or modify model context.

## Problem

Oz users cannot attach deterministic automation to the first-party harness lifecycle. They must ask the model to run checks or external automation and rely on the model to comply.

Claude Code and Codex support lifecycle hooks for policy checks, audit logging, state capture, and notifications. Warp exposes those third-party harnesses, but their native hook systems do not apply to Oz. Oz also splits execution across the Warp client, the multi-agent server, and cloud workers. A client-only hook implementation would miss server-owned compaction and tool boundaries.

The feature must provide one clear contract for local and cloud Oz runs. It must not weaken Warp permissions or expose secrets to hook processes.

## Goals

- Support these events for first-party Oz runs:
  - `SessionStart`
  - `SessionEnd`
  - `UserPromptSubmit`
  - `Stop`
  - `PreToolUse`
  - `PostToolUse`
  - `PreCompact`
- Use event names and a JSON payload subset that is familiar to Claude Code and Codex hook authors.
- Execute local hooks on the local host.
- Execute cloud hooks inside the cloud worker sandbox.
- Cover client-executed and server-executed Oz tools.
- Let `PreToolUse` deny a tool call before it has side effects.
- Preserve Warp permission prompts, denials, and sandbox boundaries.
- Require explicit trust for project hook definitions.
- Redact and bound all hook payloads and outputs.
- Make configuration, execution, denial, timeout, and failure outcomes observable.

## Non-goals

- HTTP, MCP tool, prompt, agent, callback, or asynchronous hook handlers.
- Hook-driven tool-input or tool-output mutation.
- Hook-driven permission grants or persistent allow rules.
- Blocking `UserPromptSubmit`, `Stop`, `SessionEnd`, or `PreCompact`.
- Injecting hook output into model context, except for the reason from a `PreToolUse` denial.
- Claude Code or Codex feature parity beyond the named events and compatible payload subset.
- Replacing Warp execution profiles, permissions, enterprise policy, or sandboxing.
- Changing native Claude Code, Codex, Gemini, or OpenCode hook behavior.
- Automatically copying a user's local hook configuration into a cloud environment.
- A visual hook editor. Configuration is file-based in v1.
- Visual or computer-use validation.

## Product behavior

### 1. Configuration discovery

1. Oz reads user hooks from `~/.warp/hooks.json` on the host that executes Oz.
2. Oz reads project hooks from `<git-root>/.warp/hooks.json`.
3. Oz determines `<git-root>` from the session's initial working directory.
4. Oz does not search parent directories above that Git root.
5. Oz does not load a project file when the initial working directory is not inside a Git repository.
6. The cloud runtime reads files from the sandbox filesystem. It does not read `~/.warp/hooks.json` from the user's laptop.
7. Environment setup may provision a cloud user hook file before Oz starts.
8. A checked-out repository may provide a cloud project hook file.
9. Oz snapshots configuration before `SessionStart`. File changes apply to the next session.
10. User and project hooks compose. One layer never replaces the other.
11. Oz evaluates user hooks before project hooks.
12. Oz preserves declaration order within each file.

The v1 configuration schema is:

```json
{
  "schema_version": "warp.oz_hooks.config.v1",
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "^(run_shell_command|apply_patch)$",
        "hooks": [
          {
            "type": "command",
            "command": "python3 .warp/hooks/check_tool.py",
            "command_windows": "py -3 .warp/hooks/check_tool.py",
            "timeout": 10,
            "on_failure": "deny"
          }
        ]
      }
    ]
  }
}
```

The schema rules are:
- `schema_version` is required and must equal `warp.oz_hooks.config.v1`.
- `hooks` is required.
- Event keys must be one of the seven v1 event names.
- Each event contains ordered matcher groups.
- `matcher` is optional.
- An omitted matcher, an empty matcher, and `*` match every event occurrence.
- Every other matcher is a case-sensitive regular expression.
- `hooks` in a matcher group is a non-empty ordered array.
- `type` must equal `command`.
- `command` is required and must be non-empty.
- `command_windows` is optional. Windows uses it instead of `command` when present.
- `timeout` is an integer number of seconds.
- The default timeout is 10 seconds.
- A non-`SessionEnd` timeout must be from 1 through 120 seconds.
- The `SessionEnd` default is 1 second. Its configured maximum is 3 seconds.
- `on_failure` is optional and defaults to `continue`.
- `on_failure: "deny"` is valid only for `PreToolUse`.
- Unknown fields, invalid regular expressions, unsupported events, and unsupported values invalidate the containing file.
- An invalid file contributes no hooks. Hooks from another valid file still run.
- Oz reports one configuration diagnostic per invalid file.
- Each file is limited to 256 KiB and 64 command handlers.

### 2. Matcher behavior

Oz matches each event against one subject:
- `SessionStart`: `source`, with v1 values `startup` or `resume`.
- `SessionEnd`: `reason`, with v1 values `completed`, `failed`, `cancelled`, or `shutdown`.
- `PreToolUse` and `PostToolUse`: the canonical Oz tool name.
- `PreCompact`: `trigger`, with v1 values `auto` or `manual`.
- `UserPromptSubmit` and `Stop`: no match subject. Oz ignores `matcher` and runs every declared handler for the event.

Oz uses the same canonical tool name in matching, payloads, diagnostics, and tests. It does not silently map Oz tool names to Claude Code aliases such as `Bash`, `Read`, or `Write`.

### 3. Project hook trust

1. User hooks are trusted because the user controls the host-level file.
2. Project hooks are disabled until the user explicitly trusts the exact project hook definition.
3. The trust record includes the canonical Git root, config path, and SHA-256 hash of the validated file bytes.
4. Oz displays the source path, event, matcher, command, timeout, failure mode, and hash before accepting trust.
5. A new file, changed byte, changed command, changed matcher, changed timeout, or changed failure mode creates a new hash.
6. A new hash requires a new trust decision.
7. Untrusted project hooks are skipped. The session continues with a visible diagnostic.
8. Headless runs never auto-trust project hooks.
9. A cloud project hook runs only when the sandbox receives a matching explicit trust record as part of the run or environment configuration.
10. The runtime never accepts a project trust decision from the project repository itself.
11. Revoking trust prevents the definition from running in future sessions.

### 4. Lifecycle timing

Oz emits events at these boundaries:

1. `SessionStart`
   - Fires once after environment setup, configuration validation, and trust evaluation complete.
   - Fires before Oz processes the first prompt.
   - Uses `source: "startup"` for a new conversation.
   - Uses `source: "resume"` when an existing conversation resumes.
2. `UserPromptSubmit`
   - Fires once for every user prompt.
   - Fires after prompt attachments are resolved and before the prompt is sent to MAA.
   - Does not fire for internal retries or synthetic model messages.
3. `PreToolUse`
   - Fires once after a complete tool name and input exist.
   - Fires after Warp computes its native permission classification.
   - Fires before a permission prompt and before any tool side effect.
   - Fires for every Oz tool that passes native Warp denial, including MCP, file, shell, document, computer-use, orchestration, and server-executed tools.
4. `PostToolUse`
   - Fires once after an executed tool reaches a terminal success, failure, timeout, or cancellation result.
   - Fires before the result is supplied to the next model inference.
   - Does not fire when `PreToolUse` denied the tool because no tool executed.
5. `PreCompact`
   - Fires immediately before MAA begins manual or automatic context compaction.
   - MAA waits for the observational hook outcome or timeout before compaction begins.
6. `Stop`
   - Fires once when an Oz turn has produced its final assistant output.
   - Fires before the turn changes to an idle, blocked, failed, or completed state.
   - Does not force another model turn.
7. `SessionEnd`
   - Fires once during graceful session teardown after the final `Stop`, when applicable.
   - Uses the terminal `reason` value.
   - Is best effort for process crashes, force kills, host loss, and worker loss.
   - Never delays teardown for more than 3 seconds.

### 5. Execution order

1. Oz maintains one FIFO hook event queue per conversation.
2. Events in one conversation never overtake each other.
3. Different conversations may execute hooks concurrently.
4. For one event, Oz runs matching command handlers sequentially.
5. Oz runs user handlers before project handlers.
6. Oz preserves file and declaration order.
7. A `PreToolUse` denial stops that event's handler chain immediately.
8. Oz does not start later matching handlers after a denial.
9. Session cancellation terminates the active hook process group or Windows Job Object, removes pending hook events, and ignores late results.
10. Cancelling a tool execution emits `PostToolUse` with a cancelled result when the conversation runtime remains active.

This differs intentionally from Codex, which launches matching commands concurrently. Oz chooses deterministic order and denial short-circuiting so policy side effects and audit records are reproducible.

### 6. Command execution

1. A command hook receives one UTF-8 JSON object on stdin.
2. A command runs with the active session working directory.
3. macOS and Linux run `command` through `SHELL`, with `/bin/sh` as the fallback.
4. Windows runs `command_windows` when present and otherwise runs `command` through `COMSPEC`, with `cmd.exe` as the fallback.
5. A cloud command runs inside the worker task sandbox, not on the user's laptop and not in the worker control plane.
6. The hook process receives a rebuilt environment containing only:
   - `HOME`, or `USERPROFILE` on Windows
   - `PATH`
   - `SHELL` on Unix
   - `COMSPEC` and `SystemRoot` on Windows
   - `TMPDIR`, `TMP`, or `TEMP` when present
   - locale variables required for UTF-8 operation
   - `WARP_HOOK_EVENT_NAME`
   - `WARP_RUN_ID`
   - `WARP_CONVERSATION_ID`
7. The hook process does not inherit managed secret values, API keys, cloud credentials, Git credentials, MCP credentials, or the complete Oz task environment.
8. The hook remains inside the same operating-system user, filesystem, network, container, and sandbox boundaries as the Oz runtime.
9. A project hook receives no privilege that the project does not already have inside that runtime.

### 7. Input payload

Every command receives this common envelope:

```json
{
  "schema_version": "warp.oz_hook.v1",
  "hook_event_name": "PreToolUse",
  "session_id": "opaque-session-id",
  "run_id": "opaque-run-id",
  "conversation_id": "opaque-conversation-id",
  "cwd": "/workspace/repository",
  "hook_source": "user",
  "model": "model-id",
  "permission_mode": "supervised"
}
```

Event-specific fields are:
- `SessionStart`: `source`.
- `SessionEnd`: `reason`.
- `UserPromptSubmit`: `prompt`.
- `PreToolUse`: `tool_name`, `tool_use_id`, and `tool_input`.
- `PostToolUse`: `tool_name`, `tool_use_id`, `tool_input`, and `tool_response`.
- `PreCompact`: `trigger`.
- `Stop`: `turn_status`.

Compatibility rules:
- Common and event-specific field names use snake case, matching the overlapping Claude Code and Codex command-hook shape.
- Event names preserve Claude Code and Codex capitalization.
- Warp-specific fields are additive.
- `hook_source` identifies the config layer and does not replace the Claude/Codex `SessionStart.source` field.
- Consumers must ignore unknown fields.
- Warp may add optional fields within `warp.oz_hook.v1`.
- A breaking change requires a new `schema_version`.

### 8. Redaction and size limits

Oz redacts data before JSON serialization, protocol transport, hook execution, telemetry, or logs.

The payload must never include:
- the full process environment
- resolved secret values
- managed secret payloads
- API keys or authorization headers
- raw attachment bytes
- file contents
- an absolute transcript path
- a complete conversation transcript
- unbounded tool input or tool output

The payload preserves useful structure:
- tool name and tool-use ID
- file and directory paths
- argument object keys
- scalar type information
- shell command text with known secrets and credentials replaced
- permission and risk categories
- result status, exit code, duration, byte counts, item counts, and omitted counts
- bounded error and output previews after redaction

The size contract is:
- Serialized stdin: 256 KiB maximum.
- Redacted prompt: 64 KiB maximum.
- Redacted tool input: 128 KiB maximum.
- Redacted tool response: 64 KiB maximum.
- Captured stdout: 64 KiB maximum.
- Captured stderr: 64 KiB maximum.
- Denial reason delivered to the user and model: 4 KiB maximum.

Oz truncates only at valid UTF-8 boundaries. It adds explicit truncation metadata. It never writes an unredacted value before truncation.

### 9. `PreToolUse` decisions

A `PreToolUse` handler can return no decision or deny the current tool.

Continue:
- Exit code 0 with empty stdout means no decision.
- Exit code 0 with `{}` means no decision.

Deny with structured JSON:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "Repository policy blocks this operation."
  }
}
```

Deny with an exit code:
- Exit code 2 denies the tool.
- Non-empty stderr becomes the denial reason.

Rules:
- `permissionDecision: "allow"` is invalid.
- `permissionDecision: "ask"` is invalid.
- `updatedInput`, `updatedMCPToolOutput`, `additionalContext`, `continue`, and `decision` control fields are invalid in v1.
- A denial blocks only the current tool call.
- Oz gives the bounded reason to the model so it can choose another action.
- Oz also surfaces the reason to the user.
- A denial never ends the session.
- A denial never creates an allow rule.

Outputs from the other six events are observational:
- Exit code 0 is success.
- Stdout and stderr are bounded diagnostics only.
- Oz does not parse them as model context or control instructions.
- Exit code 2 has no blocking meaning outside `PreToolUse`.

### 10. Permission composition

Warp permissions remain authoritative.

For each tool call:
1. Warp computes the native permission result.
2. If Warp denies the tool, Oz rejects it. A hook cannot override that result.
3. If Warp allows or would prompt, Oz runs `PreToolUse`.
4. If a hook denies, Oz rejects the tool without showing a permission prompt.
5. If hooks return no decision, Warp applies the original allow or prompt result.
6. The tool executes only after both systems permit it.

Hooks can reduce authority. Hooks cannot increase authority.

### 11. Failure and timeout behavior

A hook failure includes:
- process spawn failure
- timeout
- non-zero exit other than a valid `PreToolUse` exit 2
- invalid or oversized `PreToolUse` JSON
- unsupported output fields
- mismatched `hookEventName`
- stdout or stderr above its limit
- protocol correlation failure

Behavior:
- Observational event failures always continue the Oz lifecycle.
- A `PreToolUse` failure with `on_failure: "continue"` records the failure and continues to the original Warp permission decision.
- A `PreToolUse` failure with `on_failure: "deny"` denies the tool with a generic bounded reason.
- `on_failure` applies independently to each handler.
- A failed handler does not stop later handlers unless its failure mode denies.
- A timeout kills the command's process group before the lifecycle continues.
- `SessionEnd` ignores `on_failure`, does not retry, and never exceeds its teardown cap.
- Oz does not retry command hooks automatically.

### 12. Observability

For each hook invocation, Oz provides a local diagnostic record containing:
- event name
- config source
- config path
- definition hash
- matcher
- start and finish timestamps
- duration
- result: `succeeded`, `continued`, `denied`, `failed`, `timed_out`, or `cancelled`
- exit code when available
- whether output was truncated
- failure category

Remote telemetry omits the config path, matcher, and timestamps. It uses only the metadata allowlist in the technical specification.

Oz does not record raw payloads, raw stdout, raw stderr, raw prompts, raw tool inputs, raw tool responses, or secret values in telemetry.

The user can distinguish:
- no configured hook
- unmatched hook
- untrusted project hook
- invalid configuration
- successful hook
- denied tool
- failed-open hook
- failed-closed hook
- timed-out hook

### 13. Local and cloud parity

1. The seven event names and JSON schemas are identical for local and cloud Oz.
2. The execution host differs:
   - Local Oz uses the local Warp host.
   - Cloud Oz uses the worker task sandbox.
3. `cwd` is a path in the execution host.
4. A local path is never sent to a cloud hook as its working directory.
5. A cloud hook cannot execute on the worker daemon host outside the task sandbox.
6. Worker Direct, Docker, Kubernetes, and command-dispatched backends preserve the same contract when they support first-party Oz.
7. A backend that cannot execute the contract must reject hook-enabled runs rather than silently run with partial lifecycle coverage.
8. An MAA deployment that does not acknowledge `warp.oz_hook.v1` must reject or be rejected by a hook-enabled runtime before the runtime applies any tool action.

## Decisions

### Warp-native configuration

Options:
- Read Claude Code and Codex configuration directly.
- Define Warp configuration with familiar event names.

Decision:
- Use `~/.warp/hooks.json` and `.warp/hooks.json`.

Why:
- Oz needs Warp-specific trust, redaction, permissions, cloud, and protocol semantics.
- Reusing third-party files would imply compatibility that v1 does not provide.

### Full tool coverage

Options:
- Hook only client-executed actions.
- Add a blocking MAA protocol round trip for server-owned tool and compaction boundaries.

Decision:
- Add the protocol round trip.

Why:
- Partial coverage would make policy hooks unreliable.
- A hook author must not need to know where a tool happens to execute.
- The added protocol work is preferable to a false enforcement guarantee.

### Deny-only control

Options:
- Support allow, ask, deny, and input mutation.
- Support deny only.

Decision:
- Support deny only in `PreToolUse`.

Why:
- Deny composes safely with Warp permissions.
- Allow and mutation could bypass user intent or invalidate the audited tool request.

### Sequential execution

Options:
- Match Codex and launch all handlers concurrently.
- Run handlers sequentially in deterministic order.

Decision:
- Run handlers sequentially and stop after a denial.

Why:
- Deterministic side effects and diagnostics are easier to reason about.
- Policy hooks can avoid unnecessary work after a denial.
- The trade-off is higher cumulative latency.

### Fail-open default with explicit fail-closed policy

Options:
- Always fail open.
- Always fail closed.
- Default to fail open and allow `PreToolUse` handlers to opt into fail closed.

Decision:
- Use the third option.

Why:
- Observability hooks must not break agent work.
- Security policy hooks need an explicit availability-over-progress choice.

### Host-local execution

Options:
- Execute every hook on the user's local machine.
- Execute hooks where the Oz runtime executes.

Decision:
- Execute hooks where Oz executes.

Why:
- Cloud paths, files, tools, and sandbox boundaries exist only in the worker task environment.
- Sending cloud tool data to a laptop would add latency and a new data boundary.

## Assumptions

- The exact trust-review presentation can use Warp's existing confirmation patterns. A separate visual editor is not required.
- Cloud environments that need user hooks will provision `~/.warp/hooks.json` and matching trust material before the Oz session starts.
- `manual` is reserved for a future explicit compaction action even if the first implementation only emits `auto`.
- Hook event and handler limits are sufficient for v1 policy and observability use cases.
- The 120-second non-`SessionEnd` timeout maximum is sufficient for synchronous v1 handlers.

## Out of scope

- Enterprise-managed hook layers and mandatory organization policy.
- Hook credential injection.
- Plugin-packaged hooks.
- Per-hook enable and disable controls.
- Live configuration reload.
- Hook retries.
- Cross-session hook state managed by Warp.
- A guarantee that `SessionEnd` runs after a process crash or infrastructure loss.
- Native alias matching for Claude Code tool names.

## Validation criteria

1. Config parser tests run with `cargo test -p warp oz_hooks_config` and cover:
   - valid user and project files
   - invalid schema versions
   - unknown fields and events
   - invalid regular expressions
   - timeout bounds
   - `on_failure` restrictions
   - file and handler limits
2. Merge-order tests run with `cargo test -p warp oz_hooks_ordering` and prove user-before-project and declaration-order execution.
3. Trust tests run with `cargo test -p warp oz_hooks_trust` and prove new, changed, revoked, and untrusted project definitions do not run without the exact hash.
4. Payload golden tests run with `cargo test -p warp oz_hooks_payload` and cover all seven events.
5. Redaction tests run with `cargo test -p warp oz_hooks_redaction` and prove secrets, environments, attachments, file contents, transcripts, and oversized values never reach serialized payloads or logs.
6. Runtime tests run with `cargo test -p warp oz_hooks_runtime` and cover sequential execution, deny short-circuiting, process-group cancellation, timeout, spawn failure, non-zero exit, malformed JSON, unsupported fields, oversized output, fail-open, and fail-closed behavior.
7. Permission tests run with `cargo test -p warp oz_hooks_permissions` and prove hooks cannot upgrade a Warp deny or bypass a Warp prompt.
8. Local Oz integration tests emit all seven events, prove exact ordering, and prove a denied tool produces no side effect and no `PostToolUse`.
9. Multi-agent server tests run with `go test ./logic/ai/multi_agent/...` and prove server-owned tools and `PreCompact` pause for a correlated hook result.
10. Proto generation runs with `./script/generate -a multi_agent -v v1`. A following `git diff --exit-code` in `warp-proto-apis` proves generated bindings are current.
11. Cloud worker tests run with `go test ./internal/worker/...` and prove hook commands execute in the task workspace for Direct and containerized backends without inheriting worker credentials.
12. A cloud Oz integration test emits all seven events from the worker sandbox and proves its payload contract matches the local golden payloads.
13. Regression tests start Oz, Claude Code, Codex, Gemini, and OpenCode harnesses and prove only `HarnessKind::Oz` activates this hook runtime.
14. `./script/presubmit` passes in `warp`.
15. No visual or computer-use validation is required.

## Open questions

None. The requester approved this v1 direction. Implementation should begin only after the spec PR is approved.
