# Dev Container build progress and streaming logs

## Summary
The `/devcontainer` command can spend several minutes building an image, starting a container, and running lifecycle commands before Warp creates a terminal session. The current prototype shows only a toast during this work and buffers all command output. Replace that toast-only wait with a focused right split that shows the current phase and streams build logs through Warp's terminal grid. Permanently replace that split with the Dev Container session when attach setup succeeds.

This change deliberately changes the prototype behavior. The prototype pushes the finished Dev Container session over the originating shell in the same pane. The new behavior keeps the originating shell visible and usable. Both the build surface and the finished Dev Container session live in the new split.

All code references in this document refer to prototype commit `cc690d815ed8e3e270f4d495c6df0edc49f79baf` from PR #15516.

## Product behavior
1. Warp keeps the current synchronous precondition checks:
   - The command must run from a local terminal session with a canonical working directory.
   - `<workspace>/.devcontainer/devcontainer.json` must exist.
   - A precondition failure shows the current error toast and does not create a split.
2. After the precondition checks pass, `/devcontainer` immediately creates a right split next to the originating pane.
3. Warp focuses the new split.
4. The originating shell remains visible, running, and usable during the complete Dev Container operation.
5. The new split shows a Warp-owned header and an output-only terminal grid.
6. The header shows the workspace and exactly one current phase:
   - `Build`
   - `Preflight`
   - `Staging`
   - `Attach`
7. The `Build` phase starts before Warp resolves or launches the required CLI processes. A missing `devcontainer` CLI, a missing Docker CLI, or a launch error becomes a failure in the build surface.
8. While `devcontainer up` runs, Warp streams its text log output into the grid. Output appears before the process exits.
9. The grid supports normal terminal selection, copying, scrolling, wrapping, and ANSI interpretation.
10. The grid does not show a prompt, editable input, command text, block label, snackbar header, subshell flag, or ordinary block hover actions.
11. The view follows new output while the user is at the bottom. It does not force the user back to the bottom after the user scrolls upward.
12. Warp updates the phase from its own operation state. Warp does not infer phases by parsing CLI log text.
13. After `devcontainer up` reports success, Warp runs the existing attachability check in the `Preflight` phase.
14. Warp copies and secures the init and bootstrap scripts in the `Staging` phase.
15. Warp enters the `Attach` phase before it constructs the real Dev Container terminal pane and replaces the build pane. Warp does not add an artificial delay to make a short `Attach` phase visible.
16. When attach setup succeeds, Warp permanently replaces the build pane with the real Dev Container terminal pane in the same split.
17. Warp focuses the replacement Dev Container pane. The originating shell remains in its original pane.
18. The replaced build pane is destroyed. It does not remain in a pane back stack.
19. A failure in CLI resolution, build, result parsing, preflight, staging, or attach setup leaves the build split open and focused.
20. A failed surface keeps the logs that were already rendered.
21. The failed header identifies the phase that failed and shows the best available structured error:
   - Prefer `message` from the final `devcontainer up` stdout JSON object.
   - Otherwise use `description` from that object.
   - Otherwise use the process, I/O, or attach error plus a bounded tail of available stderr.
22. A failed surface shows `Retry` and `Close`.
23. `Retry` reuses the same split. It clears the prior attempt's grid and error, increments the attempt identity, returns to `Build`, and starts a new operation.
24. `Close` on a failed surface closes the split.
25. Closing the build split while any phase is running immediately cancels the current operation and closes the split.
26. Closing the tab or window that contains the build split performs the same cancellation.
27. Cancellation terminates the active operation-owned process group. A late process or future completion cannot create or replace a pane.
28. Warp does not remove a container or image on cancellation. The CLI can reuse existing resources, and Warp cannot safely infer which resources it exclusively owns.
29. Warp allows operations for different canonical workspace/configuration pairs to run concurrently.
30. Warp allows only one active surface for the same canonical workspace/configuration pair.
31. A repeated `/devcontainer` invocation for a key with a running operation focuses the existing build surface. It does not launch another CLI process or create another split.
32. A repeated invocation for a key with a retained failed surface focuses that surface so the user can select `Retry` or `Close`.
33. If the registry contains a stale surface locator, Warp removes that entry and starts a new operation.
34. A restart never resumes a Dev Container build. Warp does not persist the transient build pane. The originating shell persists and restores through the existing session-restoration path.

## Technical design

### Current flow
The prototype implements the flow in `app/src/terminal/view/dev_container/mod.rs`.

