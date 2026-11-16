# REMOTE-1459: Tech Spec — Consistent agent view entry for non-oz cloud conversations

## Problem

When a user opens a non-oz (claude, gemini) cloud agent conversation, the resulting UX depends on which entry point they used: some land in a shared-session viewer, some land in a plain transcript viewer, and none consistently enter the agent view. For 3p harnesses the block snapshot is a single block in the terminal model with no surrounding agent-view chrome (pane header, exit affordance, details panel), so opening a 3p conversation feels unlike every other agent interaction in Warp.

Reference issue: https://linear.app/warpdotdev/issue/REMOTE-1459/fix-agent-view-entry-with-3rd-party-harnesses

REMOTE-1454 handled the *locally-started* cloud-mode flow for 3p harnesses (setup v2 UI, queued prompt, harness-command-started transition). This spec handles the two *viewer* entry points 1454 did not cover.

## Scope

In scope — both converge on the user opening an existing 3p cloud conversation:

- Live shared-session viewer join (from `warp://shared_session/{id}`, management view `OpenAmbientAgentSession`, or conversation list `OpenAmbientAgentSession`).
- Transcript viewer load (from `warp://conversation/{id}` or management / conversation-list `OpenConversationTranscriptViewer`).

Out of scope:

- `AgentViewEntryBlock` click for CLI agents: this block is only rendered when an `AIConversation` is in `BlocklistAIHistoryModel`, which never happens for CLI agent conversations.
- `HistoricalCLIAgent` restore via `replace_loading_pane_with_terminal`: only reachable from `RestoreOrNavigateToConversation`, which `ConversationOrTask::get_open_action` dispatches only for local `Conversation` items, not cloud `Task` items.

## Design

A 3p cloud run has no materialized `AIConversation` on the client — the server API returns only a `SerializedBlock` snapshot for claude/gemini runs. To wrap the harness's content in agent-view chrome, we mint a fresh empty Oz-style `AIConversation` as the agent-view-state *vehicle* and retag the run's blocks and rich content onto that vehicle so they pass `should_hide_block`'s agent view filter.

The two entry paths are structurally different and call the entry directly from their respective setup sites:

- **Transcript viewer** — harness is known synchronously from `CloudConversationData::CLIAgent`. `load_data_into_transcript_viewer` (`app/src/pane_group/mod.rs`) restores the snapshot, enters agent view, and retags — the snapshot block is in the list before the retag runs.
- **Live shared-session viewer** — harness is resolved asynchronously by `enter_viewing_existing_session` calling `set_harness` on `AmbientAgentViewModel`, which emits `HarnessSelected`. The `HarnessSelected` handler in `app/src/terminal/view/ambient_agent/view_impl.rs` invokes `maybe_enter_agent_view_for_shared_third_party_viewer`, which enters agent view and retags blocks + pre-agent-view rich content.

## Changes

### 1. `AgentViewEntryOrigin::ThirdPartyCloudAgent` variant

`app/src/ai/blocklist/agent_view/controller.rs`

Distinct from `CloudAgent`: `CloudAgent` is emitted when *starting* a new cloud-mode run from the local session, `ThirdPartyCloudAgent` when *viewing* an existing run. The existing `origin == CloudAgent` guard in `enter_agent_view_internal` stays as-is, so the new variant does not touch `AmbientAgentViewModel::enter_setup` / `enter_composing_from_setup` — important because the live shared-session viewer is already in `Status::AgentRunning` and the transcript viewer has status `NotAmbientAgent`.

`try_enter_agent_view` adds the new variant to the set of origins exempt from the long-running-command guard, alongside transcript viewers. Without the bypass, agent view entry in the live shared-session viewer path surfaces a "cannot enter agent view while a command is long running" error toast because the harness CLI block is classified as long-running.

`enter_agent_view_for_new_conversation` bypasses its "can start new conversation" check for `ThirdPartyCloudAgent` because the vehicle conversation is not a new conversation in the user-visible sense.

Telemetry: `TelemetryAgentViewEntryOrigin::ThirdPartyCloudAgent` is added and `From<AgentViewEntryOrigin>` maps the new variant (`app/src/server/telemetry/events.rs`).

### 2. Transcript viewer entry — direct call from `load_data_into_transcript_viewer`

`app/src/pane_group/mod.rs`

The `CloudConversationData::CLIAgent` branch maps `AIAgentHarness` → `warp_cli::agent::Harness`, restores the block snapshot, calls `set_harness` to keep the viewer's `AmbientAgentViewModel::harness` in sync with the loaded run, calls `enter_agent_view_for_new_conversation(None, ThirdPartyCloudAgent, ctx)`, and then calls `attach_non_startup_blocks_to_conversation(vehicle_conversation_id)` to retag the snapshot block. All four steps run inside a single `terminal_view.update` closure so the flow reads linearly.

### 3. Live shared-session viewer entry — `HarnessSelected` handler

`app/src/terminal/view/ambient_agent/view_impl.rs`

`maybe_enter_agent_view_for_shared_third_party_viewer` is called from the existing `HarnessSelected` arm of `handle_ambient_agent_event`. Guards:

- `!agent_view_state.is_active()` — idempotency; `HarnessSelected` can fire more than once.
- `is_third_party_harness()` — only 3p runs; oz is unchanged. This implicitly checks `FeatureFlag::AgentHarness`.
- `is_shared_ambient_agent_session()` — only the live shared-session context. Load-bearing: `HarnessSelected` also fires on the local spawner's harness selector dropdown, and there the REMOTE-1454 flow handles entry; the transcript viewer path calls the entry directly (§2) so we intentionally do not handle it here.

After entering agent view, it retags non-startup blocks via `attach_non_startup_blocks_to_conversation` and retags pre-existing rich content (setup-commands summary, tombstone, …) via `set_rich_content_agent_view_conversation_id`. Retagging rich content is necessary because items with `agent_view_conversation_id == None` are hidden in fullscreen agent view by `RichContentItem::should_hide_for_agent_view_state`.

### 4. `DispatchedAgent` short-circuit for viewer surfaces

`app/src/terminal/view/ambient_agent/view_impl.rs`

`DispatchedAgent` is only meaningful on the spawner's view. The handler now short-circuits for shared-ambient-session viewers and transcript viewers to make this explicit and avoid ever inserting cloud-mode rich content on a viewer.

### 5. Zero-state block suppression

`app/src/terminal/view.rs`

`ThirdPartyCloudAgent` is added to the `EnteredAgentView` handler's skip list alongside `CreateEnvironment` and `SlashInit`, so no `AgentViewZeroStateBlock` is inserted for 3p viewer entries.

### 6. Block visibility retag — `BlockList::attach_non_startup_blocks_to_conversation`

`app/src/terminal/model/blocks.rs`

Walks the blocks and calls `Block::add_attached_conversation_id(conversation_id)` on every block that isn't flagged `is_oz_environment_startup_command` (REMOTE-1454's setup command rows, which are hidden by their own mechanism). Skips blocks already tagged with this conversation as `origin_conversation_id`. Shares AI / agent-view rich content dirty bookkeeping with `set_agent_view_state` via the private `mark_agent_view_rich_content_dirty` helper.

### 7. Rich content retag — `TerminalView::set_rich_content_agent_view_conversation_id`

`app/src/terminal/view/agent_view.rs`

Generalized from the old `move_ai_block_to_agent_view_conversation`. Updates both the local `rich_content_views` entry and the block list's `update_agent_view_conversation_id_for_rich_content` so `should_hide_for_agent_view_state` picks up the new association. Existing call sites in `terminal/view.rs` are updated to use the new name.

## Testing and validation

Manual:

- **Claude / Gemini, live, shared session link** — start a run on one client, join from another via `warp://shared_session/{id}`. Confirm the joining client lands in agent view.
- **Claude / Gemini, live, management view and conversation list** — same via `OpenAmbientAgentSession`.
- **Claude / Gemini, completed, transcript viewer** — open a completed task from management view / conversation list. Confirm the transcript viewer opens in agent view with the block snapshot as the content.
- **Claude / Gemini, completed, conversation link** — same via `warp://conversation/{id}`.
- **Oz** — repeat the live and completed cases; confirm no regression on any branch we didn't touch.
- **`AgentHarness` disabled** — confirm `is_third_party_harness()` returns false and both new paths no-op.
- **Idempotency** — a run that spams `HarnessSelected` only enters agent view once.

Unit:

- `AgentViewEntryOrigin::ThirdPartyCloudAgent` does not trigger `AmbientAgentViewModel::enter_setup` / `enter_composing_from_setup` when passed to `enter_agent_view_internal`.
- `maybe_enter_agent_view_for_shared_third_party_viewer` no-ops when agent view is already active, when the harness is oz, and when the context is not a shared ambient agent session.
- `load_data_into_transcript_viewer` with a `CloudConversationData::CLIAgent` argument leaves the `AgentViewController` in an active state with the restored block still in the terminal block list.

## Risks and mitigations

- **`HarnessSelected` is multi-purpose**: fires both on viewer-side task-fetch resolution and on local spawner selector changes. The `is_shared_ambient_agent_session()` guard in `maybe_enter_agent_view_for_shared_third_party_viewer` is load-bearing.
- **Empty vehicle conversation in the conversation list**: the fresh Oz conversation appears in the user's conversation list until they exit agent view, at which point the standard empty-conversation cleanup in `ExitedAgentView` removes it. Since the vehicle has 0 exchanges, existing empty-conversation handling hides fork / copy-link affordances and title fallbacks are never consulted (pane title comes from the harness CLI's terminal-title escape).
- **Setup-command rendering on the live viewer**: REMOTE-1454's setup-command summary and pending-prompt block key off `is_cloud_agent_pre_first_exchange` and `is_third_party_harness`. Entering agent view does not change the exchange count or harness-command-started state, so no cross-interaction.

## Follow-ups

- Revisit whether `AgentViewEntryBlock` and `HistoricalCLIAgent` restore paths should also route into agent view for any future flows that materialize CLI agent conversations locally.
