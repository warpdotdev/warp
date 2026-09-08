*Spec: TUI — insert typeahead into the input editor when an LRC finishes (CODE-1895)*

Ticket: https://linear.app/warpdotdev/issue/CODE-1895/tui-characters-typed-during-an-lrc-are-discarded-instead-of-being
Originating thread: https://warpdotdev.slack.com/archives/C0BDQDW8V5E/p1784552071087109
Target repo: warpdotdev/warp (the Warp client)
All code references below are commit-pinned to `abea51cd1e102b363935f1b25ef03d335bc7b36f`.

== PRODUCT ==

*Summary:* In the Warp TUI (`crates/warp_tui`), characters typed while a
user-controlled long-running command (LRC) is running (e.g. `sleep 5`) are
currently discarded when the command finishes. They should instead be inserted
into the TUI input editor when the LRC completes — matching the Warp GUI and
general shell typeahead behavior (bash/zsh buffer keystrokes typed during a
command and flush them into the next prompt). The typeahead accumulation already
exists in the *shared* terminal model; the TUI is simply missing the view-side
wiring that observes it and writes it into the editor.

*Key design choices:*
1. *Reuse the shared model machinery unchanged.* Typeahead detection
   (`EarlyOutput`, `TypeaheadMode`) and the `TerminalEvent::Typeahead`
   notification already live in the shared `TerminalModel` and already reach the
   TUI as `ModelEvent::Typeahead`. The fix is view-side wiring only — no new
   detection logic and no new event plumbing.
2. *Share the typeahead policy, keep editor application backend-local.* The
   "should we insert, and what text / how much to overwrite" decision
   (AI-requested-command exclusion + `advance_typeahead`) lives in one shared
   `TerminalModel` method used by both front-ends. Applying that result stays
   local to each editor backend: the GUI terminal input uses a zero-based CRDT
   buffer, while the TUI uses the one-based core editor. The core editor gains a
   general `replace_first_n_characters` primitive; each input view explicitly
   performs the trivial replace-then-move-to-end sequence.
3. *Scope the reported bug to the common (ShellReported) path; treat
   InputMatching as best-effort.* For modern shells (zsh, bash ≥ 4) the shell
   reports its input buffer, so the model already accumulates typeahead and only
   the observe-and-insert wiring is needed. `InputMatching` (bash 3.2 only)
   additionally needs keystroke reporting on the TUI's PTY-forward path; this is
   wired "where feasible" and is not required to fix the reported repro.

*Behavior* (numbered, testable invariants; "input editor" = the TUI prompt's
`CodeEditorModel`):
1. *Happy path (ShellReported shell).* At a TUI shell prompt, run a silent LRC
   (`sleep 5`). While it runs, type `echo hi`. When the LRC finishes, `echo hi`
   appears in the input editor with the cursor at the end. (Today: the input is
   empty.)
2. *InputMatching shell (where feasible).* The same end result holds on a
   `InputMatching` shell (bash 3.2), achieved by reporting the typed keystrokes
   to the model so it can match them against the PTY echo. If not implemented,
   this path may still discard typeahead, but no other behavior regresses.
3. *AI/agent-requested-command exclusion.* Typeahead typed during an
   agent-requested / agent-driven LRC is NOT auto-inserted into the input editor,
   matching the GUI's deliberate exclusion (the agent follows up immediately and
   the input stays interactive during such commands).
4. *PTY passthrough is preserved.* Characters typed during the LRC still reach
   the foreground process exactly as today (the LRC can still read stdin). The
   typeahead insertion reflects only what the shell echoes as the basis of the
   next command; it does not change what the running process receives.
5. *Incremental events overwrite, not append.* When the model emits multiple
   `Typeahead` events for one prompt (partial buffers arriving over time), the
   final editor buffer equals the shell's reported input buffer, not a
   concatenation of the partials (e.g. events `ec` then `echo hi` yield
   `echo hi`, never `ececho hi`). This uses the model's existing
   `typeahead_chars_inserted` overwrite count.