- `TerminalView::find_and_start_dev_container` validates the current directory and configuration, resolves the CLI paths, and shows the progress toast.
- `TerminalView::bring_up_dev_container` calls `Command::output().await`. This buffers stdout and stderr until `devcontainer up` exits.
- `interpret_dev_container_up_output` parses the final stdout JSON object.
- `TerminalView::preflight_and_attach_dev_container` verifies attach requirements and stages scripts.
- `TerminalView::create_and_push_dev_container` creates the real terminal and pushes it onto the originating `PaneStack`.

The implementation must retain the result parsing, preflight, staging, session ID, sandbox ID, and terminal creation behavior unless this specification changes it.

### Split creation and permanent replacement
Use a whole-pane loading and replacement flow. Do not add `PaneStack::replace_top`.

Warp already uses this design for conversation restoration:

- `PaneGroup::add_loading_conversation_pane` creates and focuses a right-hand loading `TerminalPane` in `app/src/pane_group/mod.rs:6499`.
- `PaneGroup::replace_loading_pane_with_terminal` creates the real pane and requests a permanent replacement in `app/src/pane_group/mod.rs:6536`.
- `PaneGroup::replace_pane` attaches the replacement, swaps the pane-tree leaf, cleans up the old pane, and focuses the replacement in `app/src/pane_group/mod.rs:5021`.
- `Workspace::restore_conversation_in_split_pane` owns the complete create-load-replace-fail lifecycle in `app/src/workspace/view.rs:13444`.

Add a Dev Container-specific loading-pane constructor and replacement entry point that reuse these pane operations. The build pane must be a `TerminalPane` backed by a terminal model that is configured for Dev Container build output.

The originating `TerminalView` must send a request to its `PaneGroup` after synchronous precondition validation. The request must include:

- The canonical workspace folder.
- The canonical configuration path.
- The resolved originating pane ID.
- The pane configuration needed by the finished terminal.

The `PaneGroup` must:

1. Claim the operation key in the global registry.
2. Create a focused right split relative to the originating pane.
3. Bind the build operation to the new pane.
4. Start asynchronous CLI resolution and execution.
5. Permanently replace the build pane with a newly created `TerminalPane` after staging succeeds.

If split creation fails, Warp must release the registry claim, show an error toast, and not start `devcontainer up`.

### Build surface
Add a `DevContainerBuildOperation` model. The build `TerminalView` owns a strong handle to this model and observes it for header, status, and action updates. The registry stores only a weak operation handle and weak pane locator. Do not represent the build as a synthetic user command block.

The operation model must contain:

- Operation key.
- Operation ID.
- Attempt ID.
- Canonical workspace and configuration paths.
- Current phase.
- Running, failed, cancelling, cancelled, or completed status.
- Structured failure details.
- A cancellation handle.

Configure the terminal model with one commandless output grid. Add a narrowly named production API for starting this output mode instead of depending on the test-oriented name `start_active_block_as_background_block`.

The existing background-block behavior is the rendering precedent:

- `Block::start_background` creates an output block with no command in `app/src/terminal/model/block.rs:1234`.
- Background blocks omit prompt/command and middle padding in `app/src/terminal/model/block.rs:2055` and `app/src/terminal/model/block.rs:2090`.
- Background blocks suppress prompt labels, subshell flags, and snackbar headers in `app/src/terminal/block_list_element.rs:345` and `app/src/terminal/block_list_element.rs:3345`.

The generic block hover toolbelt can still appear for a background block. The Dev Container build mode must explicitly suppress those actions.

Use the terminal's configured scrollback bound for the output grid. Do not retain an additional unbounded stderr buffer.

### Operation ownership and registry
Add an application-scoped `DevContainerBuildRegistry` singleton.

Define the key as:

```rust
struct DevContainerBuildKey {
    workspace_folder: PathBuf,
    config_file: PathBuf,
}
```

Both paths must be canonical before lookup. Registry claim and insertion must run as one main-thread model update before any process starts.

Each registry entry must contain:

- A unique operation ID.
- A weak locator for the window, pane group, and build pane.
- The current attempt ID.
- Running or failed surface status.

The build pane owns the strong operation handle. The registry must not keep a closed window or pane alive.

Every asynchronous completion must verify all of these values before changing UI or creating a terminal:

- The registry still maps the key to the same operation ID.
- The attempt ID still matches.
- The operation is not cancelled or completed.
- The target build pane still exists.

Close must mark the operation cancelled before it terminates processes or removes the pane. Retry must increment the attempt ID before it starts new work. These rules make late completions no-ops.

