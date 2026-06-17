//! Multi-prompt queue panel rendered between the warping indicator and the input editor in
//! [`TerminalView`].
//!
//! Reads from the `QueuedQueryModel` singleton (keyed by `AIConversationId`) for the queue of the
//! currently-active conversation in its parent terminal view, looked up via
//! [`BlocklistAIHistoryModel::active_conversation_id`]. Tracks panel-only UI state (collapse,
//! hover, drag) locally. Emits high-level events for immediate submission, deletion, and edit
//! completion, which the host uses to submit or update the input editor.
use std::collections::HashMap;

use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;
use warp_core::features::FeatureFlag;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig};
use warpui::elements::{
    Border, ChildAnchor, ChildView, Clipped, ClippedScrollStateHandle, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, DragAxis, Draggable, DraggableState, Empty, Expanded, Fill,
    Flex, Hoverable, MinSize, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Radius, SavePosition, ScrollbarWidth, Stack, Text,
    DEFAULT_UI_LINE_HEIGHT_RATIO,
};
use warpui::fonts::{Properties, Style, Weight};
use warpui::keymap::Keystroke;
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, BlurContext, Element, Entity, EntityId, FocusContext, ModelHandle, SingletonEntity,
    TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::agent_view::shortcuts::render_keystroke_with_color_overrides;
use crate::ai::blocklist::block::cli_controller::{CLISubagentController, CLISubagentEvent};
use crate::ai::blocklist::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, QueuedQueryEvent, QueuedQueryId,
    QueuedQueryModel, QueuedQueryOrigin,
};
use crate::appearance::Appearance;
use crate::editor::{
    EditorOptions, EditorView, Event as EditorEvent, PropagateAndNoOpEscapeKey,
    PropagateAndNoOpNavigationKeys, PropagateHorizontalNavigationKeys, TextOptions,
};
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::TelemetryEvent;
use crate::terminal::cli_agent_sessions::{CLIAgentSessionsModel, CLIAgentSessionsModelEvent};
use crate::terminal::input::suggestions_mode_model::InputSuggestionsModeModel;
use crate::ui_components::icons::Icon as TerminalIcon;
use crate::util::truncation::truncate_from_end;
use crate::view_components::action_button::{ActionButton, ButtonSize, NakedTheme};

const MAX_PROMPT_LINES: f32 = 5.;
/// Max characters shown in a row's single-line preview before truncation.
const PROMPT_PREVIEW_MAX_CHARS: usize = 500;
const INITIAL_CLOUD_MODE_PROMPT_TOOLTIP: &str = "The first cloud-mode prompt cannot be changed.";
const SEND_NOW_DURING_CLOUD_SETUP_TOOLTIP: &str =
    "Prompts cannot be sent until environment setup is complete.";
const SEND_NOW_TO_FULL_TERMINAL_USE_AGENT_TOOLTIP: &str = "Send to full terminal use agent";
const SEND_NOW_AS_READ_ONLY_VIEWER_TOOLTIP: &str = "Read-only viewers cannot send prompts.";

/// Returns the position-cache id used to look up a row's bounding rect during a drag.
/// Indexed by the row's current visual index so swaps maintain stable lookups.
fn queue_row_position_id(panel_view_id: EntityId, index: usize) -> String {
    format!("queued_prompts_panel:{panel_view_id:?}:row:{index}")
}

fn build_row_state(
    query_id: QueuedQueryId,
    origin: QueuedQueryOrigin,
    text: &str,
    ctx: &mut ViewContext<QueuedPromptsPanelView>,
) -> QueuedPromptRowState {
    let is_initial_cloud_mode_prompt = origin == QueuedQueryOrigin::InitialCloudMode;
    // The send-now tooltip is owned by `update_send_now_availability`, which swaps in a
    // "wait for the cloud agent" message while send-now is disabled; "Send now" is the default.
    let (edit_tooltip, delete_tooltip) = if is_initial_cloud_mode_prompt {
        (
            INITIAL_CLOUD_MODE_PROMPT_TOOLTIP,
            INITIAL_CLOUD_MODE_PROMPT_TOOLTIP,
        )
    } else {
        ("Edit", "Delete")
    };

    let send_now_button = ctx.add_typed_action_view(move |_| {
        ActionButton::new("", NakedTheme)
            .with_icon(TerminalIcon::ArrowUp)
            .with_tooltip("Send now")
            .with_size(ButtonSize::XSmall)
            .with_disabled_theme(NakedTheme)
            .on_click(move |ctx| {
                ctx.dispatch_typed_action(QueuedPromptsPanelAction::SendNow(query_id));
            })
    });
    let edit_button = ctx.add_typed_action_view(move |_| {
        ActionButton::new("", NakedTheme)
            .with_icon(TerminalIcon::Pencil)
            .with_tooltip(edit_tooltip)
            .with_size(ButtonSize::XSmall)
            .with_disabled_theme(NakedTheme)
            .on_click(move |ctx| {
                ctx.dispatch_typed_action(QueuedPromptsPanelAction::StartEditingRow(query_id));
            })
    });
    let delete_button = ctx.add_typed_action_view(move |_| {
        ActionButton::new("", NakedTheme)
            .with_icon(TerminalIcon::Trash)
            .with_tooltip(delete_tooltip)
            .with_size(ButtonSize::XSmall)
            .with_disabled_theme(NakedTheme)
            .on_click(move |ctx| {
                ctx.dispatch_typed_action(QueuedPromptsPanelAction::DeleteRow(query_id));
            })
    });

    if is_initial_cloud_mode_prompt {
        edit_button.update(ctx, |button, ctx| button.set_disabled(true, ctx));
        delete_button.update(ctx, |button, ctx| button.set_disabled(true, ctx));
    }

    QueuedPromptRowState {
        preview_text: truncate_from_end(
            &text.lines().collect::<Vec<_>>().join(" "),
            PROMPT_PREVIEW_MAX_CHARS,
        ),
        mouse_state: MouseStateHandle::default(),
        drag_handle_tooltip_state: MouseStateHandle::default(),
        send_now_button,
        edit_button,
        delete_button,
        draggable_state: DraggableState::default(),
    }
}

