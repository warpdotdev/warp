# Recover cloud agent shells by respawning the PTY

## Summary
When a cloud agent command terminates the persistent Bash process, Warp will create a new PTY and Bash process inside the same sandbox. Warp will restore only state the terminal already owns, return an interruption result to the agent, and continue the conversation. The normal command path will not change.

Tracks [REMOTE-2243](https://linear.app/warpdotdev/issue/REMOTE-2243/recover-oz-cloud-sessions-when-an-agent-command-exits-the-shell).

## Context
Cloud agent commands run directly in one persistent Bash process. `exit`, `logout`, `kill $$`, `exec`, a sourced `exit`, or `set -e` followed by a failure can terminate that process.

The current path is:

- [`DockerSandboxShellStarter`](https://github.com/warpdotdev/warp/blob/df5cacf89120ae782b9f6996f122531d3d7f8967/crates/warp_terminal/src/local_tty/docker_sandbox.rs#L65-L139) starts one Bash-backed sandbox PTY.
- The [PTY event loop](https://github.com/warpdotdev/warp/blob/df5cacf89120ae782b9f6996f122531d3d7f8967/crates/warp_terminal/src/local_tty/event_loop.rs#L363-L499) reports `ShellProcessExited` or `PtyDisconnected`, but it does not carry a child exit status.
- [`TerminalModel::exit`](https://github.com/warpdotdev/warp/blob/df5cacf89120ae782b9f6996f122531d3d7f8967/app/src/terminal/model/terminal_model.rs#L1502-L1523) force-finishes the active block with exit code `0`.
- [`TerminalView`](https://github.com/warpdotdev/warp/blob/df5cacf89120ae782b9f6996f122531d3d7f8967/app/src/terminal/view.rs#L12348-L12404) converts any post-bootstrap exit into `Event::Exited` and fails the conversation through `AgentExitedShell`.
- [`TerminalDriver`](https://github.com/warpdotdev/warp/blob/df5cacf89120ae782b9f6996f122531d3d7f8967/app/src/ai/agent_sdk/driver/terminal.rs#L825-L860) marks the shell dead and rejects later commands.
- [`ShellCommandExecutor`](https://github.com/warpdotdev/warp/blob/df5cacf89120ae782b9f6996f122531d3d7f8967/app/src/ai/blocklist/action_model/execute/shell_command.rs#L280-L365) waits for block completion and currently has no interrupted-but-recovered result.

Environment setup runs before the cloud agent conversation starts. A setup command that exits the shell must keep the existing `SetupCommandExitedShell` failure contract.

## Technical design

### 1. Preserve the normal command path
Add a client feature flag named `CloudAgentShellRespawn`.

The recovery path is active only when all conditions are true:

- The flag is enabled.
- The run uses the native cloud agent harness.
- The terminal is a cloud agent session.
- Environment setup completed and the conversation started.
- An agent-requested shell action owns the active block, or the immediately preceding block for a post-command `set -e` exit.
- The terminal reports `ShellProcessExited`.

All commands continue to run directly in the persistent Bash process. Do not wrap commands in an inner shell or supervisor.

This gate reacts only after an agent-issued command causes `ShellProcessExited`. It does not intercept or prevent commands from shutting down the shell. Manual PTY shutdown, run cancellation, and other intentional lifecycle controls keep their current behavior.

The following events remain terminal:

- An exit during bootstrap or environment setup.
- `PtyDisconnected`, PTY spawn failure, sandbox loss, worker loss, or an event-loop failure.
- Manual PTY shutdown or run cancellation.
- A shell exit with no attributable in-flight agent shell action.

### 2. Carry a typed termination outcome
Replace the boolean-only child-exit notification with a typed outcome:

```rust
enum ObservedExitStatus {
    Code(i32),
    Signal(i32),
    Unavailable,
}
```

The direct child path must retain the `ExitStatus` returned by `try_wait`. The terminal-server path must include the status in its child-termination protocol. If the OS, terminal server, or sandbox launcher does not provide a status, use `Unavailable`.

Propagate the terminal reason and observed status through the PTY event loop, `TerminalModel`, `TerminalView`, and the agent shell action. Do not convert shell death to a successful block with exit code `0`.

The observed status is the status of the PTY child (`sbx run`), not a guaranteed parse of the command text. Return it when the sandbox launcher propagates it. Otherwise return `Unavailable`; never infer a status from `exit N`.

### 3. Reuse state the terminal already owns
Do not add a recovery checkpoint service, background probes, or a new shell-state serializer.

When the shell exits, read the recovery inputs already held by the terminal and driver:

- The sandbox identity and original shell launch configuration.
- The original configured environment used to start the run.
- The last working directory already stored in active session or block metadata.
- The exported environment already stored in the session model from the latest shell hook, if present.

Use the original environment as the reliable baseline. If the session model has an exported environment at exit, reuse it after dropping shell-integration variables, stale session identifiers, and exported Bash functions (`BASH_FUNC_*`). New bootstrap values win. Do not fail recovery when that dynamic environment is absent.

Restore the last known working directory when it still exists. Otherwise use the harness working directory.

This best-effort path can lose aliases, functions, shell-local variables, shell options, traps, jobs, process groups, open file descriptors, unflushed command history, or mutations made by the fatal command. The tool result must tell the agent that shell state can be lost.

### 4. Respawn inside the existing sandbox
Add a terminal-manager operation that replaces the dead PTY without replacing the terminal view, conversation, or sandbox filesystem.

Recovery performs these steps:

1. Capture the action's partial output, observed status, and recovery inputs already available in the terminal model.
2. Mark the shell action as interrupted.
3. Stop routing writes to the dead event loop.
4. Start a fresh PTY and Bash process in the same sandbox. Generate a new shell session ID and run the normal Warp bootstrap.
5. Register the replacement session in the existing terminal model and shared-session surface.
6. Restore the available environment and working directory. Report the cwd result and warn that other shell state may be lost.
7. Wait for `SessionBootstrapped`.
8. Route later commands to the new PTY.
9. Resolve the original tool call exactly once with the recovery result.

The Docker sandbox adapter must use a supported attach or exec operation against the existing sandbox. It must not create an empty replacement sandbox. If the adapter cannot prove that the filesystem and sandbox identity were retained, recovery fails.

Do not replay the interrupted command or any setup command. The command can have partial side effects.

Keep the existing shared-session link and conversation. Viewers can see the interrupted block and later replacement-shell blocks. If the shared-session surface cannot bind to the replacement PTY, fail recovery instead of continuing invisibly.

### 5. Return a recovery result to the agent
Add a backward-compatible `RunShellCommandResult` variant in `warp-proto-apis`, then map it through the Warp action result and warp-server formatter. The result contains:

- Partial command output.
- `observed_exit_status`: code, signal, or unavailable.
- `shell_recovered`.
- The restored working directory and whether Warp used the fallback directory.
- A stable interruption reason.

The model-facing text must state:

> This command terminated the persistent cloud shell. Warp started a replacement shell and did not replay the command. Some shell state might be lost. Do not use `exit`, `logout`, `exec`, `kill $$`, or source a script that exits. Run risky exit logic in a subshell, and use the tool result to inspect its exit code. Check the reported restored state and partial side effects before retrying.

When status is unavailable, the text must say `Exit status unavailable`; it must not show `0`. Emit the tool result only after recovery succeeds. If recovery fails, use the existing terminal task-failure path with a recovery-specific cause.

### 6. Bound recovery and failure
Allow one respawn attempt for each detected shell death and three respawn attempts per run. Each detected death consumes one attempt. A respawn timeout or bootstrap failure ends the run immediately. A fourth shell death uses the existing `AgentExitedShell` failure path and states that the recovery limit was reached.

No command can start while the state is `Recovering`. Pending writes fail or wait behind the same recovery future; they must not target the dead PTY.

### 7. Telemetry, rollout, and rollback
Emit non-UGC telemetry for:

- Detection classification and whether status was available.
- Recovery started, succeeded, failed, or exhausted.
- Recovery duration and attempt number.
- Whether an existing session environment was available and whether cwd restoration used the fallback.
- Replacement bootstrap and shared-session rebind failures.

Do not include command text, paths, or environment values in new events.

Add `CloudAgentShellRespawn` to `LOCAL_FLAGS` and `DOGFOOD_FLAGS` only. Dogfood cloud runs use staging. Do not add the flag to `PREVIEW_FLAGS`, `RELEASE_FLAGS`, or production server experiments in v1. Production remains off until staging data shows successful follow-up commands, no repeated recovery loops, and no increase in sandbox or shared-session failures.

Rollback is to disable the flag. The flag-off path keeps the current terminal failure behavior. Protocol readers must treat the new result as an additive variant so rollback does not require data migration.

## Decisions

- **Respawn the PTY and shell.** This keeps every normal command on the existing execution path and isolates new behavior to shell death.
- **Reject an inner-shell supervisor.** A supervisor adds a process boundary to every command and can change shell state, signals, job control, hooks, and terminal integration.
- **Use an explicit unavailable status.** Capturing the child status is more accurate than the current `0`, but the sandbox launcher might not expose the command's intended status.
- **Reuse only state Warp already owns.** A separate checkpoint mechanism adds complexity and another source of truth. Missing dynamic state is acceptable when the agent receives an explicit warning.
- **Do not change server shell-exit validation in v1.** The existing reprompt remains a prevention layer. Making validation fail closed is separate because it changes command acceptance semantics.

## Assumptions

- V1 uses a three-attempt per-run recovery budget and one respawn attempt per incident.
- The sandbox runtime can start a replacement shell in the existing sandbox. Implementation must validate the exact `sbx` operation before relying on it.
- Existing terminal and session model data can be incomplete at shell death. Recovery continues with the original launch state and reports the loss.

## Out of scope

- Local user terminals, third-party harnesses, and setup-command recovery.
- Worker, VM, container, or sandbox restart.
- Exact restoration of aliases, functions, jobs, traps, shell-local variables, or process state.
- A new recovery checkpoint or shell-state serialization subsystem.
- Command replay.
- Changes to `validateNoShellExit`, command parsing, or prompt-only prevention.
- Changes in `oz-agent-worker` or `warp-agent-docker`.

## Testing and validation

Unit tests must cover:

- Exit code, signal, and unavailable status propagation through direct and terminal-server PTY paths.
- The recovery classification table, including setup and `PtyDisconnected`.
- Removal of the fabricated exit code `0`.
- At-exit state collection, new-session overrides, cwd fallback, and secret-free telemetry.
- Exactly-once tool-result delivery, write blocking during recovery, and the three-attempt cap.
- Feature-flag and native-cloud-agent-only gating.

Integration tests must run a native cloud agent session and verify:

- `exit 7`, `logout`, `kill $$`, `exec`, sourced `exit`, and `set -e` failure each return a recovery result and allow a later `run_shell_command`.
- The observed status is returned when available; unavailable status is explicit.
- Filesystem changes survive recovery. The prior cwd and a previously exported variable survive when the existing terminal model has them.
- Missing dynamic environment or cwd state still recovers, and the tool result warns that shell state may be lost.
- Aliases, functions, jobs, traps, and non-exported variables are documented as lost.
- The interrupted command is not replayed.
- A setup command that exits still fails setup.
- PTY disconnect, sandbox loss, respawn failure, and the fourth shell death fail the run.
- The shared-session link continues to show the interrupted block and later commands.

Run:

- `cargo nextest run -p warp_terminal` for direct and terminal-server exit-status propagation.
- `cargo nextest run -p ai -p warp shell_recovery` for at-exit state, classification, action-result, and driver tests added under a shared `shell_recovery` test-name prefix.
- `cargo run --bin integration -- test_cloud_agent_shell_respawn` for the new PTY respawn integration case.
- Generated-binding and formatter tests in `warp-proto-apis` and `warp-server` when the result variant is added.
- One local cloud run and one staging cloud run for each status class before any production rollout.

No visual proof is required. The change has no intended UI behavior.

## Parallelization
Implement the Warp PTY lifecycle and action routing sequentially because they share one state machine. After the result shape is fixed, protocol generation and warp-server formatting can proceed in parallel on separate repository branches. Land protocol support before a Warp implementation that emits the new variant.