Remove the registry entry after permanent pane replacement or Close. Keep the entry in failed status while the failure surface remains open.

The registry is a correctness requirement. A real test with two `devcontainer up` processes started 500 ms apart for one workspace created two distinct running containers with identical Dev Container labels. The CLI does not serialize this operation.

### Process lifecycle
Replace `Command::output()` with explicit piped stdout and stderr.

Launch `devcontainer up` with:

```text
devcontainer up --workspace-folder <workspace> --log-format text
```

Use the process-group and cancellation pattern from `app/src/terminal/model/session/command_executor/local_command_executor.rs`:

- Start an operation-owned process group.
- Enable kill-on-drop.
- Track the active child/process-group guard.
- On cancellation, terminate the complete active process group.
- Await or reap the child after termination without applying a late result.

Drain stdout and stderr concurrently. A reader failure must fail the attempt and cancel the remaining process work. Do not wait for one full pipe before reading the other.

Apply the same cancellation token and attempt checks to preflight, staging, and attach setup. Dropping or cancelling a phase future must terminate its active child process. A completion from an earlier phase or retry must not advance the state machine.

### Stdout result path
Keep stdout separate from the display stream.

Retain at most the final 1 MiB of stdout. If bytes were discarded because the limit was exceeded, fail the result as oversized instead of parsing a potentially partial JSON object. At process exit:

1. Select the last non-empty stdout line.
2. Parse it as `DevContainerUpResult`.
3. Combine the parsed result with the process exit status through `interpret_dev_container_up_output`.
4. Enter `Preflight` only when the outcome is `ReadyToAttach`.

A missing, oversized, invalid, unsuccessful, or incomplete result must enter the failed state. Do not feed stdout JSON into the terminal grid.

The existing parsing functions in `app/src/terminal/view/dev_container/mod.rs:554-647` remain the semantic source of truth.

### Stderr display path
Feed stderr bytes into one persistent `warp_terminal::model::ansi::Processor`.

`Processor::parse_bytes` is public at `crates/warp_terminal/src/model/ansi/mod.rs:397`. The local PTY event loop demonstrates persistent processor ownership at `crates/warp_terminal/src/local_tty/event_loop.rs:208-275`.

Use `std::io::sink()` as the response writer. The producer is non-interactive. Parser replies are write-only side effects, and `Processor` does not wait for a reply. The measured Dev Container streams emitted no terminal-response requests.

Keep the same `Processor` instance for the complete attempt. Creating one processor per read chunk would break parser state when an escape sequence spans chunks.

After each parsed batch:

- Update the live background-block height.
- Send a wakeup/notify event.
- Preserve the current user scroll anchor.

### Newline normalization
Normalize the piped stderr stream before `Processor::parse_bytes`.

Piped Dev Container text output uses bare line feed bytes because it does not pass through a PTY. A terminal `LF` moves down one row but preserves the current column. Feeding the measured stream without normalization produced staircase indentation.

The normalizer must:

- Convert an `LF` that is not immediately preceded by `CR` into `CRLF`.
- Preserve an existing `CRLF` as `CRLF`.
- Preserve a standalone `CR`.
- Carry the previous-byte state across read chunks.
- Flush a final standalone `CR` unchanged at end of stream.

A per-chunk `replace("\n", "\r\n")` is invalid. It turns a `CRLF` split across two chunks into `CRCRLF`.

Required split-boundary examples include:

```text
chunks: ["first\r", "\nsecond\n", "third\r", "fourth\n"]
output: "first\r\nsecond\r\nthird\rfourth\r\n"
```

The normalizer must also cover a lone `LF` at the final position of a chunk followed by a non-newline byte in the next chunk.

### Phase transitions
Use an explicit state machine:

```text
Build -> Preflight -> Staging -> Attach -> Completed
   \          \           \          \
    +----------+-----------+-----------> Failed

Running phase -> Cancelling -> Cancelled
Failed -> Retry -> Build with a new attempt ID
```

Phase boundaries are:

- `Build`: CLI resolution, Docker CLI resolution, `devcontainer up`, pipe draining, and result parsing.
- `Preflight`: the existing `docker exec` attachability check.
- `Staging`: the existing `prepare_dev_container` work.
- `Attach`: real terminal construction and permanent pane replacement.

Emit the phase update and notify the build view before starting the next phase. Do not parse log content to determine a phase.

### Failure and retry
Keep the grid model and rendered logs alive when an attempt fails.

