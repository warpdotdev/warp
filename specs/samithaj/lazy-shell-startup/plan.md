# Plan: don't start a restored tab's shell until it is opened

**Status:** v3 — **implemented** in `bf5fd8a65`, behind `FeatureFlag::LazyShellStartup`.
v1 was rejected in review; its central architectural premise was wrong (§7). v2 was discovery only.
**Related:** [`survey.md`](./survey.md) (the reach-through audit that gated this),
[`../resume-project-task/plan.md`](../resume-project-task/plan.md) (the project rail that made the
lag visible), [#9416](https://github.com/warpdotdev/warp/issues/9416) (session/PTY ownership).

---

## 1. The problem, observed

Restoring a window with ~50 tabs spawns ~50 shells at once. Measured, not estimated: the stable
WarpOss install with 49 restored tabs holds **49 login shells** as descendants of its
terminal-server. Symptoms, all one cause:

- Startup is visibly laggy; panes show *"Seems like your shell is taking a while to start…"*.
- Work is wasted: most restored tabs are never touched, yet each pays a full shell startup — with a
  heavy `zsh`/`starship`/`pyenv` profile that is seconds of CPU each.

## 2. Goal

A restored tab does not spawn its shell until the user opens it. The rail, the tab bar, and session
restoration behave exactly as they do now — including across a second save/restore cycle.

## 3. Non-goals

- Keeping shells *alive* across relaunch ([#9416](https://github.com/warpdotdev/warp/issues/9416)).
- Changing behaviour for newly-created tabs.
- Lazy-loading block history. Restored blocks already render from persistence; only the PTY defers.

---

## 4. Prerequisites (shipped separately, before any deferral)

| Commit | What it did | Why deferral needs it |
|---|---|---|
| `4354e20ca` | `TerminalPane` retains its restored directory; `PaneGroup::session_path` falls back to it | A tab whose shell never starts would otherwise sit in the rail's "Other" bucket forever |
| `82b18af85` | `snapshot()` falls back per field via `preserved_on_save` for `cwd` / `shell_launch_data` | Saving a never-started pane would otherwise persist `None` and permanently lose its directory and shell — and re-save that loss as the new truth each cycle |

Both were real bugs already, independent of deferral: quitting during the startup window already
triggered them. Deferral turns that window from *seconds* into *forever*, which is what made them
blockers rather than edge cases.

## 5. The key finding: the seam already existed

v2 assumed the hard part was building "a surface without a PTY". It isn't — one already exists on
every launch, for a few milliseconds.

`LocalTtyTerminalManager::create_model_with_manager`
(`app/src/terminal/local_tty/terminal_manager.rs`) builds everything **synchronously** — channels,
the `TerminalModel` *with its restored blocks*, the `TerminalView` surface, the manager struct — and
only *schedules* the shell, last:

```rust
let terminal_manager_model = ctx.add_model(|ctx| {
    ctx.spawn(async move { /* resolve shell */ },
              move |mgr, shell_starter_source, ctx| on_shell_determined(mgr, …));
    terminal_manager
});
```

`on_shell_determined` → `create_pty` → `Pty::new` → fork/exec.

Three supporting facts:

- The view renders restored blocks straight from the model, before any shell exists.
- `ShellLaunchState::DeterminingShell` already models "shell not resolved yet", and is constructed
  *before* the spawn with a non-empty `display_name` — so a deferred pane has a shell name to show.
- Input sent before the PTY exists is already buffered on the `mio_channel` and drained when the
  event loop starts (`enqueue_init_script` relies on this today).

**Consequence: the manager stays non-optional.** `PaneStack::AssociatedData`, the `Option`-ing of
`child_data`, and the drop-order tuple are all untouched — avoiding the 6 write-sites and 2
read-sites that v2's "optional manager slot" would have required. All 7 surveyed call sites keep
working unchanged.

## 6. What was built

**Defer the spawn, not the construction.** `DeferredShellStart` holds the spawn's inputs
(`startup_directory`, `env_vars`, banner handle, `ShellStartupResources`, shell-starter/WSL name) in
an `Option` field on `TerminalManager`. `ShellStartupResources` is not `Clone`, so the payload must
be *moved* — which is what makes idempotency structural:

```rust
pub(crate) fn ensure_shell_started(&mut self, ctx: &mut ModelContext<Box<dyn TerminalManagerTrait>>) {
    let Some(deferred) = self.deferred_shell_start.take() else { return };  // second call no-ops
    Self::spawn_shell_start(deferred, ctx);
}
```

`Option::take` makes "exactly once" a property of the shape, not a flag a caller can forget. That
matters because the triggers are independent — any of them may be the first.

Exposed on `TerminalManagerTrait` with a **default no-op**, so remote and mock managers need no
change and callers need no downcast.

**Only the restore path defers.** `create_session` gained a `defer_shell_start: bool`; the
`LeafContents::Terminal` restore arm passes `FeatureFlag::LazyShellStartup.is_enabled()`, and the
four other call sites pass `false`. New tabs, splits, and panes with no local shell of their own
(cloud, shared-session/conversation viewers, ambient and child agents) are unchanged.

### 6a. Triggers

| Site | Action | Why |
|---|---|---|
| `Workspace::focus_active_tab` | **start** | A tab became the active tab. Restoration's single activation (`workspace/view.rs:4098` → `activate_tab_internal`) goes through here, so the front tab starts exactly one shell with no special case |
| `PaneGroup::focus_pane` | **start** | Focus moved to a pane *by id* — clicking a split, and every id-based focus helper. Splits the user has not touched stay deferred until clicked |
| `TerminalPane::focus` (survey site 1) | **skip** | Looks like the right trigger and is not — see §6c |
| `PaneGroup::send_sync_event_to_session` (survey §4b) | **start** | Synchronized input is opt-in *user input*; silently dropping keystrokes would be wrong. Bounded by how many panes the user chose to sync |
| `send_prompt_change_bindkey_to_all_sessions` (site 2) | **skip** | A settings push, not input. Starting 50 shells to deliver a bindkey defeats the feature entirely |
| `close_pane_with_confirmation` (site 4) | **skip** | A pane with no shell trivially has no long-running command to confirm. Block cleanup goes through `delete_blocks`, which uses `self.uuid` and never touches the manager |
| `load_data_into_transcript_viewer` (site 3) | unaffected | Viewer panes have no local shell today |
| attach-execution-session (site 6) | unaffected | Already logs and bails when there is no manager |

**`active_session_terminal_model` (site 5) needs no trigger** — v2's table listed it as one. Every
caller reaches it through `Workspace::active_tab_pane_group`, i.e. the focused tab, which `focus`
has already started. The single exception, `active_session_ps1_grid_info`
(`workspace/view.rs:18796`), walks *all* tabs as a read-only `find_map` fallback and correctly skips
a pane with no prompt grid yet. Starting shells there would be precisely wrong.

### 6c. The trigger that looked right and wasn't

The first implementation put the start on `TerminalPane::focus`, reasoning that focusing a pane is
what "opening a tab" means. **Measured: 46 of 48 restored tabs still spawned a shell** — against a
baseline of 50, essentially no change.

`PaneGroup::new_internal` focuses every pane group *as it constructs it*, behind
`DragTabsToWindows` — which lives in `RELEASE_FLAGS`, so it is on everywhere. Restoring 49 tabs
constructs 49 pane groups, each of which focuses itself, each of which started its shell. Deferral
was working exactly as designed and then immediately undone, one tab at a time.

The fix is to hang the start off paths that only run when a user opens something (§6a). Neither is
reachable from `new_internal`, which calls `PaneGroup::focus` directly.

Worth recording because **no static check could have caught this**: it compiled, passed clippy, read
correctly, and matched the survey. The call graph was right — `focus` really is reached on tab
activation. What was wrong was the assumption that it is reached *only* then. Only the shell count
found it, which is the argument for measuring rather than reasoning about a feature whose entire
purpose is a runtime resource count.

### 6b. The label fix deferral forced

`display_working_directory` reads a prompt chip or `pwd()` — **both require a live shell**. A
deferred pane returned `None`, so `tab_info_text`'s `WorkingDirectory` branch fell through
`rail_task_label`'s chain all the way to the shell name: ~50 tabs and rail rows reading `zsh`
instead of their folders. It now falls back to the restored directory
(`tab_title.rs::restored_working_directory`), mirroring what `PaneGroup::session_path` already does
for project attribution.

The other label sources were already safe: `AgentSession` and `UserInstruction` come from the
durable handle store and restored blocks, not from a live shell.

## 7. What was wrong in v1

Recorded so the error isn't repeated:

1. **False premise.** v1 asserted `TerminalPane` holds a non-optional manager and proposed putting a
   `SessionState` enum there. It holds no manager — the manager is `PaneStack` associated data for
   the backing pane view. A doc comment was misread as a field declaration.
2. **"Activation as the only trigger" is not a safe first step** — other operations address
   background panes directly, notably synchronized/broadcast input. Resolved in §6a.
3. **Snapshot losslessness was filed as "open question 5"**, i.e. optional. It was a blocker; see §4.
4. **"Mechanical after the survey"** understated the lifecycle split — which then turned out to be
   *smaller* than v2 feared, for a different reason (§5).

## 8. Verification

**Static** — `cargo clippy -p warp --all-targets` clean; touched-area nextest green (35 tests).
`--all-features` surfaces three pre-existing `collapsible_if` errors in `warp_completer`, a crate
this change does not touch.

**Not unit-testable, and deliberately not faked.** `TerminalSurface` has two implementors, both
heavy (`TerminalView`, `TuiTerminalSessionView`), and `TerminalView::new_for_test` bypasses
`create_model` entirely — it fakes the `InitShell` lifecycle. So the deferral branch cannot be
reached from a unit test without spawning a real PTY on the eager side. The only remaining seam is
`Option::take`, and a test asserting that `take()` empties an `Option` verifies nothing. Those
assertions moved to the e2e run below, where they are directly observable. `preserved_on_save`
(§4) *is* unit-tested, and its first case is the one every deferred pane takes on every save.

**End to end**, in `WarpOss Dev` (its own profile, 49 restored tabs). Run
`./script/count-session-shells [bundle]`, which walks the process tree down from *that bundle's*
`terminal-server` — global `pgrep zsh` is useless here, since the machine runs ~110 zsh processes
belonging to other apps and two Warp builds are usually up at once. Take the reading 30s+ after the
window paints; the tree is still being built before that.

| Check | Expected |
|---|---|
| Baseline: stable WarpOss, eager | **49 shells** (measured) |
| Dev app shortly after launch | **1** — the active tab only |
| Click an unopened tab | count increases by exactly **1** |
| Click the same tab again | count **unchanged** (idempotency) |
| Toggle the Warp-prompt/PS1 setting | count **unchanged** (broadcast skips) |
| Rail on the first frame | every tab under its real project, with no shells at all |
| Quit and relaunch | tabs still in the right directories with the right shells |

**The shell count discriminates three ways, so don't read "low number = success":** ~49 means
deferral did not take; **0 means the initially-active tab never started** — a dead front tab, §6a
broken; 1 is correct. And 0 with **zero `Creating terminal model with N restored blocks` lines** in
the log means neither: that log fires *before* the deferral branch, so its absence means restoration
never ran at all. Check it before concluding anything about deferral.

### 8a. The e2e run is blocked on macOS security prompts, not on this change

`script/install-warposs-dev` re-signs the bundle, and `security find-identity -p codesigning`
reports **0 valid identities** on this machine, so it falls back to `--sign -` (ad-hoc). An ad-hoc
signature is content-derived: every reinstall is a brand-new identity to macOS, so neither the TCC
grant nor the Keychain ACL carries over. Two synchronous blocking calls during startup then wait on
a human:

| Where | Stack | Prompt |
|---|---|---|
| `warp::run_internal` → `warp_assets::Assets::get` → `rust_embed_utils::read_file_from_fs` → `open()` | blocked in `libsystem_kernel` | Documents-folder access. Debug builds read assets from `app/assets` — under `~/Documents`, which is TCC-protected — instead of embedding them |
| `initialize_app` → `TemplatableMCPServerManager::new` → `mcp::oauth::load_credentials_from_secure_storage` → `SecKeychainFindGenericPassword` → `mach_msg` | blocked in `libsystem_kernel` | Keychain authorization for the stored MCP OAuth credentials |

Symptom is identical in both cases and easy to misread as a deadlock in app code: **no window, 0%
CPU, ~36 MB RSS, and the log stopping mid-startup.** `sample <pid>` is what distinguishes them —
both leaves are kernel syscalls inside Security/`open`, not a `FairMutex`.

Neither is reachable from this change: the app never gets as far as session restoration. The fix is
a click (Allow), or a stable signing identity so the grants persist — see the ad-hoc warning now
printed by `script/install-warposs-dev`.

Launching the binary from a shell instead of via `open` clears the *first* prompt (TCC attribution
is inherited from the parent) but not the second — the Keychain ACL is keyed on the code identity
itself. Also note the data profile follows the **bundle id**, so
`target/debug/.../WarpOss.app` and `/Applications/WarpOssDev.app` read different profiles; only the
latter holds the 49 restored tabs.

**Status: OBSERVED, 2026-08-05.** After the TCC/Keychain grants were given, a 48-model restore
measured `shells=1` at t+30s and t+60s (stable), against the eager baseline of 49-50. Note the fix
that made this true was NOT the original trigger placement: two earlier attempts each measured 46 —
see §6c. Remaining interactive checks (click-to-start +1, idempotent second click, settings
broadcast starting none) are pending user confirmation in the running build.

There is **no `FIRST_FRAME_DRAWN` instrumentation** in this tree — v2's verification section named a
marker that does not exist. The shell count is the better metric anyway: it measures the *cause* of
the lag rather than one of its effects.

## 9. Risks

| Risk | Status |
|---|---|
| Silent snapshot degradation | Addressed by `82b18af85` before deferral was enabled (§4) |
| A deferred tab's label regresses | **Found and fixed** — this was the plan's stated top risk and it was real (§6b) |
| Typing into a tab before noticing it has no prompt | Input is already buffered on the `mio_channel` and drained at start (§5) |
| "Synchronize All Panes in All Tabs" then typing starts every deferred pane at once | Accepted: an explicit user action, and a stated consequence rather than a surprise |
| A trigger was missed | Flag-gated, so revert is one setting. Survey covered the 7 reach-throughs plus sync input |
| Cloud/shared-session/ambient panes | Pass `false`; unchanged |
