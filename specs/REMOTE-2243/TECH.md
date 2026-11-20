# Supervised inner shell for Oz cloud runs

## Summary
Oz cloud runs currently use one interactive shell for setup and agent commands. If an agent command exits that shell, Warp finalizes the conversation and reports the task as `FAILED`. The REMOTE-2243 incident context reports more than 100 occurrences in three days. This change keeps the bootstrapped shell alive as a supervisor, runs agent commands in a replaceable child shell, and recovers the conversation after the child exits. It also makes the server guardrail fail closed for high-confidence shell-killing commands.

This specification is based on:

- `warpdotdev/warp` at `061318ff7fc424e41fbd77e30432995d483c99e4`.
- `warpdotdev/warp-server` at `f1d9f3d34e2dffdff6dfcf1a91b0318194ffd6fa`.
- `warpdotdev/warp-proto-apis` as read on 2026-08-28. The implementer must pin the current base SHA in the implementation PR that changes the protocol.

The first rollout applies only to Oz-harness cloud runs. Local Oz runs and third-party cloud harnesses keep their current process model.

## Product behavior
1. Warp starts an Oz cloud run with a supervisor shell and one managed child shell.
2. Environment setup commands finish before Warp starts the managed child shell.
3. The managed child inherits the supervisor's exported environment and post-setup working directory.
4. Agent shell tools run only in the managed child.
5. The following actions must not end the run when they end only the managed child:
   - A direct `exit` command.
   - A direct `logout` command.
   - `kill $$`, with or without a literal signal argument.
   - `exec <program>` followed by that program exiting.
   - A sourced script that exits its caller.
   - A command failure after the managed child enables `set -e`, `set -u`, or both.
   - A monitor command that exits the current shell to report completion.
6. When the managed child exits during a tool call:
   - Warp does not replay the command.
   - Warp returns an error for that tool call.
   - The error states that the command ended the managed shell and that Warp created a replacement.
   - The error tells the agent to inspect side effects before it retries.
   - Warp starts the next model turn only after the replacement shell is ready.
7. Warp restores the latest known managed-shell working directory before it releases the replacement shell for new commands.
8. If the latest directory cannot be restored:
   - Warp continues in the post-setup starting directory.
   - The interrupted tool-call error names both the failed target and the fallback directory.
   - Warp records the restoration failure in telemetry.
9. A successful recovery does not mark the conversation or cloud task terminal.
10. Warp allows three recoveries during one conversation. The count does not reset with time or after a successful recovery.
11. On the fourth managed-shell exit:
    - Warp does not create another child.
    - Warp uses the existing `RenderableAIError::AgentExitedShell` terminal failure path.
    - The cloud task reaches `FAILED`.
    - The final error names the secret-redacted command when one is available.
12. An actual PTY exit or supervisor-shell exit remains terminal on the first occurrence.
13. A user-created nested shell does not consume the recovery budget. Returning from a nested shell to the managed child is normal shell behavior.
14. Shared-session viewers remain connected across a successful recovery.
15. Shared-session viewers see the interrupted command and its tool error through the normal conversation stream.
16. Shared-session viewers do not see supervisor bootstrap commands, replacement-shell launch commands, or a false run-termination event.
17. Warp does not show a new recovery banner in the first release.
18. Environment setup commands keep their current failure contract. A setup command that exits the supervisor shell fails setup and does not trigger managed-shell recovery.
19. When the client supervision flag is disabled, the existing single-shell behavior is unchanged.
20. When the server guardrail flag is disabled, the existing server validation behavior is unchanged.

## Technical design

### Current system

#### Shell creation and setup
`DockerSandboxShellStarter` stores one client-generated session ID for the sandbox shell. The Unix PTY starter runs `sbx run ... -- -c 'cd /home/agent && exec bash --rcfile <init> --noprofile'`. The resulting Bash process is the PTY child and is the only durable interactive shell.

References:

- `crates/warp_terminal/src/local_tty/docker_sandbox.rs:49-136`
- `crates/warp_terminal/src/local_tty/unix.rs:65-99`
- `app/src/ai/agent_sdk/driver.rs:2753-3238`

`AgentDriver::run_internal` waits for terminal bootstrap, starts dependent services, prepares repositories, runs environment setup commands, and loads Oz skills before it starts the Oz conversation. Setup commands execute through `TerminalDriver`.

#### Session identity
The shell-hook protocol already carries enough identity for death detection:

- `Precmd`, `Preexec`, `CommandFinished`, `Bootstrapped`, and `InitShell` carry a `session_id`.
- `DProtoHook::session_id()` extracts the identity.
- `DProtoHook::requires_registered_session()` requires recognized sessions for state-changing hooks.
- `TerminalModel::register_session_id()` registers client-generated IDs before Warp trusts those hooks.

References:

