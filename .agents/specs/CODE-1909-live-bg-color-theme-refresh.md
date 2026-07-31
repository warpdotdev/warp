# Live terminal background refresh for auto theme (CODE-1909)
Linear: https://linear.app/warpdotdev/issue/CODE-1909/live-detect-background-color-changes-and-refresh-theme

## Product
### Summary
When the headless TUI theme is set to `auto`, Warp keeps its resolved light/dark theme and background-blended surfaces synchronized with the host terminal’s current background color without requiring a restart.

### Behavior
1. With `appearance.theme = auto`, Warp probes the terminal’s default background using OSC 11 whenever the terminal gains focus. A focus-triggered probe is skipped while terminal input is already pending, so a changed background is reflected after the next successful quiet focus-gain probe.

2. A light/dark luminance change applies the corresponding Warp theme through `Appearance::set_theme`. Each existing terminal session then refreshes its terminal-model ANSI palette from the new theme so shell-command and terminal content use the new colors. A same-luminance RGB change keeps the current theme and terminal palette but redraws views so tinted and blended surfaces use the new background.

3. An unchanged RGB value is a no-op: no theme update and no redraw.

4. Explicit `light` and `dark` themes disable live probing and are never overridden by terminal background changes. `/theme auto` re-enables probing; `/theme light` and `/theme dark` disable it.

5. Missing OSC 11 replies do not change the current theme or cached background:
   - Three consecutive missing replies permanently stop probing for the current TUI session.
   - A successful reply before the cutoff resets the consecutive-miss count.
   - Selecting an explicit theme preserves the current miss count.
   - Once stopped, selecting `auto` again does not restart probing until a new TUI session.

6. Runtime probing runs on the terminal reader thread so it never competes with a second stdin reader. Rendered frames and OSC queries share a stdout mutex, and the reader checks downstream channel lifetimes at each loop boundary and immediately before writing a query.

7. On unsupported non-Unix terminals, the query writer emits nothing and the reply reader returns no background. Auto theme retains its startup resolution, and the normal missing-reply cutoff disables further focus-triggered queries for the session.

### Scope
- Headless TUI only (`crates/warp_tui` and the TUI runtime in `warpui_core`).
- No new user setting; behavior is inherent to `appearance.theme = auto`.
- Parsing terminal-emitted theme-change protocols such as DEC mode 2031 or DSR 997 is out of scope. Focus-triggered OSC 11 queries are the detection mechanism.

## Technical design
### Ownership and state
`TuiThemeSettings` is the authoritative source for the user’s `auto`, `light`, or `dark` selection. `Appearance` owns the resolved `WarpTheme` currently used by views. `TuiHostTerminalBackground` is the TUI-only WarpUI singleton that owns the detected host-terminal background and live-probe policy connecting those two models.

Its foreground-owned `TerminalBackgroundState` contains:
- `background: Option<ProbedRgb>` — the latest successful OSC 11 result.
- `TerminalBackgroundProbeBudget`:
  - `Available { consecutive_misses }`
  - `Exhausted`

The singleton shares only an `Arc<AtomicBool>` eligibility gate with the dedicated terminal reader thread. The foreground updates that gate from `TuiThemeSettings` and the probe budget; the reader checks it only after `FocusGained`. Background, miss-budget, and theme-selection state never cross threads.

Probe configuration is private to the singleton:
- Live reply deadline: 50 milliseconds.
- Consecutive-miss cutoff: 3.

### Startup
`TuiHostTerminalBackground::register`:
1. Runs the existing startup background probe before the TUI driver owns stdin.
2. Waits up to 100 milliseconds for the OSC 11 reply and DA1 sentinel. When no reply is available, luminance resolution falls back to `COLORFGBG`, then to the historical dark default when that is also unknown.
3. Stores the optional RGB result in `TerminalBackgroundState`.
4. Resolves the selected `TuiTheme`.
5. Registers the singleton and its foreground probe-result stream.
6. Constructs the runtime `TuiProbe`, which carries the boolean eligibility callback, query/reply callbacks, and result sender without exposing TUI theme policy to `warpui_core`.

`session::init` applies the returned theme and passes the probe to `spawn_tui_driver`.

### Runtime probe flow
1. The reader thread blocks on terminal input. When it receives `FocusGained`, it reads the singleton-owned atomic eligibility gate. An enabled probe runs if no other terminal input is pending; a disabled or absent probe is skipped.

2. After `FocusGained`, `event::poll(Duration::ZERO)` must report no pending input before probing starts.

3. The probe runs in two phases:
   - `write_terminal_background_query` holds the shared stdout mutex while writing and flushing OSC 11 plus the DA1 sentinel.
   - The mutex is released before `read_terminal_background_reply` waits for and parses the reply.

