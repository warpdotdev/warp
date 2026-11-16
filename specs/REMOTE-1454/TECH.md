# REMOTE-1454: Tech Spec — Cloud Mode setup UI for non-oz harnesses
## Problem
The `CloudModeSetupV2` setup UI only transitions out of the "setup commands" phase when an Oz `AppendedExchange` event fires. Non-oz harness runs (claude, gemini) never produce an Oz exchange — the sandboxed `oz agent run --harness=<name>` invokes the harness CLI (e.g. `claude --session-id … < /tmp/oz_prompt`) as a shell command in the shared session, and that command is permanently flagged as an environment startup command. See `specs/REMOTE-1454/PRODUCT.md` for the desired UX.
## Relevant code
- `crates/warp_features/src/lib.rs:757,826` — `FeatureFlag::AgentHarness`, `FeatureFlag::CloudModeSetupV2`.
- `app/src/terminal/view/ambient_agent/model.rs:79-151,240-250,384-418,442-488,726-727,886-899` — `AmbientAgentViewModel` (status, `harness` field, `spawn_agent`, `enter_viewing_existing_session`, `AmbientAgentViewModelEvent`, `DispatchedAgent` emission).
- `app/src/terminal/view/ambient_agent/view_impl.rs:91-228,230-300` — `handle_ambient_agent_event` (inserts `CloudModeInitialUserQuery` on `DispatchedAgent`), `maybe_insert_setup_command_blocks` (gated on `is_cloud_agent_pre_first_exchange`).
- `app/src/terminal/view/ambient_agent/mod.rs:81-114` — `is_cloud_agent_pre_first_exchange` helper (checks `exchange_count() == 0`).
- `app/src/terminal/view/ambient_agent/block/query.rs` — `CloudModeInitialUserQuery` rich content (top-of-conversation prompt block).
- `app/src/terminal/view/ambient_agent/block/setup_command_text.rs:122-157` — summary row copy keys off `is_cloud_agent_pre_first_exchange`.
- `app/src/terminal/view/ambient_agent/block/setup_command.rs:56-91` — per-command row, listens for `UpdatedSetupCommandVisibility` and `BlockCompleted`.
- `app/src/terminal/model/terminal_model.rs:1213-1241` — `new_for_cloud_mode_shared_session_viewer` sets `is_executing_oz_environment_startup_commands = true` when `CloudModeSetupV2` is enabled.
- `app/src/terminal/model/blocks.rs:1284-1302,2486-2489` — block-list flag and per-block `is_oz_environment_startup_command`.
- `app/src/terminal/model/block.rs:1462-1468` — `Block::is_oz_environment_startup_command` / setter.
- `app/src/terminal/view.rs:5042-5053` — existing `AppendedExchange` handler that flips the block-list flag off (Oz path).
- `app/src/terminal/view.rs:10413` — `ModelEvent::AfterBlockStarted` branch calls `maybe_insert_setup_command_blocks(block_id, ctx)`.
- `app/src/terminal/view.rs:6609-6634,6785-6793,6841-6863` — uses of `is_cloud_agent_pre_first_exchange` (tombstone, input visibility, input hide logic).
- `app/src/ai/blocklist/block/pending_user_query_block.rs` — `PendingUserQueryBlock` view, takes `interruptible: bool`, always renders a close button plus optional "Send now".
- `app/src/terminal/view/pending_user_query.rs` — `TerminalView::insert_pending_user_query_block`, `remove_pending_user_query_block`, `send_user_query_after_next_conversation_finished`. Tracked by `TerminalView::pending_user_query_view_id`.
- `app/src/terminal/view.rs:2789-2796,4160` — `TerminalView::pending_user_query_view_id`, `queued_prompt_callback` fields.
- `app/src/terminal/view.rs:3237-3246,4648-4650` — existing removal paths for the pending-query block on agent-view/conversation transitions.
- `app/src/ai/ambient_agents/task.rs:26-122,212-238` — `AgentConfigSnapshot.harness: Option<HarnessConfig>`, `AmbientAgentTask.agent_config_snapshot`.
- `app/src/terminal/cli_agent.rs:107-260` — `CLIAgent` enum (`Claude`, `Gemini`, …) with `command_prefix`, `CLIAgent::detect`.
- `app/src/terminal/view/use_agent_footer/mod.rs:354-387` — `TerminalView::detect_cli_agent_from_model` (returns the `CLIAgent` for the active long-running block using alias-aware detection).
- `app/src/terminal/shared_session/viewer/terminal_manager.rs:871-896,1186-1209` — `is_cloud_agent_pre_first_exchange` use in remote input / input-mode gating.
- `app/src/terminal/view/shared_session/view_impl.rs:1700-1721` — viewer-driven sizing helpers: `restore_pty_to_sharer_size`, `resize_from_viewer_report`, and the new `force_report_viewer_terminal_size` entry point used at harness start.
- `app/src/terminal/view/ambient_agent/block/setup_command.rs:176-197` — per-command row, currently renders via `with_agent_output_item_spacing`; needs a third-party-harness branch that uses `*terminal::view::PADDING_LEFT` horizontal margins.
- `app/src/terminal/view/ambient_agent/block/setup_command_text.rs:184-206` — summary row, same story as the per-command row.
- `app/src/ai/blocklist/block/view_impl.rs:1264-1312` — `CONTENT_HORIZONTAL_PADDING`, `CONTENT_ITEM_VERTICAL_MARGIN`, and the `WithContentItemSpacing::with_agent_output_item_spacing` helper used by the oz setup UI.
- `app/src/terminal/view.rs:746-752` — `PADDING_LEFT` (20px, or 16px behind `LessHorizontalTerminalPadding`) — the horizontal padding used by regular terminal command blocks.
- `app/src/pane_group/mod.rs:3763-3792,5682-5692` — CLI-agent conversation restoration (`FeatureFlag::AgentHarness` gate for replay).
- `warp_cli::agent::Harness` (`crates/warp_cli/src/agent.rs:120-131`) — `Oz`, `Claude`, `Gemini`.
## Current state
`AmbientAgentViewModel::harness` is populated on spawn for the local spawner via the harness selector. For viewers that join a shared cloud session, the field is left at its default `Oz` even for claude / gemini runs — `enter_viewing_existing_session` fetches the task but only reads `agent_config_snapshot.environment_id`. The viewer therefore reports the wrong harness until this is fixed.
When the cloud-mode terminal model is created, `is_executing_oz_environment_startup_commands` is set to `true` unconditionally under `CloudModeSetupV2`. The block-list flag:
- Marks every new block `is_oz_environment_startup_command = true` and hides it.
- Is flipped to `false` only in `ModelEvent::AfterBlockStarted`'s sibling `BlocklistAIHistoryEvent::AppendedExchange` handler (view.rs:5042-5053).
`is_cloud_agent_pre_first_exchange` is a view-level helper that returns `true` while the active cloud-agent conversation has `exchange_count() == 0`. It gates: the "Setting up environment" warping indicator, the pre-harness input-box hiding, remote-input suppression in `shared_session/viewer/terminal_manager.rs`, and `maybe_insert_setup_command_blocks`. For non-oz there are no exchanges, so this stays `true` forever.
`maybe_insert_setup_command_blocks` runs in the `AfterBlockStarted` path and, when pre-first-exchange, inserts the `CloudModeSetupTextBlock` summary row (once) plus a `CloudModeSetupCommandBlock` row for the block. It has no knowledge of whether the block is the harness CLI command.
`PendingUserQueryBlock::new` currently takes `interruptible: bool` and always renders a close button; the "Send now" button is added only when `interruptible`. `TerminalView::insert_pending_user_query_block` subscribes to `Dismissed`/`SendNow` events; `send_user_query_after_next_conversation_finished` also registers a `queued_prompt_callback` so the prompt is re-submitted when the current conversation finishes.
## Proposed changes
### 1. `PendingUserQueryBlock` — per-button bools
`app/src/ai/blocklist/block/pending_user_query_block.rs`
Replace the `interruptible: bool` constructor param with two bool fields on the block itself:
- `show_close_button: bool` — whether to render the dismiss ("X") button.
- `show_send_now_button: bool` — whether to render the "Send now" button.
Update `PendingUserQueryBlock`:
- Make `close_button: Option<ViewHandle<ActionButton>>` (was unconditional), constructed only when `show_close_button` is true.
- `send_now_button: Option<ViewHandle<ActionButton>>` stays optional, constructed only when `show_send_now_button` is true.
- Build the buttons column with 0, 1, or 2 children. If it ends up empty, skip the whole "buttons" container.
- Constructor signature becomes `PendingUserQueryBlock::new(prompt, user_display_name, profile_image_path, show_close_button, show_send_now_button, ctx)`.
This keeps the wire-up trivial for each call site (pass the two bools) and avoids introducing an enum just to express three combinations.
### 2. `TerminalView` helpers — thread the button bools and add a cloud-mode insertion path
`app/src/terminal/view/pending_user_query.rs`
- Change `insert_pending_user_query_block`'s `interruptible: bool` param to `show_close_button: bool, show_send_now_button: bool` and pass them to `PendingUserQueryBlock::new`.
- Keep subscribing to `PendingUserQueryBlockEvent::Dismissed` and `PendingUserQueryBlockEvent::SendNow`. When both bools are `false`, the block never emits these events, so the subscription is effectively inert.
- Change `send_user_query_after_next_conversation_finished`'s `interruptible: bool` param to the same two bools and pass them through. Update existing callers:
  - `app/src/workspace/view.rs:11605` (`handle_forked_conversation_prompts`, `/fork-and-compact`) — `show_close_button: true, show_send_now_button: false` (was `false`).
  - `app/src/workspace/view.rs:11701` (`summarize_active_ai_conversation`, `/compact-and`) — `show_close_button: true, show_send_now_button: false` (was `false`).
  - `app/src/workspace/view.rs:20955` (`QueuePromptForConversation`, `/queue`) — `show_close_button: true, show_send_now_button: true` (was `true`).
