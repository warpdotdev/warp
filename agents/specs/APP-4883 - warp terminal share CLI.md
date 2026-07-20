# Spec: Add `warp terminal share` CLI command

- **Linear issue:** [APP-4883](https://linear.app/warpdotdev/issue/APP-4883/add-warp-terminal-share-cli-command-for-headless-session-sharing)
- **Originating thread:** https://warp-dev.slack.com/archives/C0BDQDW8V5E/p1784535009992799
- **Target repo:** `warpdotdev/warp` (Warp Rust client)
- **Estimate:** L (5)
- **Codebase references are pinned to commit** `abea51cd1e102b363935f1b25ef03d335bc7b36f`.

---

## PRODUCT

### Summary
Add a new `warp terminal share` CLI command that starts a headless Warp terminal
session, shares it, prints the resulting join (share) link to stdout, and blocks
until the session exits. Recipients are configured with the existing `--share`
recipient syntax (`team` / `public` / `<email>`, each `:view` or `:edit`). The
command reuses the existing `TerminalDriver` (the same driver that hosts ambient
agent terminal sessions) rather than introducing a new session runtime.

The goal is to let a user (or a script) spin up a shareable terminal from the
command line — for pairing, remote assistance, or demos — without opening the
Warp GUI.

### Key design choices
1. **Reuse `TerminalDriver` directly, hosted inside the already-headless CLI
   `AppContext`** — the CLI dispatch (`agent_sdk::dispatch_command`) already runs
   inside a fully-booted, headless `AppContext` for every command, and
   `TerminalDriver::create` already runs headlessly today for local
   `warp agent run`. So "the headless bootstrap" is largely a matter of hosting
   the driver from a new dispatch arm — not building a new runtime. We drive
   `TerminalDriver` directly (not the full `AgentDriver`) because there is no AI
   agent/turn to run — only a shared shell session.
2. **Arg types in `warp_cli`, execution + driver hosting in `app`** — the new
   command's clap types live in the `warp_cli` crate (which already owns
   `ShareArgs`), while all code that touches the `pub(crate)` `TerminalDriver`
   lives in the `app` crate (a new `terminal` handler module next to the other
   `agent_sdk` command handlers). This keeps the `warp_cli` ↔ `app` visibility
   boundary clean — `warp_cli` never needs access to `app` internals.
3. **Gate the new surface behind a dedicated feature flag** (`TerminalShareCommand`),
   following the established per-subcommand gating pattern in
   `warp_cli::Args::from_env` / `clap_command`.
4. **v1 is a headless shared session with no local-TTY attach** — the command
   prints the link and blocks; interaction happens through the shared session
   (guests joining via the link). Forwarding the invoking terminal's stdin/stdout
   into the PTY is explicitly deferred (see *Design alternatives*). **This is the
   one product decision worth an explicit reviewer confirmation** (see *Open
   questions resolved* Q1).

### Behavior (numbered, testable invariants)
1. **Discoverability.** `warp terminal --help` lists a `share` subcommand, and
   `warp terminal share --help` documents the `--share` flag with the same
   recipient possible-values as elsewhere (`team[:view|:edit]`,
   `public[:view|:edit]`, `<user@email.com>[:view|:edit]`). When the
   `TerminalShareCommand` feature flag is disabled, `warp terminal` is rejected
   with `error: unrecognized subcommand 'terminal'` and hidden from top-level
   `--help` (matching every other gated subcommand).
2. **Starts a headless shared session.** Running `warp terminal share` (while
   logged in) starts a headless terminal session by reusing `TerminalDriver`
   with `should_share = true`; no GUI window is shown (the command reports
   `is_headless() == true`).
3. **Prints the join link.** Once the session is shared, the command prints
   exactly the `join_url` from
   `TerminalDriverEvent::EstablishedSharedSession { join_url, .. }` to **stdout**,
   on its own line, and nothing else non-diagnostic is written to stdout before
   it. (Diagnostics/log output go to stderr or the log file per the CLI's
   existing `LogDestination` behavior.)