- `crates/warp_terminal/src/model/ansi/dcs_hooks.rs:31-78`
- `crates/warp_terminal/src/model/ansi/dcs_hooks.rs:184-230`
- `crates/warp_terminal/src/model/ansi/dcs_hooks.rs:458-572`
- `app/src/terminal/bootstrap.rs:99-171`
- `app/src/terminal/view.rs:15198-15213`

`ModelEventDispatcher` stores the `session_id` from the latest `Precmd`, but it emits `AnsiHandlerEvent::Precmd` without the ID. `TerminalDriver` therefore cannot distinguish a prompt from the current shell from a prompt that means control returned to its parent. `Preexec` has the same loss at the app event boundary. The implementation needs the `Precmd` ID. It does not need `Preexec` ID for death detection.

References:

- `app/src/terminal/model_events.rs:35-98`
- `app/src/terminal/model_events.rs:172-199`
- `app/src/terminal/model_events.rs:480-500`

#### Existing failure behavior
`TerminalDriver` has a permanent `shell_exited` Boolean. `TerminalView::Event::Exited` sets it and fails setup command waiters. This event represents PTY death, not the exit of a nested shell.

References:

- `app/src/ai/agent_sdk/driver/terminal.rs:130-184`
- `app/src/ai/agent_sdk/driver/terminal.rs:475-545`
- `app/src/ai/agent_sdk/driver/terminal.rs:768-879`

`TerminalModel::exit` force-finishes the active block with exit code `0` because no more PTY output can arrive. That behavior is valid only for PTY termination. It must not represent a recovered managed-child exit.

Reference: `app/src/terminal/model/terminal_model.rs:1467-1490`

The conversation failure path is:

- `BlocklistAIController::fail_conversation_due_to_shell_exit`.
- `RenderableAIError::AgentExitedShell`.
- `LocalAgentTaskSyncModel::classify_renderable_error`, which maps the error to `FAILED` and `InvalidRequest`.
- `TelemetryEvent::AgentExitedShellProcess`.

References:

- `app/src/ai/blocklist/controller.rs:2823-2916`
- `app/src/ai/agent/mod.rs:723-740`
- `app/src/ai/blocklist/local_agent_task_sync_model.rs:635-651`
- `app/src/server/telemetry/events.rs:5638`

#### Shell tool completion
`ShellCommandExecutor` resolves a requested command after it receives later block metadata. It currently has no result for "the command ended the shell, and Warp recovered." `RunShellCommandResult` also has no error variant. Treating this case as `CancelledBeforeExecution` loses the fact that the command ran. Treating it as exit code `0` or a synthetic nonzero exit code reports false command semantics.

References:

- `app/src/ai/blocklist/action_model/execute/shell_command.rs:40-110`
- `app/src/ai/blocklist/action_model/execute/shell_command.rs:238-341`
- `crates/ai/src/agent/action_result/mod.rs:191-271`
- `crates/ai/src/agent/action_result/convert.rs:18-102`
- `warp-proto-apis/apis/multi_agent/v1/task.proto:1247-1275`

#### Server guardrail
The `run_shell_command` tool prompt says not to end the active shell or enable strict mode. It does not name `logout`, `exec`, `kill $$`, sourced-script exits, or monitor patterns.

Reference: `warp-server/logic/ai/multi_agent/utils/output/tool_call/shared/run_shell_command.go:17-40`

`validateNoShellExit` currently uses broad regular expressions for the word `exit` and for strict-mode text. It returns one retryable error and then returns `nil` after the retry budget is exhausted. Tests require the second dangerous command to pass through.

References:

- `warp-server/logic/ai/multi_agent/utils/output/tool_call/shared/run_command_validation.go:12-144`
- `warp-server/logic/ai/multi_agent/utils/output/tool_call/shared/run_command_validation_test.go:11-138`

### Process model
The initial sandbox Bash process remains the supervisor. Do not replace the `sbx` process model and do not add a shell-side `while true` loop.

The launch sequence is:

1. Start and bootstrap the current sandbox shell. Its registered ID becomes `supervisor_session_id`.
2. Run cloud-provider setup, repository preparation, environment setup commands, and skill preparation with the current sequence.
3. Capture the post-setup working directory as `fallback_cwd`.
4. If the run is an Oz cloud run and `SupervisedOzCloudShell` is enabled, call `TerminalDriver::start_managed_shell`.
5. Generate `managed_session_id` on the client and register it with `TerminalModel` before writing child-launch or bootstrap bytes.
6. Start one interactive Bash child from the supervisor through a hidden infrastructure command.
7. Reuse the existing `InitSubshell` bootstrap path, but let the managed-shell caller provide the pre-registered ID instead of generating an unrelated ID inside `write_init_subshell_bytes_to_pty`.
8. Wait for `Bootstrapped { session_id: managed_session_id, is_subshell: true }` and the first `Precmd` from that ID.
9. Use the existing seven-second subshell bootstrap timeout.
10. Start the Oz conversation only after the child is ready.