Maintain a bounded stderr tail only for constructing a fallback error. The displayed logs remain in the terminal grid.

Retry must:

1. Confirm that the pane and registry entry still refer to the same operation.
2. Increment the attempt ID.
3. Cancel and reap any remaining child from the prior attempt.
4. Clear the output grid and structured error.
5. Reset generated sandbox/session identifiers.
6. Return the phase to `Build`.
7. Start the complete flow again in the same pane.

The final Dev Container terminal must receive one sandbox ID and session ID generated for the successful attempt. IDs from a failed or cancelled attempt must not be reused.

### Persistence
The build pane is transient.

Add `LeafContents::DevContainerBuild`. `TerminalPane::snapshot` must return this variant when its active view is a Dev Container build surface. `LeafContents::is_persisted` must return `false` for this variant, following the Network Log and Environment Management precedents at `app/src/app_state.rs:169`.

The SQLite traversal already skips non-persisted leaves before it writes `pane_nodes` at `app/src/persistence/sqlite.rs:1114`. Use that path so the build leaf does not become an orphan persisted node. Add exhaustive non-persisted match arms beside the existing Network Log and Environment Management arms. Restoration must reject `DevContainerBuild` if a stale snapshot somehow contains it.

When a branch contains the transient build leaf and a normal originating shell, restart restores the normal shell and collapses the invalid/omitted branch as needed. It must not restore an empty replacement terminal for the build pane.

### Research evidence and limits
The feasibility experiment used `@devcontainers/cli` 0.88.0 and disposable Docker fixtures.

- Successful JSON-mode execution emitted 176 read chunks, 44,546 stderr bytes, and 213 valid JSON events.
- Successful text-mode stderr contained 5,838 bytes, 80 `LF`, 6 `CR`, and 74 lone `LF`.
- Failed text-mode stderr contained 3,092 bytes, 30 `LF`, 6 `CR`, and 24 lone `LF`.
- Text-mode stdout was final-only JSON on both success and controlled failure.
- The normalized text stream retained delayed build steps, package installation, container startup, post-create output, failure details, and the complete JavaScript stack trace.
- Feeding real captures through a persistent `Processor` in 137-byte chunks rendered cleanly after newline normalization.
- The real captures did not request terminal responses.
- Deliberate device-status queries wrote replies to the supplied writer, but parsing and later rendering continued when those replies were discarded.
- The tested legacy Docker builder emitted no ANSI color or spinner redraw sequences.

The experiment proves incremental text delivery, result separation, plain-text grid rendering, failure rendering, newline normalization, and safe discarded replies for the measured streams. It does not prove the visual quality of color, spinner, or cursor-redraw output from other Docker backends. Such bytes use Warp's ordinary ANSI processor, but no product claim depends on them.

## Decisions

### New right split instead of the originating pane stack
Selected: create a focused right split, then permanently replace that pane on success.

Advantages:

- Reuses the existing loading-pane and permanent pane-replacement path.
- Keeps the user's shell visible and usable during a multi-minute operation.
- Gives Close its ordinary meaning: close and cancel only the build split.
- Avoids adding a replace-top primitive to shared `PaneStack` code.
- Avoids persistence logic that must reach through a transient top stack entry.

Disadvantages:

- Changes where the finished session appears compared with the prototype.
- Uses additional screen space.
- Initially moves focus away from the originating shell.

Rejected: push the build surface onto the originating pane stack.

- It would require a new atomic replace-top operation.
- A push over the build entry would leave stale loading UI under the real session.
- A separate pop and push can expose intermediate state and complicate late-completion checks.
- Close would need special cancel-and-pop behavior instead of ordinary pane close behavior.

### Direct terminal-grid output instead of synthetic command blocks
Selected: render a commandless output grid with a persistent ANSI processor.

Advantages:

- Reuses Warp's terminal parser, grid, selection, wrapping, scrolling, and bounded retention.
- Streams arbitrary text without defining a second log renderer.
- Avoids shell-hook, command lifecycle, session ID, persistence, and telemetry coupling.

Disadvantages:

- Requires explicit input and hover-action suppression.
- Requires piped-newline normalization.
- Terminal response bytes must be discarded.

Rejected: inject a synthetic command block into the originating shell.

- The output does not belong to that shell or PTY.
- Concurrent shell output and prompts can interleave with external process output.
- Live command blocks depend on shell hooks and command completion state.

### Text log format instead of JSON event rendering
Selected: `--log-format text` on stderr plus final stdout JSON.

Advantages:

