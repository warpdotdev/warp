# /continue-locally slash command — Tech Spec

Pairs with `PRODUCT.md`.

## Context
The cloud-to-local fork pipeline already exists. `WorkspaceAction::ForkAIConversation` (`app/src/workspace/action.rs:462-475`) — and its narrower sibling `WorkspaceAction::ContinueConversationLocally` (`app/src/workspace/action.rs:478-481`) — both land in `Workspace::fork_ai_conversation` (`app/src/workspace/view.rs:11460-11668`), which loads the source conversation, calls `BlocklistAIHistoryModel::fork_conversation`, opens the fork in the chosen `ForkedConversationDestination`, and ends with `Self::show_fork_toast` (`app/src/workspace/view.rs:11744-11772`) producing the `Forked "<title>"` toast referenced by PRODUCT.md invariant 7.

The two existing button entrypoints, both gated to Oz harness only, dispatch the narrower action:
- Conversation details panel: `app/src/ai/conversation_details_panel.rs:512-520, 1981-1992`, with the harness gate at `continue_locally_conversation_id` (`app/src/ai/conversation_details_panel.rs:564-602`).
- Conversation-ended tombstone: `app/src/terminal/view/shared_session/conversation_ended_tombstone_view.rs:485-520`, with the harness gate at `render_action_buttons` (`app/src/terminal/view/shared_session/conversation_ended_tombstone_view.rs:464-511`).

Slash commands: definitions in `app/src/search/slash_command_menu/static_commands/commands.rs`, dynamic per-session filtering in `app/src/terminal/input/slash_commands/data_source/mod.rs:146-235`, and execution dispatch in `app/src/terminal/input/slash_commands/mod.rs:296-847`. `Availability` is a `u8` bitfield with all 8 bits already in use (`app/src/search/slash_command_menu/static_commands/mod.rs:18-37`), so the cloud-Oz availability constraint must be expressed as a runtime filter — `/orchestrate` and `/feedback` already use this pattern at lines 211-225 of the data source. `/fork`'s handler (`app/src/terminal/input/slash_commands/mod.rs:718-742`) is the closest template: it pulls `conversation_id` via `ai_context_model.selected_conversation_id`, picks the destination from `trigger.is_cmd_or_ctrl_enter()`, and dispatches `ForkAIConversation`.

`AIConversation::task_id()` (`app/src/ai/agent/conversation.rs:718`) returns `Option<AmbientAgentTaskId>` for cloud-backed conversations. `AgentConversationsModel::get_task_data()` (`app/src/ai/agent_conversations_model.rs:1546-1548`) returns the in-memory `AmbientAgentTask`; the harness lives at `task.agent_config_snapshot.harness.harness_type` and defaults to `Harness::Oz` when the snapshot is present but no explicit harness is set (matching `enrich_from_task` in `conversation_ended_tombstone_view.rs:131-141`).

## Proposed changes

### 1. Static command definition
`app/src/search/slash_command_menu/static_commands/commands.rs`

Add a `CONTINUE_LOCALLY` `LazyLock<StaticCommand>` next to the rest of the fork family (`FORK`, `FORK_AND_COMPACT`, `FORK_FROM`):

- `name: "/continue-locally"`
- `description: "Continue this cloud conversation locally"`
- `icon_path: "bundled/svg/arrow-split.svg"` (same as `FORK`)
- `availability: Availability::AGENT_VIEW | Availability::ACTIVE_CONVERSATION | Availability::AI_ENABLED`
- `auto_enter_ai_mode: true`
- `argument: Some(Argument::optional().with_hint_text("<optional prompt to send in forked conversation>"))`

Push it onto `all_commands()` inside the existing `if !cfg!(target_family = "wasm")` block (`commands.rs:572-578`) alongside `FORK` / `FORK_AND_COMPACT`. No feature flag — the underlying action and all gating are stable.

### 2. Cloud-Oz runtime filter in the data source
`app/src/terminal/input/slash_commands/data_source/mod.rs`

Express PRODUCT.md invariant 2 as a runtime filter, mirroring `/orchestrate` and `/feedback`:

```rust path=null start=null
.filter(|(_, command)| {
    command.name != commands::CONTINUE_LOCALLY.name
        || self.active_conversation_is_cloud_oz(ctx)
})
```

Add a private helper `fn active_conversation_is_cloud_oz(&self, ctx: &AppContext) -> bool` that, in order:

1. Reads the active conversation id from `agent_view_controller.agent_view_state().active_conversation_id()` (returns false if `None`).
2. Looks up the `AIConversation` in `BlocklistAIHistoryModel`. Reads `conversation.task_id()`; returns false if `None` (local conversation).
3. Reads the task via `AgentConversationsModel::get_task_data(task_id)`.
4. Returns true iff the task's `agent_config_snapshot.harness.harness_type` is `Harness::Oz`, or the snapshot is absent / harness is unset (the same permissive default the tombstone uses, so we don't hide the command while the task fetch is still pending — `Some(Harness::Claude)` / `Some(Harness::Gemini)` are the only states that hide the command).

Subscribe `recompute_active_commands` to:
- `AgentConversationsModelEvent::TasksUpdated` — so the menu updates as the task fetch resolves and as task harness becomes known.
- `BlocklistAIHistoryEvent::SetActiveConversation` — so the menu updates when the active conversation switches (e.g. user navigates between cloud and local conversations).

Both event types are already routed to other consumers; the subscription pattern follows the existing `ctx.subscribe_to_model` calls at `data_source/mod.rs:72-127`.

### 3. Slash command handler
`app/src/terminal/input/slash_commands/mod.rs`