The child inherits:

- The supervisor's exported environment, including exported values established by setup.
- The supervisor's post-setup working directory.
- The sandbox filesystem and process namespace.

The child does not inherit non-exported variables, aliases, functions, traps, or shell options that setup created. This is an intentional limitation. The implementation must record telemetry for initial managed-shell bootstrap failure and return a setup-stage `AgentDriverError::ManagedShellBootstrapFailed`. It must not start a conversation against the supervisor as a silent fallback.

The supervisor remains alive and waits for the child process while the child runs. When the child exits, the supervisor returns to its prompt and emits the death signal. The supervisor must not execute agent commands. Only `TerminalDriver` may write managed-shell infrastructure operations.

### Managed-shell state
Replace the overloaded meaning of `shell_exited` with explicit outer- and inner-shell state. `shell_exited` may keep its name if all call sites and comments state that it means PTY or supervisor exit.

`TerminalDriver` owns this state:

```rust
enum ManagedShellPhase {
    Disabled,
    Starting,
    Active,
    Recovering,
    Exhausted,
}

struct ManagedShellState {
    phase: ManagedShellPhase,
    supervisor_session_id: SessionId,
    managed_session_id: SessionId,
    fallback_cwd: String,
    latest_managed_cwd: String,
    recovery_count: u8,
    generation: u64,
}
```

The concrete field names may follow local naming conventions. The state and invariants must remain the same:

- There is one supervisor ID.
- There is at most one active managed ID.
- Each replacement gets a fresh registered ID.
- There is at most one recovery future.
- Duplicate hooks for one generation cannot increment the count twice.
- New agent commands cannot start in `Starting`, `Recovering`, or `Exhausted`.
- `shell_exited` remains terminal for `TerminalView::Event::Exited`.

### Death detection
Change `AnsiHandlerEvent::Precmd` to carry `Option<SessionId>`. Preserve the ID when `ModelEventDispatcher` translates `HandlerEvent::Precmd`. Expose a narrow managed-shell lifecycle event to `TerminalDriver` and `ShellCommandExecutor`.

The detector uses these rules:

1. `Precmd(managed_session_id)` means the managed child is ready. It also means a nested command or nested shell returned normally to the managed child.
2. `Precmd(supervisor_session_id)` while the phase is `Active` means the managed child exited.
3. `Precmd(supervisor_session_id)` while the phase is `Recovering` is a duplicate or an infrastructure prompt. It does not consume another recovery.
4. A prompt from another registered session is a nested shell or remote session. It does not consume a recovery.
5. A hook with no ID or an unregistered ID cannot establish death. Existing hook validation continues to reject untrusted session identity.
6. `TerminalView::Event::Exited` means the PTY or supervisor exited. It bypasses managed recovery and uses the current terminal failure path.

The managed-child exit event must reach `ShellCommandExecutor` before ordinary block-completion metadata for the supervisor prompt. Add an ordering regression test. This invariant prevents an `exit 0` tool call from resolving as a successful command before recovery starts.

`Preexec` identity is not needed for the transition. Keep it available for future diagnostics only if preserving it is a small change in the same event conversion.

### Block routing and hidden infrastructure
Use the managed session ID as the session identity for agent command blocks. Keep the supervisor session ID on supervisor prompts and infrastructure blocks.

Managed-shell launch, bootstrap, cwd restoration, and supervisor prompts are infrastructure:

- Do not attach agent action metadata to them.
- Do not return them as tool-call blocks.
- Do not add them to user command history.
- Hide them from the local and shared block stream by reusing the existing hidden-subshell-block mechanism.
- Do not send a session-finished event while recovery is possible.

Do not hide the interrupted agent command block. Preserve its output for debugging. Mark its action as interrupted through the dedicated shell-interruption result. Do not mutate its result to exit code `0`.

### Recovery sequence
On the first three transitions from the managed ID to the supervisor ID:

1. Atomically change `Active` to `Recovering`.
2. Increment `recovery_count`.
3. Stop accepting new agent shell actions.
4. Mark the active `run_shell_command`, pending command start, long-running command poll, or write waiter as interrupted.
5. Do not replay the interrupted command.
6. Generate and register a fresh managed session ID.
7. Launch and bootstrap a new Bash child through the same hidden path as the initial child.
8. Restore `latest_managed_cwd`.
9. If restoration fails, remain in `fallback_cwd`.
10. Change the phase to `Active`.
11. Resolve the interrupted tool action with a structured error that includes the recovery and cwd outcome.
12. Allow the next model turn.

Use the existing seven-second subshell bootstrap timeout for each replacement. A launch, bootstrap, or restoration failure consumes the current recovery. A launch or bootstrap failure may immediately try the next recovery while the count is below three. A cwd restoration failure does not start another recovery because the replacement shell is usable.

On the fourth detected child exit, or when the third recovery cannot produce a ready child:

