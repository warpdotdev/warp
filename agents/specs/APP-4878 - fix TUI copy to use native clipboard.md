# Spec: Fix TUI copy to use native clipboard with accurate feedback

Linear: [APP-4878](https://linear.app/warpdotdev/issue/APP-4878/copy-failing-consistently-in-warp-tui)
Originating thread: https://warpdev.slack.com/archives/C0BA99TSDB2/p1784335796549789
Estimate: M (3)
Commit-pinned references below are anchored at `warpdotdev/warp@e6aaaf9b8f84f786035cd797224a349abb56a4f5`.

## PRODUCT

**Summary:** Copying a selection or running `/export-to-clipboard` in the headless
Warp TUI (`crates/warp_tui`) fails consistently in the common case — a user
running the TUI on their own machine, most notably inside the Warp terminal
itself — yet the TUI still reports "copied to clipboard". The copy path relies
*entirely* on OSC 52, which only works if the host terminal accepts programmatic
clipboard writes; Warp's own terminal denies them by default. This change makes
the TUI write directly to the OS clipboard when it runs locally (so copy
actually works without the user changing any setting), keeps OSC 52 as the
transport for remote/SSH sessions, and makes the success/failure feedback
truthful.

**Key design choices:**
1. **Add a native OS-clipboard transport for local runs** (via the `arboard`
   crate, already a workspace dependency) and keep OSC 52 only for remote/SSH
   sessions, where the local clipboard isn't reachable. This is the minimal
   correct fix and does **not** touch the security-sensitive host-terminal
   `terminal.osc52_clipboard_access` default.
2. **Make the copy result honest.** The copy function returns a three-way
   outcome — confirmed native copy, best-effort OSC 52 send (host may reject),
   or genuine error — and the footer hint reflects which happened, so a copy is
   never falsely reported as "copied to clipboard".
3. **Detect local vs remote from the environment** (`SSH_CONNECTION` / `SSH_TTY`)
   rather than a new user setting, so the fix works out of the box.

**Behavior** (numbered, testable invariants from the TUI user's view):
1. **Local copy works with no setting change (default/happy path).** With
   `terminal.osc52_clipboard_access` at its default `Deny`, running the TUI on
   the user's own machine (including `./script/run-tui` inside the Warp
   terminal), selecting transcript text and releasing the selection places that
   exact text on the OS clipboard; pasting into another app yields the selected
   text. This holds on macOS, Linux, and Windows.
2. **`/export-to-clipboard` works locally.** Running `/export-to-clipboard` on a
   local TUI places the conversation markdown on the OS clipboard; pasting
   yields the exported markdown.
3. **Accurate success feedback.** The footer shows the success hint
   ("copied to clipboard" for selection; the export success message for
   `/export-to-clipboard`) **only** when the text was actually placed on the
   clipboard (a confirmed native write).
4. **Honest best-effort feedback for remote/SSH.** In a remote/SSH session (no
   reachable local OS clipboard), the TUI uses OSC 52 and shows a message that
   states the text was sent to the terminal (best-effort), distinct from the
   confirmed-copy message — it does not claim a guaranteed clipboard write,
   because OSC 52 host acceptance cannot be confirmed by the TUI.
5. **Accurate failure feedback.** When copy genuinely fails (e.g. the native
   backend is unavailable *and* the OSC 52 write to stdout errors), the footer
   shows the existing failure hint ("failed to copy to clipboard") — never the
   success hint.
6. **Local fallback when there is no OS clipboard.** On a local run where the OS
   clipboard is unavailable (e.g. a headless Linux box with no display server),
   the TUI falls back to OSC 52 (behaviour 4) rather than reporting a hard
   failure, so it is never worse than today's behaviour.
7. **Clipboard content survives while the TUI runs (Linux).** After a local copy
   on Linux (X11/Wayland), pasting into another application while the TUI process
   is still running succeeds — the copied text is not lost immediately after the
   copy action.

**Non-goals:**
- Changing the host terminal's `terminal.osc52_clipboard_access` default
  (`Deny`). That is a whole-terminal security-posture decision requiring separate
  product/security sign-off (see *Design alternatives*).
- Clipboard *read*/paste-into-TUI behaviour. This change is copy-out only.
- Adding a new user-facing setting to choose the transport.

## TECH

**Context — how copy works today:**
- `copy_to_clipboard(text)` (`crates/warp_tui/src/clipboard.rs:12 @ e6aaaf9`)
  base64-encodes `text` and writes OSC 52 sequences
  (`ESC]52;c;<b64>BEL` for the clipboard and `;p;` for the Linux PRIMARY
  selection, tmux-wrapped when `$TMUX` is set) to `stdout`, then flushes. It
  returns `io::Result<()>` and yields `Ok(())` whenever the **write + flush**
  succeed — i.e. as soon as the bytes leave the process, regardless of whether
  the host terminal honoured them.
- Selection copy: `TuiSelectable::on_copy`
  (`crates/warp_tui/src/transcript_view.rs:653 @ e6aaaf9`) →
  `TuiTranscriptViewEvent::SelectionEnded(text)` handled at
  `crates/warp_tui/src/terminal_session_view.rs:842 @ e6aaaf9`, which calls
  `copy_to_clipboard` and, on `Ok`, calls `show_copy_hint` →
  `show_success_hint(COPY_SELECTION_HINT)` (`terminal_session_view.rs:1492 @ e6aaaf9`);
  on `Err` it shows `COPY_FAILED_HINT`.
- `/export-to-clipboard`: `terminal_session_view.rs:2156 @ e6aaaf9` calls
  `copy_to_clipboard(markdown)`, showing a success hint on `Ok` and
  `COPY_FAILED_HINT` on `Err`.
- Hint constants: `COPY_SELECTION_HINT = "copied to clipboard"` and
  `COPY_FAILED_HINT = "failed to copy to clipboard"`
  (`terminal_session_view.rs:157-158 @ e6aaaf9`).

**Why it fails (root cause, confirmed by code review):** the callers already
distinguish `Ok`/`Err`, but `copy_to_clipboard` **never returns `Err` on host
rejection** — a successful stdout write is reported as success. When the host is
Warp's own terminal, inbound OSC 52 is gated by `terminal.osc52_clipboard_access`
(`app/src/terminal/settings.rs:26-52,186-195 @ e6aaaf9`), whose default is
`Deny` (`#[default] Deny`, line 27-29). On `ModelEvent::ClipboardStore`, the host
only writes to the real clipboard when `access.allows_write()` is true
(`WriteOnly`/`ReadWrite`); otherwise it drops the write and shows a "blocked"
banner **in the host grid**, which the TUI's alt screen hides
(`app/src/terminal/view.rs:11676-11687 @ e6aaaf9`). So a local TUI-in-Warp user
sees a false "copied to clipboard" and an empty clipboard, consistently and
independent of OS. Any other host terminal that doesn't support/enable OSC 52
behaves the same. The selection-copy feature is ~1 week old (added `86dfca99`,
`a77348c6`); this is not a regression — OSC-52-only never worked against a
default Warp host.

**`arboard` availability:** `arboard = "3.6.1"` is already a workspace dependency
(`Cargo.toml:112 @ e6aaaf9`, `default-features = false`) and is used by
`crates/warpui` and `crates/warpui_core` — but **only** for
`cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))`
(`crates/warpui/Cargo.toml:175-179`, `crates/warpui_core/Cargo.toml:113-114 @ e6aaaf9`);
on macOS the GUI uses `NSPasteboard` via `objc2-app-kit` directly. `arboard`
itself **does** support macOS, so the TUI can depend on it for all desktop
targets (macOS/Linux/Windows/FreeBSD) to get a single cross-platform native
transport. The reporter is on macOS, so macOS coverage is mandatory.

### Design alternatives

- **Transport strategy** (how to make local copy work):
  - *Native for local + OSC 52 for remote* — **chosen.** Writes directly to the
    OS clipboard when local (fixes the common case with no setting change),
    falls back to / uses OSC 52 for SSH/remote where the local clipboard isn't
    reachable. Pros: fixes the reported bug directly; no security-posture change;
    reuses an existing dependency. Cons: needs a local-vs-remote heuristic;
    slightly more code than today.
  - *Always send both native and OSC 52* — rejected. On a remote host the native
    write lands on the **server's** clipboard (wrong machine) and can shadow the
    OSC 52 result; also muddies success reporting.
  - *Flip `terminal.osc52_clipboard_access` default to `WriteOnly`* — rejected
    for this fix. It is a whole-terminal security-posture change (affects every
    program the host runs, not just the TUI) and needs separate product/security
    sign-off; it also would not help against non-Warp hosts that don't support
    OSC 52.
- **Native clipboard crate:**
  - *`arboard` for all desktop targets* — **chosen.** Already a workspace dep,
    cross-platform including macOS, text-only usage needs no extra features.
  - *Platform-specific (`NSPasteboard` on macOS, `x11rb`/`arboard` elsewhere)* —
    rejected as unnecessary complexity when `arboard` covers all three desktop
    OSes.
- **Transport ordering — env-detection, NOT try-then-fallback:**
  - *Environment-detected transport (`SSH_CONNECTION` / `SSH_TTY` present ⇒
    remote ⇒ OSC 52; otherwise local ⇒ native `arboard`, with OSC 52 as a
    last-resort fallback only if `arboard` errors)* — **chosen.** The transport
    is selected up front from the environment; native is **not** merely a
    fallback behind an OSC 52 attempt.
  - *Try OSC 52 first, native as fallback* — **rejected.** OSC 52 is entirely
    fire-and-forget: the host sends **no acknowledgment**, so the TUI cannot
    detect whether the write was accepted or dropped without a fragile
    before/after clipboard poll. "OSC 52 first" would therefore always look like
    success and never trigger the native fallback — re-introducing the exact
    false-success bug this change fixes. Detecting the session up front avoids
    relying on an unobservable signal. (This is the pattern the Codex Rust TUI
    uses in `codex-rs/tui/src/clipboard_copy.rs`.)
  - *Try native first, always (ignoring the environment)* — rejected as the
    *primary* signal: on a remote host `arboard` may "succeed" writing to the
    **server's** clipboard (wrong machine), so native success can't be trusted
    to mean the user's machine got the text. The env heuristic gates native to
    the local case; the try-then-fallback pattern is used only *within* the
    local case (native → OSC 52 on `arboard` error, behaviour 6).
  - The env heuristic is imperfect for exotic setups (mosh, nested sessions);
    acceptable because the local fallback (behaviour 6) keeps those no worse
    than today.
- **Feedback for the OSC 52 path:** keep a distinct, honest message
  ("sent to terminal…") rather than reusing the confirmed-copy hint, because the
  TUI cannot observe whether the host accepted the sequence. Reusing the success
  hint would re-introduce the false-success bug for remote users.
- **Feature flag / gate:** none. This is a correctness fix restoring expected
  copy behaviour on an explicit user action; there is no partial-rollout risk
  worth a flag, and the local fallback bounds the downside. (Alternative
  considered: gate behind a flag — rejected as unnecessary ceremony for a bug
  fix.)

### Proposed changes

1. **`crates/warp_tui/Cargo.toml`** — add `arboard` (workspace, text-only, no
   extra features) for desktop targets:
   ```toml
   [target.'cfg(any(target_os = "macos", target_os = "linux", target_os = "freebsd", target_os = "windows"))'.dependencies]
   arboard = { workspace = true }
   ```
2. **`crates/warp_tui/src/clipboard.rs`** — restructure the copy path around an
   explicit outcome and a selectable transport:
   - Introduce an outcome type returned to callers:
     ```rust
     pub(crate) enum ClipboardCopy {
         /// Text was written to the OS clipboard and confirmed (native path).
         Copied,
         /// Text was emitted via OSC 52 to the host terminal (best-effort;
         /// the host may silently reject it, e.g. Warp's default Deny).
         SentToTerminal,
     }
     ```
     `copy_to_clipboard(text) -> anyhow::Result<ClipboardCopy>` (or an equivalent
     error type). `Ok(Copied)` means confirmed; `Ok(SentToTerminal)` means
     best-effort OSC 52; `Err` means genuine failure (native unavailable *and*
     OSC 52 stdout write errored).
   - Decision logic:
     - **Remote/SSH** (`std::env::var_os("SSH_CONNECTION").is_some()` or
       `SSH_TTY`): write OSC 52 (existing code path) → `Ok(SentToTerminal)`, or
       `Err` if the stdout write fails.
     - **Local:** attempt a native write via `arboard`; on success →
       `Ok(Copied)`; on native error, fall back to OSC 52 → `Ok(SentToTerminal)`
       or `Err`.
   - Keep the existing OSC 52 encoding helpers (`osc52_sequences`,
     `tmux_passthrough`, `write_osc52_sequences`) intact — they remain the
     remote/fallback transport and keep their current unit tests.
   - Make the decision **unit-testable without touching a real OS clipboard**:
     factor the transport selection so tests inject (a) the environment
     (local vs SSH) and (b) a native-clipboard backend (a small trait or fn
     pointer, with a fake that returns `Ok`/`Err`) and (c) the OSC 52 writer
     (already parameterised via `write_osc52_sequences(_, _, &mut impl Write)`).
     Only the real `copy_to_clipboard` entry point binds the real `arboard`
     backend and real `stdout`.
   - **Linux clipboard-ownership caveat (a real, mandatory implementation
     detail — the "ClipboardLease" pattern):** on X11/Wayland the clipboard
     contents are *served by the process that owns the selection*, so **dropping
     the `arboard::Clipboard` handle makes the copied text disappear**. Because
     the TUI is a long-lived process, hold the `arboard::Clipboard` for the
     **TUI's entire lifetime** (lazily initialised and retained, e.g. a
     `OnceLock<Mutex<Clipboard>>` process-lifetime handle) and reuse it for every
     copy — never construct-and-drop a `Clipboard` per copy. Codex names this a
     `ClipboardLease`; the naming is optional but the lifetime requirement is
     not. This satisfies behaviour 7. Calls happen on the UI thread
     synchronously, matching the current OSC 52 write.
   - **Platform behaviour of the native path (`arboard`), confirmed against
     prior art:**
     - *macOS* — works with no GUI window; `NSPasteboard` is a system service.
       (This is the reporter's platform.)
     - *Linux X11/Wayland* — works when `$DISPLAY` / `$WAYLAND_DISPLAY` is set,
       subject to the ClipboardLease lifetime requirement above.
     - *Linux headless / SSH* — `arboard` fails (no display / not the local
       machine); the transport selection routes these to OSC 52 (SSH by env
       detection; a displayless local run by the native→OSC 52 fallback,
       behaviour 6).
     - *Windows* — works via the system clipboard.
     Prior art using `arboard` in a Rust TUI in exactly this way: the Codex TUI
     (`codex-rs/tui/src/clipboard_copy.rs`, env-detected native/OSC 52 with a
     `ClipboardLease`). The opencode TUI (Go) achieves the same effect via
     `atotto/clipboard`, which shells out to `pbcopy`/`xclip`/`wl-copy`.
3. **`crates/warp_tui/src/terminal_session_view.rs`** — update the two call
   sites to map the new outcome to hints:
   - Selection copy (`:842`): `Ok(ClipboardCopy::Copied)` → `show_copy_hint`
     (existing `COPY_SELECTION_HINT`); `Ok(ClipboardCopy::SentToTerminal)` → a
     new best-effort hint (e.g. `COPY_SENT_TO_TERMINAL_HINT =
     "copied via terminal"`); `Err` → `COPY_FAILED_HINT` (unchanged).
   - `/export-to-clipboard` (`:2156`): `Copied` → the confirmed success message
     ("Conversation copied to clipboard"); `SentToTerminal` → the existing
     "Conversation sent to terminal clipboard" wording (already best-effort in
     tone); `Err` → `COPY_FAILED_HINT`.
   - Add the new hint constant next to `COPY_SELECTION_HINT`/`COPY_FAILED_HINT`
     (`:157-158`). Keep messages short (single footer row).
4. **Tests** — update `crates/warp_tui/src/clipboard_tests.rs` for the new API
   and add the regression/decision tests below.

**Open questions resolved:**
- *Which clipboard crate, and does it cover macOS?* `arboard` (already in the
  workspace); it supports macOS, so add it for all desktop targets — resolved
  from `Cargo.toml` and crate docs.
- *How to tell local from remote?* `SSH_CONNECTION`/`SSH_TTY` env presence —
  resolved as the chosen heuristic with a documented local fallback.
- *What to show when we can't confirm the copy (OSC 52)?* A distinct best-effort
  message, never the confirmed-copy hint — resolved (behaviour 4).
- *Do we change the host OSC 52 default?* No — explicitly out of scope
  (Non-goals) pending separate security sign-off.
- *Feature flag?* No — resolved (see *Design alternatives*).

**Risks / blast radius:**
- Scope is confined to `crates/warp_tui` (one new dependency edge, one module,
  two call sites). No change to the host terminal or its settings, so no
  security-posture change.
- New dependency-compile surface: `arboard` is already built for Linux/Windows
  in the workspace; adding the macOS target pulls `arboard`'s macOS backend
  (`objc2`-based) into the TUI build. Mitigation: text-only, `default-features =
  false`; confirm the TUI still builds on macOS in presubmit/CI.
- Linux clipboard ownership (behaviour 7) — mitigated by the process-lifetime
  `Clipboard` handle; called out as an explicit test criterion.
- SSH heuristic false-negatives (exotic setups) fall back to OSC 52 — no
  worse than today.

## Validation & verification criteria (must ALL pass before merge)

1. **Reproduction is fixed (hands-on).** With `terminal.osc52_clipboard_access`
   left at its default `Deny`, run the headless TUI inside the Warp terminal
   (`./script/run-tui`), select transcript text (and separately run
   `/export-to-clipboard`), then paste into another application: the pasted
   content is the selected text / exported markdown. (Triage recorded an
   env-mismatch skip because the triage sandbox had no interactive Warp GUI
   terminal; the implementation must perform this hands-on check on a machine
   with a real display, per `factory-verification`.) *Checked by: manual
   TUI-in-Warp copy→paste on macOS (reporter's platform) and, if available,
   Linux.*
2. **Regression test — no false success.** A new unit test asserts that the
   OSC 52-only path returns `Ok(ClipboardCopy::SentToTerminal)` (not `Copied`)
   and that a native-unavailable + failing-stdout path returns `Err`; and that
   the view maps `SentToTerminal`/`Err` to the best-effort/failure hints and
   only `Copied` to `COPY_SELECTION_HINT`. This test fails against the current
   code (which returns `io::Result<()>` and cannot distinguish) and passes after
   the refactor. *Checked by: `cargo nextest run -p warp_tui`; suggested names
   `local_copy_reports_copied`, `ssh_session_reports_sent_to_terminal`,
   `native_failure_falls_back_to_osc52`, `hard_failure_reports_err`.*
3. **Transport selection is correct and injectable.** Unit tests drive the
   decision function with an injected environment and fake native backend:
   (a) SSH env set ⇒ OSC 52 chosen, native never called; (b) local + native
   `Ok` ⇒ native chosen, `Copied`; (c) local + native `Err` ⇒ OSC 52 fallback,
   `SentToTerminal`; (d) local + native `Err` + OSC 52 write `Err` ⇒ `Err`. No
   test touches a real OS clipboard. *Checked by: `cargo nextest run -p warp_tui`.*
4. **Existing OSC 52 encoding behaviour preserved.** The current
   `clipboard_tests.rs` assertions for `osc52_sequences` (clipboard + PRIMARY,
   UTF-8 payload), `tmux_passthrough`, and stdout-error propagation still pass
   (updated only as needed for the new entry-point signature). *Checked by:
   `cargo nextest run -p warp_tui`.*
5. **Native copy round-trips the exact text.** Where a real clipboard is
   available (local dev/CI with a display, or the manual check in #1), copying
   `"hello 日🙂"` and reading the OS clipboard yields the identical string
   (no truncation/encoding loss). *Checked by: manual paste verification in #1;
   optionally an ignored/`#[cfg]`-gated integration test that is not run in
   headless CI.*
6. **Linux clipboard content persists while running.** On Linux (X11), after a
   local copy, pasting into another app while the TUI process is still alive
   succeeds (behaviour 7). *Checked by: manual copy→paste on a Linux desktop
   session during the hands-on check.*
7. **Builds on all desktop targets.** The `warp_tui` crate compiles on macOS,
   Linux, and Windows with the new `arboard` dependency edge. *Checked by:
   `./script/presubmit` (and CI's per-platform builds).*
8. **No collateral damage.** `/export-to-file` and the other footer hints are
   unaffected; the footer still lays out one row; no new clippy/fmt violations.
   *Checked by: `./script/presubmit` (`./script/format` + `cargo clippy
   --workspace --all-targets --all-features --tests -- -D warnings`) and the
   `warp_tui` test suite.*
9. **Presubmit passes.** `./script/presubmit` is green. *Checked by: running it.*

## Notes for implementation
- The TUI is user-facing, but it is the **headless** front end: verification is
  by running `./script/run-tui` and observing the rendered result / real
  copy→paste (per the `tui-verify-change` and `tui-testing` skills), plus the
  deterministic unit tests above — **not** the GUI `computer_use` /
  integration-test path.
- Keep imports at the top of the module and prefer inline format args per repo
  conventions (`AGENTS.md`). Place the new unit tests in
  `crates/warp_tui/src/clipboard_tests.rs` (and, if view-level mapping is
  tested, alongside the existing `terminal_session_view` tests).