#[derive(Clone)]
struct QueuedPromptRowState {
    /// Cached single-line preview; refreshed only when the row's text changes.
    preview_text: String,
    mouse_state: MouseStateHandle,
    drag_handle_tooltip_state: MouseStateHandle,
    send_now_button: ViewHandle<ActionButton>,
    edit_button: ViewHandle<ActionButton>,
    delete_button: ViewHandle<ActionButton>,
    draggable_state: DraggableState,
}

/// View for the multi-prompt queue panel.
pub struct QueuedPromptsPanelView {
    view_id: EntityId,
    /// Terminal view this panel belongs to. Used to resolve the active conversation via
    /// [`BlocklistAIHistoryModel`].
    terminal_view_id: EntityId,
    /// Input's suggestions-mode model. Used by [`Self::should_render`] to hide the panel while an
    /// inline menu (slash commands, model selector, etc.) is open.
    suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
    /// Cached active conversation for this panel. `None` means there is no active conversation in
    /// the parent terminal view; the panel renders nothing in that case.
    active_conversation_id: Option<AIConversationId>,
    /// Reusable editor for whichever row is currently in edit mode.
    edit_editor: ViewHandle<EditorView>,
    edit_editor_is_single_logical_line: bool,
    edit_editor_scroll_state: ClippedScrollStateHandle,
    /// Panel-only UI state: whether the body is collapsed. Owned here (not on the singleton)
    /// because no other view reads this. Reset whenever the active conversation changes or the
    /// queue is cleared.
    collapsed: bool,
    /// Host-pushed: whether this terminal can send prompts at all (false for read-only
    /// shared-session viewers). Gates the send-now buttons, empty-Enter sends, and the hint.
    can_send_prompt: bool,
    /// Host input's editor. An empty input is what makes Enter send the top queued row, so
    /// Enter-send and hint decisions read its emptiness live.
    host_editor: ViewHandle<EditorView>,
    /// Last observed emptiness of `host_editor`; only damps re-render notifications to
    /// empty <-> non-empty transitions. Decisions always read the editor live.
    host_editor_was_empty: bool,
    header_mouse_state: MouseStateHandle,
    row_states: HashMap<QueuedQueryId, QueuedPromptRowState>,
    dragging_query_id: Option<QueuedQueryId>,
    drag_start_index: Option<usize>,
    /// Controller for the active long-running-command subagent (the "full terminal use agent").
    /// Used to retarget the send-now tooltip while that subagent is in control.
    cli_subagent_controller: ModelHandle<CLISubagentController>,
}

#[derive(Clone, Debug)]
pub enum QueuedPromptsPanelAction {
    ToggleCollapsed,
    SendNow(QueuedQueryId),
    StartEditingRow(QueuedQueryId),
    DeleteRow(QueuedQueryId),
    StartDrag(QueuedQueryId),
    DragMoved { rect: RectF },
    DropEnd,
}

/// Events emitted to the host input view.
#[derive(Clone, Debug)]
pub enum QueuedPromptsPanelEvent {
    /// A row's send-now button was clicked. The row is left in the queue so the host can dispatch
    /// it according to its kind, read prompt attachments by id, and remove it after dispatch.
    SendNow {
        conversation_id: AIConversationId,
        query_id: QueuedQueryId,
        text: String,
        is_command: bool,
    },
    /// A row was deleted via the trash button. The host should refocus the input.
    RowDeleted,
    /// An inline edit was committed or cancelled. The host should refocus the input.
    EditEnded,
}

impl Entity for QueuedPromptsPanelView {
    type Event = QueuedPromptsPanelEvent;
}

