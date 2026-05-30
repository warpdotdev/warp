//! Multi-prompt queue panel rendered between the warping indicator and the input editor in
//! [`TerminalView`].
//!
//! Reads from the `QueuedQueryModel` singleton (keyed by `AIConversationId`) for the queue of the
//! currently-active conversation in its parent terminal view, looked up via
//! [`BlocklistAIHistoryModel::active_conversation_id`]. Tracks panel-only UI state (collapse,
//! hover, drag) locally. Emits two high-level events: [`QueuedPromptsPanelEvent::RowDeleted`] and
//! [`QueuedPromptsPanelEvent::EditEnded`], which the host uses to update the input editor.
use std::collections::HashMap;

use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use warp_core::features::FeatureFlag;
use warpui::elements::new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig};
use warpui::elements::{
    Border, ChildView, Clipped, ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, DragAxis, Draggable, DraggableState, Empty, Expanded, Fill, Flex,
    Hoverable, MinSize, MouseStateHandle, ParentElement, Radius, SavePosition, ScrollbarWidth,
    Text, DEFAULT_UI_LINE_HEIGHT_RATIO,
};
use warpui::fonts::{Properties, Style, Weight};
use warpui::platform::Cursor;
use warpui::{
    AppContext, BlurContext, Element, Entity, EntityId, FocusContext, ModelHandle, SingletonEntity,
    TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, QueuedQueryEvent, QueuedQueryId,
    QueuedQueryModel,
};
use crate::appearance::Appearance;
use crate::editor::{
    EditorOptions, EditorView, Event as EditorEvent, PropagateAndNoOpEscapeKey,
    PropagateAndNoOpNavigationKeys, PropagateHorizontalNavigationKeys, TextOptions,
};
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::TelemetryEvent;
use crate::terminal::input::suggestions_mode_model::InputSuggestionsModeModel;
use crate::ui_components::icons::Icon as TerminalIcon;
use crate::util::truncation::truncate_from_end;
use crate::view_components::action_button::{ActionButton, ButtonSize, NakedTheme};

const MAX_PROMPT_LINES: f32 = 5.;

/// Returns the position-cache id used to look up a row's bounding rect during a drag.
/// Indexed by the row's current visual index so swaps maintain stable lookups.
fn queue_row_position_id(panel_view_id: EntityId, index: usize) -> String {
    format!("queued_prompts_panel:{panel_view_id:?}:row:{index}")
}

fn build_row_state(
    query_id: QueuedQueryId,
    ctx: &mut ViewContext<QueuedPromptsPanelView>,
) -> QueuedPromptRowState {
    let edit_button = ctx.add_typed_action_view(move |_| {
        ActionButton::new("", NakedTheme)
            .with_icon(TerminalIcon::Pencil)
            .with_tooltip("Edit queued prompt")
            .with_size(ButtonSize::XSmall)
            .on_click(move |ctx| {
                ctx.dispatch_typed_action(QueuedPromptsPanelAction::StartEditingRow(query_id));
            })
    });
    let delete_button = ctx.add_typed_action_view(move |_| {
        ActionButton::new("", NakedTheme)
            .with_icon(TerminalIcon::Trash)
            .with_tooltip("Delete queued prompt")
            .with_size(ButtonSize::XSmall)
            .on_click(move |ctx| {
                ctx.dispatch_typed_action(QueuedPromptsPanelAction::DeleteRow(query_id));
            })
    });

    QueuedPromptRowState {
        mouse_state: MouseStateHandle::default(),
        edit_button,
        delete_button,
        draggable_state: DraggableState::default(),
    }
}

#[derive(Clone)]
struct QueuedPromptRowState {
    mouse_state: MouseStateHandle,
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
    header_mouse_state: MouseStateHandle,
    row_states: HashMap<QueuedQueryId, QueuedPromptRowState>,
    dragging_query_id: Option<QueuedQueryId>,
    drag_start_index: Option<usize>,
}

#[derive(Clone, Debug)]
pub enum QueuedPromptsPanelAction {
    ToggleCollapsed,
    StartEditingRow(QueuedQueryId),
    DeleteRow(QueuedQueryId),
    StartDrag(QueuedQueryId),
    DragMoved { rect: RectF },
    DropEnd,
}

/// Events emitted to the parent view ([`TerminalView`]). Two variants cover everything the host
/// needs: place text on delete, and refocus the input box after an edit-mode transition.
#[derive(Clone, Debug)]
pub enum QueuedPromptsPanelEvent {
    /// A row was deleted via the trash button. The host should place `text` into the input editor
    /// when the editor is empty, and focus the input.
    RowDeleted { text: String },
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
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let edit_editor = build_edit_editor(ctx);