6. *No typeahead → no change.* If the user types nothing during the LRC, the
   input editor is unchanged (empty) when it completes; an empty typeahead event
   is a no-op.
7. *Alt-screen apps are unaffected.* While a full-screen (alt-screen) application
   owns the terminal, no typeahead is accumulated or inserted (the shared model
   already no-ops `push_user_input` when the alt screen is active). Full-screen
   app behavior does not regress.
8. *Stale typeahead does not leak.* After the next command begins, typeahead from
   the previous command is cleared and never appears in a later prompt (the
   shared model already clears on `precmd`).

== TECH ==

*Context — how typeahead works today.*
The typeahead model is entirely shared between the GUI and the TUI (both use the
same `TerminalModel`):
- `EarlyOutput` models output received while no block is running and accumulates
  typeahead, in one of two modes (`TypeaheadMode`) chosen in `init_session`:
  `ShellReported` (the shell reports its input buffer via a DCS/OSC hook —
  `ansi::Handler::input_buffer`) or `InputMatching` (Warp matches PTY-echoed
  chars against user input recorded via `push_user_input`).
  `crates/../app/src/terminal/model/early_output.rs:50 @ abea51c` (struct),
  `:26-35 @ abea51c` (`TypeaheadMode`), `:108-168 @ abea51c`
  (`push_user_input` / `handle_potential_typeahead`), `:173-187 @ abea51c`
  (`advance_typeahead`), `:326-364 @ abea51c` (`input_buffer`). It emits
  `TerminalEvent::Typeahead` at `:164-165 @ abea51c` and `:358 @ abea51c`.
- `TerminalModel::push_user_input` forwards to the early-output model and already
  no-ops while the alt screen is active:
  `app/src/terminal/model/terminal_model.rs:2285-2290 @ abea51c`.
- The event reaches the view as `ModelEvent::Typeahead`
  (`app/src/terminal/model_events.rs:228 @ abea51c`).

*GUI wiring (the reference behavior).*
- While an LRC is running, `keydown_on_terminal` / `typed_characters_on_terminal`
  call `report_possible_typeahead` (→ `push_user_input`) *and* write the bytes to
  the PTY: `app/src/terminal/view.rs:9125-9141 @ abea51c`,
  `:9169-9198 @ abea51c`, `report_possible_typeahead` at `:9331-9336 @ abea51c`.
- `handle_typeahead_event` consumes `ModelEvent::Typeahead`, looks up the just-
  completed block, skips insertion when that block was agent-requested
  (`agent_interaction_metadata().is_some()`), calls `advance_typeahead`, and
  inserts into the input editor via `input.insert_typeahead_text(...)`:
  `app/src/terminal/view.rs:9477-9519 @ abea51c`; dispatched from
  `:12675-12677 @ abea51c`.
- `insert_typeahead_text` replaces the first N characters (the previous
  typeahead) with the new full typeahead, then moves the cursor to the end:
  `app/src/terminal/input.rs:8894-8904 @ abea51c`, delegating to the GUI editor
  view's `replace_first_n_characters` at
  `app/src/editor/view/mod.rs:4579-4595 @ abea51c`.

*TUI gap (what is missing).*
- The TUI reuses the same `TerminalModel`, so typeahead accumulation already
  happens — but the two view-side hooks are missing:
- (a) *Never inserts.* The TUI already receives `ModelEvent::Typeahead` but its
  handler only redraws — it never reads the accumulated typeahead into the input
  editor: `crates/warp_tui/src/terminal_session_view.rs:1077-1100 @ abea51c`
  (the `ModelEvent::Typeahead` arm at `:1096` is grouped into a `ctx.notify()`).
  The input editor is a shared `CodeEditorModel` created at
  `crates/warp_tui/src/terminal_session_view.rs:812-813 @ abea51c` and owned by
  `TuiInputView` (`crates/warp_tui/src/input/view.rs:150-154 @ abea51c`).