4. **Applies recipient flags.** The recipients parsed from `--share` are applied
   to the shared session via `TerminalDriver::add_share_requests`, mapping
   `:view` → `Role::Reader` and `:edit` → `Role::Executor`, and `team` /
   `public` / `<email>` to the corresponding `ShareSubject` — identical to the
   ambient-agent path. When no recipients are specified — the `--share` flag is
   omitted, or passed bare with no value — the session is shared **owner-only**
   (share-with-self): the invoking user already has access and no additional ACL
   is applied. Recipients are added only when explicitly passed. (See Q2.)
5. **Blocks until the session exits.** The command runs (does not return) while
   the session is live, and returns only when the underlying session/shell PTY
   exits (e.g. a guest with edit access runs `exit`, or the process receives
   SIGINT/Ctrl-C). On clean session exit the process exits `0`.
6. **Sharing disabled.** If session sharing is not enabled for the user/team
   (`FeatureFlag::CreatingSharedSessions` off, surfaced as
   `ShareSessionError::Disabled`), the command prints the disabled message to
   stderr and exits non-zero **without** printing a join link.
7. **Share failure / timeout.** If sharing fails or times out
   (`ShareSessionError::Failed` / `Timeout` / `Interrupted` / `Internal`), the
   command prints the corresponding error to stderr and exits non-zero without a
   join link.
8. **Bootstrap failure.** If the terminal session fails to bootstrap
   (`BootstrapError::PtySpawnFailed` / `TimedOut` / `InternalError`), the command
   prints the error to stderr and exits non-zero.
9. **Not logged in.** If the user is not authenticated, the command fails with
   the standard "please log in" error used by other auth-required CLI commands
   (the command is registered as `requires_auth == true`).

---

## TECH

### Context: how the pieces work today
- **CLI parser** — `crates/warp_cli/src/lib.rs:540` defines the `CliCommand`
  enum. There is no `terminal` group today. New top-level subcommands are gated
  in two places: a pre-parse check in `Args::from_env`
  (`crates/warp_cli/src/lib.rs:195`) that emits `unrecognized subcommand` when
  the flag is off, and a `mut_subcommand(..., hide(true))` in `clap_command`
  (`crates/warp_cli/src/lib.rs:304`) that hides it from `--help`.
- **Share arg types** — `crates/warp_cli/src/share.rs`: `ShareArgs` (line 11,
  the `--share` flag), `ShareRequest` (line 30), `ShareSubject`, and
  `ShareAccessLevel`. These already parse the recipient syntax and are already
  consumed by the driver.
- **Command dispatch** — `app/src/ai/agent_sdk/mod.rs:142` `dispatch_command`
  matches each `CliCommand` arm and runs its handler with `ctx: &mut AppContext`.
  `command_requires_auth` (`app/src/ai/agent_sdk/mod.rs:1512`) and
  `command_to_telemetry_event` (`app/src/ai/agent_sdk/mod.rs:1720`) are
  exhaustive matches over `CliCommand` and must gain the new arm.
- **Headless bootstrap already exists** — `app/src/lib.rs:751` routes every
  `CliCommand` through `run_internal(LaunchMode::CommandLine { .. })`, which boots
  the full app runtime. `LaunchMode::is_headless` (`app/src/lib.rs:542`) returns
  `true` for all CLI commands except `agent run --gui`, so a new
  `terminal share` command is headless by default. Dispatch runs **inside** this
  booted `AppContext`.
- **`TerminalDriver`** — `app/src/ai/agent_sdk/driver/terminal.rs` (crate-private):
  - `TerminalDriver::create(options, ctx)` (line 200) builds the terminal view
    via `open_new_with_workspace_source` and wraps it in the driver model. This
    is the path already exercised headlessly by `AgentDriver::new`
    (`app/src/ai/agent_sdk/driver.rs:709`).
  - `TerminalDriverOptions` (line 108): `working_dir`, `env_vars`,
    `should_share`, `task_id`, `conversation_restoration`.
  - `TerminalDriverEvent::EstablishedSharedSession { session_id, join_url }`
    (line 117) — the driver emits this with `join_url` from
    `shared_session::join_link` (`app/src/terminal/shared_session/mod.rs:340`).
  - `add_share_requests` (line 326), `wait_for_session_bootstrapped` (line 596),
    `wait_for_session_shared` (line 622).
  - Session-exit signal: the backing terminal view emits
    `crate::terminal::view::Event::Exited` when the shell PTY exits (handled
    pre-bootstrap in `handle_terminal_view_event`, line 734).
