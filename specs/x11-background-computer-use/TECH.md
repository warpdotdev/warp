# Linux X11 Background Computer Use — TECH.md

Status: implemented on `daniel/x11-background-computer-use`; this spec describes the design as
shipped at commit `1af88ebc` and the remaining validation gaps.

## Context

Background computer use lets the agent drive one specific window — clicks, typing, scrolling,
per-window screenshots — without moving the user's cursor, stealing the user's keyboard focus,
or perturbing the user's modifier state. The cross-platform contract predates this work and is
platform-neutral:

- [`Target` @ 1af88ebc](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/lib.rs#L143) — per-action `Screen` vs `Window { window_id, pid }` targets; [`Options.background_enabled`](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/lib.rs#L445) forces the byte-identical legacy full-screen path when off.
- [`ActionResult`](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/lib.rs#L469) — returns a refreshed [`WindowInfo`](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/lib.rs#L184) list (so the model can pick targets) and [`CapturedWindow`](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/lib.rs#L198) metadata mapping window-local coordinates onto window screenshots.
- Capability gate: [`background_supported()` @ 1af88ebc](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/lib.rs#L87) (which also documents the per-platform limitations) feeds [`supports_background_computer_use` in the agent request settings](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/app/src/ai/agent/api/impl.rs#L104); execution is additionally gated by [`FeatureFlag::BackgroundComputerUse`](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/app/src/ai/blocklist/action_model/execute/request_computer_use.rs#L95).

macOS implements this by posting Quartz events directly to the owning process
([`PostTarget::Pid` / `CGEventPostToPid` @ 1af88ebc](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/mac/post.rs#L9)
plus private window-addressing fields), so it can drive fully covered windows with zero visible
side effects. X11 has no process- or window-addressed delivery that applications honor: the only
window-addressed primitive, `XSendEvent`, marks events `send_event=true`, which GTK, Qt,
Chromium, and WINE deliberately ignore. The X11 design therefore had to be built on genuinely
different primitives, and its trade-offs differ from macOS.

## Design

### Core mechanism: an MPX "agent seat"

X11 (XInput2 ≥ 2.0, any server since 2009, including Xvfb) supports multiple independent master
pointer/keyboard pairs (Multi-Pointer X). Each master pair has its own on-screen cursor and its
own keyboard focus, and the server creates matching XTEST slave devices for every pair. The X
server routes a client's *core* input requests — XTEST fake input, `WarpPointer`,
`QueryPointer`, `SetInputFocus` — through that client's "ClientPointer" master pair
(`PickPointer`/`PickKeyboard` in the server's `Xext/xtest.c`).

[`seat.rs::AgentSeat::new` @ 1af88ebc](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/linux/x11/seat.rs#L61)
exploits exactly that: it opens a dedicated connection, creates a private master pair named
`warp-agent-cu-<pid>-<seq>` via `XIChangeHierarchy(AddMaster)`, and points the connection's
ClientPointer at it via `XISetClientPointer`. The pre-existing XTEST mouse/keyboard code then
drives the agent seat completely unchanged. Consequences:

- Applications receive real, server-generated device input (`synthetic NO`) — full compatibility
  with every toolkit, unlike `XSendEvent`.
- The user's cursor position, keyboard focus, and modifier state are properties of the user's
  own master pair and are never touched. Agent typing follows the agent keyboard's focus, set
  per action with `XISetFocus`, even while the target window is covered.
- A second visible cursor exists while a window-targeted batch runs (the seat lives for the
  `Actor`'s lifetime, i.e. one tool call).

Rejected alternatives: `XSendEvent` synthetic events (silently ignored by major toolkits);
focus/pointer save-restore juggling on the user's seat (races with concurrent user input, not
actually background).

### Seat lifecycle and leak safety

Master devices are server-global and outlive the creating connection, so
[`seat.rs`](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/linux/x11/seat.rs)
treats leaks as a first-class concern:

- `Drop` removes the pair ([`impl Drop` @ L155](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/linux/x11/seat.rs#L155)); every construction failure path after `AddMaster` either removes the pair or unregisters it so the next creation reaps it.
- A process-local live-seat registry ([`live_seats` @ L177](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/linux/x11/seat.rs#L177)) reserves names before `AddMaster` so concurrent creations cannot reap an in-construction pair.
- [`remove_stale_seats` @ L271](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/linux/x11/seat.rs#L271) runs at every creation: same-pid seats absent from the registry (leaked by failed constructions) and seats of dead pids (crashed processes, parsed from the seat name) are removed. Foreign pid reuse can delay reaping until the reusing process exits — a bounded, accepted residual.
- The `use_computer` CLI returns `ExitCode` instead of calling `process::exit`, so `Drop` always
  runs on error exits.

### Per-target action routing

[`x11/mod.rs::perform_actions` @ 1af88ebc](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/linux/x11/mod.rs#L171):

- Prevalidates the batch (rejects the `window_id: 0` sentinel, macOS parity) and creates the
  seat up front so failures cannot half-apply a batch. `Screen` actions keep the legacy path on
  the user's core pointer/keyboard; with `background_enabled == false` behavior is identical to
  the pre-existing implementation.
- Window coordinates are window-local pixels, translated to root coordinates with bounds
  validation ([`windows.rs::window_local_to_root` @ L265](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/linux/x11/windows.rs#L265)) — an out-of-bounds point would otherwise land on an unrelated window.
- X11 delivers pointer events to the topmost window under the pointer, so clicks and scrolls
  first hit-test the target at the point ([`window_hit_at_point` @ L298](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/linux/x11/windows.rs#L298) walks the same root-down `TranslateCoordinates` chain the server uses for event picking, which also handles WM reparenting frames). A covered target is raised *without focus* via `ConfigureWindow(Above)` and re-checked with a 500 ms poll ([`ensure_window_clickable_at` @ L135](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/linux/x11/mod.rs#L135)); the action fails with an actionable error if the raise does not take effect.
- Key/type actions set the agent keyboard's focus to the target first; clicks do the same to
  mirror a real click's focus effect on the agent seat only.

### Window enumeration and screenshots

- [`windows.rs::enumerate_windows` @ L74](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/linux/x11/windows.rs#L74): EWMH `_NET_CLIENT_LIST_STACKING` (reversed to front-to-back) with `_NET_CLIENT_LIST` and a `QueryTree` fallback for WM-less servers (bare Xvfb in cloud environments); titles from `_NET_WM_NAME`/`WM_NAME`, pid from `_NET_WM_PID`, app name from `WM_CLASS`. On X11 the `pid` is informational — delivery is addressed by window id.
- [`screenshot.rs::take_window` @ L72](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/linux/x11/screenshot.rs#L72): Composite-extension capture ([`capture_via_composite` @ L161](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/linux/x11/screenshot.rs#L161): per-client `RedirectWindow(Automatic)` — auto-released at disconnect — plus `NameWindowPixmap`, honoring `border_width`) sees full contents of covered windows; falls back to direct `GetImage` on the window drawable. Native capture size is capped at [`MAX_WINDOW_CAPTURE_PIXELS` @ L18](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/linux/x11/screenshot.rs#L18) (32 Mi px, covers 8K) since the reply and RGB buffers are allocated at native size before downscaling limits apply.
- Capability probing: [`linux/mod.rs::background_supported` @ L34](https://github.com/warpdotdev/warp/blob/1af88ebc4f77304eecd17b3ff5d47e85301afa7a/crates/computer_use/src/linux/mod.rs#L34) requires X11 (not Wayland — the portal path has no per-window targeting) plus a `OnceLock`-cached XI 2.x probe.

## Testing and validation

Validated on a remote Linux agent against the pre-hardening tree (all passing under
Xvfb 1280x800, no WM):

1. Enumeration lists windows front-to-back with ids/pids; empty display yields an empty list.
2. Background click delivers `ButtonPress`/`Release` with exact window-local coordinates
   (verified via `xev`) while the user's core pointer position is unchanged.
3. Background typing delivers `KeyPress`/`Release`; typing into a *covered* xterm lands
   (verified via `cat > file` in the xterm).
4. Clicking a covered point auto-raises the target and succeeds; stacking order confirms.
5. Covered-window screenshots capture correct dimensions and content (no bleed-through from the
   covering window); region capture crops correctly.
6. Device hygiene: `xinput list` shows the seat (master pair + XTEST slaves) only during a run
   and is byte-identical to baseline afterwards.
7. Legacy regression: screen-targeted actions still move the core pointer.

Static checks: `cargo check -p computer_use`, `cargo clippy --all-targets -- -D warnings`, and
`cargo fmt --check` pass on Linux for the initial implementation; macOS host build unaffected.

Remaining gaps to close before/at rollout:

- The review-hardening commit (`1af88ebc`, error-path cleanup + CLI validation + capture cap)
  compiles on macOS but its Linux-only files have only been hand-reviewed; a Linux
  `cargo check`/clippy run (CI or a one-off remote agent) plus a re-run of the Xvfb matrix above
  is cheap and recommended.
- Real-WM desktops (Mutter/KWin/i3, with a compositor): raise redirection and click-to-focus
  side effects are documented but not yet exercised.
- Non-US layouts and XI2-native clients (keymap divergence, see Risks).

## Parallelization

None proposed: the implementation is complete on this branch, and the remaining work is a
single small remote validation job with no independent subtasks worth fanning out.

## Risks and mitigations

1. Under click-to-focus WMs, an agent *click* can trigger the WM's own core passive grab, which
   may raise the window and move the user's focus to it (typing has no such side effect).
   Documented on `background_supported`; WM-less cloud environments are unaffected.
2. A WM's focus-stealing prevention may deny the auto-raise; the action then fails after 500 ms
   with an error naming the covered point. Follow-up: EWMH `_NET_RESTACK_WINDOW` with a pager
   source is the sanctioned mechanism.
3. Keymap divergence: keycodes are resolved against the core keyboard map, which mainstream
   toolkits also use to interpret events, but the agent seat's XTEST slave carries a
   server-default keymap that XI2-native clients could consult; combined with the pre-existing
   Shift-only resolver, non-US layouts (AltGr/levels/groups/dead keys) are not fully supported —
   same as the legacy screen path.
4. Composite capture of regions covered *before* redirection may be stale until the app
   repaints; agent interactions trigger repaints, and the direct-capture fallback degrades
   gracefully.
5. A batch that presses on one target and releases on another can strand held input on the
   first seat. The macOS implementation has the same semantic; in practice the model emits
   down/up pairs against one target per call.
6. Hit-test-then-click is not atomic; another client can restack between the check and the
   press. Accepted: the same race applies to a human click, and serializing via `GrabServer`
   would freeze all clients.

## Follow-ups

- XKB-aware key resolution (AltGr/levels/groups) and cloning the user's keymap onto the agent
  keyboard.
- `_NET_RESTACK_WINDOW`-based raising for WM desktops.
- Scanline-stride/byte-order-aware image conversion in the (pre-existing) X11 image converter.
- Cursor-position reporting for keyboard-only window batches currently falls back to the user's
  core pointer position (cosmetic).
- Optionally reject target switches while buttons/keys are held (risk 5).