4. The result is sent to `TuiHostTerminalBackground` on the foreground executor. Before processing it, the singleton rereads `TuiThemeSettings`; this discards an in-flight probe result if the user switched to an explicit theme. After processing, it refreshes the atomic gate from the current selection and updated miss budget.

5. `TerminalBackgroundState::record_probe_result` returns one mutually exclusive `ProbeResultAction`:
   - `None` for a missing reply, unchanged RGB, or result received after the probe budget was exhausted.
   - `Repaint` for a changed RGB that retains the current light/dark theme.
   - `SetTheme(WarpTheme)` for a changed RGB that resolves to the other light/dark theme.

6. The reader checks both downstream channel receivers at each loop boundary and immediately before writing a query. If either is already closed, it exits without another query. A reader blocked in a terminal read exits at its next loop boundary or failed channel send.

### Theme selection
After `/theme` successfully persists the new `TuiTheme`, `TuiHostTerminalBackground::select_theme` refreshes the atomic eligibility gate and resolves the selection against the latest cached background. The caller then applies that resolved theme through `Appearance::set_theme`, which invalidates all views and emits `AppearanceEvent::ThemeChanged`. Each `TuiTerminalSessionView` subscribes to that event and replaces its `TerminalModel` color list with colors derived from the new `WarpTheme`, ensuring existing shell-command and terminal content transitions with the rest of the TUI. Selecting `auto` re-enables an available probe budget; an exhausted budget remains disabled.

### Rendering
`TuiUiBuilder::from_app` snapshots the latest background from `TuiHostTerminalBackground` alongside the current `Appearance` theme. Base-background calculations use that RGB snapshot, falling back to the resolved Warp theme background when the singleton or a detected terminal background is unavailable. A same-theme RGB change invalidates all views so new builders recompute pre-blended surfaces from the new snapshot.

### Relevant files
- `crates/warp_tui/src/terminal_background.rs:36` and `crates/warp_tui/src/terminal_background.rs:111` — foreground state, atomic eligibility gate, singleton, probe policy, and theme application.
- `crates/warp_tui/src/session.rs:121` — startup registration and driver wiring.
- `crates/warp_tui/src/terminal_session_view.rs:1240`, `crates/warp_tui/src/terminal_session_view.rs:1895`, and `crates/warp_tui/src/terminal_session_view.rs:3938` — appearance-event subscription, terminal-model palette refresh, and `/theme` lifecycle updates.
- `crates/warp_tui/src/tui_builder.rs:49` — singleton-owned background consumption.
- `crates/warpui_core/src/runtime/mod.rs:506` and `crates/warpui_core/src/runtime/mod.rs:601` — driver wiring, focus-trigger handling, quiet-input gate, stdout serialization, and result delivery.
- `crates/warpui_core/src/runtime/terminal_probe.rs:35` and `crates/warpui_core/src/runtime/terminal_probe.rs:41` — boolean probe registration, OSC 11 I/O, parsing, and luminance classification.

## Validation
Automated coverage verifies:
- Exact-background comparisons and `None`, `Repaint`, and `SetTheme` decisions.
- Eligibility for auto versus explicit themes; miss preservation; the three-miss cutoff; and successful-reply reset.
- Focus-reporting lifecycle and the requirement that only a quiet, enabled `FocusGained` event starts a live probe.
- The exact OSC 11 + DA1 query bytes, BEL/ST reply parsing, component scaling, DA1 detection, luminance classification, malformed replies, and the `COLORFGBG` classification heuristic.
- Selection of a detected terminal-background snapshot as the base background consumed by TUI blending recipes.
- Refreshing an existing terminal model’s foreground color when the `Appearance` theme changes.

The following runtime properties are enforced by `run_tui_input_reader` and `TuiScreen::draw` but are not isolated by dedicated unit tests:
- The stdout mutex is released before reply reading.
- Closed event or probe-result receivers suppress subsequent probe writes at the reader’s explicit closure checks.

Repository checks:
- `cargo fmt --all -- --check`
- Focused `warp_tui` terminal-appearance and builder tests.
- Focused `warpui_core` terminal-probe and general TUI runtime tests.
- Clippy for the touched crates under their normal feature sets.

Manual verification in an OSC 11-capable terminal:
1. In `auto`, switch dark → light → dark while the TUI is unfocused and confirm theme and blended surfaces update after refocusing the terminal.
2. Select an explicit theme and confirm terminal background changes no longer affect Warp.
3. Select `auto` again and confirm live refresh resumes unless the session has reached the missing-reply cutoff.