        ctx.subscribe_to_view(&edit_editor, |me, _, event, ctx| {
            me.handle_edit_editor_event(event, ctx);
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

        let mut me = Self {
            view_id: ctx.view_id(),
            terminal_view_id,
            suggestions_mode_model,
            active_conversation_id,
            edit_editor,
            edit_editor_is_single_logical_line: true,
            edit_editor_scroll_state: Default::default(),
            collapsed: false,
            header_mouse_state: MouseStateHandle::default(),
            row_states: HashMap::new(),
            dragging_query_id: None,
            drag_start_index: None,
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

    /// Reseed `row_states` for `conv_id`'s queue, dropping any state for rows not in that queue.
    fn seed_row_states_for(&mut self, conv_id: AIConversationId, ctx: &mut ViewContext<Self>) {
        let query_ids: Vec<QueuedQueryId> = QueuedQueryModel::as_ref(ctx)
            .queue(conv_id)
            .iter()
            .map(|q| q.id())
            .collect();
        self.row_states.retain(|id, _| query_ids.contains(id));
        for id in query_ids {
            self.row_states
                .entry(id)
                .or_insert_with(|| build_row_state(id, ctx));
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
                    editor.select_all(ctx);
                });
                ctx.focus(&self.edit_editor);
            }
            QueuedQueryEvent::EditCommitted { .. } | QueuedQueryEvent::EditCancelled { .. } => {
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
                self.row_states
                    .entry(*query_id)
                    .or_insert_with(|| build_row_state(*query_id, ctx));
            }
            QueuedQueryEvent::Reordered { .. }
            | QueuedQueryEvent::QueueNextPromptToggled { .. } => {}
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
                    ctx.emit(QueuedPromptsPanelEvent::RowDeleted {
                        text: removed.text().to_owned(),
                    });
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

        let appearance = Appearance::as_ref(app);
        let queue_model = QueuedQueryModel::as_ref(app);
        let queue: Vec<_> = queue_model.queue(conv_id).to_vec();
        let editing_row_id = queue_model.editing_row(conv_id);
        let collapsed = self.collapsed;

        let panel_view_id = self.view_id;
        let header = render_header(queue.len(), collapsed, &self.header_mouse_state, appearance);
        let mut panel = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header);

        if !collapsed {
            let mut body = Flex::column();

            for (index, query) in queue.iter().enumerate() {
                let row_state = self
                    .row_states
                    .get(&query.id())
                    .expect("queued row state should be seeded by model event")
                    .clone();
                let is_in_edit_mode = editing_row_id == Some(query.id());
                let is_being_dragged = self.dragging_query_id == Some(query.id());
                let row = render_row(RenderRowProps {
                    query_id: query.id(),
                    panel_view_id,
                    index,
                    text: query.text().to_owned(),
                    is_in_edit_mode,
                    is_being_dragged,
                    edit_editor: &self.edit_editor,
                    edit_editor_is_single_logical_line: self.edit_editor_is_single_logical_line,
                    edit_editor_scroll_state: &self.edit_editor_scroll_state,
                    row_state,
                    appearance,
                });
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
    let text_options = TextOptions::ui_text(Some(appearance.ui_font_size()), appearance);
    ctx.add_typed_action_view(|ctx| {
        let options = EditorOptions {
            autogrow: true,
            soft_wrap: true,
            text: text_options,
            propagate_and_no_op_escape_key: PropagateAndNoOpEscapeKey::PropagateFirst,
            propagate_and_no_op_vertical_navigation_keys: PropagateAndNoOpNavigationKeys::Always,
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
    header_mouse_state: &MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let label_text = header_label_text(count);
    let sub_text_color: ColorU = theme.sub_text_color(theme.surface_1()).into();
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
        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.)
            .with_child(chevron)
            .with_child(label)
            .finish();
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
    text: String,
    is_in_edit_mode: bool,
    is_being_dragged: bool,
    edit_editor: &'a ViewHandle<EditorView>,
    edit_editor_is_single_logical_line: bool,
    edit_editor_scroll_state: &'a ClippedScrollStateHandle,
    row_state: QueuedPromptRowState,
    appearance: &'a Appearance,
}

fn render_row(props: RenderRowProps<'_>) -> Box<dyn Element> {
    let RenderRowProps {
        query_id,
        panel_view_id,
        index,
        text,
        is_in_edit_mode,
        is_being_dragged,
        edit_editor,
        edit_editor_is_single_logical_line,
        edit_editor_scroll_state,
        row_state,
        appearance,
    } = props;

    let theme = appearance.theme();
    let dimmed_color: ColorU = theme.sub_text_color(theme.surface_1()).into();
    let foreground_color: ColorU = theme.foreground().into();
    let row_hover_background: Fill = theme.surface_overlay_1().into();
    let ui_font_family = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();
    let editor_line_height = ui_font_size * DEFAULT_UI_LINE_HEIGHT_RATIO;
    let max_prompt_height = editor_line_height * MAX_PROMPT_LINES;
    let preview_text = truncate_from_end(&text, 200);
    let editor_handle = edit_editor.clone();
    let editor_scroll_state = edit_editor_scroll_state.clone();

    let QueuedPromptRowState {
        mouse_state,
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
            .with_max_height(max_prompt_height)
            .finish()
        } else {
            ConstrainedBox::new(
                Text::new(preview_text.clone(), ui_font_family, ui_font_size)
                    .with_color(foreground_color)
                    .with_selectable(false)
                    .finish(),
            )
            .with_max_height(max_prompt_height)
            .finish()
        };

        let drag_handle: Box<dyn Element> = ConstrainedBox::new(
            TerminalIcon::DragIndicator
                .to_warpui_icon(dimmed_color.into())
                .finish(),
        )
        .with_height(24.)
        .with_width(24.)
        .finish();

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(drag_handle)
            .with_child(Expanded::new(1., prompt_text_or_editor).finish());

        if state.is_hovered() && !is_being_dragged {
            let mut buttons = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(4.);
            if !is_in_edit_mode {
                buttons.add_child(ChildView::new(&edit_button).finish());
            }
            buttons.add_child(ChildView::new(&delete_button).finish());
            row.add_child(buttons.finish());
        }

        let row_content = ConstrainedBox::new(row.finish())
            .with_min_height(32.)
            .finish();
        let mut container = Container::new(row_content)
            .with_horizontal_padding(8.)
            .with_vertical_padding(4.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
        if is_being_dragged || state.is_hovered() {
            container = container.with_background(row_hover_background);
        }
        container.finish()
    })
    .finish();

    let position_id = queue_row_position_id(panel_view_id, index);

    if is_in_edit_mode {
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
