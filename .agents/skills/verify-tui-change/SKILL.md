---
name: verify-tui-change
description: Verify a change to Warp's headless TUI front-end (crates/warp_tui) by building and running it in a real interactive terminal and observing the rendered output — no computer_use required. Use whenever you change TUI UI, rendering, input, or behavior and need to confirm the real on-screen result.
---

# verify-tui-change

Verify a change to Warp's **headless TUI** front-end (`crates/warp_tui`, and the
cell-grid element library in `crates/warpui_core/src/elements/tui`) by actually
running the TUI and looking at what it renders.

The whole point: **the TUI is a console program, so you can run it directly and
watch it in a terminal — you do NOT need `computer_use`, a real display, or a
cloud screenshot agent** the way GUI verification does (contrast the GUI-only
`onboarding-verification-skill`, `warp-integration-test`, `integration-test-video`,
and the `computer_use`/`verify-ui-change-in-cloud` flow). This is the fast path,
and it's the preferred way to confirm a TUI change end-to-end.

This skill covers the **manual live-run** verification. For a durable regression
that runs in CI, also add a render-to-lines unit test per `warp-tui-testing` —
the two are complementary: the live run confirms real behavior; the snapshot
test locks it in.

## When to use

- You changed anything a TUI user sees or interacts with: an element/layout in
  `crates/warpui_core/src/elements/tui`, a TUI view/screen in `crates/warp_tui`
  (transcript, input, zero state, login placeholder, etc.), TUI keybindings, or
  TUI rendering/behavior.
- You want to confirm the *actual* rendered result, not just that it compiles or
  a unit test passes.

If your change is GUI-only (`app/`, WarpUI pixel `Element`/`View`), this is the
wrong skill — use the GUI verification path instead.

## Step 1 — Build the TUI

Always build with a small job count so the large `warp` dependency tree doesn't
OOM the machine:

```bash
cd <warp-repo-root>
CARGO_BUILD_JOBS=2 cargo build -p warp_tui --bin warp-tui-oss
```

- `warp-tui-oss` is the OSS channel binary and the safest default (no internal
  `warp-channel-config` generator required). `./script/run-tui` does the
  equivalent, selecting the internal `local` binary when the generator is
  available and falling back to `warp-tui-oss` otherwise.
- Fix all compile errors before running (see `fix-errors`). A cold build of this
  tree takes a while; subsequent runs are fast.

### Logged-in vs logged-out (important)

The OSS build starts **logged out** and stops at a `Sign in to continue`
placeholder (it drives a device-authorization login flow that needs a browser).
The login-gated root has three pre-session states you may see:

- `AwaitingLogin` → a centered placeholder that reads `Sign in to continue`,
  then `Opening your browser…` (or, once the device code is known, `Open <uri> in
  your browser` and `and enter code: <code>`). It does **not** show a Ctrl-C hint.
- `LoggedIn` → briefly `Starting terminal…`, then the **zero state** (`Warp
  Agent` + version, a "What's new" list, and the project context section) with the
  input view.
- `Failed` → `Login failed: <message>` followed by `Press Ctrl-C to exit.`

(Exact strings live in `crates/warp_tui/src/ui.rs` — verify against it if you're
asserting on placeholder text.)