- **Termination** — a CLI command completes by calling
  `ctx.terminate_app(TerminationMode::ForceTerminate, ...)`; see
  `create_and_run_driver` (`app/src/ai/agent_sdk/mod.rs:1501`) and
  `report_fatal_error` (line 1702). This is how the process returns an exit code.

### Design alternatives
- **Reuse `TerminalDriver` vs. reuse the full `AgentDriver`.** Chosen: drive
  `TerminalDriver` directly. `AgentDriver` layers AI turns, harness setup,
  snapshotting, and conversation handling on top of the terminal — none of which
  applies to "share a plain shell." Reusing `AgentDriver` would drag in an
  agent/turn lifecycle we don't want. Directly hosting `TerminalDriver` (as the
  ticket requests) is the minimal correct surface and mirrors how `AgentDriver`
  itself creates the terminal driver.
- **New headless runtime vs. reuse the existing CLI `AppContext`.** Chosen:
  reuse the existing headless CLI `AppContext`. The triage flagged "no headless
  entry point today," but dispatch already runs inside a headless, booted
  `AppContext` and `TerminalDriver::create` already works headlessly there. A new
  runtime would be redundant and risky. The only genuinely new code is the
  dispatch arm + a small handler that drives the session to completion.
- **Where the arg types live.** Chosen: parser types in `warp_cli`, driver
  hosting in `app`. Putting anything that references `TerminalDriver` in
  `warp_cli` would require exposing `app` internals across the crate boundary
  (the visibility risk triage called out). Keeping only clap types in `warp_cli`
  avoids that entirely; `ShareArgs` already lives there as precedent.
- **Feature gating.** Chosen: a dedicated `FeatureFlag::TerminalShareCommand`
  gating the CLI surface (pre-parse rejection + `--help` hiding), plus the
  existing runtime `CreatingSharedSessions` gate that already governs whether a
  share succeeds. Alternative — reuse `CreatingSharedSessions` alone for both the
  surface and the capability — is rejected because it couples "is the CLI
  command available" to "can this user share," producing a confusing
  `unrecognized subcommand` for users who simply lack the sharing entitlement.
- **Local-TTY attach vs. link-only (v1 scope).** Chosen: link-only. Forwarding
  the invoking terminal's stdin/stdout into the shared PTY (raw-mode TTY
  handling, SIGWINCH/resize propagation, restoring cooked mode on exit) is a
  substantial, platform-sensitive addition and is not required by the ticket
  ("headless"). Deferred as a follow-up. See Q1.

### Proposed changes
1. **New clap types in `warp_cli`** (`crates/warp_cli/src/terminal.rs`, new
   module; `pub mod terminal;` added to `crates/warp_cli/src/lib.rs`):
   - `TerminalCommand` (a `Subcommand` enum) with a `Share(TerminalShareArgs)`
     variant.
   - `TerminalShareArgs` flattening the existing `ShareArgs`
     (`#[clap(flatten)] pub share: ShareArgs`), plus an optional
     `--working-dir <PATH>` (defaulting to the current directory) so the shared
     shell's cwd is controllable. (Include `as_str_for_tracing` for the tracing
     path used by `CliCommand::as_str_for_tracing`.)
   - Add `Terminal(crate::terminal::TerminalCommand)` to the `CliCommand` enum
     (`crates/warp_cli/src/lib.rs:540`) with `#[command(subcommand)]`, and extend
     `CliCommand::as_str_for_tracing` (line 612).
