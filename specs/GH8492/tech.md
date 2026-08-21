# Tech Spec: Native tmux integration (control mode)

**Issue:** [warpdotdev/warp#8492](https://github.com/warpdotdev/warp/issues/8492)

## Problem

Warp's terminal owns the PTY for each pane and renders bytes into its block/grid model. tmux, run normally, paints its own full-screen TUI into one pane, so Warp's native tabs/splits, command search, and blocks stop working. tmux **control mode** (`tmux -CC`) inverts this: tmux stops painting and instead emits a line-oriented `%`-notification protocol describing windows, panes, and output, while accepting tmux commands on stdin. The host renders its own native UI. This spec adds a tmux control-mode backend so tmux windows become Warp tabs and tmux panes become Warp split panes, with the session persisting across restarts.

## Relevant code

Warp's terminal pipeline is already parameterized on the byte source, which is what makes this feasible without touching the renderer:

- `app/src/terminal/terminal_manager.rs:24` — `TerminalManager` trait (`fn model(&self) -> Arc<FairMutex<TerminalModel>>`; shared `create_terminal_model` at :111). Existing impls in `local_tty/`, `remote_tty/terminal_manager.rs:190`, and `mock_terminal_manager.rs:19` prove non-PTY backends slot in here. **This is the seam a `TmuxTerminalManager` implements.**
- `app/src/terminal/remote_tty/event_loop.rs:35` — a WebSocket "network PTY": binary frames → `process_pty_bytes` (:225) into `TerminalModel`; text frames = control channel. The structural template for a tmux transport (one connection, two message kinds).
- `app/src/terminal/model/ansi/mod.rs:394` — `Processor::parse_bytes`, the byte→model entry every backend calls. **⚠️ `mod.rs:760-784` + `dcs_hooks.rs` show the DCS hook buffers the entire payload until `unhook()`** — unusable for a session-lifetime control-mode DCS (see Risks).
- `app/src/terminal/writeable_pty/pty_controller.rs:833` — `EventLoopSender::send(Message)`; `Message` enum at `writeable_pty/message.rs:7`, with `Resize(SizeInfo)` at :22 delivered **per pane**.
- `app/src/pane_group/tree.rs:110` — `PaneNode`/`PaneBranch` split tree; `app/src/app_state.rs:100-206` — `PaneNodeSnapshot`/`BranchSnapshot`/`LeafSnapshot` (the tmux layout shape); `pane_group/mod.rs:1497` — `PaneGroup::restore_pane_tree`; `workspace/view.rs:12793` — `Workspace::add_tab_with_pane_layout(PanesLayout::Snapshot(..))`.
- `app/src/util/bindings.rs:897` — `is_binding_pty_compliant`, the consume-as-Warp-action vs forward-to-shell gate.
- `app/src/local_control/handlers/layout.rs:38` — `create_tab`, the existing external-description→native-tab mapping to mirror.
- **Existing tmux code (context, nothing to reuse):** there is no control-mode parser in the tree today. The only tmux code is (a) the **already-deprecated** SSH-wrapper flow, now removed and left only as migration tombstones - `app/src/terminal/warpify/settings.rs` (`warpify.ssh.use_ssh_tmux_wrapper`, `warpify.ssh.ssh_tmux_deprecation_notice_pending`) and the one-time `app/src/terminal/view/ssh_tmux_deprecation_banner.rs` - and (b) `crates/warp_terminal/src/model/escape_sequences.rs:144` `tmux_passthrough()`, which only wraps OSC52 clipboard DCS. Neither parses the control-mode `%`-protocol or drives panes, so the new parser is greenfield with nothing to extend.

## Current state

There is no tmux control-mode rendering path. Running `tmux -CC` in Warp emits raw `%output`/`%layout-change` text into a normal pane. Block boundaries come only from `ansi::PromptMarker` injected by Warp's shell integration (`writeable_pty/bootstrap_file/`), which tmux-spawned shells do not carry by default.

## Proposed changes

Two layers. The **pure protocol core** (octal escaping, layout parse/emit, `%`-notification parsing + `%begin/%end` reply assembly, and command serialization — no app dependencies) lives in `crates/warp_terminal/src/tmux/`, where it is unit-tested in isolation. The **app-integration layer** (`TmuxTerminalManager`, the fan-out event loop, controller, window opener, and UI hooks) lives in a new `app/src/terminal/tmux_tty/` module (mirroring `remote_tty/`) that consumes `warp_terminal::tmux`. Everything is feature-flagged behind `tmux_control_mode` (default off during rollout).

### 1. `gateway.rs` — streaming, byte-oriented `TmuxGateway`
Tokenize and un-escape on `&[u8]` (never `String`: `%output` carries raw non-UTF-8; only bytes `<0x20` and `\` are octal-escaped). Parse `%begin/%end/%error` (correlate replies to commands by number), `%output`, `%extended-output`, `%pause`/`%continue`, `%window-*`, `%layout-change`, `%session-*`, `%pane-mode-changed`, `%exit`. All writes to the control fd go through one serializer that **rejects bare/empty lines** (a bare newline detaches the session) and coalesces input.

### 2. `layout.rs` — `TmuxLayout`
Parse/emit the layout string `checksum,WxH,x,y{...}[...]` (`{}` = left-right split, `[]` = top-bottom, leaf = `WxH,x,y,paneid`; children sum to parent minus one divider). Recompute the 16-bit rotate-add checksum to emit layouts for `select-layout`. Convert to/from `BranchSnapshot`/`LeafSnapshot`.

### 3. `event_loop.rs` — fan-out reader, single wire-ordered apply
Read the diverted control fd, feed the gateway, and apply notifications and `%output` in **strict wire order**. Per-pane VT emulation (`ansi::Processor::parse_bytes` into that pane's `TerminalModel`) may run concurrently, but a `%layout-change`/`%pane-mode-changed` for pane N is a **barrier** against N's own later `%output`. Buffer `%output` for a not-yet-known pane until its manager exists, then flush in order. Enable `pause-after` at attach and honor `%pause`/`%continue` with bounded per-pane buffers.

### 4. `terminal_manager.rs` — `TmuxTerminalManager` (one per pane)
`impl TerminalManager` owning one `TerminalModel` via `create_terminal_model` (template: `mock_terminal_manager.rs`). Its `EventLoopSender`:
- Input → `send-keys -l`/`-H -t %N` (**literal/hex**, never key-name parsing, so `;` cannot inject commands), coalesced.
- Emulator-generated query replies (DSR/DA/cursor-position; collected as `response_seqs`) → `send-keys` too, or vim/htop hang.
- Resize split by kind: pane divider → `resize-pane -t %N -x -y`; window/attach → `refresh-client -C`.

### 5. `controller.rs` — registry, idempotent + echo-safe
Bidirectional maps `@window↔TabId`, `%pane↔(TmuxTerminalManager, PaneId)`, `$session↔WindowId`. Layout application is **idempotent** (diff-and-apply); Warp-initiated mutations are tagged by command number so their echoed `%window-add`/`%layout-change` reconcile to a no-op (no duplicate panes / resize oscillation).

### 6. `window_opener.rs` - bootstrap with honest reattach
Send an initial `refresh-client -C` (size tmux before enumerating), then `list-windows`/`list-panes -F`. Per pane: a plain-text pane is seeded with `capture-pane -p -e -J` (pinned to pane width); an **alt-screen/TUI/copy-mode pane** (`#{alternate_on}`, `#{pane_in_mode}`) is **not** text-seeded, because `capture-pane` cannot restore emulator mode/cursor state. Build the tab via `add_tab_with_pane_layout(PanesLayout::Snapshot(..))`.

**Forcing and verifying the TUI repaint (success criteria #9, #18).** An alt-screen pane is adopted "pending first paint"; its content comes from making the running app redraw itself, not from a snapshot. The trigger is a size change: `refresh-client -C <WxH>` at attach sets the client/pane size, and the tmux server sends `SIGWINCH` to the foreground process of any pane whose dimensions actually change, which full-screen apps (vim, htop, less) answer with a full repaint. When the restored size is unchanged (so no `SIGWINCH` would fire), Warp forces one by nudging the pane one column with `resize-pane` and immediately restoring it. The repaint arrives as ordinary `%output %N` and parses into pane N's `TerminalModel` like any live output; the pane's pending-first-paint marker is cleared on that first post-nudge `%output`, which is how the repaint is verified.

**Fallback if no repaint arrives.** Some apps ignore `SIGWINCH`, or are idle and not currently drawing. If no `%output` lands for a pending pane within a bounded window, Warp does not leave it blank: it falls back to a one-shot `capture-pane -p -e` snapshot of the visible screen (text+SGR only) and marks the pane "may be stale until next redraw", so the worst case is a static-but-correct screen instead of an empty pane. The pane still repaints normally on the app's next output or the user's next keystroke.

### 7. Trigger + UI
On the control-mode DCS (`ESC P 1000 p`), divert the raw stream at the fd/transport level to the gateway, **not** through the buffering DCS hook. A tab-bar `+` menu and command-palette entry offer New / Attach (from `tmux ls`); hand-typed `tmux -CC` surfaces an inline convert banner (pattern: `terminal/view/inline_banner`), no auto-flip. While the banner is pending the diverted stream is held and not rendered (no raw `%output`); **Convert** attaches natively, and dismissing detaches the control-mode client so the pane falls back to a normal shell with the session left running.

### 8. Resource bounds and malformed-stream teardown (local and SSH)
A control stream can originate from a remote `ssh host tmux -CC`, so it is untrusted input that drives UI/model allocation through panes, windows, layouts, titles, and output. The gateway enforces hard caps and treats any breach as a protocol error rather than a growable buffer:
- **Frame/line size:** each `%`-notification line is bounded (`MAX_CONTROL_LINE`); an over-long line is a protocol error, not an unbounded read.
- **Layout depth:** the layout parser is bounded by a recursion-depth cap (`MAX_LAYOUT_DEPTH = 128`) and returns `Malformed` instead of overflowing the stack on a pathological nested string.
- **Pane/window counts:** adopted panes and windows are capped (`MAX_PANES` / `MAX_WINDOWS`); a stream that exceeds them is rejected rather than allocating unbounded `TerminalModel`s and tabs.
- **Title length and content:** window/pane titles are truncated to a fixed maximum and content-sanitized before reaching the tab/pane UI: non-UTF-8 is replaced (lossy), and C0/C1 control bytes and bidi-override characters are stripped, so a remote title cannot spoof or corrupt UI surfaces.
- **Per-pane output:** buffered `%output` for a not-yet-registered pane and flow-control backlog are each bounded (256KiB/pane). Flow control (`%pause`/`%continue`, section 3) is the normal backpressure path and keeps a well-behaved pane under the cap. Exceeding it is treated as a desync, never a silent splice: rather than feeding truncated mid-sequence bytes into `TerminalModel` (which would corrupt SGR/cursor state), Warp discards that pane's pending buffer as a unit, marks the pane desynced, and reseeds from tmux ground truth (`capture-pane -p -e` for a text pane, or a forced repaint per section 6 for an alt-screen pane), surfacing a one-line "output truncated, resynced" notice. If the reseed fails or the cap is breached repeatedly, that pane/session tears down on the `%exit` path below instead of rendering from a corrupt buffer.
- **Malformed-stream teardown:** on any parse error, an oversized frame, a `%begin` without a matching `%end`, or a decode failure, the control session is torn down cleanly (panes closed with a surfaced message) on the same path as `%exit`, instead of continuing to allocate.

### 9. `scrollback.rs` - on-demand history backfill
Initial adoption seeds only the visible screen (section 6). Scrolling above the seed fetches older lines from tmux history on demand: Warp maps the requested scroll range to tmux line offsets and issues `capture-pane -p -e -J -t %N -S <start> -E <end>` (a negative `-S` counts back from the top of the visible region). Fetched lines are prepended into that pane's `TerminalModel` scrollback above the seeded region, with the seed/fetch boundary de-duplicated so no line is doubled or dropped (invariant #11). Fetches are chunked and bounded; requesting past tmux's `history-limit` returns nothing and renders as a clean top-of-history (no error, no infinite request loop). Alt-screen/TUI panes have no meaningful scrollback and are excluded.

### Concurrency / lock order
One global order: never acquire a `TerminalModel` lock while holding a `PaneGroup`/UI lock; apply layout/tab mutations via the UI thread's message queue, not inline from the reader thread.

## End-to-end flow (attach)

1. User clicks **Attach → "agents"**; Warp spawns `tmux -CC attach -t agents` as a byte source.
2. The parser sees `\033P1000p` and diverts the stream to `TmuxGateway`.
3. Gateway sends `refresh-client -C <client size>`, then `list-windows`/`list-panes -F '#{window_layout}#{alternate_on}...'`.
4. Per pane: text pane → `capture-pane` seed; TUI pane → live repaint. `add_tab_with_pane_layout(Snapshot)` builds native tabs + splits, one `TmuxTerminalManager` per pane.
5. Live: `%output %N` → (wire-ordered) `Processor::parse_bytes` → model N → view; `%layout-change` → idempotent diff-apply.
6. Keys: Warp binding? yes → Warp action; no → `send-keys -l -t %N`. Emulator DSR/DA reply → `send-keys`.
7. Native split/close/resize/new-tab → `split-window`/`kill-pane`/`resize-pane`/`new-window`.
8. Flood → `%pause %N` honored, other panes stay live, `%continue` after drain.

## Risks and mitigations

- **Reattach corrupts TUI/alt-screen panes if text-seeded.** `capture-pane` returns text+SGR only, not modes/cursor. Mitigation: TUI panes repaint live via a forced `SIGWINCH` (resize nudge) with a `capture-pane` static-snapshot fallback if no repaint arrives (section 6); only text panes restore contents (scopes success criteria #9, #18).
- **DCS lifetime.** The existing DCS hook buffers to `unhook()`; a control-mode DCS stays open for the session lifetime → OOM / renders nothing. Mitigation: divert at the fd/transport level; the origin pane's `TerminalModel` becomes a dead host.
- **Collision with pre-existing tmux code.** Resolved: there is no pre-existing control-mode parser to collide with. The only tmux code in the tree is the already-deprecated SSH-wrapper tombstones (`app/src/terminal/warpify/settings.rs`, `app/src/terminal/view/ssh_tmux_deprecation_banner.rs`) and OSC52 clipboard passthrough (`crates/warp_terminal/src/model/escape_sequences.rs:144 tmux_passthrough()`); neither parses the `%`-protocol or drives panes, and both are left untouched. The control-mode protocol core is therefore a single fresh, self-contained parser under `crates/warp_terminal/src/tmux/` (canonical, unit-tested in isolation), so exactly one control-mode parser and one set of protocol tests are canonical.
- **Hostile or corrupt control stream (especially over SSH).** An untrusted remote `tmux -CC` could try to exhaust memory or the stack via oversized frames, deeply nested layouts, or huge pane/window counts, or drive the UI with over-long titles. Mitigation: the hard caps and malformed-stream teardown in section 8 (frame/line size, layout depth 128, pane/window counts, title length, bounded per-pane buffers); a breach tears the session down instead of allocating.
- **Head-of-line blocking** (single control channel): flow control from the first render PR; never block the UI thread on a `%begin/%end` round trip.
- **Self-echo double-apply / oscillation:** idempotent diff-apply + command-number tagging.
- **Ordering:** single wire-ordered apply; a resize acts as a barrier for that pane's later output.
- **Non-UTF-8 / empty-line detach:** byte-oriented parser; serializer rejects bare newlines.
- **Lock inversion (model vs UI):** one documented lock order; layout mutations on the UI queue.
- **`window-size` side effects:** prefer per-window `refresh-client -C @win:WxH`; avoid mutating the persistent `window-size` option; render the gray-margin state rather than stretch.
- **Version skew:** detect `#{version}`; gate 3.2+ features (`%extended-output`, `%pause`, subscriptions); degrade with a surfaced notice.

## Testing and validation

- **Unit:** gateway framing/escaping incl. a lone `0x80`–`0xFF` byte in `%output`; a proof that no input sequence yields a bare newline on the control fd; layout parse+emit incl. `e123,204x53,0,0{102x53,0,0,1,101x53,103,0[101x26,103,0,2,101x26,103,27,3]}`; query-response routing; unknown-pane buffering; self-echo idempotency.
- **Resource bounds (section 8):** a deeply nested layout string returns `Malformed` instead of overflowing the stack; an over-long `%`-line and pane/window counts past the caps are rejected/truncated; an over-long or control-char/bidi-laden title is truncated and sanitized before reaching the UI; a per-pane buffer driven past its cap reseeds the pane (asserting no truncated mid-sequence bytes reach the model) and a repeated breach tears the pane down; a parse error or `%begin` without `%end` triggers the teardown path rather than unbounded allocation.
- **Scrollback backfill (section 9):** a transcript that seeds a screen then serves `capture-pane -S/-E` history ranges asserts fetched lines prepend correctly, the seed/fetch boundary has no duplicate or missing line, and a fetch past `history-limit` stops cleanly.
- **Fixture/transcript tests (CI-safe, no live tmux):** replay recorded `%`-streams through gateway+controller and assert the tab/split tree, pane contents, and the seed↔live-`%output` boundary (no duplicate/missing line). Mirrors `app/src/terminal/ref_tests/data/tmux_htop`. CI has no `tmux` binary, so these are the primary protocol tests.
- **Integration (`crates/integration/`, gated on tmux ≥3.2):** drive real `tmux -CC`; assert attach, `%layout-change` splits, write-back round-trips, reattach with a running TUI (repaint, no garble), flood-without-freeze, `%exit` teardown. Skips cleanly where tmux is absent.
- **SSH control mode (`ssh host tmux -CC`):** a fixture/transcript test replays an SSH-originated control stream (transport diverted at the fd, not a local PTY) and asserts the same tab/split/output invariants; an integration scenario gated on tmux ≥3.2 plus an SSH target asserts attach, command serialization over the SSH channel, and write-back round-trips, since transport diversion and serialization differ from a locally spawned tmux. Skips cleanly where no SSH target is configured.
- **CI (implementation PRs):** `./script/presubmit` (fmt + clippy `-D warnings` + nextest) green; a `CHANGELOG-NEW-FEATURE` line on the PRs that ship user-facing code; screen-recording proof. This spec-only PR carries `CHANGELOG-NONE`.

## Follow-ups

Native copy-mode/selection parity; affinity persistence; a session dashboard (list/kill/attach); nested tmux; Windows; block reconstruction when remote shell integration is detected.

## Delivery

Implemented as a stack of small PRs, each behind the `tmux_control_mode` flag and independently reviewable: (1) protocol core (gateway + layout parser, unit/fixture tested); (2) single-window render (stable, with flow control + attach resize); (3) multi-window + reattach; (4) bidirectional write-back; (5) scrollback + polish.