1. Change the phase to `Exhausted`.
2. Resolve internal waiters so no future hangs.
3. Call `fail_conversation_due_to_shell_exit` with the secret-redacted active or last command.
4. Emit the cap-exhausted telemetry event.
5. Do not close the PTY before conversation failure and task-state events flush.

Three recoveries bound accidental loops while allowing an agent one initial mistake and two subsequent corrections. A lifetime counter is deterministic, easy to test, and does not require timers. Telemetry will show whether the cap needs adjustment.

### Working-directory restoration
Store cwd as the shell-reported string. Do not require the path to exist on the client host.

Update `latest_managed_cwd` from:

- `Precmd` block metadata for `managed_session_id`.
- `BlockWorkingDirectoryUpdated` events, including OSC 7 updates, for `managed_session_id`.

References:

- `app/src/terminal/model_events.rs:386-399`
- `app/src/terminal/view.rs:7886-7926`

Ignore cwd updates from the supervisor and from nested session IDs.

Run cwd restoration inside the replacement child as a hidden, shell-escaped `builtin cd -- <target>` operation. Wait for its completion before setting the phase to `Active`. If it fails, confirm the child remains in `fallback_cwd` and include both paths in the tool error.

Do not restore arbitrary shell state. Do not rerun setup commands.

### Tool-call error protocol
Add a structured error to `RunShellCommandResult` in `warp-proto-apis`:

```proto
message RunShellCommandResult {
  // Existing fields omitted.
  oneof result {
    LongRunningShellCommandSnapshot long_running_command_snapshot = 4;
    ShellCommandFinished command_finished = 5;
    PermissionDenied permission_denied = 6;
    ShellCommandInterrupted error = 7;
  }
}

message ShellCommandInterrupted {
  enum Reason {
    REASON_UNSPECIFIED = 0;
    MANAGED_SHELL_EXITED = 1;
  }
  Reason reason = 1;
  bool shell_recovered = 2;
  string restored_cwd = 3 [(sensitive) = true];
  string requested_cwd = 4 [(sensitive) = true];
}
```

Generate the Go binding with:

`./script/generate -a multi_agent -v v1`

Add `RequestCommandOutputResult::Error` in `crates/ai/src/agent/action_result/mod.rs`. Update both conversion directions and all exhaustive renderers. The model-facing error text is:

> The command ended the managed shell. Warp started a replacement shell and did not replay the command. Inspect partial side effects before retrying.

Append one cwd sentence:

- Success: `Warp restored the working directory to <restored_cwd>.`
- Fallback: `Warp could not restore <requested_cwd> and continued in <restored_cwd>.`

The result is a tool error, not a cancellation and not a completed command. Persist it in conversation history so a resumed conversation retains the failure reason.

If the recovery cap is exhausted, use the existing terminal conversation error instead of sending a recoverable tool result.

### Executor behavior
`ShellCommandExecutor` must:

- Subscribe to the managed-shell lifecycle signal.
- Resolve the active requested-command action only after recovery succeeds or exhausts.
- Prefer the interruption result over block completion when both events refer to one command.
- Remove block-finished and force-refresh senders for the interrupted action.
- Resolve `ReadShellCommandOutput`, `WriteToLongRunningShellCommand`, and transfer-control waiters for the interrupted block without hanging.
- Return the managed-shell interruption error for the action that was active at death.
- Return `BlockNotFound` for later operations that reference an interrupted long-running block after the interruption result has been consumed.
- Reject or hold a new command while recovery is in progress. It must not write it to the supervisor.

`TerminalDriver` must:

- Keep setup-command handling unchanged.
- Start the first managed child after setup and before `execute_run`.
- Own the recovery count, bootstrap waiter, and cwd restoration outcome.
- Treat `Event::Exited` as outer-shell death.
- Fail its own `waiting_command` and `pending_command_start` only for outer-shell death or setup failure.
- Emit a distinct managed-shell recovery result for the action executor.

### Prompt and guardrail changes
Update the `run_shell_command` tool description with explicit forbidden examples:

- Do not run `exit`, `exit 0`, or `logout` in the active shell.
- Do not run `kill $$` or `kill -TERM $$`.
- Do not use `exec <program>` to replace the active shell.
- Do not enable `set -e`, `set -u`, or `set -euo pipefail` in the active shell.
- Do not `source` or dot-source a script that can exit its caller or enable persistent strict mode.
- Do not end a monitor with `exit` to report completion.

State the safe alternatives:

- Execute a script as a subprocess instead of sourcing it when it may exit.
- Put strict-mode work in `bash -c 'set -euo pipefail; ...'`.
- Return a normal command status from a monitor without exiting the interactive shell.

Replace the regex-only `commandMayExitShell` classifier with a Bash AST classifier using `mvdan.cc/sh/v3/syntax`. Add the dependency directly to `go.mod`.