2. **Feature-flag gating in `warp_cli`**:
   - Add `FeatureFlag::TerminalShareCommand` (per the `add-feature-flag` skill,
     in `crates/warp_core/src/features.rs`; default-on for dogfood via
     `DOGFOOD_FLAGS` while it bakes).
   - In `Args::from_env` (`crates/warp_cli/src/lib.rs:195`) add a pre-parse guard
     that rejects `terminal` when the flag is off; in `clap_command`
     (line 304) hide the `terminal` subcommand when the flag is off — mirroring
     the existing `secret` / `artifact` / etc. blocks.
3. **New execution handler in `app`**
   (`app/src/ai/agent_sdk/terminal.rs`, new module; `mod terminal;` added to
   `app/src/ai/agent_sdk/mod.rs`):
   - `pub(crate) fn run(ctx: &mut AppContext, global_options: GlobalOptions, command: TerminalCommand) -> anyhow::Result<()>`
     matching on `TerminalCommand::Share`.
   - For `Share`: construct `TerminalDriverOptions { working_dir, env_vars:
     <inherited/empty>, should_share: true, task_id: None,
     conversation_restoration: None }`, call `TerminalDriver::create(options,
     ctx)`, then (following the `create_and_run_driver` pattern):
     - `add_share_requests(parsed_share_requests, ctx)`,
     - subscribe to `TerminalDriverEvent::EstablishedSharedSession` → print
       `join_url` to stdout (`println!`),
     - `spawn` an async flow that awaits `wait_for_session_bootstrapped()` then
       `wait_for_session_shared()`, mapping their errors to a non-zero
       termination via `report_fatal_error`,
     - subscribe to the terminal view's `Event::Exited` (post-bootstrap) to
       detect session exit and call
       `ctx.terminate_app(TerminationMode::ForceTerminate, None)` with success.
   - Because `terminal.rs` is inside the `app` crate, it can access the
     `pub(crate)` `TerminalDriver` and `crate::terminal::view::Event` directly.
4. **Dispatch + exhaustive-match wiring in `app/src/ai/agent_sdk/mod.rs`**:
   - `dispatch_command` (line 142): add
     `CliCommand::Terminal(cmd) => { if !FeatureFlag::TerminalShareCommand.is_enabled() { return Err(anyhow::anyhow!("invalid value 'terminal'")); } terminal::run(ctx, global_options, cmd) }`.
   - `command_requires_auth` (line 1512): add `CliCommand::Terminal(_) => true`.
   - `command_to_telemetry_event` (line 1720): add a
     `CliCommand::Terminal(..)` arm with a new `CliTelemetryEvent::TerminalShare`
     variant (per the `add-telemetry` skill).
5. **Exhaustive matches elsewhere.** The repo's convention forbids wildcard
   `_` arms. Adding a `CliCommand::Terminal` variant will surface compile errors
   at every exhaustive `match` over `CliCommand` — resolve each explicitly. Known
   sites: `CliCommand::as_str_for_tracing`
   (`crates/warp_cli/src/lib.rs:612`), and the three `mod.rs` matches above.
   (The implementation must let the compiler enumerate any others.)

### Open questions resolved
- **Q1 — Does v1 forward the local terminal's I/O into the shared session, or is
  it link-only?** Resolved to **link-only** for v1 (the "headless" framing in the
  ticket implies no local attach; local-TTY forwarding is deferred, see *Design
  alternatives*). This is the single decision the approval should explicitly
  confirm, because it defines what "run until the session exits" means (the
  session ends when the shared shell PTY exits or the process is interrupted, not
  when a locally-attached user types `exit`).
- **Q2 — What is the default recipient when `--share` is omitted or bare?**
  Resolved (per reviewer clarification) to **owner-only / share-with-self**: when
  no recipients are specified the session is still shared, but only the invoking
  user has access and no team/public/user ACL is applied. Both an omitted
  `--share` and a bare `--share` (no value) collapse to an empty recipient list.
  Implementation reuses `ShareArgs`/`ShareRequest` unchanged so the CLI surface
  is identical to the existing `--share` flag; no new defaulting logic is
  introduced.