- (b) *Never reports keystrokes.* While an LRC owns input, keys route to
  `TuiInputTarget::Pty` (`crates/warp_tui/src/terminal_use.rs:65-88 @ abea51c`,
  `123-133 @ abea51c`) and are forwarded as
  `TuiTerminalSessionAction::ForwardUserPtyBytes`
  (`crates/warp_tui/src/terminal_content_element.rs:129-181 @ abea51c`), whose
  handler is a raw passthrough that writes to the PTY and never calls
  `push_user_input`:
  `crates/warp_tui/src/terminal_session_view.rs:2979-2985 @ abea51c`. So
  `InputMatching` shells cannot match typeahead in the TUI.

*Design.*
- *Shared decision logic.* `TerminalModel::take_typeahead_for_input(&mut self)
  -> Option<(String, CharOffset)>` encapsulates the just-completed-block lookup,
  agent-requested exclusion, and `advance_typeahead`; both front-ends call it.
  The AI-exclusion rule and accumulated overwrite count therefore have one
  implementation on the shared `BlockList`.
- *Editor application.* The GUI terminal input uses the legacy CRDT
  `EditorModel`, while the TUI uses `CodeEditorModel` backed by the core editor;
  neither buffer can substitute for the other, and their character offsets use
  different bases. `CoreEditorModel::replace_first_n_characters` provides a
  general one-based core-editor primitive built on
  `BufferEditAction::InsertAtCharOffsetRanges`. The TUI uses it before moving
  its cursor to the end. The GUI retains its existing zero-based editor
  primitive and cursor movement.
- *Insertion target / timing in the TUI.* While the LRC runs, the input target is
  `TuiInputTarget::Pty` and the agent editor is hidden; on `BlockCompleted` the
  TUI runs `resume_after_user_controlled_command` + `update_process_input_focus`
  and returns input to `AgentEditor`
  (`crates/warp_tui/src/terminal_session_view.rs:1078-1081 @ abea51c`). Inserting
  into the `CodeEditorModel` is safe even while it is hidden — it becomes visible
  when input returns to the editor on completion. — *Chosen:* insert into the
  existing `input_editor_model`, mirroring the GUI (which likewise inserts into
  the input editor regardless of focus). Do not force a mode switch; let the
  input's normal auto-detection run as it does for any other programmatic insert.

*Proposed changes.*
1. *Shared decision method.* In `app/src/terminal/model/terminal_model.rs`, add
   `take_typeahead_for_input(&mut self) -> Option<(String, CharOffset)>` that
   reproduces the GUI's `handle_typeahead_event` decision on the shared block
   list: find the previous completed (non-background, include-hidden) block from
   the active index; if it has `agent_interaction_metadata()`, return `None`
   (AI exclusion); otherwise call `early_output_mut().advance_typeahead()` and
   return the owned typeahead string plus the previous-inserted `CharOffset`.
   Returns `None` when typeahead is empty.
2. *GUI migration to the shared decision method (required).* Change
   `TerminalView::handle_typeahead_event`
   (`app/src/terminal/view.rs:9477-9519`) to call the new shared
   `TerminalModel::take_typeahead_for_input` instead of open-coding the block
   lookup + AI-exclusion + `advance_typeahead`. This migration is a required part
   of this change (not an optional "prove the extraction" step): after it, the
   AI-exclusion rule exists in exactly one place and the GUI runs on the same
   decision path the TUI uses. The externally observable GUI behavior is
   unchanged, but the GUI must be migrated — a shared method the GUI does not use
   is explicitly not acceptable.