impl QueuedPromptsPanelView {
    pub fn new(
        terminal_view_id: EntityId,
        suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
        cli_subagent_controller: ModelHandle<CLISubagentController>,
        host_editor: ViewHandle<EditorView>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let edit_editor = build_edit_editor(ctx);

        ctx.subscribe_to_view(&edit_editor, |me, _, event, ctx| {
            me.handle_edit_editor_event(event, ctx);
        });

        // The header hint hides while the CLI-agent rich input is open (Enter submits to the
        // CLI agent there), so re-render when it opens or closes.
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), |me, _, event, ctx| {
            me.handle_cli_agent_sessions_event(event, ctx);
        });

        // Enter-send and the header hint depend on the host input's emptiness, which is read
        // live from `host_editor`; re-render when the buffer transitions between empty and
        // non-empty.
        ctx.subscribe_to_view(&host_editor, |me, _, event, ctx| {
            me.handle_host_editor_event(event, ctx);
        });

        let history_handle = BlocklistAIHistoryModel::handle(ctx);
        let active_conversation_id = history_handle
            .as_ref(ctx)
            .active_conversation_id(terminal_view_id);

        ctx.subscribe_to_model(&history_handle, move |me, _, event, ctx| {
            me.handle_history_event(event, ctx);
        });

        ctx.subscribe_to_model(&QueuedQueryModel::handle(ctx), |me, _, event, ctx| {
            me.handle_queued_query_event(event, ctx);
        });

        ctx.subscribe_to_model(&cli_subagent_controller, |me, _, event, ctx| {
            me.handle_cli_subagent_event(event, ctx);
        });

        let host_editor_was_empty = host_editor.as_ref(ctx).is_empty(ctx);
        let mut me = Self {
            view_id: ctx.view_id(),
            terminal_view_id,
            suggestions_mode_model,
            active_conversation_id,
            edit_editor,
            edit_editor_is_single_logical_line: true,
            edit_editor_scroll_state: Default::default(),
            collapsed: false,
            can_send_prompt: true,
            host_editor,
            host_editor_was_empty,
            header_mouse_state: MouseStateHandle::default(),
            row_states: HashMap::new(),
            dragging_query_id: None,
            drag_start_index: None,
            cli_subagent_controller,
        };
        if let Some(conv_id) = active_conversation_id {
            me.seed_row_states_for(conv_id, ctx);
        }
        me
    }

    fn clear_drag_state(&mut self) {
        self.dragging_query_id = None;
        self.drag_start_index = None;
    }

    /// Updates whether this terminal can send prompts (false for read-only shared-session
    /// viewers). Pushed by the host on construction and when the shared-session role changes.
    pub fn set_can_send_prompt(&mut self, can_send_prompt: bool, ctx: &mut ViewContext<Self>) {
        if self.can_send_prompt == can_send_prompt {
            return;
        }
        self.can_send_prompt = can_send_prompt;
        self.update_send_now_availability(ctx);
        ctx.notify();
    }

    /// True when pressing Enter in the host input should send the top queued row instead of
    /// performing its usual action: the panel is showing, prompts can be sent, the input is
    /// empty (read live from the host editor, so the decision cannot trail same-update buffer
    /// changes), and the CLI-agent rich input is closed (Enter submits to the CLI agent there).
    pub fn enter_sends_queued_prompt(&self, ctx: &AppContext) -> bool {
        self.should_render(ctx)
            && self.can_send_prompt
            && self.host_editor.as_ref(ctx).is_empty(ctx)
            && !CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id)
    }

    /// Whether the header shows the "⏎ to send" hint: Enter would send, no row is in inline
    /// edit mode, and the head row is sendable (not the locked initial cloud-mode prompt).
    fn should_show_enter_hint(&self, ctx: &AppContext) -> bool {
        let Some(conv_id) = self.active_conversation_id else {
            return false;
        };
        let queue_model = QueuedQueryModel::as_ref(ctx);
        self.enter_sends_queued_prompt(ctx)
            && queue_model.editing_row(conv_id).is_none()
            && queue_model
                .queue(conv_id)
                .first()
                .is_some_and(|row| !row.is_locked())
    }

    /// Re-renders when the host input transitions between empty and non-empty, so the header
    /// hint tracks whether Enter would send.
    fn handle_host_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        if !matches!(event, EditorEvent::Edited(_) | EditorEvent::BufferReplaced) {
            return;
        }
        let is_empty = self.host_editor.as_ref(ctx).is_empty(ctx);
        if is_empty != self.host_editor_was_empty {
            self.host_editor_was_empty = is_empty;
            ctx.notify();
        }
    }

    /// Re-renders the header hint when the CLI-agent rich input opens or closes for this
    /// terminal.
    fn handle_cli_agent_sessions_event(
        &mut self,
        event: &CLIAgentSessionsModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let CLIAgentSessionsModelEvent::InputSessionChanged {
            terminal_view_id, ..
        } = event
        else {
            return;
        };
        if *terminal_view_id == self.terminal_view_id {
            ctx.notify();
        }
    }

    /// Reseed `row_states` for `conv_id`'s queue, dropping any state for rows not in that queue.
    fn seed_row_states_for(&mut self, conv_id: AIConversationId, ctx: &mut ViewContext<Self>) {
        let rows: Vec<(QueuedQueryId, QueuedQueryOrigin, String)> = QueuedQueryModel::as_ref(ctx)
            .queue(conv_id)
            .iter()
            .map(|q| (q.id(), q.origin(), q.text().to_owned()))
            .collect();
        let row_ids: Vec<QueuedQueryId> = rows.iter().map(|(id, _, _)| *id).collect();
        self.row_states.retain(|id, _| row_ids.contains(id));
        for (id, origin, text) in rows {
            self.row_states
                .entry(id)
                .or_insert_with(|| build_row_state(id, origin, &text, ctx));
        }
        self.update_send_now_availability(ctx);
    }

    /// Updates each row's "send now" button: disabled, with a tooltip explaining the wait, for the
    /// locked initial cloud-mode prompt and for every row while that locked row sits at the head of
    /// the queue — i.e. while the cloud environment is still setting up, with no live agent yet to
    /// receive an immediate submission. When a long-running-command subagent (the "full terminal
    /// use agent") is in control, the enabled tooltip explains that send-now targets that subagent.
    /// Otherwise it is enabled with the default "Send now" tooltip.
    fn update_send_now_availability(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(conv_id) = self.active_conversation_id else {
            return;
        };

        let rows: Vec<(QueuedQueryId, QueuedQueryOrigin)> = QueuedQueryModel::as_ref(ctx)
            .queue(conv_id)
            .iter()
            .map(|query| (query.id(), query.origin()))
            .collect();
        let cloud_setup_in_progress = rows
            .first()
            .is_some_and(|(_, origin)| *origin == QueuedQueryOrigin::InitialCloudMode);
        let lrc_subagent_in_progress = self
            .cli_subagent_controller
            .as_ref(ctx)
            .is_agent_in_control();
        for (query_id, origin) in &rows {
            let Some(send_now_button) = self
                .row_states
                .get(query_id)
                .map(|state| state.send_now_button.clone())
            else {
                continue;
            };
            let disabled_for_cloud_setup =
                *origin == QueuedQueryOrigin::InitialCloudMode || cloud_setup_in_progress;
            let disabled = disabled_for_cloud_setup || !self.can_send_prompt;
            let tooltip = if disabled_for_cloud_setup {
                SEND_NOW_DURING_CLOUD_SETUP_TOOLTIP
            } else if !self.can_send_prompt {
                SEND_NOW_AS_READ_ONLY_VIEWER_TOOLTIP
            } else if lrc_subagent_in_progress {
                SEND_NOW_TO_FULL_TERMINAL_USE_AGENT_TOOLTIP
            } else {
                "Send now"
            };
            send_now_button.update(ctx, |button, ctx| {
                button.set_disabled(disabled, ctx);
                button.set_tooltip(Some(tooltip), ctx);
            });
        }
    }

    /// Recomputes send-now availability when the long-running-command subagent's control state
    /// changes, so the send-now tooltip stays in sync with whether the full terminal use agent
    /// is currently in control.
    fn handle_cli_subagent_event(&mut self, event: &CLISubagentEvent, ctx: &mut ViewContext<Self>) {
        match event {
            CLISubagentEvent::SpawnedSubagent { .. }
            | CLISubagentEvent::FinishedSubagent { .. }
            | CLISubagentEvent::UpdatedControl { .. }
            | CLISubagentEvent::ControlHandedBackAfterTransfer => {
                self.update_send_now_availability(ctx);
            }
            CLISubagentEvent::UpdatedLastSnapshot | CLISubagentEvent::ToggledHideResponses => {}
        }
    }

    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let is_for_this_view = event
            .terminal_view_id()
            .is_some_and(|id| id == self.terminal_view_id);
        if !is_for_this_view {
            return;
        }
        let new_active =
            BlocklistAIHistoryModel::as_ref(ctx).active_conversation_id(self.terminal_view_id);
        if new_active != self.active_conversation_id {
            self.active_conversation_id = new_active;
            self.row_states.clear();
            self.clear_drag_state();
            self.collapsed = false;
            if let Some(conv_id) = new_active {
                self.seed_row_states_for(conv_id, ctx);
            }
            ctx.notify();
        }
    }

    fn handle_queued_query_event(&mut self, event: &QueuedQueryEvent, ctx: &mut ViewContext<Self>) {
        let Some(active_conv_id) = self.active_conversation_id else {
            return;
        };
        // Filter every event to the panel's current active conversation. Other conversations'
        // events are still emitted on the singleton but are not relevant to this panel.
        let event_conv_id = match event {
            QueuedQueryEvent::Appended {
                conversation_id, ..
            }
            | QueuedQueryEvent::Removed {
                conversation_id, ..
            }
            | QueuedQueryEvent::Reordered { conversation_id }
            | QueuedQueryEvent::EditEntered {
                conversation_id, ..
            }
            | QueuedQueryEvent::EditCommitted {
                conversation_id, ..
            }
            | QueuedQueryEvent::EditCancelled {
                conversation_id, ..
            }
            | QueuedQueryEvent::Cleared { conversation_id }
            | QueuedQueryEvent::QueueNextPromptToggled { conversation_id } => *conversation_id,
            // The queue panel doesn't display the auto-queue toggle state, so a
            // change to the cached default doesn't affect what it renders.
            QueuedQueryEvent::DefaultModeChanged => return,
        };
        if event_conv_id != active_conv_id {
            return;
        }
        match event {
            QueuedQueryEvent::Removed { query_id, .. } => {
                self.row_states.remove(query_id);
                if self.dragging_query_id == Some(*query_id) {
                    self.clear_drag_state();
                }
                if !QueuedQueryModel::as_ref(ctx).has_queue(active_conv_id) {
                    self.collapsed = false;
                }
                // Removing the locked initial cloud-mode row re-enables the remaining rows.
                self.update_send_now_availability(ctx);
            }
            QueuedQueryEvent::EditEntered { query_id, .. } => {
                let initial_text = QueuedQueryModel::as_ref(ctx)
                    .queue(active_conv_id)
                    .iter()
                    .find(|row| row.id() == *query_id)
                    .map(|row| row.text().to_owned())
                    .unwrap_or_default();
                self.edit_editor_is_single_logical_line = !initial_text.contains('\n');
                self.edit_editor.update(ctx, |editor, ctx| {
                    editor.system_reset_buffer_text(&initial_text, ctx);
                });
                ctx.focus(&self.edit_editor);
            }
            QueuedQueryEvent::EditCommitted { query_id, .. } => {
                self.edit_editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer(ctx);
                });

                // The row's text changed, so refresh its cached preview.
                let row = QueuedQueryModel::as_ref(ctx)
                    .queue(active_conv_id)
                    .iter()
                    .find(|row| row.id() == *query_id);
                if let (Some(row), Some(state)) = (row, self.row_states.get_mut(query_id)) {
                    state.preview_text = truncate_from_end(
                        &row.text().lines().collect::<Vec<_>>().join(" "),
                        PROMPT_PREVIEW_MAX_CHARS,
                    );
                }
            }
            QueuedQueryEvent::EditCancelled { .. } => {
                self.edit_editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer(ctx);
                });
            }
            QueuedQueryEvent::Cleared { .. } => {
                self.row_states.clear();
                self.clear_drag_state();
                self.collapsed = false;
            }
            QueuedQueryEvent::Appended { query_id, .. } => {
                // The row could be gone if the append+remove pair were both delivered
                // before we observed the append (e.g. fast /queue -> drain). Skip row
                // state init in that case; the matching Removed event already cleaned up.
                if let Some((origin, text)) = QueuedQueryModel::as_ref(ctx)
                    .queue(active_conv_id)
                    .iter()
                    .find(|row| row.id() == *query_id)
                    .map(|row| (row.origin(), row.text().to_owned()))
                {
                    self.row_states
                        .entry(*query_id)
                        .or_insert_with(|| build_row_state(*query_id, origin, &text, ctx));
                }
                // A new row queued while the locked initial row is present must start disabled.
                self.update_send_now_availability(ctx);
            }
            QueuedQueryEvent::Reordered { .. }
            | QueuedQueryEvent::QueueNextPromptToggled { .. }
            | QueuedQueryEvent::DefaultModeChanged => {}
        }
        ctx.notify();
    }

    fn handle_edit_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Enter => self.commit_edit(ctx),
            EditorEvent::Escape => self.cancel_edit(ctx),
            // Losing focus commits the edit.
            EditorEvent::Blurred => self.commit_edit(ctx),
            EditorEvent::Edited(_) | EditorEvent::BufferReplaced => {
                self.update_edit_editor_line_state(ctx)
            }
            _ => {}
        }
    }

    fn update_edit_editor_line_state(&mut self, ctx: &mut ViewContext<Self>) {
        let is_single_logical_line = self
            .edit_editor
            .read(ctx, |editor, ctx| !editor.buffer_text(ctx).contains('\n'));
        if self.edit_editor_is_single_logical_line != is_single_logical_line {
            self.edit_editor_is_single_logical_line = is_single_logical_line;
            ctx.notify();
        }
    }

    fn editing_row_id(&self, ctx: &AppContext) -> Option<QueuedQueryId> {
        let conv_id = self.active_conversation_id?;
        QueuedQueryModel::as_ref(ctx).editing_row(conv_id)
    }

    fn commit_edit(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(conv_id) = self.active_conversation_id else {
            return;
        };
        let Some(query_id) = self.editing_row_id(ctx) else {
            return;
        };
        let origin = QueuedQueryModel::as_ref(ctx)
            .queue(conv_id)
            .iter()
            .find(|row| row.id() == query_id)
            .map(|row| row.origin());
        let new_text = self
            .edit_editor
            .read(ctx, |editor, ctx| editor.buffer_text(ctx).trim().to_owned());
        let was_empty = new_text.is_empty();
        QueuedQueryModel::handle(ctx).update(ctx, |model, ctx| {
            model.commit_edit(conv_id, new_text, ctx);
        });
        if let Some(origin) = origin {
            if !was_empty {
                send_telemetry_from_ctx!(
                    TelemetryEvent::QueuedPromptEdited {
                        origin: origin.into(),
                    },
                    ctx
                );
            }
        }
        ctx.emit(QueuedPromptsPanelEvent::EditEnded);
    }

    fn cancel_edit(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(conv_id) = self.active_conversation_id else {
            return;
        };
        if self.editing_row_id(ctx).is_none() {
            return;
        }
        QueuedQueryModel::handle(ctx).update(ctx, |model, ctx| {
            model.cancel_edit(conv_id, ctx);
        });
        ctx.emit(QueuedPromptsPanelEvent::EditEnded);
    }

    /// Visibility predicate used by the host to decide whether to render the panel.
    pub fn should_render(&self, ctx: &AppContext) -> bool {
        if !FeatureFlag::QueueSlashCommand.is_enabled() {
            return false;
        }
        if self
            .suggestions_mode_model
            .as_ref(ctx)
            .is_inline_menu_open()
        {
            return false;
        }
        let Some(conv_id) = self.active_conversation_id else {
            return false;
        };
        QueuedQueryModel::as_ref(ctx).has_queue(conv_id)
    }
}