The classifier rejects only high-confidence current-shell operations:

- A simple command whose executable word is literal `exit` or `logout`.
- `builtin exit`, `builtin logout`, `command exit`, and `command logout`.
- A simple `set` command whose literal options enable `errexit` or `nounset`.
- A literal `exec` with a command operand.
- A literal `kill`, `builtin kill`, or `command kill` whose target argument is the special parameter `$$`.

Treat a statically evaluable quoted executable word such as `"exit"` as an executable word. Do not scan quoted argument contents for keywords.

The AST walk must not reject text that is not executed in the current shell. Do not descend into:

- Single-quoted or double-quoted argument contents.
- Comments.
- Here-document bodies.
- Function declaration bodies.
- Command substitutions.
- Process substitutions.
- Explicit subshell groups.
- The string argument to `bash -c`, `sh -c`, or another interpreter.

This permits these examples:

- `printf '%s\n' 'exit 1'`
- `cat > script.sh <<'EOF'\nexit 1\nEOF`
- `bash -c 'set -e; false'`
- `(exit 1)`
- `tail exit.log`
- `git grep exit`
- `exec >output.log`
- `source .venv/bin/activate`

The server cannot know whether an external sourced file exits, enables strict mode, or installs a later exit trap because the server does not have that file. Do not reject every `source` or `.` command. That would recreate the broad false positives tracked outside REMOTE-2243 and would block common virtual-environment activation. The explicit prompt is the preventive control for sourced files. The client supervisor is the recovery control.

Keep `ShellExitCommandError` retryable once. After the tracker records one `shell_exit_command` retry, a second high-confidence violation must return `ShellExitCommandExhaustedError`. The exhausted error must not implement `output.OutputProcessingError`. This follows the fail-closed pattern in `event_hallucination_error.go`: the server rolls back the contaminated model output, sends a non-retryable model-output error, and never dispatches the tool action.

References:

- `warp-server/logic/ai/multi_agent/agents/primary/event_hallucination_error.go:1-51`
- `warp-server/logic/ai/multi_agent/agents/llm_agent/llm_agent.go:403-496`

The fail-closed behavior is gated by `enable_fail_closed_shell_exit_validation`. When disabled, retain the current validator and retry-then-allow behavior for cheap rollback.

### Feature flags and rollout
Add independent controls:

- Client runtime feature: `SupervisedOzCloudShell`.
- Server configuration: `enable_fail_closed_shell_exit_validation`.

Both default to disabled in production.

Capture the client gate once at run startup. It is enabled only when all of these conditions are true:

- `FeatureFlag::SupervisedOzCloudShell` is enabled.
- `AgentDriver` has a cloud task ID.
- The selected harness is `HarnessKind::Oz`.

Do not re-evaluate the gate during the conversation.

Roll out in this order:

1. Deploy protocol support and client support with the client flag disabled.
2. Enable the client flag in local and staging cloud runs.
3. Run the required cloud smoke matrix.
4. Enable the client flag for Warp dogfood cloud runs.
5. Enable it for 5%, 25%, 50%, and 100% of Oz cloud runs.
6. Hold each production stage for at least 24 hours and compare fatal shell exits, recovery success, setup failures, and task completion with the previous stage.
7. Deploy the AST validator and prompt changes with the server fail-closed flag disabled.
8. Enable server fail-closed validation in local, staging, dogfood, then the same production stages.

Do not enable the server fail-closed flag broadly before false-positive tests and dogfood telemetry pass.

Rollback is independent:

- Disable `SupervisedOzCloudShell` to return new runs to the single-shell client behavior.
- Disable `enable_fail_closed_shell_exit_validation` to restore the current one-retry-then-allow server behavior.
- Do not require a client release or database migration for either rollback.

Existing runs keep the flag values captured at run start. Do not change process model in the middle of a conversation.

### Telemetry
Keep `AgentExitedShellProcess` for terminal outcomes only:

- Actual PTY or supervisor exit.
- Managed-shell recovery cap exhaustion.

Do not emit it for a successful managed-shell recovery.

Add these client events:

- `ManagedShellBootstrapFailed`
- `ManagedShellRecoveryAttempted`
- `ManagedShellRecoverySucceeded`
- `ManagedShellRecoveryFailed`
- `ManagedShellRecoveryExhausted`

Include:

- Cloud task and conversation identifiers through existing telemetry context.
- Recovery count and managed generation.
- Phase at detection.
- Whether an agent shell action was active.
- Whether child bootstrap succeeded.
- Whether cwd restoration succeeded or used fallback.
- Recovery latency.
- A coarse detection source: `supervisor_precmd` or `outer_pty_exit`.

Do not include raw command text, cwd text, environment values, or raw session IDs in new telemetry.

Add server counters:

- High-confidence shell-exit validation detections by reason.
- First corrective retry.
- Exhausted fail-closed rejection.
- AST parse failure.