- The terminal grid can consume the display stream directly.
- Failure stack traces require no JSON fallback parser.
- Stdout retains the existing final result contract.

Disadvantages:

- Warp cannot apply structured styling by event type.
- Pipe output requires newline normalization.

Rejected: parse JSON lines and render decoded message fields.

- JSON mode adds incremental framing, buffering, schema handling, and fallback behavior.
- Controlled failure appended unstructured stack-trace text after valid JSON events.
- The product does not require event-type styling.

### Placeholder and streaming in one delivery
Selected: ship the phase surface and streaming log pipeline together.

Advantages:

- Streaming is low incremental effort after the pane and operation lifecycle exist.
- The requester considers a placeholder alone insufficient.
- One delivery avoids reopening the same lifecycle code.

Constraint:

- Keep the surface/operation layer separate from the stderr adapter/parser layer. The surface must remain functional with no log bytes.

Rejected: ship placeholder-only first.

- Pane lifecycle, cancellation, persistence, deduplication, and replacement are the dominant work.
- Deferring streaming would save little implementation risk.

### Global active-surface deduplication
Selected: one application-scoped registry keyed by canonical workspace and configuration.

Advantages:

- Prevents the measured duplicate-container correctness bug.
- Works across panes, tabs, and windows.
- Gives repeated invocation a deterministic focus target.

Rejected: per-pane deduplication.

- Two panes can still launch the same workspace concurrently.

Rejected: rely on the Dev Container CLI.

- A real concurrent test created two containers.

## Assumptions
- Retry clears the previous attempt's grid after the user selects `Retry`. The failed logs remain available until that action.
- A retained failed surface continues to reserve its workspace/configuration key.
- Completion and failure focus the build/replacement split, matching the selected replacement precedent.
- Cancellation does not remove Docker containers, images, volumes, or caches.
- The existing platform scope remains unchanged: Dev Container sessions require local TTY support and the prototype's supported Unix attach path.
- The new split uses Warp's ordinary right-split sizing and resize behavior.
- Only `devcontainer up` stderr is a live log stream. Later phases update the header and include failure details; they do not create synthetic command transcripts.

## Out of scope
- Changing Dev Container configuration discovery beyond `.devcontainer/devcontainer.json`.
- Supporting remote-TTY Dev Container builds.
- Adding a new Docker build progress UI, progress bars, or structured step tree.
- Guaranteeing spinner or cursor-redraw quality for backends not exercised by the research.
- Persisting or resuming build logs.
- Automatically deleting Docker resources after cancellation or failure.
- Allowing user input in the build surface.
- Changing the Dev Container terminal bootstrap protocol, staged scripts, or attach shell.
- Generalizing the build registry to unrelated container or agent operations.
- Adding `PaneStack::replace_top`.

## Validation criteria
All criteria must pass before merge.

1. Add `devcontainer_build_opens_focused_right_split`.
   - Start from one terminal pane.
   - Invoke a valid mocked `/devcontainer` request.
   - Assert that the pane count becomes two.
   - Assert that the new pane is to the right and focused.
   - Assert that the originating terminal view and process are unchanged.
   - Run with `cargo nextest run -p warp devcontainer_build_opens_focused_right_split`.

2. Add `devcontainer_build_replaces_loading_pane_permanently`.
   - Drive mocked Build, Preflight, Staging, and Attach success.
   - Assert the phase order.
   - Assert that `PaneGroup::replace_pane` leaves two panes, with the real Dev Container pane in the loading pane's tree slot.
   - Assert that the old build pane is detached and destroyed.
   - Assert that no build view remains in a `PaneStack` or Back destination.
   - Run with `cargo nextest run -p warp devcontainer_build_replaces_loading_pane_permanently`.

3. Add `devcontainer_text_stream_normalizes_newlines_across_chunks`.
   - Test bare `LF`, existing `CRLF`, standalone `CR`, empty chunks, a final `CR`, a `CRLF` split across chunks, and a lone `LF` at the end of a chunk.
   - Assert the exact byte output for the example in Newline normalization.
   - Run with `cargo nextest run -p warp devcontainer_text_stream_normalizes_newlines_across_chunks`.

4. Add `devcontainer_text_stream_renders_incrementally`.
   - Feed a fixture in small chunks through one persistent `Processor`.
   - Assert that a delayed marker is visible before simulated process exit.
   - Assert clean left alignment after bare `LF`.
   - Assert that a split ANSI sequence retains parser state.
   - Assert that emitted terminal reply bytes are discarded without blocking later output.
   - Run with `cargo nextest run -p warp devcontainer_text_stream_renders_incrementally`.