#[cfg(test)]
impl QueuedPromptsPanelView {
    /// Test accessor: whether the "send now" button for `query_id` is currently disabled.
    pub(super) fn send_now_button_disabled_for_test(
        &self,
        query_id: QueuedQueryId,
        ctx: &AppContext,
    ) -> Option<bool> {
        self.row_states
            .get(&query_id)
            .map(|state| state.send_now_button.as_ref(ctx).is_disabled())
    }

    /// Test accessor: whether the header currently shows the "⏎ to send" hint.
    pub(super) fn enter_hint_shown_for_test(&self, ctx: &AppContext) -> bool {
        self.should_show_enter_hint(ctx)
    }
}

impl TypedActionView for QueuedPromptsPanelView {
    type Action = QueuedPromptsPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        let Some(conv_id) = self.active_conversation_id else {
            return;
        };
        match action {
            QueuedPromptsPanelAction::ToggleCollapsed => {
                self.collapsed = !self.collapsed;
                send_telemetry_from_ctx!(
                    TelemetryEvent::QueuedPromptPanelCollapseToggled {
                        collapsed: self.collapsed,
                    },
                    ctx
                );
                ctx.notify();
            }
            QueuedPromptsPanelAction::SendNow(query_id) => {
                let query_id = *query_id;
                if self.editing_row_id(ctx) == Some(query_id) {
                    self.commit_edit(ctx);
                }

                // Leave the row in the queue so the host can read its attachments by id when it
                // fires; the host removes the fired row afterward. Locked rows (the initial
                // cloud-mode prompt) are not send-now-able.
                let row = QueuedQueryModel::as_ref(ctx)
                    .queue(conv_id)
                    .iter()
                    .find(|row| row.id() == query_id && !row.is_locked())
                    .map(|row| (row.text().to_owned(), row.is_command()));
                if let Some((text, is_command)) = row {
                    ctx.emit(QueuedPromptsPanelEvent::SendNow {
                        conversation_id: conv_id,
                        query_id,
                        text,
                        is_command,
                    });
                }
            }
            QueuedPromptsPanelAction::StartEditingRow(query_id) => {
                let query_id = *query_id;
                QueuedQueryModel::handle(ctx).update(ctx, |model, ctx| {
                    model.enter_edit_mode(conv_id, query_id, ctx);
                });
            }
            QueuedPromptsPanelAction::DeleteRow(query_id) => {
                let query_id = *query_id;
                let removed = QueuedQueryModel::handle(ctx)
                    .update(ctx, |model, ctx| model.remove_by_id(conv_id, query_id, ctx));
                if let Some(removed) = removed {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::QueuedPromptDeleted {
                            origin: removed.origin().into(),
                        },
                        ctx
                    );
                    ctx.emit(QueuedPromptsPanelEvent::RowDeleted);
                }
            }
            QueuedPromptsPanelAction::StartDrag(query_id) => {
                let query_id = *query_id;
                // If the row is in edit mode, cancel that edit so dragging is unambiguous.
                let editing = QueuedQueryModel::as_ref(ctx).editing_row(conv_id);
                if editing == Some(query_id) {
                    QueuedQueryModel::handle(ctx).update(ctx, |model, ctx| {
                        model.cancel_edit(conv_id, ctx);
                    });
                }
                let from_index = QueuedQueryModel::as_ref(ctx)
                    .queue(conv_id)
                    .iter()
                    .position(|q| q.id() == query_id);
                self.dragging_query_id = Some(query_id);
                self.drag_start_index = from_index;
                ctx.notify();
            }
            QueuedPromptsPanelAction::DragMoved { rect } => {
                let rect = *rect;
                let Some(source_id) = self.dragging_query_id else {
                    return;
                };
                let panel_view_id = ctx.view_id();
                let queue_len = QueuedQueryModel::as_ref(ctx).queue(conv_id).len();
                let Some(current_index) = QueuedQueryModel::as_ref(ctx)
                    .queue(conv_id)
                    .iter()
                    .position(|q| q.id() == source_id)
                else {
                    return;
                };
                let new_index =
                    calculate_updated_row_index(panel_view_id, current_index, queue_len, rect, ctx);
                if new_index == current_index {
                    return;
                }
                QueuedQueryModel::handle(ctx).update(ctx, |model, ctx| {
                    model.reorder(conv_id, source_id, new_index, ctx);
                });
                ctx.notify();
            }
            QueuedPromptsPanelAction::DropEnd => {
                let Some(source_id) = self.dragging_query_id.take() else {
                    return;
                };
                let from_index = self.drag_start_index.take();
                let model_ref = QueuedQueryModel::as_ref(ctx);
                let queue = model_ref.queue(conv_id);
                let to_index = queue.iter().position(|q| q.id() == source_id);
                let origin = to_index.map(|idx| queue[idx].origin());
                if let (Some(from_index), Some(to_index), Some(origin)) =
                    (from_index, to_index, origin)
                {
                    if from_index != to_index {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::QueuedPromptReordered {
                                origin: origin.into(),
                                from_index,
                                to_index,
                            },
                            ctx
                        );
                    }
                }
                ctx.notify();
            }
        }
    }
}

