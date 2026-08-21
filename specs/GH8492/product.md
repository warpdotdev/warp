# Product Spec: Native tmux integration (control mode)

**Issue:** [warpdotdev/warp#8492](https://github.com/warpdotdev/warp/issues/8492)
**Figma:** none provided

## Summary

Let Warp drive a tmux session through tmux **control mode** (`tmux -CC`) so tmux windows render as native Warp **tabs**, tmux panes as native **split panes**, and sessions persist across Warp restarts. A user starts or reattaches a session from a button; everything else feels like normal Warp. This implements the "Tmux control mode" item from the roadmap (#9233) and the long-standing request in this issue.

## Problem

Running tmux inside Warp today replaces Warp's UI with tmux's text TUI. Native tabs/splits, command search, and block affordances stop working, and `Ctrl-R` falls through to the shell. Developers who rely on tmux to run and persist multiple CLI agents must choose between Warp's UX and tmux's multiplexing/persistence. Control mode removes that tradeoff: tmux becomes a backend that drives Warp's native UI instead of painting a TUI.

## Goals

- tmux windows/panes appear as native Warp tabs/splits, driven live by tmux.
- Start a new tmux session, or reattach to a running one, from an explicit button.
- A tmux session keeps running when Warp closes; reattaching restores the same tabs/panes.
- Native pane actions (split, close, resize, new tab) control tmux; keystrokes reach the right pane.
- One slow/chatty pane never freezes the others.
- Works for local tmux and for `ssh host tmux -CC` typed by the user.

## Non-goals

- Native copy-mode selection parity (tmux copy-mode remains the fallback).
- Affinity persistence (remembering custom native regroupings across reattach).
- Nested tmux (`$TMUX` already set) and Windows tmux.
- Scrollback older than tmux's `history-limit`.
- Auto-hijacking a pane without a click.
- Pixel-identical restore of a mid-frame TUI on reattach (TUI panes repaint live instead).

## User experience

### Current behavior

Running tmux in Warp shows tmux's raw text TUI; Warp's native tabs/splits, command search, and keybindings are disabled inside it.

### Expected behavior

1. A **tmux button** on the tab-bar `+` control (and a command-palette entry) opens a menu: **New tmux session**, and **Attach to…** listing running local sessions with their window counts.
2. Choosing one renders that session natively: its windows are tabs, its panes are splits, content is live.
3. Typing `tmux -CC` (or `ssh box 'tmux -CC'`) by hand shows a small inline banner: "tmux control mode detected - open natively?" with a **Convert** button. While the banner is pending, Warp suppresses the raw control-mode stream, so the pane shows the banner and never raw `%output` gibberish (criterion #13). **Convert** renders it natively; dismissing the banner detaches the `-CC` client so the pane returns to a normal shell, with the tmux session left running. It never flips without a click.
4. Splitting/closing/resizing panes and adding/closing tabs with Warp's own UI is reflected in tmux, and changes made from another tmux client are reflected in Warp.
5. Closing Warp leaves the session running; reopening and reattaching restores it.

### Edge cases

- Empty session (no extra windows); a pane emitting a flood; detach vs. server exit; tmux < 3.2 (no flow control); another client attached at a different size (gray margin); a pane running a full-screen TUI (vim/htop) on reattach; the tmux server killed out from under the client.

## Success criteria (testable behavior invariants)

1. With the setting enabled, **New tmux session** opens a native tab backed by a `tmux -CC` session; `tmux ls` shows it.
2. **Attach to…** lists every running local session with its window count; selecting one renders all its windows as tabs and all panes as splits matching tmux's current layout.
3. Each tmux pane renders its live output; keystrokes typed into a focused pane reach that pane's process.
4. A tmux `%layout-change` (e.g. `split-window` from another client) updates the corresponding tab's splits within one refresh, with geometry matching the tmux layout string (no off-by-one gaps).
5. `%window-add` / `%window-close` / `%window-renamed` add / remove / rename the corresponding tab.
6. Splitting a pane in Warp runs `split-window`; closing a Warp pane runs `kill-pane`; both directions stay consistent.
7. Resizing a Warp split or window sends the size to tmux (`resize-pane` / `refresh-client -C`); tmux's resulting layout is reflected back.
8. Adding a Warp tab runs `new-window`; closing a tab runs `kill-window`.
9. Closing Warp (or detaching) leaves the session and its processes running; reattaching reproduces the same windows/panes. Plain-text panes restore their contents; panes running a full-screen/alt-screen app reattach **live and repaint** (not a frozen snapshot), with no corruption.
10. A pane emitting a large burst is paused/backpressured (`%pause` honored) without freezing other panes or dropping the connection; output resumes after drain (`%continue`).
11. Scrolling up shows history beyond the live buffer, fetched on demand, with no duplicated or missing line at the seed/live boundary; reaching `history-limit` stops cleanly.
12. tmux panes are plain scrolling terminals by default. Blocks are opt-in: they form only when the user has Warp's shell integration sourced inside their tmux shell; their absence is normal, not an error.
13. Hand-typed `tmux -CC` shows the convert banner and does not render raw `%output` text (the control stream is suppressed while pending); the pane only becomes native on **Convert**, and dismissing the banner detaches the client back to a normal shell.
14. On `%exit` (server exit / session kill), or on a malformed/oversized control stream (e.g. a hostile or corrupt remote `tmux -CC`), the native tabs/panes tear down cleanly with a clear message; Warp does not hang or allocate without bound.
15. On tmux < 3.2, the integration still renders and accepts input (basic mode), with flow-control-dependent behavior degraded and surfaced, not broken.
16. The whole feature is behind the **`terminal.tmux_control_mode`** setting (surfaced under Settings → Features, default **off** during rollout, backed by `FeatureFlag::TmuxControlMode`); disabling it returns to today's behavior.
17. A full-screen app that queries the terminal (cursor-position / device-attributes report) behaves correctly: the emulator's response reaches that pane's process, so the app does not hang.
18. On attach, each pane shows its correct current screen with no garbling, even when live output arrives while the pane is still being seeded.
19. Typing or pasting reaches the focused pane intact and can never accidentally detach or corrupt the session (input is sent literally; a bare newline is never written to the control channel).

## Validation

Each invariant is backed by a fixture/transcript test (CI-safe, no live tmux) or an integration test gated on tmux ≥3.2 availability. Manual proof: a screen recording of new/attach, split/resize/close round-tripping, reattach after quit (including a running TUI), and a flood-without-freeze demo.

## Open questions

- Preferred home for the button (tab-bar `+` menu vs. a dedicated affordance). The setting itself is resolved: `terminal.tmux_control_mode` under Settings → Features (see success criterion #16).
- Appetite for a later session **dashboard** (list/kill/attach across sessions) vs. the v1 menu.

(Resolved: the only pre-existing tmux code is the deprecated SSH-wrapper tombstones and OSC52 clipboard passthrough; neither parses control mode, so the control-mode protocol core is a single fresh parser with nothing to reconcile. See the tech spec.)
