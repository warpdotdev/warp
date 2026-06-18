# Network log in-app pane — Tech spec
Companion to `PRODUCT.md` in this directory; refer there for user-visible behavior.
## Context
- `app/src/server/network_logging.rs` — `init(..)` registers HTTP client `set_before_request_fn`/`set_after_response_fn` hooks that push `NetworkLogItem`s into a bounded `async_channel` (size 100). A background task writes items to `log_file_path()` (`warp_network.log`), truncating and reopening after every 50 items. `pub fn log_file_path()` is exposed for the tail workflow.
- `app/src/server/server_api.rs (1214-1222)` — `ServerApiProvider::new` invokes `network_logging::init` when `ContextFlag::NetworkLogConsole.is_enabled()`.
- `app/src/workflows/local_workflows.rs (213-229)` — `network_logging_workflow()` builds a hardcoded `WorkflowSource::App` workflow whose command is `tail -f <log>`. Injected into `app_workflows()`.
- `app/src/settings_view/privacy_page.rs` — `NetworkLogWidget` renders the "View network logging" link and dispatches `PrivacyPageAction::LaunchNetworkLogging` → bubbles to `SettingsViewEvent::LaunchNetworkLogging`.
- `app/src/terminal/input.rs (1754-1760, 7302-7332, 13749)` — `input:insert_network_logging_workflow` binding ("Show Warp network log") → `InputAction::InsertNetworkLoggingWorkflow` → `Input::insert_network_logging_workflow` opens the tail workflow info box.
- `app/src/workspace/view.rs (12646-12648, 12769-12793)` — `launch_network_logging_workflow_in_active_tab` clears the active input and runs the workflow.
- Pane plumbing: `app/src/pane_group/pane/mod.rs (133-153, 432-504)` — `IPaneType` enum, `PaneId::from_*_pane_ctx/view` helpers, `PaneId::render` arms. `execution_profile_editor_pane.rs` is the closest minimal template. `AIFactManager` / `ExecutionProfileEditorManager` demonstrate singleton "one pane per window" managers.
- `CodeEditorView` (`app/src/code/editor/view.rs`) supports `buffer: None`, `reset(InitialBufferState, ctx)`, and `set_interaction_state(InteractionState::ReadOnly, ctx)`; its model is defined in `app/src/code/editor/model.rs`.
- `app/src/app_state.rs (118-139)` — `LeafContents` enum. `app/src/pane_group/mod.rs (1770-1939)` — restore match; panes we don't restore return `Err(anyhow!(..))`.
## Proposed changes
### In-memory model
- Add `NetworkLogModel` in `app/src/server/network_logging.rs`: singleton entity with a `VecDeque<NetworkLogItem>` capped at `NETWORK_LOGGING_MAX_ITEMS = 50` (matches current file-rotation threshold). `push(item, ctx)` pops the front when over capacity; `snapshot_text(&self) -> String` joins items with `\n`.
- `pub(crate)` on `NetworkLogItem`; keep its `Display` impl and the existing timestamp + `{:?}` request/response format so pane output matches what `warp_network.log` used to contain.
- Rewrite `init(..)` to take the `ModelContext<ServerApiProvider>`. Keep the bounded channel and client hooks unchanged. Replace the disk-writing background task with `ctx.spawn_stream_local(rx, |_, item, ctx| NetworkLogModel::handle(ctx).update(ctx, |m, ctx| m.push(item, ctx)), |_, _| {})`. This mirrors the existing `event_receiver` pattern in `ServerApiProvider::new`.
- Delete `truncate_and_restart_log`, `log_file_path()`, the `WARP_LOGS_DIR` branch, and the `local_fs` cfg split. Remove dead `warp_core::paths` / `PathBuf` imports.
- Register the singleton in `app/src/lib.rs::initialize_app` (before `ServerApiProvider`): `ctx.add_singleton_model(|_| NetworkLogModel::default())`.
### View, pane, and manager
- `app/src/server/network_log_view.rs`: `NetworkLogView` owns a `ViewHandle<CodeEditorView>` and a `ModelHandle<PaneConfiguration>` titled "Network log". On `new`, build `CodeEditorView` with `buffer: None` + default `CodeEditorRenderOptions`, seed with `NetworkLogModel::as_ref(ctx).snapshot_text()` via `editor.reset(InitialBufferState::plain_text(text), ctx)`, and call `set_interaction_state(InteractionState::ReadOnly, ctx)`. No subscription to the model — single snapshot per open.
- `BackingView` impl: default header/chrome, no toolbelt, no overflow menu. `focus(ctx)` forwards to the editor.
- `app/src/pane_group/pane/network_log_pane.rs`: `NetworkLogPane` mirroring `ExecutionProfileEditorPane`. `snapshot() -> LeafContents::NetworkLog` (new unit variant). `attach` registers with `NetworkLogPaneManager`; `detach` deregisters. `shareable_link()` → `ShareableLink::Base`.
- `app/src/pane_group/pane/mod.rs`: add `IPaneType::NetworkLog` (+ `Display`), `PaneId::from_network_log_pane_ctx` / `..._view`, and a `ChildView<PaneView<NetworkLogView>>` arm in `PaneId::render`. `pub(super) mod network_log_pane;` + re-export.
- `app/src/server/network_log_pane_manager.rs`: `NetworkLogPaneManager` (singleton) with `HashMap<WindowId, PaneViewLocator>` and `find_pane` / `register_pane` / `deregister_pane` (pattern copied from `ExecutionProfileEditorManager`). Register in `initialize_app`.
- `app/src/app_state.rs`: `LeafContents::NetworkLog` (unit). `app/src/pane_group/mod.rs`: restore arm returns `Err(anyhow!("Network log panes are not restored"))`. `app/src/persistence/sqlite.rs`: add a round-trippable tag for the new variant only if the serde mapping requires it; no restore path needed.
### Wiring and cleanup
- `app/src/workspace/view.rs`: add `open_network_log_pane(&mut self, ctx)` following `open_execution_profile_editor_pane`: find-existing via the manager and focus, else construct `NetworkLogPane::new(ctx)` and `add_pane_with_direction(Direction::Right, pane, true, ctx)`. Replace the body of `launch_network_logging_workflow_in_active_tab` with a call to `open_network_log_pane` (and rename the method + its single caller at `12647`).
- `app/src/terminal/input.rs`: delete `Input::insert_network_logging_workflow` and the matching `InputAction::InsertNetworkLoggingWorkflow` arm. Repoint the `input:insert_network_logging_workflow` binding (keep id + "Show Warp network log" description, keep the `NetworkLogConsole` enablement gate) to a new `WorkspaceAction::OpenNetworkLogPane` that the workspace handles by calling `open_network_log_pane`. Using a `WorkspaceAction` avoids adding a one-off `Input` event just to bubble it up.
- `app/src/workflows/local_workflows.rs`: delete `network_logging_workflow` and remove its call in `app_workflows()`; the prompt-chip dogfood workflow stays.
- Privacy page: no copy changes; `PrivacyPageAction::LaunchNetworkLogging` keeps bubbling up `SettingsViewEvent::LaunchNetworkLogging`, which now calls `open_network_log_pane`.
## Testing and validation
Behavior numbers below reference `PRODUCT.md`.
- Unit test in `network_logging.rs`: push > 50 items into `NetworkLogModel`, assert len capped at 50, `snapshot_text` contains the newest 50 in chronological order. Covers invariants 1, 2, 7.
- Unit test: `NetworkLogModel::default()` + `snapshot_text()` returns `""`; used to validate invariant 9 (empty open) at the model layer.
- Grep + build verification that no code under `app/src` or `crates/` still references `warp_network.log`, `log_file_path`, or `network_logging_workflow`. Covers invariants 1, 14.
- Manual validation (dogfood build, `NetworkLogConsole` enabled by default):
  - Open Privacy settings → "View network logging" → pane opens as right-split with snapshot. Close and reopen; new requests appear after reopen. Covers invariants 4, 7, 8, 10, 11, 12.
  - Trigger the `Show Warp network log` keybinding → same pane opens / focuses. Open from both entrypoints consecutively → only one pane exists per window. Covers invariants 5, 6.
  - Disable `ContextFlag::NetworkLogConsole` (e.g. warp-home-link-only mode) → settings link hidden, keybinding unavailable. Covers invariant 3.
  - Confirm the pane is read-only: attempted typing produces no edits; find, select, copy still work. Covers invariant 10.
  - Open pane, quit app, relaunch → pane is absent, in-memory log starts empty. Covers invariant 13.
  - Search command palette and workflow list for "Tail Warp network log" → no results. Covers invariant 14.
- Presubmit: `./script/presubmit` (covers `cargo fmt` + `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings` + `cargo nextest`). Must pass before PR, per repo rules.
- WASM build: `cargo check --target wasm32-unknown-unknown -p warp` (or the repo's standard wasm command) to confirm the removed `local_fs` cfg split didn't leave wasm-only gaps.
## Risks and mitigations
- **Log items can leak sensitive tokens into memory and into the pane UI.** The formatting path is unchanged from the existing on-disk log, so redaction behavior is inherited — not improved. Flag as follow-up if stronger redaction is desired.
- **Pane type addition touches `LeafContents` and the restore match.** Missing an arm is caught at compile time thanks to exhaustive matching, but the sqlite serde tag must round-trip so existing snapshots keep loading. Verify by restoring an app-state snapshot that contains non-network-log panes after the change.
- **Keybinding id `input:insert_network_logging_workflow` is retained for backwards compat with user custom keybindings** even though the handler no longer inserts a workflow. Acceptable trade-off; renaming would break users' customized bindings.