impl View for QueuedPromptsPanelView {
    fn ui_name() -> &'static str {
        "QueuedPromptsPanelView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() && self.editing_row_id(ctx).is_some() {
            ctx.focus(&self.edit_editor);
        }
    }

    /// Commits an in-progress edit when focus leaves the panel.
    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() && self.editing_row_id(ctx).is_some() {
            self.commit_edit(ctx);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if !self.should_render(app) {
            return Empty::new().finish();
        }

        let Some(conv_id) = self.active_conversation_id else {
            return Empty::new().finish();
        };

        let queue_model = QueuedQueryModel::as_ref(app);
        let queue: Vec<_> = queue_model.queue(conv_id).to_vec();
        let editing_row_id = queue_model.editing_row(conv_id);
        let collapsed = self.collapsed;

        let panel_view_id = self.view_id;
        let header = render_header(
            queue.len(),
            collapsed,
            self.should_show_enter_hint(app),
            &self.header_mouse_state,
            app,
        );
        let mut panel = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header);

        if !collapsed {
            let show_drag_handle = queue.len() > 1;
            let mut body = Flex::column();

            for (index, query) in queue.iter().enumerate() {
                let row_state = self
                    .row_states
                    .get(&query.id())
                    .expect("queued row state should be seeded by model event")
                    .clone();
                let is_in_edit_mode = editing_row_id == Some(query.id());
                let is_being_dragged = self.dragging_query_id == Some(query.id());
                let row = render_row(
                    RenderRowProps {
                        query_id: query.id(),
                        panel_view_id,
                        index,
                        origin: query.origin(),
                        is_command: query.is_command(),
                        is_in_edit_mode,
                        is_being_dragged,
                        show_drag_handle,
                        edit_editor: &self.edit_editor,
                        edit_editor_is_single_logical_line: self.edit_editor_is_single_logical_line,
                        edit_editor_scroll_state: &self.edit_editor_scroll_state,
                        row_state,
                    },
                    app,
                );
                body.add_child(row);
            }

            panel.add_child(
                Container::new(body.finish())
                    .with_horizontal_padding(4.)
                    .with_vertical_padding(8.)
                    .finish(),
            );
        }

        panel.finish()
    }
}