Measure:

- Fatal `AgentExitedShellProcess` events per 1,000 Oz cloud runs.
- Successful recoveries divided by recovery attempts.
- Runs that reach the recovery cap.
- Task success after at least one recovery.
- Setup failure rate before and after rollout.
- Guardrail rejection rate and user-reported false positives.

The production success target is:

- At least a 90% reduction in fatal agent-caused shell exits per 1,000 Oz cloud runs over a seven-day comparison window.
- At least 95% of recovery attempts produce a ready managed shell.
- No statistically material increase in setup failures.

## Decisions

### Scope: Oz cloud runs first
Options:

- All local and cloud terminal sessions.
  - Advantage: one process model.
  - Disadvantage: large compatibility and UI surface.
- All cloud harnesses.
  - Advantage: protects third-party agents.
  - Disadvantage: third-party harnesses own long-running CLI process lifecycles and need separate integration design.
- Oz-harness cloud runs only.
  - Advantage: directly targets the measured failures with the smallest change.
  - Disadvantage: other harnesses remain unprotected.

Decision: Start with Oz-harness cloud runs only. The requester confirmed cloud-run scope.

### Recovery owner: client, not a shell loop
Options:

- A shell-side `while true` loop.
  - Advantage: small shell script.
  - Disadvantage: cannot safely register fresh session IDs, coordinate tool errors, gate new commands, or enforce conversation-scoped limits.
- A client-driven supervisor.
  - Advantage: owns session registration, block routing, telemetry, action errors, and the recovery budget.
  - Disadvantage: requires client state and event plumbing.

Decision: Use a client-driven supervisor.

### Death signal: supervisor `Precmd`
Options:

- Infer death from command strings.
  - Advantage: no hook changes.
  - Disadvantage: misses sourced exits, strict-mode exits, `exec`, and indirect failures.
- Watch OS child PIDs.
  - Advantage: direct process information.
  - Disadvantage: couples the client to container process discovery and does not solve block/session routing.
- Detect a `Precmd` transition from managed ID to supervisor ID.
  - Advantage: uses existing validated shell identity and works for all common child-exit mechanisms.
  - Disadvantage: requires preserving the ID through the app event boundary.

Decision: Detect return to the known supervisor ID.

### Recovery budget: three per conversation
Options:

- Unlimited.
  - Advantage: maximum persistence.
  - Disadvantage: permits infinite failure loops and unbounded cost.
- Rolling time window.
  - Advantage: tolerates long healthy runs.
  - Disadvantage: adds timers and makes behavior harder to reason about.
- Three for the conversation lifetime.
  - Advantage: deterministic, bounded, and enough for one mistake plus two corrections.
  - Disadvantage: a very long run can exhaust the budget across unrelated incidents.

Decision: Allow three lifetime recoveries and fail on the fourth child exit.

### State restoration: cwd only
Options:

- Restore all shell state.
  - Advantage: closest continuity.
  - Disadvantage: aliases, functions, traps, options, jobs, and process state cannot be captured safely and completely.
- Rerun setup.
  - Advantage: may rebuild state.
  - Disadvantage: setup can be expensive or non-idempotent.
- Restore cwd only and inherit the supervisor's exported setup environment.
  - Advantage: covers the most important navigation state without replaying side effects.
  - Disadvantage: managed-shell state created during the conversation is lost.

Decision: Restore cwd only. Fall back to the post-setup directory and continue if restoration fails.

### Interrupted command: structured tool error
Options:

- Replay automatically.
  - Advantage: transparent when idempotent.
  - Disadvantage: can duplicate destructive or external side effects.
- Report exit code `0` or a synthetic nonzero code.
  - Advantage: no protocol change.
  - Disadvantage: invents command semantics and can mislead the model.
- Return a structured error after recovery.
  - Advantage: accurately states what happened and lets the model inspect before retrying.
  - Disadvantage: needs a protocol addition.

Decision: Return a structured tool error and never replay automatically.

### Guardrail exhaustion: fail closed
Options:

- Keep retry-then-allow.
  - Advantage: no terminal model-output error.
  - Disadvantage: knowingly dispatches the dangerous command after one warning.
- Return a synthetic client tool rejection.
  - Advantage: conversation can continue.
  - Disadvantage: requires dispatching a tool action only to reject it later.
- Fail the model output before dispatch.
  - Advantage: dangerous output never reaches the terminal and mirrors an existing retry-exhaustion pattern.
  - Disadvantage: the model turn ends with an error after two violations.

Decision: Reprompt once, then fail the model output without dispatch.

### Guardrail parser: high-confidence AST checks
Options:

- Expand regular expressions.
  - Advantage: small diff.
  - Disadvantage: repeats known false positives in quoted text, filenames, heredocs, and nested scripts.
- Reject every source or strict-mode token.
  - Advantage: catches more potential hazards.
  - Disadvantage: blocks safe and common commands.