- Add a new public method used by Cloud Mode:
  ```rust
  pub(in crate::terminal::view) fn insert_cloud_mode_queued_user_query_block(
      &mut self,
      prompt: String,
      ctx: &mut ViewContext<Self>,
  ) {
      self.remove_pending_user_query_block(ctx);
      self.insert_pending_user_query_block(
          prompt,
          /* show_close_button */ false,
          /* show_send_now_button */ false,
          ctx,
      );
  }
  ```
  This does NOT register a `queued_prompt_callback` — the prompt is not re-submitted; it is already being carried by the harness inside the sandbox.
`remove_pending_user_query_block` already clears `queued_prompt_callback` on top of removing the view. It remains the single teardown entry point.
### 3. `AmbientAgentViewModel` — resolve harness for viewers, track harness-command-started, new event
`app/src/terminal/view/ambient_agent/model.rs`
Add a helper for the non-oz check:
```rust
impl AmbientAgentViewModel {
    /// True when the run is configured to use a non-Oz execution harness and the
    /// required feature flags are enabled.
    pub fn is_third_party_harness(&self) -> bool {
        FeatureFlag::AgentHarness.is_enabled() && self.harness != Harness::Oz
    }
}
```
Fix the viewer-side harness resolution so `AmbientAgentViewModel::harness` reflects the run's actual harness for every client, not just the spawner. In `enter_viewing_existing_session`, extend the success branch to also resolve the harness from `task.agent_config_snapshot.harness` and call `set_harness`:
```rust
Ok(task) => {
    let snapshot = task.agent_config_snapshot.as_ref();
    let environment_id = snapshot
        .and_then(|s| s.environment_id.as_deref())
        .and_then(|id| ServerId::try_from(id).ok())
        .map(SyncId::ServerId);
    let harness = snapshot
        .and_then(|s| s.harness.as_ref())
        .map(|h| parse_harness_config_name(&h.harness_type))
        .unwrap_or(Harness::Oz);

    me.set_environment_id(environment_id, ctx);
    me.set_harness(harness, ctx);
}
```
`parse_harness_config_name` is a small module-local helper that matches `"claude"` / `"gemini"` / `"oz"` explicitly and falls back to `Harness::Oz` (with a warn log) on unknown values. `set_harness` is a no-op when unchanged.
Add a harness-command-started tracker:
```rust
pub struct AmbientAgentViewModel {
    // ... existing fields ...
    harness_command_started: bool,
}

impl AmbientAgentViewModel {
    pub fn harness_command_started(&self) -> bool {
        self.harness_command_started
    }

    pub fn mark_harness_command_started(&mut self, ctx: &mut ModelContext<Self>) {
        if self.harness_command_started {
            return;
        }
        self.harness_command_started = true;
        ctx.emit(AmbientAgentViewModelEvent::HarnessCommandStarted);
    }
}
```
Initialize it to `false` in `new` and `reset_status`.
Add the event variant:
```rust
pub enum AmbientAgentViewModelEvent {
    // ... existing ...
    HarnessCommandStarted,
}
```
### 4. `is_cloud_agent_pre_first_exchange` — also honor harness-started
`app/src/terminal/view/ambient_agent/mod.rs`
Extend the helper so both Oz's "first exchange" and non-oz's "harness command started" transitions end the pre-first-exchange phase. At the site that currently returns based on `exchange_count() == 0`:
```rust
if ambient_agent_view_model.as_ref(app).harness_command_started() {
    return false;
}
BlocklistAIHistoryModel::as_ref(app)
    .conversation(&conversation_id)
    .is_some_and(|conversation| conversation.exchange_count() == 0)
```
This keeps every existing consumer (`status_bar`, input-box visibility, remote-input suppression, `setup_command_text`) correctly reading `false` after the harness block is detected.
### 5. Detect the harness block and emit `HarnessCommandStarted`
`app/src/terminal/view/ambient_agent/view_impl.rs` — `maybe_insert_setup_command_blocks`
This function is already the single hook called from `ModelEvent::AfterBlockStarted` for cloud-mode setup bookkeeping, so it's the right place to detect the harness block.
Before doing any setup-row insertion, short-circuit if the active block's CLI-agent matches the run's harness:
```rust
pub(in crate::terminal::view) fn maybe_insert_setup_command_blocks(
    &mut self,
    block_id: &BlockId,
    ctx: &mut ViewContext<Self>,
) {
    if !FeatureFlag::CloudModeSetupV2.is_enabled() {
        return;
    }

    // Only act while we're still in the pre-first-exchange phase.
    if !super::is_cloud_agent_pre_first_exchange(
        &self.ambient_agent_view_model,
        &self.agent_view_controller,
        ctx,
    ) {
        return;
    }

    // For non-oz harness runs, transition out of the setup phase when the
    // harness CLI starts. The block becomes a normal CLI-agent session rather
    // than a setup command.
    if self.ambient_agent_view_model.as_ref(ctx).is_third_party_harness()
        && self.active_block_matches_run_harness(ctx)
    {
        self.ambient_agent_view_model.update(ctx, |m, ctx| {
            m.mark_harness_command_started(ctx);
        });
        return;
    }

    // ... existing setup-row insertion logic, now guarded by the
    // is_cloud_agent_pre_first_exchange check above ...
}
```
`active_block_matches_run_harness` is a small new helper on `TerminalView` that runs `CLIAgent::detect` directly (no alias or shell-escape handling) and matches against the run's harness:
```rust
fn active_block_matches_run_harness(&self, ctx: &AppContext) -> bool {
    let command = self
        .model
        .lock()
        .block_list()
        .active_block()
        .command_with_secrets_obfuscated(false);
    let Some(cli_agent) = CLIAgent::detect(&command, None, None, ctx) else {
        return false;
    };
    match self.ambient_agent_view_model.as_ref(ctx).selected_harness() {
        Harness::Oz => false,
        Harness::Claude => matches!(cli_agent, CLIAgent::Claude),
        Harness::Gemini => matches!(cli_agent, CLIAgent::Gemini),
    }
}
```
Cloud-mode harness runs always invoke the canonical `claude` / `gemini` CLI prefix from the harness sidecar (hardcoded in `app/src/ai/agent_sdk/driver/harness/claude_code.rs:84-95` and the matching `gemini.rs`). Passing `None` for both the escape char and aliases is sufficient; if a future wrapper (e.g. `bash -l -c 'claude …'`) breaks canonical detection, the fallback trigger under *Risks and mitigations* applies. We deliberately avoid `detect_cli_agent_from_model` because it gates on `is_active_and_long_running`, which has not yet elapsed at `AfterBlockStarted` time.
### 6. Handle `HarnessCommandStarted` in the view
`app/src/terminal/view/ambient_agent/view_impl.rs` — `handle_ambient_agent_event`
Add a new arm that mirrors how `AppendedExchange` already tears down the setup phase for Oz, removes the cloud-mode queued prompt block, and forces a fresh viewer-size report to the sharer so the harness CLI lays out at our current dimensions:
```rust
AmbientAgentViewModelEvent::HarnessCommandStarted => {
    // Stop classifying new blocks as environment setup commands and let the
    // existing CLI-agent rendering take over for the harness block.
    let mut model = self.model.lock();
    if model.block_list().is_executing_oz_environment_startup_commands() {
        model
            .block_list_mut()
            .set_is_executing_oz_environment_startup_commands(false);
    }
    drop(model);

    self.remove_pending_user_query_block(ctx);
    // Collapse the setup-commands summary now that we're past setup (matches
    // the existing behavior when the first AI exchange appears).
    self.ambient_agent_view_model.update(ctx, |model, ctx| {
        model.set_setup_command_visibility(false, ctx);
    });
    // Force a fresh viewer size report to the sharer so the harness CLI
    // (e.g. the claude TUI) starts at the viewer's actual dimensions
    // instead of whatever the sandbox PTY was sized to during setup.
    self.force_report_viewer_terminal_size(ctx);
    ctx.notify();
}
```
The Oz `AppendedExchange` handler in `view.rs:5042-5053` stays as-is.
### 6a. `force_report_viewer_terminal_size` helper
`app/src/terminal/view/shared_session/view_impl.rs`
Viewer-driven resize normally dedups on the last reported natural size (`SharedSessionViewer::last_reported_natural_size`) so a noop refresh doesn't spam the sharer. At harness start we specifically want to break that dedup: the sandbox PTY was resized during setup (to whatever size the pre-harness commands dictated), and the harness TUI will fail to lay out correctly if we don't re-issue our report.
Add:
```rust
/// Forces a fresh viewer-size report to the sharer by clearing the dedup cache and
/// refreshing size. No-op when not an active viewer or when viewer-driven sizing is
/// not eligible.
pub(in crate::terminal::view) fn force_report_viewer_terminal_size(
    &mut self,
    ctx: &mut ViewContext<Self>,
) {
    if let Some(viewer) = self.shared_session_viewer_mut() {
        viewer.last_reported_natural_size = None;
    }
    self.refresh_size(ctx);
}
```
`refresh_size` already funnels through the standard viewer-driven-sizing eligibility checks (`is_viewer_driven_sizing_eligible`), so this is a no-op for sharers, for multi-viewer shared sessions, and for non-cloud-agent shared sessions. Cloud-agent shared sessions bypass the same-user check (`is_shared_session_for_ambient_agent`), so the viewer will always resize the sandbox PTY on harness start.
### 7. Route the initial prompt to the queued-prompt UI for non-oz runs
`app/src/terminal/view/ambient_agent/view_impl.rs` — `DispatchedAgent` handler (currently view_impl.rs:106-135)
Branch on the run's harness:
```rust
AmbientAgentViewModelEvent::DispatchedAgent => {
    self.update_pane_configuration(ctx);
    if FeatureFlag::CloudModeSetupV2.is_enabled() {
        let model = self.ambient_agent_view_model.as_ref(ctx);
        if model.is_third_party_harness() {
            let prompt = model
                .request()
                .map(|request| request.prompt.clone())
                .unwrap_or_default();
            if !prompt.is_empty() {
                self.insert_cloud_mode_queued_user_query_block(prompt, ctx);
            }
            // Do NOT set has_inserted_cloud_mode_user_query_block for third-party
            // harnesses — the queued block path handles its own lifecycle and the
            // flag only gates Oz-specific first-exchange dedup logic.
        } else {
            // ... existing Oz path: insert CloudModeInitialUserQuery, set
            // has_inserted_cloud_mode_user_query_block(true) ...
        }
    } else {
        // ... existing tip-reset branch, unchanged ...
    }
    ctx.notify();
}
```
For viewers / historical replay, `request()` is `None` on the viewer's model (the viewer doesn't spawn), so the queued block is not inserted — matching the product-spec rule that late joiners don't see it.
### 8. Remove pending-query block on pre-harness terminal states
`app/src/terminal/view/ambient_agent/view_impl.rs` — extend existing `Failed`, `Cancelled`, and `NeedsGithubAuth` arms:
Each arm gets the same additional line (idempotent and cheap; no-op if no block present):
```rust
self.remove_pending_user_query_block(ctx);
```
This covers the product-spec requirement that failing/cancelling/auth-required runs before the harness block starts remove the queued indicator and fall back to the existing error/cancel/auth UI. The fallback UI is already rendered by the existing status-bar and loading-screen code paths under `CloudModeSetupV2`.
### 9. Cloud-mode submission-block guard remains harness-agnostic
`app/src/terminal/input.rs:5725-5734` — `should_block_cloud_mode_setup_submission` currently blocks local input submission during the pre-harness/pre-first-exchange window. Because we updated `is_cloud_agent_pre_first_exchange` to also return false after `HarnessCommandStarted`, this guard already stops firing once the harness block starts, allowing input to flow to the harness TUI normally. No change needed.
### 10. Setup UI horizontal padding for non-oz harnesses
`app/src/terminal/view/ambient_agent/block.rs`, `block/setup_command.rs`, `block/setup_command_text.rs`
The oz setup UI wraps its per-command row and summary row in `WithContentItemSpacing::with_agent_output_item_spacing`, which applies a left margin of `CONTENT_HORIZONTAL_PADDING + icon_size + 16.` so the setup rows line up with other oz agent-output items (reasoning, actions, etc.). For non-oz harness runs, the harness CLI block that takes over after setup is a regular terminal command block and uses `*terminal::view::PADDING_LEFT` (20px, or 16px with `LessHorizontalTerminalPadding`) as its horizontal padding. Using the oz indent for non-oz setup rows causes a visible horizontal jump at harness start.
A shared helper `cloud_mode_setup_row_spacing` in `block.rs` picks the right spacing per harness:
```rust
pub(super) fn cloud_mode_setup_row_spacing(
    element: Box<dyn Element>,
    ambient_agent_view_model: &ModelHandle<AmbientAgentViewModel>,
    app: &AppContext,
) -> Container {
    if ambient_agent_view_model
        .as_ref(app)
        .is_third_party_harness()
    {
        Container::new(element)
            .with_margin_left(*PADDING_LEFT)
            .with_margin_right(*PADDING_LEFT)
            .with_margin_bottom(CONTENT_ITEM_VERTICAL_MARGIN)
    } else {
        element.with_agent_output_item_spacing(app)
    }
}
```
`CloudModeSetupCommandBlock` calls this in its collapsed-header branch (the expanded detail view keeps its own corner-radius / container treatment). `CloudModeSetupTextBlock` calls this at the end of `render` instead of the standalone `.with_agent_output_item_spacing(app).finish()`.
### 11. Feature flag gating
- All new code paths that check `is_third_party_harness` also implicitly depend on `FeatureFlag::AgentHarness` (the helper checks it).
- The outer `if FeatureFlag::CloudModeSetupV2.is_enabled()` guard in `DispatchedAgent` and `maybe_insert_setup_command_blocks` continues to gate the whole setup-v2 UI; when the flag is off, the legacy loading screen / full-screen overlay path is unchanged.
- When `AgentHarness` is disabled, `is_third_party_harness` always returns `false`, the Oz paths are taken for everything, and the spec's new behavior never activates. Replay of CLI-agent conversations is already guarded by the same flag in `pane_group/mod.rs:3763,5682` and `terminal/view/load_ai_conversation.rs:268`.
## End-to-end flow (non-oz Cloud Mode run)
1. User selects claude/gemini in the harness selector and submits a prompt.
2. `AmbientAgentViewModel::spawn_agent` builds a `SpawnAgentRequest` with `HarnessConfig::from_harness_type`; `self.harness` is non-oz. Stream emits `DispatchedAgent`.
3. `handle_ambient_agent_event` sees `is_third_party_harness()` and calls `insert_cloud_mode_queued_user_query_block(prompt)`. `PendingUserQueryBlock` is rendered with `show_close_button: false` and `show_send_now_button: false`.
4. Spawn stream progresses to `SessionReady`; the cloud-mode terminal-model viewer is built with `is_executing_oz_environment_startup_commands = true`.
5. Environment setup blocks start. `maybe_insert_setup_command_blocks` sees `is_cloud_agent_pre_first_exchange == true` and `active_block_matches_run_harness == false`, so it inserts the `CloudModeSetupTextBlock` summary row and a `CloudModeSetupCommandBlock` per block. Blocks are marked `is_oz_environment_startup_command` by the existing block-list logic.
6. Sandbox's `oz agent run` executes `claude --session-id … < /tmp/oz_prompt` (from `agent_sdk/driver/harness/claude_code.rs`). The viewer sees this as a new long-running block.
7. `maybe_insert_setup_command_blocks` detects `CLIAgent::Claude` matches `Harness::Claude`, calls `mark_harness_command_started`, and returns without inserting setup rows for this block.
8. `HarnessCommandStarted` fires → view arm flips `is_executing_oz_environment_startup_commands = false`, removes the pending-query block, collapses the setup-commands summary, and calls `force_report_viewer_terminal_size` so the sandbox PTY is resized to our current dimensions before claude's TUI lays out its first frame.
9. From now on, `is_cloud_agent_pre_first_exchange` returns false. The claude block is a normal CLI-agent session: `detect_cli_agent_from_model` + `CLIAgentSessionsModel::set_session` already run in `view.rs:10343-10378`, so the CLI-agent footer and rich-input UI activate. Setup rows rendered earlier in the run use `PADDING_LEFT` margins so the visible left edge of content does not shift when the harness block takes over.
10. On pre-harness `Failed` / `Cancelled` / `NeedsGithubAuth`: the new `remove_pending_user_query_block` calls tear down the queued indicator; the existing error/cancel/auth UI remains.
## Risks and mitigations
**Missing harness match.** If `CLIAgent::detect` fails to identify the harness (e.g. a future wrapper like `bash -c 'claude …'`), we never transition out of setup and the claude block stays flagged as a setup command. The short-circuit only covers the sandbox's canonical invocation; if detection fails, behavior regresses to the current broken state rather than getting worse. A fallback trigger (e.g. timeout-based transition) can be added as a follow-up if this becomes a problem.
**Viewer harness resolution races.** `enter_viewing_existing_session` fetches the task asynchronously. If the harness block starts before the fetch completes, `is_third_party_harness()` returns false at the time of detection and we treat the block as a setup command. In practice the task fetch is issued on join and the viewer only sees blocks after the shared session connects, which is later. If needed, we can re-evaluate the harness on task fetch completion and fire `mark_harness_command_started` retroactively.
**`is_cloud_agent_pre_first_exchange` semantics shift.** Consumers were designed around "conversation has no exchanges yet". After this change, the helper also returns false when a harness command has started. Every caller (`status_bar`, `is_input_box_visible`, `shared_session/viewer/terminal_manager`, `setup_command_text`, `maybe_insert_setup_command_blocks`) behaves correctly under the new semantics for non-oz runs and is a no-op for Oz runs (since `harness_command_started` is never set on Oz).
## Testing and validation
Unit coverage:
- Add a test for `PendingUserQueryBlock` rendering for each `(show_close_button, show_send_now_button)` combination (both-true, close-only, neither).
- Add a test for `AmbientAgentViewModel::mark_harness_command_started` idempotency and event emission, and for `is_third_party_harness` gating on `AgentHarness` + `harness` field.
- Add a test that `enter_viewing_existing_session` populates `harness` from the fetched `AmbientAgentTask.agent_config_snapshot.harness` for claude / gemini / oz.
- Extend `is_cloud_agent_pre_first_exchange` tests (or add one) to cover the harness-started early-return.
- Add a test that `force_report_viewer_terminal_size` clears `last_reported_natural_size` and calls `refresh_size`, and is a no-op for sharers / when viewer-driven sizing is ineligible.
Integration / manual validation (mirrors the product spec's validation list):
- Spawn a claude-code Cloud Mode run with multiple startup commands; confirm queued prompt block (no buttons) appears, setup-commands summary expands during setup, and on claude start the queued block is removed, summary collapses to "Ran setup commands", and the claude TUI is the active CLI-agent block. Confirm the claude TUI reports the viewer's current rows/cols (resize the pane during setup and verify the TUI lays out at the new size on harness start) and that the setup-command rows are horizontally flush with the subsequent harness terminal block (no visible left-edge jump).
- Spawn a gemini run; confirm the same behavior when `gemini` is detected.
- Spawn a claude run with no environment setup commands; confirm the queued block is removed when the harness block starts without a setup-commands summary having been inserted.
- Spawn a claude run that fails during env setup; confirm queued block is removed on `Failed` and the existing error UI renders.
- Cancel a claude run pre-harness; confirm queued block removal + cancelled UI.
- Trigger GitHub-auth-required pre-harness; confirm queued block removal + auth UI.
- Join an existing claude shared session after the harness has started; confirm no queued block, collapsed setup summary (from prior blocks), claude TUI visible.
- Replay a completed claude conversation; confirm no queued block, transcript rendered correctly via the existing CLI-agent block-snapshot path.
- Re-run all Oz validation cases from `specs/REMOTE-172/PRODUCT.md` to confirm no regression.
- Toggle `CloudModeSetupV2` off; confirm legacy cloud-mode behavior restored for all harnesses.
- Toggle `AgentHarness` off; confirm harness selector hides non-oz options and nothing in this spec activates.
## Follow-ups
- Consider telemetry for `HarnessCommandStarted` (time from `DispatchedAgent` / `SessionReady` to harness start) so we can monitor setup latency per-harness.
- If detection proves flaky in practice, add a timeout-based fallback transition (e.g. "still in setup 60s after session ready with no matching CLI agent detection → transition anyway and log a warning").
- Once `CloudModeSetupV2` ships to stable, consider unifying the "pre-first-exchange" and "pre-harness" concepts behind a single view-model state rather than the current combination of block-list flag + exchange count + harness-started bool.
