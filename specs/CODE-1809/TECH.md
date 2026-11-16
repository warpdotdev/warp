# TECH: TUI tool-call permission requests (CODE-1809)

Linear: [CODE-1809](https://linear.app/warpdotdev/issue/CODE-1809/implement-permissions-requests-ui-for-tool-calls). Product behavior: [`PRODUCT.md`](./PRODUCT.md). Code references are pinned to commit `abea51cd1e102b363935f1b25ef03d335bc7b36f`.

## Context

The shared action and permission models already support blocking, accepting, rejecting, and executing every relevant tool call. The missing layer is TUI presentation and response handling:

- [`crates/warp_tui/src/conversation_selection.rs (20-58) @ abea51cd`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/warp_tui/src/conversation_selection.rs#L20-L58) creates the first TUI conversation with `is_autoexecute_override = true`; [`conversation_selection.rs (226-246)`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/warp_tui/src/conversation_selection.rs#L226-L246) repeats the override for subsequent conversations. This is the only TUI-specific permission-policy behavior.
- [`app/src/ai/execution_profiles/profiles.rs (154-170) @ abea51cd`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/ai/execution_profiles/profiles.rs#L154-L170) treats `LaunchMode::Tui` as an app-style client and resolves the same cloud-synced execution profile as the GUI.
- [`app/src/ai/blocklist/action_model.rs (62-149) @ abea51cd`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/ai/blocklist/action_model.rs#L62-L149) defines the shared action lifecycle. The front pending action reports `AIActionStatus::Blocked` ([`action_model.rs (615-645)`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/ai/blocklist/action_model.rs#L615-L645)); confirmation emits `ActionBlockedOnUserConfirmation` ([`action_model.rs (804-827)`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/ai/blocklist/action_model.rs#L804-L827)).
- Approval already routes through `BlocklistAIActionModel::execute_action`; rejection routes through the frontend-neutral `cancel_action_with_id` using `CancellationReason::ManuallyCancelled` ([`action_model.rs (1037-1066)`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/ai/blocklist/action_model.rs#L1037-L1066)). Edited shell commands use `handle_requested_command_accepted`, which mutates the same pending action and executes it ([`action_model.rs (1232-1268)`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/ai/blocklist/action_model.rs#L1232-L1268)).
- [`crates/warp_tui/src/option_selector.rs @ abea51cd`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/warp_tui/src/option_selector.rs) already provides numbered single-select rows, keyboard and mouse navigation, a custom-text footer editor, validation, and `Confirmed` / `CustomTextSubmitted` / `Dismissed` events. Ask-question and orchestration already use it for TUI-owned blocking interactions.
- [`crates/warp_tui/src/agent_block.rs (326-524) @ abea51cd`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/warp_tui/src/agent_block.rs#L326-L524) creates stateful child views for ask-question, file-edit, shell-command, plan, and orchestration actions. Other tool calls use the stateless fallback renderer in [`agent_block_sections.rs (85-105)`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/warp_tui/src/agent_block_sections.rs#L85-L105), which cannot own permission interaction state.
- [`crates/warp_tui/src/tui_shell_command_view.rs @ abea51cd`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/warp_tui/src/tui_shell_command_view.rs) renders an executed command from its terminal block. Before execution there is no block, so a blocked command falls back to a non-interactive label.
- [`crates/warp_tui/src/tui_file_edits_view.rs (1-269) @ abea51cd`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/warp_tui/src/tui_file_edits_view.rs#L1-L269) already owns a read-only, nested, collapsible pre-apply diff. `RequestFileEditsExecutor::preprocess_action` computes and stores candidate diffs before the action can block ([`request_file_edits.rs (203-329)`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/ai/blocklist/action_model/execute/request_file_edits.rs#L203-L329)); filesystem writes happen only from `TuiDiffStorage::start_saving` after approval ([`tui_diff_storage.rs (238-322)`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/warp_tui/src/tui_diff_storage.rs#L238-L322)).
- [`crates/warp_tui/src/terminal_session_view.rs (2239-2265) @ abea51cd`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/warp_tui/src/terminal_session_view.rs#L2239-L2265) sends normal TUI prompts through `BlocklistAIController::send_user_query_in_conversation`. The controller's `send_query` path cancels the active response and all pending actions with `CancellationReason::FollowUpSubmitted` before dispatching the new prompt ([`controller.rs (646-748)`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/ai/blocklist/controller.rs#L646-L748)).

## Proposed changes

### 1. Respect the active execution profile

Update `TuiConversationSelection` so every local TUI-created conversation starts with `AIConversationAutoexecuteMode::RespectUserSettings`:

- Pass `false` for the history model's `is_autoexecute_override` argument at initial and subsequent conversation creation.
- Set replacement `PendingQueryState::New` values to `RespectUserSettings`.
- Remove the stale TODO that says the TUI always enables Fast Forward.
- Leave `toggle_pending_query_autoexecute` intact but unwired; exposing Fast Forward is explicitly outside this ticket.

Add focused tests for initial creation, replacement after clearing/removal, and explicit new-conversation selection. These tests assert the selected conversation's override, not just the boolean passed by one call site.

### 2. Reusable permission prompt

Add a stateful `TuiPermissionPrompt` child view in `crates/warp_tui/src/tui_permission_prompt.rs`. It owns:

- The pending action ID.
- A `ViewHandle<TuiOptionSelector>`.
- The local prompt phase: option selection or Other-text editing. Shell body editing remains owned by the shell view.
- A keymap-context predicate active only while this action is the front-of-queue `Blocked` action.

The selector page uses two ordinary rows (`yes`, `no`) and an `OptionFooter::CustomText` entry labeled `Other`, producing the three numbered choices from Figma. `yes` is the initial `selected_id`; the page is non-searchable. Existing selector behavior supplies number keys, arrows, mouse interaction, validation, and custom-text editing.

The shared `render_permission_card` composition stretches to the transcript width, renders the title and body with distinct semantic background colors, inserts one blank row between tool-specific details and the options, and renders the prompt's keybinding hints below the tinted card. Generic, file-edit, and shell-command hosts provide only the title and tool-specific body.

The prompt emits a small host-facing event surface:

- `AcceptRequested`
- `RejectRequested`
- `ReplacementGuidanceSubmitted(String)`
- `EditBodyRequested` when Up is pressed from the first option and the host declares an editable body
- `LayoutChanged`

The host owns acceptance and rejection because it knows whether the action uses plain `execute_action` / `cancel_action_with_id`, edited shell text, or an executor-specific decision channel. `SuggestNewConversation`, for example, must complete its decision channel before executing rather than using plain action cancellation. Escape from the option phase emits `RejectRequested`; Escape from Other editing returns to the options through the selector's existing back behavior without resolving the action. The prompt does not know tool-specific body types and does not become a second action-state model: interactivity is derived from `AIActionStatus::Blocked`, and terminal state remains owned by `BlocklistAIActionModel`.

### 3. Blocking focus and event propagation

Generalize the orchestration-only blocker projection in `TuiAIBlock::active_blocking_child`, `TuiTranscriptView::active_blocking_child`, and `TuiTerminalSessionView::sync_blocker_focus`:

- Introduce a TUI-local blocker enum that can hold either a permission prompt or the existing orchestration block and exposes its view ID/focus operation.
- Specialized and generic tool views expose their retained permission-prompt child only while their action is blocked.
- Continue deriving the active blocker from the action model's front pending action, never transcript order or a stored boolean.
- Hand focus directly from one consecutive blocker to the next; restore input focus only after the queue has no active blocker.

Prompt layout changes bubble through the host view to `TuiAIBlockEvent::LayoutInvalidated`, preserving block-list height remeasurement. Blocking-state transitions continue to emit the existing transcript/session events.

Other guidance bubbles as a new event from permission prompt → host tool view → `TuiAIBlock` → `TuiTranscriptView` → `TuiTerminalSessionView`. The session calls `send_user_query_in_conversation` for the same conversation. It must not call `cancel_action_with_id` first: the shared prompt path atomically cancels the blocked and queued actions with `FollowUpSubmitted`, preserves the correct in-progress conversation outcome, and then sends the replacement guidance.

### 4. Generic permission-capable tool view

Add a retained `TuiGenericToolCallView` when the shared action model reports a blocked action without a specialized body. `TuiAIBlock::sync_action_views` derives this from `AIActionStatus::Blocked`, so permission policy remains shared with the GUI instead of being duplicated in a TUI classifier. Unblocked actions stay on the original stateless rendering path. The generic view lazily creates its editor-backed `TuiPermissionPrompt` only when the action actually reaches `Blocked`.

The view composes:

- A structured body renderer for each supported action variant, using the existing labels and semantic `TuiUiBuilder` styles.
- The shared permission prompt while blocked.
- The existing fallback terminal label after the action resolves.

Known confirmable actions receive purpose-built questions and details; any newly blocked action falls back to its shared user-friendly label. MCP arguments use structured, wrapped JSON/text consistent with the GUI request presentation; paths, queries, glob patterns, resource URIs, task summaries, and long-running-shell input render as labeled values. `SuggestNewConversation` maps Yes/No through `NewConversationDecision::Accept` / `Reject` before executing so its async result channel always resolves. Do not print Rust `Debug` output or introduce a TUI-specific secret-redaction policy.

Keeping this as one retained view avoids one stateful view per simple tool while ensuring every action that can block has an interactive path. Ask-question, RunAgents, plans, shell commands, and file edits retain their existing specialized views.

### 5. Editable shell-command permission request

Extend `TuiShellCommandView` with:

- A retained `TuiEditorView` seeded from the streamed `RequestCommandOutput.command`.
- Local phases for option selection versus body editing.
- The shared permission prompt rendered below the command field while blocked.

Before the user edits, streamed action updates continue to refresh the displayed command. Once body editing starts, the local editor becomes authoritative so later render synchronization cannot overwrite user text.

While options own focus, `yes` is selected. Up from `yes` or the displayed edit/save binding enters body editing. While the multiline editor owns focus, the selector remains visible without an active marker; Shift+Enter inserts a newline, while Enter, Down, or the edit/save binding commits the editor contents and returns focus to `yes` without executing. Approval calls `handle_requested_command_accepted` with the current editor text. Empty or whitespace-only command text keeps the request blocked and surfaces inline validation instead of calling the action model.

After approval, the view returns to its existing terminal-block rendering and long-running-command behavior. No separate command action or result is created; the shared action model mutates the same pending action before execution.

### 6. File-edit permission request

Keep the `RequestFileEditsExecutor`, `TuiDiffStorage`, `CodeEditorModel` diff pipeline, and save path unchanged. They already guarantee that candidate diffs are available before approval and that no file operation occurs before `execute_action`.

Extend `TuiFileEditsView` to retain and render the shared permission prompt below its existing body while blocked. Approval calls plain `execute_action`; rejection uses the host's `cancel_action_with_id` path; Other bubbles to the session follow-up path.

Make collapse the universal initial state for file edits:

- Change `SectionUiState` so newly materialized summary and per-file sections default to collapsed.
- Apply that default uniformly to blocked, autoexecuted, completed, and reconstructed file-edit views.
- Preserve independent summary/per-file overrides across action-status transitions and outer-group collapse/expansion.
- The options remain actionable whether the diff is collapsed or expanded.

The read-only `TuiEditorElement` configuration remains unchanged. A diff-application failure continues to use the existing fallback/failure result and never shows an empty preview as if it were reviewable.

### 7. Existing specialized flows

Do not route `AskUserQuestion` or `RunAgents` through the new generic prompt:

- Ask-question Other text remains an answer to that same tool call through `AskUserQuestionExecutor::complete`; it does not start a new user turn.
- RunAgents retains its configuration wizard and `execute_run_agents` / `deny_run_agents` semantics.

Their existing option-selector and blocking behavior remain reference implementations for keymap scoping, focus, layout invalidation, and tests.

## End-to-end flow

1. The shared executor preprocesses the action and evaluates it against the active execution profile.
2. An allowed action executes normally. A confirmation-required action reaches the front of the queue as `AIActionStatus::Blocked`.
3. `TuiAIBlock` resolves the retained specialized or generic child and the session focuses its permission prompt.
4. The user chooses:
   - Yes: the host executes the pending action, using edited command text only for shell commands.
   - No/Escape: the host rejects the action, normally with `ManuallyCancelled` and with an executor-specific decision where required.
   - Other: the text event reaches the session, whose normal prompt API preempts the active turn and pending queue with `FollowUpSubmitted`, then sends the guidance.
5. The action model emits the terminal or next-blocked transition; the transcript remeasures and focus advances to the next blocker or returns to the input.

## Testing and validation

All TUI unit tests follow the separate `_tests.rs` convention and use render-to-lines assertions where applicable.

- `conversation_selection_tests.rs`: PRODUCT 1–2; every TUI-created/replacement conversation uses `RespectUserSettings`.
- `tui_permission_prompt_tests.rs`: PRODUCT 4–15 and 40–42; default Yes selection, keyboard/mouse/number selection, No/Escape rejection, Other editing/validation/cancellation, stale-action deactivation, and consecutive blockers.
- Permission outcome tests verify No emits host rejection while Other emits only replacement guidance and leaves the action pending for the normal follow-up path to preempt; the prompt never performs a reject-then-send sequence.
- `tui_generic_tool_call_view_tests.rs`: PRODUCT 16–19; structured arguments, lazy prompt creation from an actual blocked action, and an executor-completion regression for accepted new-conversation suggestions.
- `tui_shell_command_view_tests.rs`: PRODUCT 20–26; streamed command seeding, Up/edit-save transitions, lossless command edits, empty validation, No/Other non-execution, and edited-text dispatch through `handle_requested_command_accepted`.
- `tui_file_edits_view_tests.rs` and `tui_diff_storage_tests.rs`: PRODUCT 27–37; pre-apply rendering, no writes before approval, universal single/multi-file collapsed defaults, independent nested toggles, acceptance while collapsed, state preservation, rejection without writes, and failure fallback.
- `agent_block_tests.rs`, `transcript_view_tests.rs`, and focused session tests: PRODUCT 7–8 and 40–42; generalized blocker discovery, focus handoff, event bubbling, height invalidation, and input-focus restoration.
- Existing ask-question and orchestration suites remain green for PRODUCT 38–39.

Run:

- `./script/format`
- `cargo nextest run -p warp_tui`
- Focused `warp` action-model/controller tests covering permissions, command mutation, cancellation, and follow-up submission
- Clippy for the changed `warp_tui` and `warp` targets with warnings denied
- `./script/presubmit` before submitting the PR

Manual verification uses `./script/run-tui` and the `tui-verify-change` workflow:

- Compare generic and editable-command states against both linked Figma nodes at 80×24 and narrow widths.
- Exercise Yes, No, Other, Escape, number keys, arrows, mouse selection, command editing, and consecutive requests.
- Verify single/multi-file pre-apply diffs, universal collapsed defaults for permission-gated and autoexecuted edits, nested collapse, acceptance while collapsed, preserved collapse state, create/delete/rename, and rejection leaving files unchanged.
- Verify profile modes that always allow, agent-decide, and always ask; confirm Fast Forward is not exposed by this change.

## Parallelization

Not proposed. The shared permission prompt, generalized blocker focus, event propagation, and `TuiAIBlock::sync_action_views` are the dependency spine for generic, shell, and file-edit work, and all touch the same small set of TUI ownership files. Splitting implementation before that spine lands would create merge conflicts and inconsistent interaction APIs. A separate agent may run the longer validation suites after the implementation compiles, but code ownership should remain with one implementer.

## Risks and mitigations

- **Other implemented as manual reject plus prompt submission:** this produces the wrong cancellation outcome and a visible `Cancelled` status transition. Mitigation: Other only calls the normal conversation prompt API; regression-test status continuity.
- **Blocked actions without retained views:** removing forced Fast Forward can expose new permission categories. Mitigation: derive generic view creation from the shared action model's actual blocked status, with shared-label fallback for unknown action types.
- **Keybinding collisions:** shell body editing, Other editing, option selection, transcript controls, and input editing share keys. Mitigation: one active-blocker predicate and explicit local phases; validate bindings with the existing TUI binding validators.
- **Overwriting edited shell text with streaming updates:** the streamed action may update after the view exists. Mitigation: streamed data seeds the editor only until the first user edit; local edited state then wins.
- **Changing existing file-edit presentation:** autoexecuted and reconstructed file edits will now begin collapsed instead of expanded. This is intentional and is reflected in the updated CODE-1800 product spec; render tests must cover the new uniform default.
- **Large permission bodies:** expanded multi-file diffs or MCP JSON can exceed the viewport. Mitigation: reuse existing transcript scrolling, wrapping, and nested collapse behavior; keep the option selector independently actionable.
- **Pre-apply file staleness:** the shared GUI/TUI diff pipeline does not revalidate the base file immediately before save. This ticket intentionally preserves shared behavior; failures continue through the existing save/result path.