3. *Core-editor prefix replacement.* In `crates/editor/src/model.rs`, add
   `CoreEditorModel::replace_first_n_characters`, implemented with
   `InsertAtCharOffsetRanges` and the core editor's one-based offsets. The TUI
   input view uses it with the model's previous-inserted count, then moves the
   cursor to the end. The GUI keeps its existing
   `Input::insert_typeahead_text` behavior
   (`app/src/terminal/input.rs:8894-8904 @ abea51c`) on its zero-based CRDT
   editor.
4. *TUI observe-and-insert.* In
   `crates/warp_tui/src/terminal_session_view.rs`, split `ModelEvent::Typeahead`
   out of the notify-only arm (`:1091-1098`) into a new
   `handle_typeahead_event(ctx)` that: calls
   `self.terminal_model.lock().take_typeahead_for_input()`; on `Some((text, n))`
   updates `input_editor_model` to replace the first `n` chars with `text` and
   move the cursor to the end (via the shared helper from step 3, exposed through
   a small `TuiInputView::insert_typeahead_text` or a direct `input_editor_model`
   update); then `ctx.notify()`. `None` is a no-op redraw (parity with today).
5. *TUI keystroke reporting (InputMatching, where feasible).* Report typed input
   as potential typeahead on the PTY-forward path so `InputMatching` shells work:
   at the point the TUI forwards user input during an LRC
   (`TuiTerminalContentElement::dispatch_event` /
   `TuiTerminalSessionAction::ForwardUserPtyBytes`), call
   `terminal_model.push_user_input(<typed chars>)` using the event's *semantic*
   characters (`KeyDown.chars`, `Paste.text`) — not the already-encoded PTY
   bytes — so the model can match them against the echo. The element already
   holds the model `Arc` (`terminal_content_element.rs:66,81-84,135 @ abea51c`),
   so it can report before/alongside dispatching `ForwardUserPtyBytes`. Ensure
   Enter contributes `\r` (the model's `push_user_input` already filters to
   printable chars + `\r`, and `handle_potential_typeahead` maps `\r`→`\n`).
6. *Tests.* Add TUI unit/element tests (per `tui-testing`) and, if step 5 is
   implemented, a `terminal_use`/forward-path test asserting `push_user_input` is
   called.

*Open questions resolved.*
- *Does the TUI need new event plumbing to observe typeahead?* No — it already
  receives `ModelEvent::Typeahead` (`terminal_session_view.rs:1096`); only the
  handler body is missing.
- *Where does typeahead get inserted in the TUI, and what if the editor is
  hidden during the LRC?* Into the existing `input_editor_model`; insertion into
  the model is valid while hidden and renders when input returns to the editor on
  `BlockCompleted`. No mode switch is forced (GUI parity).
- *How is the AI-requested-command exclusion kept consistent across front-ends?*
  It lives in the single shared `TerminalModel::take_typeahead_for_input`; both
  the GUI and TUI consume it (no duplicated rule).
- *How is the overwrite (incremental typeahead) handled in the TUI?* Via the
  model's existing `typeahead_chars_inserted` count returned by
  `advance_typeahead`, applied through the shared "replace first N chars" helper —
  identical arithmetic to the GUI.
- *Where do the raw characters for `InputMatching` come from in the TUI?* From
  the input event's semantic `chars`/`text` at the forward path, not the encoded
  PTY bytes (which would be lossy). Marked "where feasible" because it only
  affects bash 3.2; the reported repro is fixed by step 4 alone.

*Risks / blast radius.* The shared policy migration and TUI editor wiring are
covered with tests across the shared model and both front-ends.
- The GUI decision migration (step 2) touches the live GUI typeahead path;
  mitigate by keeping GUI behavior observably identical (the shared method is a
  faithful extraction) and covering it with existing GUI typeahead tests
  (`app/src/terminal/model/early_output_tests.rs`) plus a new shared-method unit
  test.
- Adding `CoreEditorModel::replace_first_n_characters` affects the shared core
  editor API; mitigate with focused Unicode and offset-count coverage plus the
  existing editor suites. The legacy GUI editor implementation is unchanged.
