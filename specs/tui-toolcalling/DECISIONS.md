# TUI tool calling architectural decisions

This document records the important architectural decisions made while adding tool execution to the `warp-tui` prototype.

## 1. Use a shared-first tool executor, not separate GUI and TUI dispatch trees

### Decision

Both GUI and TUI tool execution enter the same shared `AgentToolExecutor`.

`AgentToolExecutor` owns the top-level `AIAgentActionType` dispatch:

- shared tools are handled directly by `AgentToolExecutor`;
- inherently surface-specific tools are delegated to a required `SurfaceSpecificToolExecutor` implementation.

### Why

The first TUI implementation duplicated a top-level `match AIAgentActionType`.

Moving that match into a TUI-looking shared file did not solve the problem because GUI still used `BlocklistAIActionExecutor`'s separate dispatch. The central issue was not just code location; it was that adding a new tool could still require adding one branch for GUI and another branch for TUI.

### Alternatives considered

- **Keep TUI-specific execution in `app/src/tui/tool_model.rs`.**
  Rejected because it duplicated GUI controller/executor logic and would not scale as more tools were added.
- **Move TUI execution into `basic_tool_executor.rs`.**
  Rejected because only TUI used it, so it was shared-looking but not actually shared.
- **Use an optional override registry keyed by tool family.**
  Rejected because it was too framework-like for only two surfaces and made specialization feel optional when some tools require explicit GUI and TUI decisions.

## 2. Make shared execution the default and surface-specific execution explicit

### Decision

`AgentToolExecutor` handles tools directly when the implementation can be surface-neutral.

`SurfaceSpecificToolExecutor` is required only for tool families that cannot be implemented correctly without surface-specific state.

### Why

The desired default is that a reusable tool is implemented once and automatically works in GUI and TUI.

If a tool truly depends on terminal blocks, GUI views, TUI process state, or approval UI, both surfaces should be forced to make an explicit implementation decision.

### Alternatives considered

- **Make GUI and TUI each register optional overrides.**
  Rejected because, with only GUI and TUI surfaces, if one surface needs specialization then the other usually needs to make an explicit decision too.
- **Make every tool a backend method.**
  Rejected because it would preserve per-surface duplication and make shared execution harder to see.

## 3. Share `read_files`, `grep`, and `file_glob` execution now

### Decision

These tool families are implemented as shared `AgentToolExecutor` defaults:

- `ReadFiles`
- `Grep`
- `FileGlob`
- `FileGlobV2`

### Why

These tools primarily need session, cwd, shell launch data, and permission context. They do not require GUI terminal blocks or TUI UI state.

Sharing them proves the executor is not just shared routing; it also provides real shared execution.

### Alternatives considered

- **Keep the old GUI `ReadFilesExecutor`, `GrepExecutor`, and `FileGlobExecutor` models and add TUI equivalents.**
  Rejected because it would keep duplicate execution paths.
- **Keep the old executor models as GUI fallbacks.**
  Rejected after the shared defaults compiled and worked structurally. The old models became dead weight and confused ownership.

## 4. Keep shell command execution surface-specific for v0

### Decision

Shell command tools route through `SurfaceSpecificToolExecutor`.

- GUI delegates to `ShellCommandExecutor` and terminal blocks.
- TUI uses a local `Session`-backed command execution path.

### Why

GUI command execution is terminal-block-backed: it emits terminal events, observes `TerminalModel` block output, supports long-running block reads/writes, and integrates with GUI control handoff.

TUI does not have terminal blocks and should not fake them.

### Alternatives considered

- **Reuse `ShellCommandExecutor` directly in TUI.**
  Rejected because `ShellCommandExecutor` depends on `TerminalModel` and GUI terminal events.
- **Create fake terminal blocks for TUI.**
  Rejected as misleading and likely to create brittle coupling.
- **Fully generalize shell execution now.**
  Considered desirable long-term, but too large for v0 because long-running process state, read/write follow-up, cancellation, and user control handoff need a real TUI command model.

## 5. Treat long-running shell follow-up tools as not implemented in TUI v0

### Decision

TUI v0 does not fully implement:

- `ReadShellCommandOutput`
- `WriteToLongRunningShellCommand`
- `TransferShellCommandControlToUser`

These return `BlockNotFound`-style results until TUI has its own persistent command/process registry.

### Why

These actions refer to command/block identity and ongoing process state. GUI uses `BlockId` and `TerminalModel` for that.