- **Q3 — GUI coupling / headless bootstrap risk (triage's main risk).**
  Resolved: no new runtime is needed. Dispatch already runs inside a headless
  booted `AppContext`, and `TerminalDriver::create` already runs headlessly for
  `warp agent run`. The work is a dispatch arm + handler, not a bootstrap
  rewrite.
- **Q4 — Crate-visibility of `pub(crate) TerminalDriver` across `warp_cli` ↔
  `app`.** Resolved: the execution handler lives in the `app` crate (same crate
  as `TerminalDriver`), so `pub(crate)` visibility is sufficient; `warp_cli` only
  contributes clap arg types and needs no access to `app` internals.
- **Q5 — Should the command be feature-flagged?** Resolved: yes — a dedicated
  `FeatureFlag::TerminalShareCommand` gates the surface, consistent with every
  other new subcommand; the runtime `CreatingSharedSessions` flag continues to
  gate whether sharing actually succeeds.

---

## Validation & verification criteria (must ALL pass before merge)

This is a **headless CLI feature** (no GUI surface): per `factory-verification`
it is verified with deterministic code-level checks — **no `computer_use` /
screenshot proof applies**. End-to-end session sharing requires a live
warp-server + session-sharing backend, which is not available in CI; CI
verification therefore anchors on arg-parsing tests + presubmit + compile, with
the end-to-end behavioral check performed manually against a dev server and
documented in the PR.

1. **Arg-parsing regression tests (new, required).** Add unit tests
   (following `crates/warp_cli/src/share_tests.rs` and `lib_tests.rs`) that
   parse:
   - `warp terminal share` → a `CliCommand::Terminal(TerminalCommand::Share(..))`
     with default share args,
   - `warp terminal share --share team:edit` → the correct `ShareRequest`
     (`ShareSubject::Team`, `ShareAccessLevel::Edit`),
   - `warp terminal share --share public --share user@example.com:view` → two
     `ShareRequest`s parsed correctly,
   - `warp terminal share --working-dir /tmp/x` → the working dir is captured,
   - an invalid recipient (`--share nope`) → a clap parse error.
   Each test must fail before the new types exist and pass after. Name them so
   the failure maps to this command (e.g. `terminal_share_*`).
2. **Feature-gate tests.** A test (or `lib_tests.rs` assertion) confirming that
   with `TerminalShareCommand` disabled, the `terminal` subcommand is rejected /
   hidden, and with it enabled, `terminal share --help` succeeds. If a
   feature-flag toggle is not unit-testable in this harness, cover the gating
   logic at the level used by existing gated subcommands and note the approach in
   the PR.
3. **Exhaustive-match completeness.** The build compiles with no wildcard `_`
   arm added to any `CliCommand` match — verified by `cargo build`/clippy
   (the exhaustive-matching rule in `AGENTS.md`). Every existing exhaustive match
   over `CliCommand` (tracing, auth, telemetry) explicitly handles `Terminal`.
4. **Stdout contract.** A test (or, where a live session is unavoidable, a
   documented manual check) asserting that on a successful share the **only**
   non-diagnostic line written to stdout is the join link, and that diagnostics
   go to stderr/log — matching Behavior #3.
5. **Error-path behavior.** Confirm (unit test where the error can be injected,
   otherwise documented manual check) that the sharing-disabled, share-failure/
   timeout, and bootstrap-failure paths (Behavior #6–#8) print to stderr and
   exit non-zero **without** printing a join link.
6. **Manual end-to-end behavioral check (documented in the PR).** Against a dev
   server with sharing enabled: `WITH_LOCAL_SERVER=1 ./script/run … terminal
   share --share team:view` (or the built CLI) starts a headless session, prints
   a working join link, the link opens a shared session, and the command blocks
   until the session exits then returns `0`. Record the observed before/after in
   the PR. If the environment cannot run this end-to-end, say so explicitly and
   mark it outstanding for a human check (per `factory-verification`).
7. **No collateral damage.** Existing `warp agent run --share …` behavior and the
   ambient-agent shared-session path are unchanged (they share the reused
   `ShareArgs`/`TerminalDriver` code) — confirmed by the existing `warp_cli` and
   `agent_sdk` test suites still passing.
8. **Presubmit.** `./script/presubmit` passes (fmt, clippy `-D warnings`, tests,
   build), unconditionally, per `factory-verification`.