- Step 5 must not double-write or alter what reaches the foreground process —
  reporting typeahead is additive to (not a replacement for) the existing
  `ForwardUserPtyBytes` write; covered by an existing-passthrough regression
  assertion.

*Validation & verification criteria* (must ALL pass before merge):
1. *Repro fixed (ShellReported), verifies behavior #1.* A `crates/warp_tui` test
   (per `tui-testing`) drives a completed user-controlled block with accumulated
   typeahead `echo hi` and a `ModelEvent::Typeahead`, then asserts the input
   editor (`CodeEditorModel`) content == `echo hi` with the cursor at the end.
   Fails on the pre-change code (typeahead discarded), passes after.
2. *Regression test — empty typeahead is a no-op, verifies #6.* A test that emits
   `ModelEvent::Typeahead` with empty typeahead asserts the input editor is
   unchanged. Named alongside the test in #1.
3. *AI-exclusion test, verifies #3.* Typeahead entered during a block carrying
   `agent_interaction_metadata` is NOT inserted (input editor stays empty) — a
   `crates/warp_tui` test, and/or a `TerminalModel::take_typeahead_for_input`
   unit test returning `None` for the agent-requested case.
4. *Incremental overwrite test, verifies #5.* Two `Typeahead` events (`ec`, then
   `echo hi`) result in an input buffer of `echo hi`, not `ececho hi`.
5. *PTY passthrough preserved, verifies #4.* The existing
   `TuiTerminalSessionAction::ForwardUserPtyBytes` behavior (emits
   `WriteUserInput`) still holds — existing `terminal_session_view`/`terminal_use`
   tests still pass; if step 5 lands, add an assertion that user input during an
   LRC both reaches the PTY *and* is reported via `push_user_input`.
6. *Shared decision path used by both front-ends, verifies key design choice #2.*
   Code review + grep confirm the GUI `handle_typeahead_event` and the new TUI
   handler *both* call `TerminalModel::take_typeahead_for_input`, and that the
   AI-exclusion rule (`agent_interaction_metadata` check for typeahead) plus the
   `advance_typeahead` call each exist in exactly one place (the shared method) —
   no second copy in either front-end.
7. *Editor overwrite semantics, verifies key design choice #2.* A core-editor
   unit test covers `replace_first_n_characters` with incremental multibyte
   input. TUI input/session tests verify that incremental typeahead overwrites
   rather than appends and leaves the cursor at the end. Existing GUI tests
   continue to cover its unchanged zero-based editor path.
8. *GUI parity unchanged.* Existing GUI typeahead tests
   (`app/src/terminal/model/early_output_tests.rs` and any view-level typeahead
   tests) still pass unchanged after the GUI is migrated onto the shared decision
   method (step 2).
9. *Manual TUI verification (user-facing proof), verifies #1–#4.* Per the
   `tui-verify-change` skill (the TUI's analog to computer-use, since the TUI is
   headless): run the TUI in a real terminal, execute `sleep 5`, type `echo hi`
   while it runs, and confirm on completion that `echo hi` appears in the input
   editor. Also confirm an agent-requested LRC does NOT auto-insert. Attach the
   captured terminal output/recording to the task and the PR.
10. *Alt-screen non-regression, verifies #7 (behavior).* Confirm (test or manual)
    that typing inside a full-screen app during the run does not populate the
    input editor on exit (the shared model already gates `push_user_input` on the
    alt screen).
11. *InputMatching (where feasible), verifies #2.* If step 5 is implemented, a
    test asserts keystrokes are reported on the TUI forward path (bash-3.2 /
    `InputMatching` mode); if deferred, the PR explicitly records the skip and its
    rationale, and criteria #1–#10 must still pass for ShellReported shells.
12. *Presubmit.* `./script/presubmit` passes (fmt, `cargo clippy
    --workspace --all-targets --all-features --tests -- -D warnings`, and the
    nextest suite), per repo `AGENTS.md`.