So: if your change is on the **login placeholder** or a pure element/layout, the
logged-out OSS build is enough. If your change is in the **live terminal /
transcript / input** surface, you must reach the authenticated state — run a
build/session that is already logged in rather than the plain OSS binary, or your
change will sit behind the login gate and you'll only ever see `Sign in to
continue`.

## Step 2 — Run it in a REAL interactive terminal (this is mandatory)

The TUI **requires a genuine interactive PTY**. This is the single biggest
pitfall, confirmed hands-on:

- If stdin is redirected, piped, or closed (e.g. `./warp-tui-oss </dev/null`, or
  running it where no live keyboard input exists), the TUI draws exactly **one
  frame and then exits with code 101** — it looks like a crash but it's just the
  input stream ending. Do not conclude the TUI is broken from this; give it a
  real PTY.
- It uses the terminal's **alternate screen** (full-screen). Piping stdout gives
  you a soup of escape sequences, and the alt-screen teardown wipes most of it,
  so `... | tail` shows nothing useful.
- On startup it **probes the terminal**: OSC `10`/`11` (background/foreground
  color) and a device-attributes query (`ESC [ c`). A normal PTY answers these;
  make sure the terminal has a sane size — a 1-row PTY renders nothing useful.

The reliable way to run and observe it here is the **interactive terminal
sub-agent (LRC)** — i.e. run the command in `interact` mode so a watcher drives a
live PTY. Optionally wrap it in `script` to also capture a raw typescript you can
inspect offline:

```bash
cd <warp-repo-root>
script -qfc './target/debug/warp-tui-oss' /tmp/tui-verify.log
```

(`./script/run-tui` works the same way; `script -qfc` guarantees a PTY and
records every byte to the typescript for later inspection.)

## Step 3 — Give the watcher VERY explicit "look for X" instructions

The watcher is looking at a full-screen TUI it has never seen. Vague
instructions get vague results. Tie the instructions to **your specific change**:

- State the **exact text or on-screen state** your change should produce, quoted,
  and where on screen it should appear (top/bottom, which line/section).
- State exactly **what to type or which keys to press** to trigger the change,
  step by step.
- Ask the watcher to **report verbatim** what it sees in the relevant region
  before and after the interaction — not a judgment ("looks right") but a
  description ("the footer line reads `...`").
- Tell it how to **quit cleanly** (see below) and to report the exit.
- Keep it observational: you (not the watcher) decide whether what it reports
  matches the expected result.

Template task for the watcher:

```text
You are observing the headless Warp TUI (full-screen, alternate screen). Do not
run other commands; only interact with this running program.
1. Wait for the UI to draw.
2. My change is: <one line>. It should make the screen show: <exact expected
   text/state, quoted, and where>.
3. To trigger it: <exact keystrokes/steps>. Before and after, describe verbatim
   the <specific region> of the screen.
4. Report exactly what you saw (quote text), whether <expected string> appeared,
   any glitches/flicker, and how the program exited.
5. Quit: press Ctrl-C. On the login placeholder one Ctrl-C exits. In a live
   session Ctrl-C is press-again-to-exit: the first press cancels/clears input
   and arms a 1-second window (a footer hint appears); press Ctrl-C again within
   1s to exit. Follow whatever the footer hint says and confirm the shell prompt
   returns.
```

## Step 4 — Inspect frames offline (optional)

If you captured a typescript, replay the raw frames (escape codes visible) to
confirm exactly what was drawn:

```bash
cat -v /tmp/tui-verify.log     # shows control sequences literally
```

Quitting cleanly matters: leaving the alt screen active can leave the outer
terminal in a weird state. If that happens, run `reset`.

## Step 5 — Lock it in with a snapshot test

A live run proves the change works now; it is not a regression guard. For any
non-trivial TUI rendering/behavior change, add or update a render-to-lines unit
test (`warpui_core::elements::tui::test_support::render_to_lines` /
`TuiBuffer::to_lines`) per `warp-tui-testing`, and run:

```bash
cargo nextest run -p warp_tui
cargo nextest run -p warpui_core
```

## Evidence for the PR

For a user-visible TUI change, attach the concrete rendered result — the relevant
lines from the typescript (`cat -v` output) and/or the `render_to_lines` snapshot
diff — as the verification evidence. This is the TUI equivalent of the GUI's
`computer_use` screenshot (see the TUI caveat in `review-pr-local`).

## Related skills

`warp-tui-guidelines` (the `TuiElement` cell-grid library) and `warp-tui-testing`
(render-to-lines unit tests) are companion TUI skills added alongside this one;
land them together. This skill's build/run/observe workflow stands on its own —
those cover authoring TUI UI and writing durable tests.

GUI-only counterparts (do **not** use for TUI work): `warp-integration-test`,
`integration-test-video`, `onboarding-verification-skill`.