TUI needs a `TuiCommandModel` or equivalent before these tools can be correct.

### Alternatives considered

- **Store minimal command ids in the first PR.**
  Considered but deferred because a correct implementation needs snapshots, stdin writes, cancellation, and control handoff semantics.
- **Map TUI command ids directly onto `BlockId` without a backing model.**
  Rejected because it would only satisfy the type shape, not the behavior.

## 6. Keep file edit execution surface-specific, but share diff application

### Decision

`RequestFileEdits` routes through `SurfaceSpecificToolExecutor`.

- GUI keeps `RequestFileEditsExecutor` and `CodeDiffView`.
- TUI uses shared diff application and auto-saves for v0.

### Why

Diff parsing and matching are shared logic. Approval, saving UI, and result timing are surface-specific.

GUI needs `CodeDiffView` and accept/reject behavior. TUI v0 intentionally auto-accepts and shows a minimal card.

### Alternatives considered

- **Reuse `CodeDiffView` from TUI.**
  Rejected because TUI should not depend on GUI editor/view types.
- **Build rich TUI accept/reject UI in this PR.**
  Rejected for v0 to keep the first tool-calling path focused on execution correctness.
- **Make file edits fully shared immediately.**
  Rejected because GUI and TUI have different approval/save semantics.

## 7. Extract shared queue/result state into `AgentToolActionModel`

### Decision

`AgentToolActionModel` owns shared action state:

- preprocessing queues;
- pending actions;
- running actions;
- finished results;
- action ordering;
- past results.

GUI wraps it through `BlocklistAIActionModel`; TUI uses it through `TuiToolActionModel`.

### Why

The model named `BlocklistAIActionModel` includes GUI/blocklist-specific behavior and naming.

TUI should not depend directly on a model whose name and surrounding responsibilities imply GUI Agent Mode internals.

### Alternatives considered

- **Rename `BlocklistAIActionModel` wholesale.**
  Considered, but the model still contains GUI-specific status updates and shared-session/view details, so a full rename would overstate how generic it is.
- **Let TUI use `BlocklistAIActionModel` directly.**
  Rejected because it would couple TUI to GUI/blocklist concepts.
- **Keep TUI's own independent action state.**
  Rejected because action ordering/result draining semantics should be common.

## 8. Encapsulate running action state in `AgentToolActionModel`

### Decision

`AgentToolActionModel` exposes methods for:

- recording running actions;
- finishing running actions;
- checking whether a conversation still has running actions.

TUI no longer maintains a separate `running_action_counts` map.

### Why

The duplicate TUI counter introduced a second source of truth and could fire `ActionsFinished` too early for synchronous multi-tool batches.

The shared model already owns running action state and should expose the operations needed by both surfaces.

### Alternatives considered

- **Keep `running_action_counts` in TUI.**
  Rejected because it could diverge from shared action state.
- **Expose the `running_actions` map broadly.**
  Rejected because callers should not need to understand or mutate the internal representation.

## 9. Record action order separately from action execution

### Decision

The method formerly named `start_action_batch` was renamed to `record_action_order`.

### Why

The method only records original tool-call order so finished results can be drained in a deterministic order. It does not start execution or mark actions running.

### Alternatives considered

- **Keep the old name.**
  Rejected because it made the shared model harder to understand.

## 10. Keep TUI tool UI minimal and TUI-specific

### Decision

TUI renders simple tool cards with concise summaries.

The TUI-only card type is named `TuiToolCard` and remains in `app/src/tui/tool_model.rs`.

### Why

The card is a rendering affordance, not shared tool execution state.

Keeping it TUI-specific prevents shared execution code from accumulating UI concerns.

### Alternatives considered

- **Share `AgentToolCard` as a common model.**
  Rejected because GUI does not use the same card shape and richer UI will likely diverge.
- **Let card fallback print full action result strings.**
  Rejected for shared tool results because read/grep/glob outputs can be too verbose for v0 cards.

## 11. Do not advertise unsupported TUI tools as complete behavior

### Decision

The architecture now allows all tool calls to enter `AgentToolExecutor`, but only a subset has meaningful TUI implementations.

Unsupported tools return cancelled-style results through the default surface-specific fallback.

### Why

The goal of this refactor was to centralize execution and get key tools working, not to claim full TUI parity.

The current meaningful TUI tool coverage is:

- command execution;
- file edits;
- read files;
- grep;
- file glob.

### Alternatives considered

- **Claim TUI supports all tools because all actions route through `AgentToolExecutor`.**
  Rejected as inaccurate: shared routing is not the same as meaningful execution.
- **Restrict `supported_tools_override` to the implemented subset immediately.**
  This remains a valid next step if we want the server/model to avoid requesting unsupported tools.
- **Move all GUI-only tools into shared defaults in this PR.**
  Rejected as too broad; each remaining tool needs a specific review of whether it is truly surface-neutral.

## 12. Keep tests and live TUI validation separate from compile-only cleanup when requested

### Decision

During the later cleanup pass, only formatting and `cargo check` were run.

Tests and live TUI/app execution were skipped when requested.

### Why

Some test/app commands require local password input and the user was away from the machine.

Compile-only validation still caught integration errors in the shared executor refactor without requiring interactive access.

### Alternatives considered

- **Run TUI live validation immediately.**
  Rejected for that pass because the user explicitly asked not to run code/tests.
- **Skip validation entirely.**
  Rejected because `cargo check` is non-interactive and necessary for a refactor crossing shared GUI/TUI code.

## 13. Extract the scheduling loop into a shared `AgentToolScheduler`

### Decision

The scheduling loop — preprocessing fan-out, the pending queue, serial/parallel phase admission, ordered result draining, and follow-up readiness — moved out of `BlocklistAIActionModel` into a shared `AgentToolScheduler` whose static methods are generic over an `AgentToolScheduleHost` trait.

`AgentToolActionModel` remains pure embedded state. `BlocklistAIActionModel` (GUI) and `TuiToolActionModel` (TUI) are thin host adapters: they implement `AgentToolScheduleHost` (execution primitives plus side-effect hooks) and delegate the loop to `AgentToolScheduler`.

### Why

Decisions 1–3 shared execution dispatch and decision 7 shared action state, but the scheduling loop itself was still GUI-only. The TUI reimplemented a degenerate version that marked every action running and spawned them all at once, with no serial barrier. Sharing the loop removes that divergence: the TUI now inherits identical ordering and phase semantics, gaining serial barriers for mutating tools and parallel fan-out for read-only tools.

### Alternatives considered

- **A single model parameterized by surface (`AgentToolActionModel<S>`).**
  Rejected because view-only/session-sharing behavior and the GUI's sub-executor accessors are GUI-only; folding them into one shared type would accrete surface flags and conditionals.
- **Share only decision helpers and keep a per-surface drive loop.**
  Rejected because the bug-prone drain/admission loop would be duplicated and could drift — the same class of divergence decision 8 already fixed for running-action state.

## 14. Preserve each surface's async-completion mechanism behind `try_execute`

### Decision

`AgentToolScheduler` never spawns execution futures. Each host's `try_execute` returns a synchronous `TryExecuteResult` and guarantees that `AgentToolScheduler::finish_action` is eventually called: the GUI completes via its executor's `FinishedAction` event subscription, the TUI via its own `ctx.spawn` callback. The TUI delivers even `Sync`/`NotReady`/`InvalidAction` results through a deferred ready-future spawn to avoid re-entrant scheduling.

### Why

GUI completion is event-decoupled (the executor model emits, the action model subscribes); TUI completion is inline on the owning model. Forcing one mechanism would mean rewriting the GUI executor's `async_executing_actions` tracking and cancellation, which is out of scope and risky.

### Alternatives considered

- **Move spawning into the scheduler, generic over the host context.**
  Rejected as more invasive and incompatible with the existing executor-owned async tracking/cancellation.

## 15. Test shared scheduling deterministically with a mock host

### Decision

Phase-admission behavior is tested in `app/src/ai/blocklist/action_model/scheduler_tests.rs` with a mock `AgentToolScheduleHost` whose `try_execute` records actions as running but never completes them, and whose `spawn_after_preprocess` runs its callback synchronously. Tests assert post-queue running/pending counts (two reads → both running; file-edit then command → one running, one pending).

### Why

Observing the transient running/pending state of real async execution is racy: the test executor can drain the spawned execution futures before the assertion, so the counts are not stable. A non-completing mock host makes admission state deterministic without timing dependence.

### Alternatives considered

- **Drive the real TUI executor and assert counts after yields.**
  Rejected as flaky; the first attempt at this relied on `yield` timing and could not reliably catch the transient "both running" window.