5. Add `devcontainer_up_drains_stdout_and_stderr_concurrently`.
   - Use a fake child that writes enough data to both pipes to block a sequential reader.
   - Assert progress reaches the grid before exit.
   - Assert the final stdout JSON is parsed after both pipes close.
   - Run with `cargo nextest run -p warp devcontainer_up_drains_stdout_and_stderr_concurrently`.

6. Extend `app/src/terminal/view/dev_container/mod_tests.rs`.
   - Cover successful text-mode final JSON.
   - Cover non-zero exit with structured `message`.
   - Cover `description` fallback.
   - Cover malformed, missing, oversized, and incomplete stdout.
   - Cover a bounded stderr-tail fallback.
   - Run with `cargo nextest run -p warp dev_container`.

7. Add `devcontainer_failure_retains_logs_and_retries_in_place`.
   - Fail each phase in parameterized cases.
   - Assert the failed phase, error precedence, retained grid, and `Retry`/`Close` actions.
   - Select `Retry`.
   - Assert the same pane is reused, the grid is cleared, IDs change, and a stale prior-attempt completion is ignored.
   - Run with `cargo nextest run -p warp devcontainer_failure_retains_logs_and_retries_in_place`.

8. Add `closing_devcontainer_build_cancels_process_group`.
   - Run a fake child that creates a descendant and does not exit by itself.
   - Close the build pane.
   - Assert the operation is tombstoned before process termination.
   - Assert the child and descendant exit.
   - Deliver a late success completion and assert that no terminal pane is created.
   - Assert focus returns through the ordinary pane-close path.
   - Run with `cargo nextest run -p warp closing_devcontainer_build_cancels_process_group`.

9. Add `duplicate_devcontainer_invocation_focuses_existing_surface`.
   - Invoke the command twice for the same canonical workspace/configuration.
   - Include path aliases that canonicalize to the same paths.
   - Assert one split, one process launch, one registry entry, and focus on the existing surface.
   - Repeat while the surface is failed and assert that Warp focuses it.
   - Invoke two different keys and assert that both start.
   - Run with `cargo nextest run -p warp duplicate_devcontainer_invocation_focuses_existing_surface`.

10. Add `devcontainer_build_surface_is_not_persisted`.
    - Snapshot a tab containing the originating shell and an active build split.
    - Persist and restore the snapshot.
    - Assert that the shell restores and the build pane does not.
    - Assert that no orphan pane node causes the tab to disappear.
    - Run with `cargo nextest run -p warp devcontainer_build_surface_is_not_persisted`.

11. Keep invalid invocation behavior covered.
    - A missing local working directory or configuration shows an error toast.
    - It creates no split and no registry entry.
    - A missing CLI after valid preconditions leaves a failed split with `Retry` and `Close`.
    - Run with `cargo nextest run -p warp dev_container`.

12. Run the existing pane split, permanent replacement, close/focus, terminal snapshot, and conversation-loading replacement tests.
    - Run `cargo nextest run -p warp pane_group`.
    - Run `cargo nextest run -p warp restore_conversation_in_split_pane`.

13. Run repository checks from the repository root:
    - `./script/format --check`
    - `./script/check_no_inline_test_modules`
    - `cargo clippy -p warp --all-targets --tests -- -D warnings`
    - `cargo nextest run -p warp dev_container pane_group`
    - `./script/presubmit`

14. Perform a real successful build with `@devcontainers/cli` 0.88.0 or newer and a fixture that prints three build markers one second apart and three `postCreateCommand` markers one second apart.
    - Confirm each marker appears before the process exits.
    - Confirm lines remain left-aligned.
    - Confirm the phase header advances in order.
    - Confirm the right split becomes an interactive Dev Container shell.
    - Confirm the originating shell remains usable.

15. Perform a real controlled failure whose Docker build prints three delayed markers and exits non-zero.
    - Confirm all markers, Docker failure text, CLI error, and stack trace remain visible.
    - Confirm the failed header identifies `Build`.
    - Confirm `Retry` reuses the split.
    - Confirm `Close` removes the split.

16. Build the application successfully before UI verification. Then use computer use to record:
    - A video of invoking `/devcontainer`, the focused right split opening, multiple log updates arriving over time, phase transitions, permanent replacement, and interaction with the finished Dev Container shell.
    - A video of a controlled failure, retained logs, Retry in the same split, and Close.
    - The videos must also show the unchanged originating shell.
    - Attach both videos to the implementation pull request.