fn build_edit_editor(ctx: &mut ViewContext<QueuedPromptsPanelView>) -> ViewHandle<EditorView> {
    let appearance = Appearance::as_ref(ctx);
    // Match the prompt input, which renders at the monospace font size.
    let text_options = TextOptions::ui_text(Some(appearance.monospace_font_size()), appearance);
    ctx.add_typed_action_view(|ctx| {
        let options = EditorOptions {
            autogrow: true,
            soft_wrap: true,
            text: text_options,
            propagate_and_no_op_escape_key: PropagateAndNoOpEscapeKey::PropagateFirst,
            // Keep up/down inside the inline editor so they move the cursor between lines.
            propagate_and_no_op_vertical_navigation_keys: PropagateAndNoOpNavigationKeys::Never,
            propagate_horizontal_navigation_keys: PropagateHorizontalNavigationKeys::AtBoundary,
            ..Default::default()
        };
        EditorView::new(options, ctx)
    })
}

fn calculate_updated_row_index(
    panel_view_id: EntityId,
    current_index: usize,
    queue_len: usize,
    drag_position: RectF,
    ctx: &ViewContext<QueuedPromptsPanelView>,
) -> usize {
    updated_index_from_vertical_drag(current_index, queue_len, drag_position, |index| {
        ctx.element_position_by_id(queue_row_position_id(panel_view_id, index))
    })
}