- Parse the outer Bash syntax and reject only current-shell operations.
  - Advantage: distinguishes executable shell syntax from text and nested subprocesses.
  - Disadvantage: intentionally leaves dynamic and sourced-file contents to client recovery.

Decision: Use an AST classifier and accept documented false negatives where the server lacks file or runtime context.

### Viewer UX: no new banner
Options:

- Add a recovery banner.
  - Advantage: explicit.
  - Disadvantage: adds UI scope and can distract from the tool error.
- Use the normal interrupted tool error and telemetry.
  - Advantage: keeps the first release small and preserves shared-session continuity.
  - Disadvantage: viewers do not get a separate recovery affordance.

Decision: Do not add a banner in this work.

## Assumptions
- Assumption: Cloud Oz runs continue to use Bash in the Docker sandbox for this rollout.
- Assumption: The existing `InitSubshell` bootstrap path is reliable when the caller pre-registers and supplies the session ID.
- Assumption: Three lifetime recoveries are enough to stop accidental loops without hiding repeated agent misuse. Telemetry will validate this value.
- Assumption: The post-setup directory is always available as a valid fallback while the sandbox remains alive.
- Assumption: Common environment setup relies primarily on filesystem effects, exported environment variables, and cwd. Non-exported functions and aliases are not guaranteed after the supervisor-to-child boundary.
- Assumption: A non-retryable `ProduceActions` validation error keeps the dangerous tool action out of conversation history, consistent with the event-hallucination fail-closed path.
- Assumption: Shared-session transport remains attached to the PTY rather than to one shell session ID.

## Out of scope
- Arbitrary shell-state restoration, including non-exported variables, aliases, functions, shell options, traps, shell history, activated virtual-environment shell functions, and prompt customizations.
- Recovery or adoption of background jobs, detached processes, foreground process groups, or alt-screen application state.
- Replaying the interrupted command.
- Rerunning environment setup after recovery.
- Redesigning setup-command syntax, ordering, idempotency, or failure policy.
- Supervising local Oz runs.
- Supervising Claude Code, Codex, Gemini, OpenCode, or other third-party harnesses.
- Protecting against commands that deliberately kill the supervisor, the PTY owner, the sandbox, or the worker.
- Static inspection of external sourced files on warp-server.
- A viewer-facing recovery banner.
- Changes tracked by QUALITY-1481, QUALITY-902, QUALITY-821, QUALITY-146, REMOTE-2242, or REMOTE-3055.

## Risks and mitigations
- Risk: A supervisor prompt is routed as a successful command completion before the death event.
  - Mitigation: Preserve event ordering and add a regression test that `exit 0` returns the interruption error.
- Risk: Infrastructure commands leak into shared history.
  - Mitigation: Reuse the hidden subshell block path and test serialized shared blocks.
- Risk: Setup-created non-exported shell state is unavailable in the first managed child.
  - Mitigation: Document the boundary, test exported environment and cwd inheritance, and do not claim arbitrary state continuity.
- Risk: The replacement shell starts but cwd restoration hangs.
  - Mitigation: use an in-band hidden operation with the existing bounded command machinery and treat timeout as restoration failure.
- Risk: A broad guardrail blocks harmless documentation or script generation.
  - Mitigation: use AST command positions, skip non-current-shell contexts, and keep the server flag independent.
- Risk: A sourced file still kills the managed shell.
  - Mitigation: the client supervisor is authoritative recovery; the server prompt is preventive only.
- Risk: Protocol deployment order causes old clients to ignore the new error.
  - Mitigation: land the additive proto field first, then server read support, then client write support, and enable flags last.
- Risk: Recovery loops increase run cost.
  - Mitigation: cap recoveries at three and measure attempts, latency, exhaustion, and post-recovery task success.

## Validation criteria

### Warp unit tests
Add tests named with the `managed_shell` substring and run:

`cargo nextest run -p warp managed_shell`

The tests must prove:

1. A managed-ID-to-supervisor-ID `Precmd` transition emits one child-death event.
2. Duplicate supervisor `Precmd` hooks consume one recovery.
3. A nested session returning to the managed ID consumes no recovery.
4. An unknown or missing ID cannot trigger recovery.
5. Every replacement ID is generated and registered before bootstrap bytes are accepted.
6. Initial bootstrap waits for `Bootstrapped` and managed `Precmd`.
7. A replacement bootstrap timeout consumes a recovery and does not release commands to the supervisor.
8. Recovery counts persist for the conversation lifetime.
9. The fourth child exit uses `AgentExitedShell`.
10. `Event::Exited` remains immediately terminal.
11. A cwd update from the managed ID is cached.
12. A cwd update from the supervisor or nested ID is ignored.
13. Successful cwd restoration is included in the tool error.
14. Failed cwd restoration falls back and is included in the tool error.
15. A new action cannot execute while recovery is in progress.
16. `exit 0` cannot win the race and become a successful tool result.
17. Pending start, active command, long-running poll, write, and transfer waiters all resolve.
18. The interrupted command is not replayed.
19. Client telemetry does not contain raw command, cwd, or session ID values.

