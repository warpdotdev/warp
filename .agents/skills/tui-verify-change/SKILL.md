---
name: tui-verify-change
description: Verify a change to Warp's headless TUI front-end (crates/warp_tui) by running it under tmux and reading the rendered screen back with `tmux capture-pane` — the agent sees the real frame, no computer_use required. Use whenever you change TUI UI, rendering, input, or behavior and need to confirm the real on-screen result.
---

# tui-verify-change

Verify a change to Warp's **headless TUI** front-end (`crates/warp_tui`, and the
cell-grid element library in `crates/warpui_core/src/elements/tui`) by running it
and reading back the actual rendered screen.

The whole point: **the TUI is a console program, so you can run it under `tmux`,
drive it with `tmux send-keys`, and read the real rendered frame straight back
with `tmux capture-pane`.** You (the agent) see the actual screen text — no
`computer_use`, no real display, no cloud screenshot agent, and no relying on a
separate watcher's description of what it saw. (Contrast the GUI-only
`gui-onboarding-verification-skill`, `gui-integration-test`,
`gui-integration-test-video`, and the `computer_use` / `verify-ui-change-in-cloud`
flow.) This is the fast, preferred way to confirm a TUI change end-to-end.

This skill covers the **manual live-run** verification. For a durable regression
that runs in CI, also add a render-to-lines unit test per `tui-testing` — the two
are complementary: the live run confirms real behavior; the snapshot test locks
it in.

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
- Fix all compile errors before running (see `fix-errors`). The first build of
  this tree takes a while; **incremental rebuilds after a one-line change are
  fast (~10s)**, so the edit → rebuild → re-capture loop below is quick.

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

## Step 2 — Run under tmux and read the frame back

The TUI needs a **real interactive PTY** — a tmux pane is exactly that, and it
lets you both send input and read the rendered screen back as text. Start the TUI
in a detached session with an explicit size (don't skip `-x`/`-y`; a degenerate
1-row pane renders nothing useful):

```bash
cd <warp-repo-root>
tmux kill-session -t tuicheck 2>/dev/null   # clear any prior run
tmux new-session -d -s tuicheck -x 120 -y 40 './target/debug/warp-tui-oss'
sleep 1                                       # let it draw + probe the terminal
tmux capture-pane -t tuicheck -p              # <-- the rendered screen, as text
```

`tmux capture-pane -p` prints the pane's current contents (the TUI's alternate
screen) to stdout, so **you read the real frame directly** and assert on it. Add
`-e` to include ANSI escape sequences when you need to check colors/styles:

```bash
tmux capture-pane -t tuicheck -p -e          # includes color/style escapes
```

Drive interactions with `tmux send-keys`, sleeping to let the UI settle, then
capture again. For example (type a line, submit it, wait, then read the screen):

```bash
tmux send-keys -t tuicheck "What is 2+2? Answer in one short sentence." && \
  sleep 1 && tmux send-keys -t tuicheck Enter && sleep 5 && \
  tmux capture-pane -t tuicheck -p -e
```

Send special keys by name (`Enter`, `Escape`, `C-c` for Ctrl-C, `Up`/`Down`).
When done, tear the session down: `tmux kill-session -t tuicheck`.

### Iterate loop

Because incremental rebuilds are ~10s, iterate tightly: edit the TUI code →
`cargo build -p warp_tui --bin warp-tui-oss` → `tmux kill-session` + restart the
session → `capture-pane` and compare. Verified before/after example: changing the
login placeholder string and rebuilding flips the captured line from
`Sign in to continue` to the new text, visible directly in `capture-pane` output.

## Step 3 — Check the captured frame against your change

You have the real screen text, so verify it yourself: grep/scan the
`capture-pane` output for the **exact string or layout** your change should
produce, and diff the before/after captures. No watcher interpretation needed —
if the expected text isn't in the capture, the change isn't rendering.

## Pitfalls (learned hands-on)

- **It needs a live PTY.** Run it *inside* tmux (or another real terminal). If
  you run it with redirected/piped/closed stdin (`./warp-tui-oss </dev/null`,
  `... | tail`), it draws one frame and exits with code **101** — that's the input
  stream ending, not a crash.
- **In a constrained/headless environment** (e.g. a cloud runner without a fully
  interactive terminal) the TUI may still exit ~1s after the first frame even
  under tmux. Work around it by **polling `capture-pane` right after launch** to
  catch the frame before it exits:

  ```bash
  tmux new-session -d -s tuicheck -x 120 -y 40 './target/debug/warp-tui-oss'
  for i in $(seq 1 15); do
    frame=$(tmux capture-pane -t tuicheck -p | sed 's/[[:space:]]*$//' | grep .)
    [ -n "$frame" ] && { echo "$frame"; break; }
    sleep 0.2
  done
  tmux kill-session -t tuicheck 2>/dev/null
  ```

  In a normal interactive terminal the session persists and you can
  `send-keys`/`capture-pane` repeatedly without racing.
- **Alt screen is handled for you.** `capture-pane` reads the alternate screen,
  so you don't need to fight escape-sequence soup the way piping stdout would.
- **Startup probe.** On launch the TUI emits terminal probes (OSC `10`/`11` +
  a device-attributes query) to pick a theme; a normal tmux pane answers them.
  Give it ~0.5–1s (a `sleep`) before the first capture.
- **Quitting.** Ctrl-C is `tmux send-keys -t tuicheck C-c`. On the login
  placeholder one Ctrl-C exits; in a live session it's press-again-to-exit (first
  press cancels/clears input and arms a ~1s window; a second within the window
  exits). Always `tmux kill-session` at the end so a stray session doesn't linger.

## Step 4 — Lock it in with a snapshot test

A live run proves the change works now; it is not a regression guard. For any
non-trivial TUI rendering/behavior change, add or update a render-to-lines unit
test (`warpui_core::elements::tui::test_support::render_to_lines` /
`TuiBuffer::to_lines`) per `tui-testing`, and run:

```bash
cargo nextest run -p warp_tui
cargo nextest run -p warpui_core
```

## Evidence for the PR

For a user-visible TUI change, attach the concrete rendered result — the relevant
lines from `tmux capture-pane` (and/or the `render_to_lines` snapshot diff) — as
the verification evidence. This is the TUI equivalent of the GUI's `computer_use`
screenshot (see the TUI caveat in `review-pr-local`).

## Related skills

`tui-ui-guidelines` (the `TuiElement` cell-grid library) and `tui-testing`
(render-to-lines unit tests) are companion TUI skills added alongside this one;
land them together. This skill's build/run/capture workflow stands on its own —
those cover authoring TUI UI and writing durable tests.

GUI-only counterparts (do **not** use for TUI work): `gui-integration-test`,
`gui-integration-test-video`, `gui-onboarding-verification-skill`.
