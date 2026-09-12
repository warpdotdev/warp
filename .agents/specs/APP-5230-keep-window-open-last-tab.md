# Spec: Configurable last-tab window closing

Linear: [APP-5230](https://linear.app/warpdotdev/issue/APP-5230/setting-keep-window-open-when-closing-the-last-tab)

Baseline: `warpdotdev/warp` at `5688e06d9fd7a9b1530d28da1c1e2b295c37602e`

## Product

### Summary

Warp always closes a desktop window when its final tab is closed. Add a global setting that preserves this behavior by default but lets a user explicitly close the final tab while keeping the window open. The resulting window has no tabs, a visually blank content region, a persistent new-session control, and a faded keyboard hint in the tab bar.

### Key design choices

1. The new **Close window when all tabs are closed** setting defaults on, is globally persisted and synced, and appears in **Settings > Features > General** directly below **Quit when all windows are closed** where that macOS-only setting is present.
2. When the setting is off, explicitly closing the final tab produces a real zero-tab workspace rather than a placeholder tab or an automatically created session. `Workspace` represents that state with no active-tab index or active pane group, never with index `0` as a sentinel.
3. Last-tab drag is intentionally asymmetric with last-tab close: successfully dragging the only tab into another window still closes the source window, regardless of the setting, because the source window itself is the floating drag preview and restoring it empty would be visually surprising.
4. This is an active-tab invariant migration, not primarily a settings change. The baseline has 267 non-test source locations across 159 functions that participate in the invariant. The true zero-tab variant is therefore sized **L (5)**. [APP-5242](https://linear.app/warpdotdev/issue/APP-5242/alternate-approach-open-a-fresh-tab-instead-of-an-empty-pane-when-the) preserves the existing invariant by opening a fresh tab and is materially smaller.

### Behavior

1. **Default behavior is unchanged in a close-capable host.** With **Close window when all tabs are closed** on, explicitly closing the final tab closes its window using the existing window-close and quit-warning behavior.
2. **The opt-in behavior removes the final tab without closing the window.** With the setting off, explicitly closing the final tab leaves the same window open with zero tabs and a visually blank main content region.
   - The replacement invariant is: **`active_tab_index` is `Some(i)` exactly when `tabs` is non-empty and `i < tabs.len()`; it is `None` exactly when `tabs` is empty. An empty workspace has no active `PaneGroup`.**
   - No code may treat index `0`, the closed tab, or a hidden placeholder as active while `tabs` is empty.
3. **The zero-tab tab bar remains actionable.**
   - The tab presentation remains visible with its new-session `+` control, but no active-tab indicator or tab slot is rendered.
   - It contains faded text reading **Press ⌘+T to create a new session** on macOS. On other platforms or with a customized binding, the displayed shortcut uses Warp's resolved new-tab keybinding while retaining the same sentence.
   - In horizontal-tabs mode, the hint occupies the empty tab-strip area immediately before the `+` control.
   - In vertical-tabs mode, the hint and `+` appear in the otherwise empty vertical tab presentation. If the vertical-tabs panel is collapsed, the top header presents a compact zero-tab recovery strip containing both until a tab is created.
   - The hint is single-line and lower emphasis than normal tab text. At narrow widths, it ellipsizes and then hides before displacing the `+`, window controls, or existing right-side controls.
4. **The window remains a window, not a launcher.** Title bar, window controls, tab chrome, Settings access, command palette, menus, and existing sidebars remain rendered. Tab- or pane-dependent controls are disabled or show their existing no-active-content state; no control may retain or dereference the closed pane group. The window title falls back to Warp's app-level title rather than retaining the closed tab's title. The blank main content region contains no message, button, terminal, launcher, or placeholder tab.
5. **All standard new-session entry points recover a zero-tab window.** The tab-bar `+`, the resolved New Tab shortcut (⌘T by default on macOS / Ctrl+T by default elsewhere), File-menu New Tab, Command Palette New Tab, and the new-session menu create the first real tab at index `0`, focus it, and restore normal workspace behavior. The keyboard hint is informational, not a separate button.
6. **Normal tab-close safety applies when the window will remain open.** Closing the final tab into the zero-tab state follows the same shared-session, long-running-process, and unsaved-code confirmation paths as closing any other tab. Cancellation leaves the tab and window unchanged. The final tab is eligible for the normal Undo Close behavior.
7. **Window-close safety remains unchanged when the window will close.** When the setting is on in a close-capable host, the final-tab path continues to defer to the existing window-close confirmation rather than presenting both a tab-close and a window-close confirmation.
8. **Explicit Close Window and Quit are unchanged.** They continue to close/quit and warn according to existing settings whether the window has tabs or is empty. Transitioning to zero tabs does not itself count as closing a window and must not trigger **Quit when all windows are closed**.
9. **Hosts without `ContextFlag::CloseWindow` can reach zero tabs.** Closing their final tab must no longer be a no-op. Because the host cannot close its containing window, final-tab close produces the zero-tab state regardless of the setting's value. The last-tab close affordance is available while a tab exists, and Close Window remains unavailable.
10. **Dragging the final tab remains window-closing.** If the only tab is successfully dragged or moved into another existing window, the target receives the tab and the source window closes, with both setting values and in `CloseWindow`-disabled contexts where the host supports the drag operation. A cancelled drag returns to the original one-tab state. The source must not flash or reappear as an empty window.
11. **Empty windows are not restored.** The preference persists and syncs, but a zero-tab window is omitted from session persistence. Relaunching Warp creates/restores the normal initial session instead of reconstructing an empty window.
12. **The setting is global.** It applies to all windows and has no per-window override.

### Out of scope

- Customizable empty-state copy, actions, artwork, or content.
- A per-window variant of the setting.
- Changes to explicit Close Window, Quit, **Quit when all windows are closed**, or their warning policy.
- Changing the successful single-tab cross-window drag workflow to preserve an empty source window.
- A tab-drag redesign or a separate floating preview window.
- Documentation-repository changes.

## Tech

### Current context

- `GeneralSettings` defines globally synced GUI lifecycle preferences including quit warnings, **Quit when all windows are closed**, and session restoration in `app/src/terminal/general_settings.rs:8` at the baseline commit.
- The General section conditionally inserts `QuitWhenAllWindowsClosedWidget`, and the widget dispatches a `FeaturesPageAction`, in `app/src/settings_view/features_page.rs:2774` and `app/src/settings_view/features_page.rs:4927`.
- `Workspace::remove_tab` treats `tabs.len() == 1` as a request to close the window and never removes the final `TabData`. `Workspace::close_tab` also makes a single tab a no-op when `ContextFlag::CloseWindow` is unavailable and unconditionally skips tab-level confirmation for the final tab in `app/src/workspace/view.rs:12067` and `app/src/workspace/view.rs:12377`.
- The Close Active Tab binding is available only for `Workspace_CloseWindow` or `Workspace_MultipleTabs`, so a single tab cannot be closed in a `CloseWindow`-disabled host (`app/src/workspace/mod.rs:1087`).
- The workspace currently asserts that zero tabs are invalid and unconditionally dereferences the active pane group during keymap-context construction and rendering (`app/src/workspace/view.rs:26369`, `app/src/workspace/view.rs:22026`, and `app/src/workspace/view.rs:26542`).
- New-tab insertion already has explicit empty-workspace branches that insert at index `0`, which can be retained and expanded (`app/src/workspace/view.rs:12802`).
- App-state collection already omits a window whose snapshot contains no tabs, matching the requested non-restoration behavior without a persistence schema change (`app/src/app_state.rs:385`).
- A single-tab cross-window drag uses the source window as its preview and closes it after handoff. That existing behavior should remain separate from the explicit tab-close path (`app/src/workspace/view.rs:28241` and `app/src/workspace/cross_window_tab_drag.rs:1225`).

### Replacement invariant and call-site audit

The new invariant is:

> `Workspace.active_tab_index: Option<usize>` is `Some(i)` exactly when `Workspace.tabs` is non-empty and `i < tabs.len()`. It is `None` exactly when `tabs.is_empty()`. `active_tab_pane_group()` likewise returns `Option<&ViewHandle<PaneGroup>>`; there is no sentinel index and no retained active pane group in the empty state.

This representation is deliberate. Keeping `active_tab_index: usize` at `0` while `tabs` is empty would allow existing code to compile while preserving phantom-active-tab arithmetic and stale-pane bugs. Using `Option` turns the compiler into the migration checklist.

A baseline syntactic audit of non-test Rust code found **267 unique source locations across 159 functions in 17 files** participating in the workspace active-tab invariant:

- **175 calls in 118 functions across 13 files** use `Workspace::active_tab_pane_group()`, which currently ends in `.expect("Active tab index entry should exist")`. If any unguarded call executes at zero tabs, it panics.
- **87 direct reads or writes in 45 functions** use `Workspace.active_tab_index` in `app/src/workspace/view.rs` and `app/src/workspace/view/tab_grouping.rs`. These include selection, grouping, movement, persistence, render state, and index arithmetic.
- **9 Workspace `active_tab_index()` getter calls in 6 functions across 5 files** expose the assumed index to snapshots, file-tree events, and Local Control.
- Four lines overlap the categories above, yielding 267 unique locations rather than 271 summed occurrences.
- The count is a migration inventory, not a claim that all 267 locations are independently reachable crashes. Some sites already guard `tabs.is_empty()` or insert a tab before accessing it. Changing the field and accessors to `Option` nevertheless obligates every site to choose its zero-tab behavior explicitly.

The 175 panicking accessor calls classify by owning behavior:

- **Render/keymap hot paths — 17 calls in 11 functions.** `Workspace::keymap_context` has four panicking accesses; `render_banner_and_active_tab` directly indexes `tabs[active_tab_index]` before rendering active content; `render`, `render_panels`, `render_tab_bar_contents`, `tab_bar_mode`, and the WASM render helpers also assume an active pane. Zero tabs cannot currently survive one normal UI/context pass. **Required behavior:** branch on `None` before active content, paint the blank content slot, publish `Workspace_NoTabs`, omit active-tab flags, and render only global chrome and zero-tab recovery controls.
- **Event/state/cache transitions — 37 calls in 21 functions.** `set_active_tab_index`, `update_active_session`, `process_updated_sync_state`, window/settings/file-tree event handlers, and panel reconciliation update active-pane-derived models. **Required behavior:** clearing the final tab must atomically clear active session/focus, synchronized-input membership, left/right panel pane-group handles, tab rename/selection state, feature-intro ownership, and other active-pane caches. A late event from the removed pane group is ignored by identity; a window-global event runs without manufacturing an active pane.
- **Active-content actions and mutations — 86 calls in 57 functions.** These cover pane focus, splits, code/notebook/workflow opens, Agent actions, sharing, panel actions, environment creation, and `handle_action`. **Required behavior:** actions that require existing tab content are unavailable under `Workspace_NoTabs` and their handlers are idempotent no-ops if dispatched anyway. Actions whose purpose is to create top-level content first create a tab at index `0`, establish `Some(0)`, then continue. Local Control mutations targeting `active` return the existing structured `MissingTarget`/state-conflict error rather than acting on index `0`.
- **Query/metadata helpers — 24 calls in 23 functions.** These include active-session lookups, selected text, URI terminal resolution, conversation navigation, startup-directory lookup, Local Control metadata, and debug data. **Required behavior:** return `None`, an empty collection, or a response with no active target as appropriate. They must not panic and must not report the closed tab as active.
- **First-tab/startup/recovery — 11 calls in 6 functions.** New-session and workspace configuration paths often consult active content or assume activation has already completed. **Required behavior:** before the first insertion they use default shell/profile/startup-directory behavior and no inherited group/color/pane state; after insertion they set `Some(0)` and run the ordinary activation/focus/panel pipeline exactly once.

The 18 panicking accessor calls outside `app/src/workspace/view.rs` are specifically:

- environment creation in `app/src/root_view.rs:1016`, `:1049`, `:3313`, and `:3358`;
- active-terminal resolution in `app/src/uri/mod.rs:1570`, `app/src/workspace/util.rs:325`, and `:377`;
- startup-directory and shell inheritance in `app/src/workspace/view/startup_directory.rs:21` and `:35`;
- WASM details-panel rendering/updating in `app/src/workspace/view/wasm_view.rs:129` and `:147`;
- MCP server log opening in `app/src/settings_view/mcp_servers/list_page.rs:624`;
- conversation selection/navigation in `app/src/search/command_palette/conversations/data_source.rs:129` and `app/src/ai/conversation_navigation/mod.rs:160`;
- Local Control active-chain metadata in `app/src/local_control/handlers/metadata.rs:626`;
- onboarding active-terminal updates in `app/src/pane_group/pane/get_started_view.rs:381`;
- Agent SDK terminal creation in `app/src/ai/agent_sdk/driver/terminal.rs:216`; and
- editor Ctrl+C routing in `app/src/editor/view/mod.rs:4418`.

The most dangerous direct-index/arithmetic sites are:

- `render_banner_and_active_tab` indexes `self.tabs[self.active_tab_index]` and therefore panics immediately at zero tabs (`app/src/workspace/view.rs:22026`).
- `remove_tab` computes `self.tabs.len() - 1` after removal, which underflows when removing the final tab (`app/src/workspace/view.rs:12129`).
- `activate_prev_tab` computes `self.tabs.len() - 1` when the active index is `0`, which underflows at zero tabs (`app/src/workspace/view.rs:11952`).
- `new_tab_index_and_group` computes `active_tab_index + 1`; a `0` sentinel would propose index `1` for an empty workspace instead of the required index `0` (`app/src/workspace/view.rs:12779`).
- Four color/file-tree paths and active-content rendering directly index `tabs[active_tab_index]` (`app/src/workspace/view.rs:12863`, `:12875`, `:16626`, `:16627`, and `:22026`).
- Tab grouping and selection methods use the active index as an anchor. At zero tabs they must no-op and return empty selections rather than treating a sentinel `0` as a selected tab.
- Local Control's active tab resolver currently returns `workspace.active_tab_index()` even when no matching `TabEntry` could exist. Active selectors must resolve to `MissingTarget`; list operations must return an empty tab list.

### Design alternatives

- **Setting ownership — `GeneralSettings` selected over `TabSettings`.** `TabSettings` groups tab layout and placement, but this setting governs window lifecycle, must sit beside other window/app lifecycle settings, and needs the same global sync semantics as `GeneralSettings`. Define it in `GeneralSettings` with TOML path `general.close_window_when_all_tabs_closed`, default `true`, GUI surface, all-platform support, and global cloud sync respecting the user's sync preference.
- **Optional active-tab state selected over a sentinel or separate mode enum.** Keeping `usize` at `0` is rejected because it allows unsafe arithmetic and indexing to compile. A `WorkspaceContentState::{Empty, ActiveTab(usize)}` enum would encode the same state more explicitly but would still require the same 267-site migration and would duplicate `tabs.is_empty()`. `Option<usize>` gives the required compiler enforcement with the least new state.
- **True zero-tab workspace selected over a placeholder tab.** A fake tab or pane would preserve existing active-pane invariants but would leak into tab counts, navigation, persistence, menus, and drag logic and would violate the requested empty tab bar. Keep `Workspace.tabs` genuinely empty and make active-tab access explicit at zero tabs.
- **Blank content branch selected over a dedicated launcher or automatic replacement session.** A launcher is more discoverable and an automatic session minimizes code changes, but both conflict with the requester's visually blank pane. Recovery stays in the tab bar and standard new-session actions.
- **APP-5230 true zero tabs retained for this prototype, but APP-5242 is the lower-risk product alternative.** APP-5242 immediately opens a fresh tab, so `tabs` remains non-empty and the 267-site invariant migration is unnecessary. It can reuse the existing empty-workspace/new-session path and is sized S (2) by its ticket. Its tradeoff is semantic: closing the final tab does not yield the explicitly requested blank, tabless reset state; it creates a new session and inherits new-session profile/directory behavior. If hands-on comparison shows that a fresh tab satisfies the user need, APP-5242 should be preferred on engineering risk and scope.
- **Explicit close and drag remain separate.** Initially, uniform “last tab removed” behavior was considered. Investigation showed that a single-tab drag moves the source window as its preview. The requester deliberately chose to keep the existing source-window close after successful drag rather than snap an empty source window back to its pre-drag bounds.
- **No feature flag.** The default-on setting itself protects existing behavior, and the off state is the requested opt-in. A second rollout control would duplicate that gate without reducing the zero-tab implementation work.
- **Tab-bar hint selected over content-area copy.** A centered content hint was recommended for discoverability, but the requester explicitly chose the tab bar. The implementation must preserve that placement and degrade the hint before essential controls at narrow widths.

### Proposed changes

#### 1. Add the global setting and its discoverability surfaces

- Add `close_window_when_all_tabs_closed` to `GeneralSettings` with:
  - type `bool`;
  - default `true`;
  - `SupportedPlatforms::ALL`;
  - `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`;
  - GUI surface;
  - public TOML path `general.close_window_when_all_tabs_closed`.
- Add a `CloseWindowWhenAllTabsClosedWidget` to **Features > General** immediately after the conditional `QuitWhenAllWindowsClosedWidget` insertion. Its visible label is exactly **Close window when all tabs are closed**, and its search terms cover close/window/tab/last/empty/keep open.
- Add the matching `FeaturesPageAction`, toggle-and-save handler, settings telemetry, and local-only/sync icon state.
- Add Command Palette entries **Enable closing the window when all tabs are closed** and **Disable closing the window when all tabs are closed** using `ToggleSettingActionPair`, plus a context flag populated by `Workspace::add_toggle_setting_context_flags`.

#### 2. Make zero tabs an explicit workspace state
- Change `active_tab_index: usize` to `active_tab_index: Option<usize>`. `active_tab_index()` returns `Option<usize>`, and `active_tab_pane_group()` returns `Option<&ViewHandle<PaneGroup>>`. Do not retain an unchecked public accessor or a `0` sentinel for the empty state.
- Preserve the invariant at every transition:
  - construction/restoration with tabs establishes `Some(valid_index)`;
  - removing the final tab changes `Some(0)` to `None` in the same update that empties `tabs`;
  - adding the first tab changes `None` to `Some(0)` before any active-pane-derived update;
  - multi-tab activation/removal/move/group operations always leave `Some(i)` in range.
- Final-tab removal must perform normal tab cleanup:
  - detach/shut down the closing pane group only after confirmation;
  - unsubscribe from it;
  - prune its MRU and group state;
  - add it to Undo Close when requested;
  - clear stale tab selection, rename, focus, sidecar, panel, synchronized-input, and active-session state;
  - set `active_tab_index` to `None`;
  - save app state and notify rendering.
- Do not create an invisible pane group, placeholder terminal, or special `TabData`.
- Add explicit clear/reset operations for left and right panel active-pane handles, active-session and focus models, synchronized input, window title, and active-tab-owned transient UI. The neutral title must not preserve the closed tab title.
- When the first tab is added to an empty workspace, use default startup behavior rather than trying to inherit from an active session, rebuild active-tab-derived panel/focus/session state through the normal activation path, and guarantee an ungrouped insertion at index `0`.

#### 3. Migrate each active-tab call-site class

- **Render and keymap:** branch on `None` before `render_banner_and_active_tab`, active-pane panel rendering, or synchronized-input context generation. Add `Workspace_NoTabs`; omit `Workspace_SingleTab`, `Workspace_MultipleTabs`, active group/pin, active session, and pane-drag flags.
- **Events and cached state:** event handlers use the emitting pane-group identity, not whichever tab is active. Events from a pane group that is no longer present are dropped. Window-global handlers continue without active content. Panel views receive an explicit clear/no-active-pane operation.
- **Actions:** active-content actions are context-disabled and handler-guarded. New Tab, open-top-level-content actions, Settings, Command Palette, menus, and window actions remain available. A programmatic dispatch of a disabled active-content action has no side effect.
- **Queries and Local Control:** optional queries return no value; collections return empty; active target resolution returns `MissingTarget`. Tab/window inspection must represent a zero-tab window without marking tab `0` active or changing the external protocol to invent a tab.
- **Recovery:** all first-tab paths establish the active tab before running code that requires it, and all inherit-from-active-session decisions fall back to their existing no-previous-session/default behavior.

#### 4. Separate “close the window” from “remove the final tab”

- For explicit final-tab close, calculate whether the action can and should close the window:
  - `close_window_when_all_tabs_closed == true` **and** `ContextFlag::CloseWindow` enabled → use the existing window-close path;
  - otherwise → use the normal confirmed tab-removal path and enter zero tabs.
- Skip tab-level confirmation only in the first branch where window close will actually run. The keep-open branch must not set `skip_confirmation` merely because the tab is final.
- Replace the Close Active Tab binding predicate with one based on the presence of at least one tab, not window-close capability. Add a `Workspace_NoTabs` (or equivalently named) keymap context and use it to hide/disable Close Tab and all active-session commands in the empty state while leaving New Tab and global window commands available.
- Preserve the cross-window drag cleanup branches. A successful last-tab handoff continues to return/handle `CloseSourceWindow`; it must not call the new explicit-close zero-tab transition. Add regression coverage so future cleanup refactors do not unify these paths.

#### 5. Render the zero-tab state safely

- Branch before `render_banner_and_active_tab` and before any active-pane-dependent panel rendering. Paint only the normal workspace background in the main content slot.
- Keep global window chrome and sidebars rendered. Tab-scoped panel models must be cleared or rendered without an active pane group; stale closed pane content is forbidden.
- Render the zero-tab hint and `+` in the otherwise empty tab presentation as specified in Product behavior #3. Use the existing resolved new-tab binding display helper so custom and platform bindings are represented correctly.
- Do not render tab slots, an active-tab indicator, tab menus, tab overflow derived from tab contents, or an active content banner when there are no tabs.
- Ensure both horizontal and vertical tab layouts, including collapsed vertical-tabs mode and narrow windows, follow the same recovery contract.

#### 6. Preserve non-restoration and lifecycle behavior

- Retain the existing app-state rule that drops snapshots with no tabs. Add a regression test proving an empty workspace contributes no restorable window while the setting value itself persists.
- Do not alter `on_should_close_window`, `on_should_terminate_app`, or `quit_on_last_window_closed`. An explicit close of an empty window continues through those existing callbacks.
- Ensure `handle_reopen`, workspace registries, window titles, synchronized input, and focus change notifications tolerate no active pane group.

### Open questions resolved

1. **Empty content:** visually blank; no dedicated empty-state component or button.
2. **Recovery UI:** the tab bar stays visible with `+`; the faded shortcut hint lives inside the tab bar, not the content region.
3. **New-session entry points:** every standard New Tab path must work from zero tabs.
4. **Restart:** an empty window is not restored; relaunch seeds/restores a normal session.
5. **Confirmations:** normal tab/session confirmation applies only when the final tab is removed without closing the window; explicit window/quit behavior is unchanged.
6. **Cross-window drag:** the later geometry discussion deliberately superseded the initial preference for uniform removal behavior. Successful final-tab drag always closes the source window because that window served as the moving preview.
7. **`CloseWindow`-disabled hosts:** they are in scope and may reach zero tabs even while the global setting is on, because they cannot honor its window-close side.
8. **Scope:** the preference is global, persisted, and synced; no per-window override or customizable empty state.
9. **Chrome:** global window chrome remains; active-session actions must fail closed rather than panic or operate on stale content.
10. **Prototype:** implementation must provide a Dogfood/feature-branch build the requester can try, plus computer-use video proof.
11. **Active-tab representation:** the prior spec's `usize` sentinel is rejected. `Option<usize>` is required so all call sites make zero-tab behavior explicit at compile time.
12. **Competing fresh-tab variant:** APP-5242 avoids this invariant migration and should be preferred if hands-on testing shows that opening a fresh session meets the product need. APP-5230 remains specified for a true blank, tabless prototype so the requester can compare the actual tradeoff.

### Risks and mitigations

- **Active-tab invariant is pervasive and is the dominant cost.** The audit found 267 non-test source locations across 159 functions. Rendering and keymap generation currently make zero tabs an immediate panic, while event, action, query, and index paths can panic, underflow, silently target a phantom index, or mutate stale pane state. Mitigate with `Option<usize>` compiler enforcement, the per-class obligations above, a dedicated no-tabs context, and regression tests that exercise actions/events while empty.
- **Closing can lose process or editor state.** The old final-tab path relied on window-close confirmation. Mitigate by selecting the confirmation owner from the actual outcome and testing shared-session, long-running-process, unsaved-code, cancel, and confirm cases.
- **Cross-window code could accidentally use the new removal path.** Mitigate with separate explicit-close and transfer APIs plus tests proving a successful last-tab handoff closes the source for both setting values.
- **A fake or stale pane can leak into persistence.** Mitigate by requiring a true empty `tabs` vector, clearing pane-group references, retaining snapshot filtering, and testing relaunch behavior.
- **The tab-bar hint can crowd controls.** Mitigate with low-priority shrink/ellipsis/hide behavior and visual verification at normal and minimum supported widths in horizontal, vertical, and collapsed-vertical layouts.
- **Platform behavior diverges.** macOS can remain alive with no windows while Linux/Windows normally terminate on final-window close, and web/link hosts cannot close windows. Mitigate by testing the effective-close decision independently from the global preference and using cross-platform CI as the backstop.

### Size assessment

**Revised estimate: L (5), up from M (3).** The setting, widget, and close-path branch are small. The true zero-tab state is not: the audit found 267 active-tab-invariant locations across 159 functions, including unavoidable render/keymap crashes, 87 direct index uses, cross-module APIs, panel caches, Local Control, WASM, and asynchronous event paths. The `Option` migration makes these decisions explicit but also guarantees a broad compile-and-test surface across the app.

APP-5242 is the strategic alternative if the goal is simply “do not close the window.” It keeps at least one tab alive, avoids the invariant migration, and remains approximately S (2). APP-5230 is justified only if the blank, genuinely tabless state is important enough to pay the L-sized implementation and regression cost. The requested side-by-side Dogfood prototypes are therefore not cosmetic validation; they decide whether the expensive invariant change creates enough product value to keep.

## Validation and verification criteria

All criteria must pass before merge.

1. **Setting contract and compatibility (Behavior #1, #12):** a unit/settings test proves `general.close_window_when_all_tabs_closed` defaults to `true`, persists, globally syncs under the normal sync preference, and the Settings switch plus Command Palette enable/disable actions mutate the same value. Existing user configurations with no key retain current close-capable desktop behavior.
2. **Settings placement and copy (Behavior #1):** a running GUI shows **Close window when all tabs are closed** in **Settings > Features > General**, directly below **Quit when all windows are closed** on macOS and in the corresponding position after the conditional slot on other platforms. Settings search finds it using “close last tab” and “keep window open.”
3. **Replacement invariant is structural (Behavior #2):** the implementation changes `Workspace.active_tab_index`, `active_tab_index()`, and `active_tab_pane_group()` to optional forms. State-machine unit tests exercise construction, activation, multi-tab removal, `1 → 0`, `0 → 1`, Undo Close, and transferred-tab insertion and assert after every transition that `(tabs.is_empty() && active_tab_index == None) || active_tab_index.is_some_and(|i| i < tabs.len())`. No sentinel `0`, placeholder `TabData`, or retained active pane group is accepted.
4. **Call-site migration is complete:** compile failures from the optional field/accessors are resolved at all 267 audited baseline locations. The PR includes an updated grep/audit summary showing each remaining `active_tab_pane_group()`/`active_tab_index` use is one of: guarded by `Some`, after a proven insertion, an optional query, or a checked active-content action. No zero-tab-reachable path uses `unwrap`, `expect`, unchecked indexing, `len() - 1`, or active-index arithmetic without a non-empty proof.
5. **Default explicit-close path (Behavior #1, #7):** with the setting on in a `CloseWindow`-capable desktop window, closing the final tab from the tab close button/menu and the resolved Close Tab shortcut closes the window. Existing window/quit warning behavior is exercised and no transient zero-tab frame is shown.
6. **Opt-in explicit-close path (Behavior #2, #6):** add a regression test in `app/src/workspace/view_tests.rs` that fails on the baseline and passes after the change: set the preference off, close the final ordinary tab, and assert the window close path is not requested, `tab_count() == 0`, `active_tab_index() == None`, `active_tab_pane_group() == None`, active-tab-derived state is cleared, and the closed tab is placed on the normal Undo Close stack.
7. **Confirmation ownership (Behavior #6, #7, #8):** automated tests cover a final tab with a shared session and the available unsaved/long-running summary seam:
   - setting off → tab-level confirmation appears; Cancel keeps one tab; Confirm produces zero tabs;
   - setting on in a close-capable host → tab-level confirmation is skipped and existing window-close confirmation owns the decision;
   - explicit Close Window and Quit are unchanged from an empty window.
8. **Render/keymap hot paths (Behavior #3, #4):** render-to-element/integration coverage enters zero tabs and runs `Workspace::keymap_context`, horizontal render, vertical render, collapsed-vertical render, panel render, and the applicable WASM render helpers. None panic or dereference active content. `Workspace_NoTabs` is present; single/multiple/active group/pin/session flags are absent; the main content is blank; global chrome remains.
9. **Event and cached-state safety (Behavior #4):** after final-tab removal, tests assert active session/focus notifications report `None`, synchronized-input state has no current tab, left/right panel active-pane handles are cleared, tab rename/multi-selection/sidecar/feature-intro state does not point to the removed tab, and the title no longer contains the closed tab title. A synthetic late event from the removed pane group is ignored; window-global settings/auth/update events still complete without a tab.
10. **Active-content action safety (Behavior #4):** a zero-tab action matrix covers Close Tab, previous/next/last tab, tab grouping/pinning/movement, pane focus/split/close, share/session/Agent/editor commands, panel commands that require a pane, and environment creation. Each is context-disabled and produces no state change if dispatched programmatically. Settings, Command Palette, menus, explicit Close Window/Quit, and actions that create top-level content remain available.
11. **Query and Local Control semantics (Behavior #4):** zero-tab tests prove active-session/terminal/selected-text/conversation/startup-directory queries return `None` or empty data; metadata/debug output contains no stale pane ID; `tab.list` returns `[]`; `tab.inspect`, `tab.close active`, pane/session active selectors, and other active-target mutations return the existing structured `MissingTarget` or target-state error; `tab.create` succeeds and reports the new tab at active index `0`.
12. **Zero-tab UI (Behavior #3, #4):** computer-use verification shows a visually blank main content region, no fake tab/session or active indicator, the persistent `+`, the faded shortcut hint inside the tab bar, an app-level window title, and usable global chrome. It covers horizontal tabs, vertical tabs, collapsed vertical tabs, and a narrow window where the hint yields before essential controls.
13. **Recovery through every standard entry point (Behavior #5):** parameterized workspace/integration coverage starts from zero tabs and separately creates a first session through the `+`, resolved New Tab shortcut, File-menu New Tab, Command Palette New Tab, and new-session menu. Each produces exactly one focused, ungrouped tab at index `0`, changes the active index from `None` to `Some(0)`, uses default rather than stale closed-session inheritance, clears the hint, and restores normal focus, title, panel, and session state.
14. **Undo Close recovery (Behavior #6):** from zero tabs, Undo Close restores the closed final tab as the sole focused tab without creating a second default session and changes the active index from `None` to `Some(0)`.
15. **`CloseWindow`-disabled host (Behavior #9):** a context-controlled test proves Close Tab is offered while one tab exists, final-tab close reaches zero tabs with the preference both on and off, Close Window remains unavailable, and all recovery paths work. Close Tab is unavailable after reaching zero.
16. **Cross-window asymmetry (Behavior #10):** cross-window regression coverage proves that a successful only-tab handoff closes the source window with the preference both on and off, adds exactly one tab to the target, does not duplicate or lose the transferred pane group, and never renders the source as empty. A cancelled drag restores the original one-tab source.
17. **Persistence and restart (Behavior #11):** an app-state test proves zero-tab windows are omitted from `AppState.windows`; restart/session restoration creates or restores a normal session rather than an empty window; the global preference retains its saved value.
18. **Adjacent multi-tab behavior:** existing tests for closing horizontal/vertical tabs, active-neighbor selection, tab groups, pinned tabs, Close Other Tabs, Close Tabs Right/Below, tab drag, pane focus, synchronized input, Local Control tab targeting, and session-close confirmation continue to pass unchanged.
19. **Large-change deterministic test gate:** `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2` and `cargo test --doc` pass. Any added GUI integration test also passes through the repository's `crates/integration` harness. Cross-platform PR CI is required because the change touches native and WASM workspace behavior.
20. **Repository quality gates:** `./script/format` plus every clippy configuration in `script/presubmit` complete successfully before the PR is promoted from draft.
21. **Hands-on prototype and visual proof:** implementation supplies a runnable Dogfood/feature-branch build for the requester to try. Computer use records and attaches a video that demonstrates, in one coherent flow:
   - the default-on setting closing the final tab's window;
   - toggling the setting off in the specified Settings location;
   - final-tab confirmation and the resulting zero-tab UI;
   - creating the first replacement session with both the `+` and ⌘T;
   - a successful drag of an only tab into another window closing its source despite the setting being off.
   The video is attached to both the Linear task and the reused implementation PR.
22. **Variant decision gate:** before APP-5230 is merged, the requester tries this Dogfood build alongside APP-5242's fresh-tab prototype and explicitly confirms that the blank, truly zero-tab state provides enough value to justify the L-sized invariant migration. If the fresh-tab behavior is acceptable, implementation should proceed with APP-5242 and APP-5230 should not merge.