Add action-result conversion tests and run:

`cargo nextest run -p ai managed_shell`

They must prove the structured error round-trips through the API and remains an error in text, JSON, Markdown, and restored conversation formatting.

### Warp integration tests
Add Linux PTY integration tests under `crates/integration` and run:

`cargo run -p integration -- test managed_shell`

Use a real supervisor Bash and managed Bash. Prove:

1. Setup `export REMOTE_2243_SETUP_VALUE=present` and `cd <fixture>` are inherited by the first managed child.
2. Direct `exit`, `kill $$`, and `exec bash -c 'exit 0'` each return control to the supervisor and produce a ready replacement.
3. `source <fixture-that-exits>` produces a ready replacement.
4. `set -e; false` and `set -u; printf '%s' \"$REMOTE_2243_UNSET\"` produce a ready replacement.
5. A monitor fixture that calls `exit 0` produces a ready replacement.
6. Files written before each exit remain on disk.
7. Cwd is restored after a successful recovery.
8. Removing the prior cwd causes fallback to the post-setup directory.
9. A nested `bash` that exits returns to the managed shell without a recovery.
10. `logout` in the non-login managed Bash does not end the run or consume recovery budget.
11. One shared-session viewer remains connected and receives the interrupted action error and the next normal command.
12. The viewer receives no visible bootstrap or replacement command and no session-finished event.
13. Killing the supervisor still produces the existing terminal failure.

### warp-server tests
Run:

`go test ./logic/ai/multi_agent/utils/output/tool_call/shared -run 'Test(CommandMayExitShell|ValidateNoShellExit|ShellExitCommand)'`

The tests must reject:

- `exit`, `exit 1`, `logout`.
- `cmd && exit`, `cmd || exit 1`, and brace-group current-shell forms.
- `builtin exit` and `command exit`.
- `set -e`, `set -u`, `set -eu`, `set -o errexit`, and `set -o nounset`.
- `exec sleep 1` and `exec -- sleep 1`.
- `kill $$`, `kill -9 $$`, `builtin kill -TERM $$`, and `command kill $$`.

The tests must allow:

- `echo exit`, `printf '%s' 'kill $$'`, and `tail exit.log`.
- A heredoc or file write whose contents contain `exit` or strict mode.
- Function declarations whose uncalled body contains `exit`.
- `bash -c 'exit 1'`, `bash -c 'set -e; false'`, `(exit 1)`, and command substitution containing `exit`.
- `exec >output.log`.
- `source .venv/bin/activate`.
- `set -x`, `set -o verbose`, and `set -o pipefail` without `errexit`.

The retry tests must prove:

1. The first high-confidence violation returns `ShellExitCommandError`.
2. The retry tracker records one `shell_exit_command` retry.
3. A second violation returns `ShellExitCommandExhaustedError`.
4. No action is produced after the exhausted error.
5. Disabling the server flag restores the current retry-then-allow behavior.
6. A parser failure is counted and does not reject solely because of unparsed text.

Run the relevant LLM-agent test package to prove the exhausted error rolls back model output and terminates the model turn without dispatching a shell tool:

`go test ./logic/ai/multi_agent/agents/llm_agent/... -run ShellExit`

### Protocol verification
In `warp-proto-apis`, run:

`./script/generate -a multi_agent -v v1`

Verify:

- Generated Go bindings contain field `7` for the additive interruption error.
- Rust builds regenerate the matching type.
- No existing field number or enum value changes.

### Cloud smoke verification
Run the smoke matrix in a staging Oz cloud environment with session sharing enabled. Capture task IDs, conversation IDs, client event logs, server guardrail logs, and a video of the shared-session viewer for each case.

Required cases:

1. `exit`
2. `kill $$`
3. `exec bash -c 'exit 0'`
4. A sourced fixture that exits.
5. `set -e` followed by a failing command.
6. A monitor fixture that exits to report completion.
7. A normal command after each recovery.
8. Four sequential child exits to verify cap exhaustion.
9. A nested shell exit to verify no false recovery.
10. A killed supervisor to verify the terminal path.
11. A shared viewer connected before and after recovery.
12. The same run with the client flag disabled.

For cases 1-6, the run must remain `IN_PROGRESS` after the interrupted tool error and must accept the next command. For case 8, the run must reach `FAILED` only after the fourth child exit. For case 11, the video must show uninterrupted viewer connectivity, the action error, no infrastructure blocks, and the next successful command.

### Presubmit
Run in `warp`:

- `./script/format --check`
- `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`
- `./script/presubmit`

Run in `warp-server`:

- `./script/presubmit-go`

All required checks must pass before either rollout flag is enabled.