fn updated_index_from_vertical_drag(
    current_index: usize,
    item_count: usize,
    drag_position: RectF,
    mut item_rect: impl FnMut(usize) -> Option<RectF>,
) -> usize {
    let dragged_midpoint_y = (drag_position.min_y() + drag_position.max_y()) / 2.;

    if current_index > 0 {
        if let Some(neighbor_rect) = item_rect(current_index - 1) {
            let neighbor_midpoint_y = (neighbor_rect.min_y() + neighbor_rect.max_y()) / 2.;
            if dragged_midpoint_y < neighbor_midpoint_y {
                return current_index - 1;
            }
        }
    }

    if current_index + 1 < item_count {
        if let Some(neighbor_rect) = item_rect(current_index + 1) {
            let neighbor_midpoint_y = (neighbor_rect.min_y() + neighbor_rect.max_y()) / 2.;
            if dragged_midpoint_y > neighbor_midpoint_y {
                return current_index + 1;
            }
        }
    }

    current_index
}

fn render_header(
    count: usize,
    collapsed: bool,
    show_enter_hint: bool,
    header_mouse_state: &MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let label_text = header_label_text(count);
    let sub_text_color: ColorU = theme.sub_text_color(theme.surface_1()).into();
    // The keycap is dimmed relative to the header text so it reads as a secondary affordance.
    let keycap_color: ColorU = internal_colors::text_disabled(theme, theme.surface_1());
    let banner_background: Fill = theme.surface_overlay_1().into();
    let border_color: Fill = theme.split_pane_border_color().into();
    let chevron_icon = if collapsed {
        TerminalIcon::ChevronRight
    } else {
        TerminalIcon::ChevronDown
    };
    let ui_font_family = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();
    Hoverable::new(header_mouse_state.clone(), move |_state| {
        let chevron =
            ConstrainedBox::new(chevron_icon.to_warpui_icon(sub_text_color.into()).finish())
                .with_height(16.)
                .with_width(16.)
                .finish();
        let label = Text::new(label_text.clone(), ui_font_family, ui_font_size)
            .with_style(Properties {
                style: Style::Normal,
                weight: Weight::Normal,
            })
            .with_color(sub_text_color)
            .with_selectable(false)
            .finish();
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.)
            .with_child(chevron)
            .with_child(label);
        if show_enter_hint {
            // Follow the message-bar hint spacing conventions (see
            // `render_message_bar_items`): 8px between the label and the keycap, 4px between
            // the keycap and its text. The row's 4px flex spacing provides part of each gap.
            let keycap = render_keystroke_with_color_overrides(
                &Keystroke {
                    key: "enter".to_owned(),
                    ..Default::default()
                },
                Some(keycap_color),
                None,
                app,
            );
            row.add_child(Container::new(keycap).with_margin_left(4.).finish());
            row.add_child(
                Text::new("to send", ui_font_family, ui_font_size)
                    .with_style(Properties {
                        style: Style::Normal,
                        weight: Weight::Normal,
                    })
                    .with_color(sub_text_color)
                    .with_selectable(false)
                    .finish(),
            );
        }
        let row = row.finish();
        Container::new(row)
            .with_horizontal_padding(16.)
            .with_vertical_padding(8.)
            .with_background(banner_background)
            .with_border(Border::top(1.).with_border_fill(border_color))
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(|ctx, _, _| {
        ctx.dispatch_typed_action(QueuedPromptsPanelAction::ToggleCollapsed);
    })
    .finish()
}

struct RenderRowProps<'a> {
    query_id: QueuedQueryId,
    panel_view_id: EntityId,
    index: usize,
    origin: QueuedQueryOrigin,
    /// Whether this row is a shell command (rendered with a blue `!` prefix) vs an agent prompt.
    is_command: bool,
    is_in_edit_mode: bool,
    is_being_dragged: bool,
    show_drag_handle: bool,
    edit_editor: &'a ViewHandle<EditorView>,
    edit_editor_is_single_logical_line: bool,
    edit_editor_scroll_state: &'a ClippedScrollStateHandle,
    row_state: QueuedPromptRowState,
}