Add a handler arm in `Input::execute_slash_command_action` (next to the existing `/fork` arm at `slash_commands/mod.rs:718-742`):

```rust path=null start=null
continue_locally if command.name == commands::CONTINUE_LOCALLY.name => {
    let Some(conversation_id) = self
        .ai_context_model
        .as_ref(ctx)
        .selected_conversation_id(ctx)
    else {
        show_error_toast("/continue-locally requires an active conversation".to_owned(), ctx);
        return true;
    };

    if !active_conversation_is_cloud_oz(conversation_id, ctx) {
        show_error_toast(
            "/continue-locally requires an active cloud Oz conversation".to_owned(),
            ctx,
        );
        return true;
    }

    let destination = if trigger.is_cmd_or_ctrl_enter() {
        ForkedConversationDestination::NewTab
    } else {
        ForkedConversationDestination::SplitPane
    };

    send_telemetry_from_ctx!(
        AgentManagementTelemetryEvent::SlashCommandContinueLocally,
        ctx
    );

    ctx.dispatch_typed_action(&WorkspaceAction::ForkAIConversation {
        conversation_id,
        fork_from_exchange: None,
        summarize_after_fork: false,
        summarization_prompt: None,
        initial_prompt: argument.cloned(),
        destination,
    });
}
```

`active_conversation_is_cloud_oz(conversation_id, ctx)` is a small private free function in the same module that performs the same harness lookup as the data source helper, factored to take an explicit `conversation_id` (the data source uses the agent view's active id; the handler already has it from `selected_conversation_id`). It is the defensive runtime gate from PRODUCT.md invariant 8.

The handler is gated `cfg(not(target_family = "wasm"))`. Empty/whitespace `argument` is normalized to `None` by `ForkAIConversation`'s downstream `fork_ai_conversation` (`app/src/workspace/view.rs:11483-11490`), so PRODUCT.md invariant 4 is satisfied without further work here.

Success toasts (PRODUCT.md invariant 7) and error toasts on fork failure (invariant 8) are produced by `Workspace::show_fork_toast` and the existing failure paths inside `fork_ai_conversation` — no toast plumbing in the slash command handler itself.

### 4. Footer tip
`app/src/ai/blocklist/agent_view/agent_message_bar.rs`

Extend `ForkSlashCommandMessageProducer::produce_message` (lines 727-777) to also match `commands::CONTINUE_LOCALLY.name`. Add it to the existing equality check and reuse the `/fork` label branch (`(" new pane", " new tab")`), since `/continue-locally` follows the same `Enter → split pane`, `Cmd/Ctrl+Enter → new tab` mapping.

### 5. Telemetry
`app/src/ai/agent_management/telemetry.rs`

Add a new variant `SlashCommandContinueLocally` to `AgentManagementTelemetryEvent` (`telemetry.rs:108-113`), wasm-gated like the existing `TombstoneContinueLocally` / `DetailsPanelContinueLocally`, with empty payload, `EnablementState::Always`, and the discriminant entries:

- Name: `"AgentManagement.SlashCommandContinueLocally"`
- Description: `"User invoked /continue-locally to fork a cloud conversation locally"`

Fire it from the slash command handler immediately before dispatching `ForkAIConversation` (see §3). The existing `ConversationForked` event is fired downstream by the fork pipeline and does not need to be touched.

## Testing and validation

References below are to PRODUCT.md numbered invariants.

Unit tests, colocated with the modules being changed:

- `app/src/search/slash_command_menu/static_commands/mod_test.rs`: add a registration test for `/continue-locally` covering name, icon, optional argument with hint text, `auto_enter_ai_mode: true`, and the static availability set. (Inv. 1)
- `app/src/terminal/input/slash_commands/data_source/` tests (matching the existing layout): cover the four cloud-Oz filter outcomes — local conversation (no `task_id`) hides the command (Inv. 2); cloud Oz task shows it (Inv. 2, 3); cloud Claude/Gemini task hides it (Inv. 2); cloud task whose data hasn't been fetched yet shows it permissively and recomputes on `TasksUpdated` (Inv. 2). Plus AI-disabled hides it via the static `AI_ENABLED` requirement (Inv. 3).
- `app/src/ai/agent_management/telemetry.rs` discriminant tests: assert the new variant maps to the expected name and description, mirroring the existing test pattern for `DetailsPanelContinueLocally`. (Inv. 9)

Manual:

- Cloud Oz run, in-progress, agent input visible: `/continue-locally` appears in the slash menu; Enter forks into a split pane and shows `Forked "<title>"`; Cmd-Enter forks into a new tab. (Inv. 2, 3, 5, 6, 7)
- Same run, type `/continue-locally do X next`, press Enter: forked conversation receives `do X next` as its first user query. (Inv. 4, 5, 6)
- Source-load failure (simulate by clearing the conversation from history before pressing Enter): error toast surfaces with the existing copy. (Inv. 8)
- Cloud Oz run viewed via shared session, completed but input still visible: command works as above. (Inv. 3)
- Cloud Oz run viewed via completed transcript viewer: input is hidden; command is unreachable, existing tombstone button still works. (Inv. 2, Non-goals)
- Cloud Claude run: command absent from menu. (Inv. 2)
- Local conversation (no `task_id`): command absent from menu. (Inv. 2)

Run `./script/presubmit` before opening the PR.

## Follow-ups
- If the menu visibility lag during the initial task fetch becomes noticeable in practice, switch the permissive `None` harness branch to fetch on demand via `AgentConversationsModel::get_or_async_fetch_task_data` (`app/src/ai/agent_conversations_model.rs:1564-1645`) and rely on the `TasksUpdated` recompute. We default to permissive for parity with the tombstone today.