fn render_row(props: RenderRowProps<'_>, app: &AppContext) -> Box<dyn Element> {
    let RenderRowProps {
        query_id,
        panel_view_id,
        index,
        origin,
        is_command,
        is_in_edit_mode,
        is_being_dragged,
        show_drag_handle,
        edit_editor,
        edit_editor_is_single_logical_line,
        edit_editor_scroll_state,
        row_state,
    } = props;

    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    // Match the prompt input, which renders at the monospace font size.
    let queued_input_font_size = appearance.monospace_font_size();
    // Blue used for the `!` prefix on command rows, matching shell-mode input styling.
    let command_prefix_color = theme.ansi_fg_blue();

    let row_action_button_size = ButtonSize::XSmall.button_height(appearance, app);
    let editor_handle = edit_editor.clone();
    let editor_scroll_state = edit_editor_scroll_state.clone();

    let QueuedPromptRowState {
        preview_text,
        mouse_state,
        drag_handle_tooltip_state,
        send_now_button,
        edit_button,
        delete_button,
        draggable_state,
    } = row_state;

    let row_inner = Hoverable::new(mouse_state, move |state| {
        let prompt_text_or_editor: Box<dyn Element> = if is_in_edit_mode {
            let editor_scrollable = NewScrollable::vertical(
                SingleAxisConfig::Clipped {
                    handle: editor_scroll_state.clone(),
                    child: ChildView::new(&editor_handle).finish(),
                },
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                Fill::None,
            )
            .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
            .with_propagate_mousewheel_if_not_handled(true)
            .finish();
            let editor_viewport = Clipped::new(editor_scrollable).finish();
            let editor_viewport = if edit_editor_is_single_logical_line {
                MinSize::new(editor_viewport).finish()
            } else {
                editor_viewport
            };

            ConstrainedBox::new(
                Container::new(editor_viewport)
                    .with_border(Border::all(1.).with_border_fill(theme.outline()))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .with_horizontal_padding(4.)
                    .finish(),
            )
            .with_max_height(
                queued_input_font_size * DEFAULT_UI_LINE_HEIGHT_RATIO * MAX_PROMPT_LINES,
            )
            .finish()
        } else {
            // Single-line preview that truncates by width with a trailing ellipsis.
            let preview = Text::new(
                preview_text.clone(),
                appearance.ui_font_family(),
                queued_input_font_size,
            )
            .with_color(theme.foreground().into())
            .with_selectable(false)
            .soft_wrap(false)
            .with_clip(ClipConfig::ellipsis())
            .finish();
            // Command rows are prefaced with a blue `!` so they read as shell commands; prompt
            // rows render their text directly.
            if is_command {
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(4.)
                    .with_child(
                        Text::new("!", appearance.ui_font_family(), queued_input_font_size)
                            .with_color(command_prefix_color)
                            .with_selectable(false)
                            .finish(),
                    )
                    .with_child(Expanded::new(1., preview).finish())
                    .finish()
            } else {
                preview
            }
        };

        let drag_handle: Box<dyn Element> = if !show_drag_handle {
            // Reserve the handle's footprint without drawing it (single-row queue).
            ConstrainedBox::new(Empty::new().finish())
                .with_height(20.)
                .with_width(20.)
                .finish()
        } else if origin == QueuedQueryOrigin::InitialCloudMode {
            let ui_builder = appearance.ui_builder().clone();
            let disabled_color = internal_colors::text_disabled(theme, theme.surface_1());
            Hoverable::new(drag_handle_tooltip_state.clone(), move |drag_state| {
                let icon = ConstrainedBox::new(
                    TerminalIcon::DragIndicatorVertical
                        .to_warpui_icon(disabled_color.into())
                        .finish(),
                )
                .with_height(20.)
                .with_width(20.)
                .finish();
                let mut stack = Stack::new().with_child(icon);
                if drag_state.is_hovered() {
                    stack.add_positioned_overlay_child(
                        ui_builder
                            .tool_tip(INITIAL_CLOUD_MODE_PROMPT_TOOLTIP.to_owned())
                            .build()
                            .finish(),
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., -4.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::TopLeft,
                            ChildAnchor::BottomLeft,
                        ),
                    );
                }
                stack.finish()
            })
            .finish()
        } else {
            ConstrainedBox::new(
                TerminalIcon::DragIndicatorVertical
                    .to_warpui_icon(theme.sub_text_color(theme.surface_1()))
                    .finish(),
            )
            .with_height(20.)
            .with_width(20.)
            .finish()
        };

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(drag_handle)
            .with_child(Expanded::new(1., prompt_text_or_editor).finish());

        // Trailing actions reveal on hover. When hidden, reserve their exact footprint so the
        // prompt text never reflows. send-now and delete always show; edit only outside edit mode.
        let show_actions = state.is_hovered() && !is_being_dragged;
        let action_spacing = 4.;
        let actions: Box<dyn Element> = if show_actions {
            let mut buttons = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(action_spacing);
            buttons.add_child(ChildView::new(&send_now_button).finish());
            if !is_in_edit_mode {
                buttons.add_child(ChildView::new(&edit_button).finish());
            }
            buttons.add_child(ChildView::new(&delete_button).finish());
            buttons.finish()
        } else {
            let count = if is_in_edit_mode { 2. } else { 3. };
            ConstrainedBox::new(Empty::new().finish())
                .with_width(count * row_action_button_size + (count - 1.) * action_spacing)
                .finish()
        };
        row.add_child(actions);

        let row_content = ConstrainedBox::new(row.finish())
            .with_min_height(32.)
            .finish();
        let mut container = Container::new(row_content)
            .with_horizontal_padding(8.)
            .with_vertical_padding(4.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
        if is_being_dragged || state.is_hovered() {
            container = container.with_background(theme.surface_overlay_1());
        }
        container.finish()
    })
    .finish();

    let position_id = queue_row_position_id(panel_view_id, index);

    if is_in_edit_mode || origin == QueuedQueryOrigin::InitialCloudMode || !show_drag_handle {
        return SavePosition::new(row_inner, &position_id).finish();
    }

    let draggable = Draggable::new(draggable_state, row_inner)
        .with_drag_axis(DragAxis::VerticalOnly)
        .on_drag_start(move |ctx, _, _| {
            ctx.dispatch_typed_action(QueuedPromptsPanelAction::StartDrag(query_id));
        })
        .on_drag(|ctx, _, rect, _| {
            ctx.dispatch_typed_action(QueuedPromptsPanelAction::DragMoved { rect });
        })
        .on_drop(|ctx, _, _, _| {
            ctx.dispatch_typed_action(QueuedPromptsPanelAction::DropEnd);
        })
        .finish();

    SavePosition::new(draggable, &position_id).finish()
}

/// Returns the user-visible header label for `count` queued prompts.
fn header_label_text(count: usize) -> String {
    format!("{count} queued")
}
