//! Multi-prompt queue panel rendered between the warping indicator and the input editor in
//! [`TerminalView`].
//!
//! Reads from [`QueuedQueryModel`] and emits high-level
//! [`QueuedPromptsPanelEvent`]s for the host view to handle (for example, focusing the input
//! editor after canceling an edit).
use std::collections::HashMap;

use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use warp_core::features::FeatureFlag;
use warpui::elements::{
    Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DragAxis,
    Draggable, DraggableState, Empty, Expanded, Fill, Flex, Hoverable, MouseStateHandle, Padding,
    ParentElement, Radius, SavePosition, Text, DEFAULT_UI_LINE_HEIGHT_RATIO,
};
use warpui::fonts::{Properties, Style, Weight};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::{
    AppContext, BlurContext, Element, Entity, EntityId, EventContext, FocusContext, ModelHandle,
    SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::blocklist::context_model::BlocklistAIContextModel;
use crate::ai::blocklist::{QueuedQueryEvent, QueuedQueryId, QueuedQueryModel, QueuedQueryOrigin};
use crate::appearance::Appearance;
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions, TextOptions};
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::TelemetryEvent;
use crate::ui_components::icons::Icon;

/// Returns the position-cache id used to look up a row's bounding rect during a drag.
/// Indexed by the row's current visual index so swaps maintain stable lookups.
fn queue_row_position_id(panel_view_id: EntityId, index: usize) -> String {
    format!("queued_prompts_panel:{panel_view_id:?}:row:{index}")
}

#[derive(Clone, Default)]
struct QueuedPromptRowState {
    mouse_state: MouseStateHandle,
    edit_button_mouse_state: MouseStateHandle,
    delete_button_mouse_state: MouseStateHandle,
    draggable_state: DraggableState,
}

/// View for the multi-prompt queue panel.
pub struct QueuedPromptsPanelView {
    /// Cached view id; used to namespace per-row `SavePosition` ids so live-reorder lookups are
    /// scoped to this panel even if multiple panels share a window.
    view_id: EntityId,
    queued_query_model: ModelHandle<QueuedQueryModel>,
    ai_context_model: ModelHandle<BlocklistAIContextModel>,
    /// Reusable editor for whichever row is currently in edit mode.
    /// Created once and reused across edit sessions to avoid view churn.
    edit_editor: ViewHandle<EditorView>,
    /// Mouse state for the header row, used to highlight on hover.
    header_mouse_state: MouseStateHandle,
    /// Per-row hover/button/drag state keyed by `QueuedQueryId`.
    row_states: HashMap<QueuedQueryId, QueuedPromptRowState>,
    /// The id of the row currently being dragged, if any.
    dragging_query_id: Option<QueuedQueryId>,
    /// The index where the current drag started, used for telemetry after live swaps.
    drag_start_index: Option<usize>,
}

/// Actions dispatched by hover buttons inside the panel.
#[derive(Clone, Debug)]
pub enum QueuedPromptsPanelAction {
    ToggleCollapsed,
    StartEditingRow(QueuedQueryId),
    DeleteRow(QueuedQueryId),
    /// Dispatched when the user begins dragging a row.
    /// Cancels any in-progress edit on that row.
    StartDrag(QueuedQueryId),
    /// Fired as the dragged row moves; carries the dragged row's bounding rect so the handler
    /// can compare its midpoint against neighbor rows and live-swap rows in the queue.
    DragMoved {
        rect: RectF,
    },
    /// Fired when the user releases the dragged row; clears in-progress drag state.
    DropEnd,
}

/// Events emitted to the parent view ([`TerminalView`]).
#[derive(Clone, Debug)]
pub enum QueuedPromptsPanelEvent {
    /// A row was removed.
    RowRemoved {
        query_id: QueuedQueryId,
        was_via_edit_commit: bool,
    },
    /// A row's text was committed via the inline editor.
    RowEdited { query_id: QueuedQueryId },
    /// The collapse chevron was toggled.
    CollapseToggled { collapsed: bool },
    /// The user pressed Escape inside the inline editor and the edit was cancelled.
    EditCancelled { query_id: QueuedQueryId },
    /// A row entered edit mode.
    RowEditEntered { query_id: QueuedQueryId },
    /// The user requested to delete a row whose text should be placed in the input
    /// editor when the editor is empty.
    /// The host owns the input editor so it performs the placement.
    RowDeletedForInputPlacement { text: String },
    /// A row was reordered via drag-and-drop.
    RowReordered {
        query_id: QueuedQueryId,
        from_index: usize,
        to_index: usize,
    },
}

impl Entity for QueuedPromptsPanelView {
    type Event = QueuedPromptsPanelEvent;
}

impl QueuedPromptsPanelView {
    pub fn new(
        queued_query_model: ModelHandle<QueuedQueryModel>,
        ai_context_model: ModelHandle<BlocklistAIContextModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let edit_editor = build_edit_editor(ctx);

        ctx.subscribe_to_view(&edit_editor, |me, _, event, ctx| {
            me.handle_edit_editor_event(event, ctx);
        });

        ctx.subscribe_to_model(&queued_query_model, Self::handle_queued_query_event);
        ctx.subscribe_to_model(&ai_context_model, |_, _, event, ctx| {
            if matches!(
                event,
                crate::ai::blocklist::BlocklistAIContextEvent::PendingQueryStateUpdated
            ) {
                ctx.notify();
            }
        });

        Self {
            view_id: ctx.view_id(),
            queued_query_model,
            ai_context_model,
            edit_editor,
            header_mouse_state: MouseStateHandle::default(),
            row_states: HashMap::new(),
            dragging_query_id: None,
            drag_start_index: None,
        }
    }

    fn clear_drag_state(&mut self) {
        self.dragging_query_id = None;
        self.drag_start_index = None;
    }

    fn handle_queued_query_event(
        &mut self,
        _: ModelHandle<QueuedQueryModel>,
        event: &QueuedQueryEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            QueuedQueryEvent::Removed { query_id, .. } => {
                self.row_states.remove(query_id);
                if self.dragging_query_id == Some(*query_id) {
                    self.clear_drag_state();
                }
            }
            QueuedQueryEvent::EditEntered {
                conversation_id,
                query_id,
            } => {
                let initial_text = self
                    .queued_query_model
                    .as_ref(ctx)
                    .queue_for(*conversation_id)
                    .iter()
                    .find(|row| row.id() == *query_id)
                    .map(|row| row.text().to_owned())
                    .unwrap_or_default();
                self.edit_editor.update(ctx, |editor, ctx| {
                    editor.system_reset_buffer_text(&initial_text, ctx);
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
            }
            QueuedQueryEvent::Appended { query_id, .. } => {
                self.row_states.entry(*query_id).or_default();
            }
            QueuedQueryEvent::Replaced { .. }
            | QueuedQueryEvent::Reordered { .. }
            | QueuedQueryEvent::CollapseToggled { .. }
            | QueuedQueryEvent::QueueNextPromptToggled => {}
        }
        ctx.notify();
    }

    fn handle_edit_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Enter => self.commit_edit(ctx),
            EditorEvent::Escape => self.cancel_edit(ctx),
            // Losing focus commits the edit.
            EditorEvent::Blurred => self.commit_edit(ctx),
            _ => {}
        }
    }

    fn editing_row_id(&self, ctx: &AppContext) -> Option<QueuedQueryId> {
        let conversation_id = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)?;
        self.queued_query_model
            .as_ref(ctx)
            .editing_row(conversation_id)
    }

    fn toggle_collapsed(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(conversation_id) = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        else {
            return;
        };
        let collapsed = !self
            .queued_query_model
            .as_ref(ctx)
            .is_collapsed(conversation_id);
        self.queued_query_model.update(ctx, |model, ctx| {
            model.set_collapsed(conversation_id, collapsed, ctx);
        });
        send_telemetry_from_ctx!(
            TelemetryEvent::QueuedPromptPanelCollapseToggled { collapsed },
            ctx
        );
        ctx.emit(QueuedPromptsPanelEvent::CollapseToggled { collapsed });
    }

    fn start_editing_row(&mut self, query_id: QueuedQueryId, ctx: &mut ViewContext<Self>) {
        let Some(conversation_id) = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        else {
            return;
        };
        self.queued_query_model.update(ctx, |model, ctx| {
            model.enter_edit_mode(conversation_id, query_id, ctx);
        });
        ctx.emit(QueuedPromptsPanelEvent::RowEditEntered { query_id });
    }

    fn delete_row(&mut self, query_id: QueuedQueryId, ctx: &mut ViewContext<Self>) {
        let Some(conversation_id) = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        else {
            return;
        };
        let removed = self.queued_query_model.update(ctx, |model, ctx| {
            model.remove_by_id(conversation_id, query_id, ctx)
        });
        if let Some(ref removed) = removed {
            send_telemetry_from_ctx!(
                TelemetryEvent::QueuedPromptDeleted {
                    origin: removed.origin().into(),
                },
                ctx
            );
        }
        ctx.emit(QueuedPromptsPanelEvent::RowRemoved {
            query_id,
            was_via_edit_commit: false,
        });
        if let Some(removed) = removed {
            ctx.emit(QueuedPromptsPanelEvent::RowDeletedForInputPlacement {
                text: removed.into_text(),
            });
        }
    }

    fn commit_edit(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(query_id) = self.editing_row_id(ctx) else {
            return;
        };
        let Some(conversation_id) = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        else {
            return;
        };
        let origin = self
            .queued_query_model
            .as_ref(ctx)
            .queue_for(conversation_id)
            .iter()
            .find(|row| row.id() == query_id)
            .map(|row| row.origin());
        let new_text = self
            .edit_editor
            .read(ctx, |editor, ctx| editor.buffer_text(ctx).trim().to_owned());
        let was_empty = new_text.is_empty();
        self.queued_query_model.update(ctx, |model, ctx| {
            model.commit_edit(new_text, ctx);
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
        if was_empty {
            ctx.emit(QueuedPromptsPanelEvent::EditCancelled { query_id });
        } else {
            ctx.emit(QueuedPromptsPanelEvent::RowEdited { query_id });
        }
    }

    fn cancel_edit(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(query_id) = self.editing_row_id(ctx) else {
            return;
        };
        self.queued_query_model.update(ctx, |model, ctx| {
            model.cancel_edit(ctx);
        });
        ctx.emit(QueuedPromptsPanelEvent::EditCancelled { query_id });
    }

    fn start_drag(&mut self, query_id: QueuedQueryId, ctx: &mut ViewContext<Self>) {
        // If the row is in edit mode, cancel that edit so dragging is unambiguous.
        let Some(conversation_id) = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        else {
            return;
        };
        let editing = self
            .queued_query_model
            .as_ref(ctx)
            .editing_row(conversation_id);
        if editing == Some(query_id) {
            self.queued_query_model.update(ctx, |model, ctx| {
                model.cancel_edit(ctx);
            });
        }
        let from_index = self
            .queued_query_model
            .as_ref(ctx)
            .queue_for(conversation_id)
            .iter()
            .position(|q| q.id() == query_id);
        self.dragging_query_id = Some(query_id);
        self.drag_start_index = from_index;
        ctx.notify();
    }

    /// On every `on_drag` tick, compare the dragged row's midpoint against neighbor row midpoints
    /// and swap with the neighbor when the threshold is crossed. This produces live, single-step reordering as the
    /// user drags so the queue visibly reflows under the cursor.
    fn drag_moved(&mut self, rect: RectF, ctx: &mut ViewContext<Self>) {
        let Some(source_id) = self.dragging_query_id else {
            return;
        };
        let Some(conversation_id) = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        else {
            return;
        };
        let panel_view_id = ctx.view_id();
        let queue_len = self
            .queued_query_model
            .as_ref(ctx)
            .queue_for(conversation_id)
            .len();
        let Some(current_index) = self
            .queued_query_model
            .as_ref(ctx)
            .queue_for(conversation_id)
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
        self.queued_query_model.update(ctx, |model, ctx| {
            model.reorder(conversation_id, source_id, new_index, ctx);
        });
        ctx.notify();
    }

    fn drop_end(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(source_id) = self.dragging_query_id.take() else {
            return;
        };
        let from_index = self.drag_start_index.take();
        let Some(conversation_id) = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        else {
            ctx.notify();
            return;
        };
        let queue = self
            .queued_query_model
            .as_ref(ctx)
            .queue_for(conversation_id);
        let to_index = queue.iter().position(|q| q.id() == source_id);
        let origin = to_index.map(|idx| queue[idx].origin());
        // Only emit reorder telemetry/event if the row's index actually changed during the drag.
        if let (Some(from_index), Some(to_index), Some(origin)) = (from_index, to_index, origin) {
            if from_index != to_index {
                send_telemetry_from_ctx!(
                    TelemetryEvent::QueuedPromptReordered {
                        origin: origin.into(),
                        from_index,
                        to_index,
                    },
                    ctx
                );
                ctx.emit(QueuedPromptsPanelEvent::RowReordered {
                    query_id: source_id,
                    from_index,
                    to_index,
                });
            }
        }
        ctx.notify();
    }

    /// Visibility predicate used by the host to decide whether to render the panel.
    pub fn should_render(&self, ctx: &AppContext) -> bool {
        if !FeatureFlag::QueueSlashCommand.is_enabled()
            || !FeatureFlag::PendingUserQueryIndicator.is_enabled()
        {
            return false;
        }
        let Some(conversation_id) = self
            .ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        else {
            return false;
        };
        self.queued_query_model
            .as_ref(ctx)
            .has_queue(conversation_id)
    }
}

impl TypedActionView for QueuedPromptsPanelView {
    type Action = QueuedPromptsPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            QueuedPromptsPanelAction::ToggleCollapsed => self.toggle_collapsed(ctx),
            QueuedPromptsPanelAction::StartEditingRow(id) => self.start_editing_row(*id, ctx),
            QueuedPromptsPanelAction::DeleteRow(id) => self.delete_row(*id, ctx),
            QueuedPromptsPanelAction::StartDrag(id) => self.start_drag(*id, ctx),
            QueuedPromptsPanelAction::DragMoved { rect } => self.drag_moved(*rect, ctx),
            QueuedPromptsPanelAction::DropEnd => self.drop_end(ctx),
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
        let Some(conversation_id) = self
            .ai_context_model
            .as_ref(app)
            .selected_conversation_id(app)
        else {
            return Empty::new().finish();
        };

        let appearance = Appearance::as_ref(app);
        let queue_model = self.queued_query_model.as_ref(app);
        let queue: Vec<_> = queue_model.queue_for(conversation_id).to_vec();
        let collapsed = queue_model.is_collapsed(conversation_id);
        let editing_row_id = queue_model.editing_row(conversation_id);

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
                    origin: query.origin(),
                    is_in_edit_mode,
                    is_being_dragged,
                    edit_editor: &self.edit_editor,
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
    // Register the editor as a child so focus events bubble through the panel.
    let appearance = Appearance::as_ref(ctx);
    let text_options = TextOptions::ui_text(Some(appearance.ui_font_size()), appearance);
    ctx.add_typed_action_view(|ctx| {
        let options = SingleLineEditorOptions {
            text: text_options,
            ..Default::default()
        };
        EditorView::single_line(options, ctx)
    })
}

/// Computes the dragged row's new index based on its current rect and the rects of its immediate
/// neighbors.
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
        Icon::ChevronRight
    } else {
        Icon::ChevronDown
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
    origin: QueuedQueryOrigin,
    is_in_edit_mode: bool,
    is_being_dragged: bool,
    edit_editor: &'a ViewHandle<EditorView>,
    row_state: QueuedPromptRowState,
    appearance: &'a Appearance,
}

fn render_row(props: RenderRowProps<'_>) -> Box<dyn Element> {
    let RenderRowProps {
        query_id,
        panel_view_id,
        index,
        text,
        origin,
        is_in_edit_mode,
        is_being_dragged,
        edit_editor,
        row_state,
        appearance,
    } = props;

    let theme = appearance.theme();
    let user_managed = origin.is_user_managed();
    let dimmed_color: ColorU = theme.sub_text_color(theme.surface_1()).into();
    let foreground_color: ColorU = theme.foreground().into();
    let row_hover_background: Fill = theme.surface_overlay_1().into();
    let ui_font_family = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();
    let editor_line_height = ui_font_size * DEFAULT_UI_LINE_HEIGHT_RATIO;
    let editor_handle = edit_editor.clone();

    let QueuedPromptRowState {
        mouse_state,
        edit_button_mouse_state,
        delete_button_mouse_state,
        draggable_state,
    } = row_state;

    let row_inner = Hoverable::new(mouse_state, move |state| {
        let prompt_text_or_editor: Box<dyn Element> = if is_in_edit_mode {
            ConstrainedBox::new(ChildView::new(&editor_handle).finish())
                .with_height(editor_line_height)
                .finish()
        } else {
            Text::new_inline(text.clone(), ui_font_family, ui_font_size)
                .with_color(foreground_color)
                .with_selectable(false)
                .with_clip(ClipConfig::ellipsis())
                .finish()
        };

        let drag_handle: Box<dyn Element> = if user_managed {
            ConstrainedBox::new(
                Icon::DragIndicator
                    .to_warpui_icon(dimmed_color.into())
                    .finish(),
            )
            .with_height(24.)
            .with_width(24.)
            .finish()
        } else {
            ConstrainedBox::new(Empty::new().finish())
                .with_height(24.)
                .with_width(24.)
                .finish()
        };

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(drag_handle)
            .with_child(Expanded::new(1., prompt_text_or_editor).finish());

        if state.is_hovered() && user_managed && !is_being_dragged {
            let mut buttons = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(4.);
            if !is_in_edit_mode {
                buttons.add_child(render_action_button(
                    Icon::Pencil,
                    foreground_color,
                    dimmed_color,
                    edit_button_mouse_state.clone(),
                    move |ctx| {
                        ctx.dispatch_typed_action(QueuedPromptsPanelAction::StartEditingRow(
                            query_id,
                        ));
                    },
                ));
            }
            buttons.add_child(render_action_button(
                Icon::Trash,
                foreground_color,
                dimmed_color,
                delete_button_mouse_state.clone(),
                move |ctx| {
                    ctx.dispatch_typed_action(QueuedPromptsPanelAction::DeleteRow(query_id));
                },
            ));
            row.add_child(buttons.finish());
        }

        let row_content = ConstrainedBox::new(row.finish())
            .with_min_height(32.)
            .finish();
        let mut container = Container::new(row_content)
            .with_horizontal_padding(8.)
            .with_vertical_padding(4.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
        if is_being_dragged || (state.is_hovered() && user_managed) {
            container = container.with_background(row_hover_background);
        }
        container.finish()
    })
    .finish();

    let position_id = queue_row_position_id(panel_view_id, index);

    // Non-draggable rows still register a `SavePosition` so live-reorder can measure their bounds
    // when neighbors are dragged across them.
    if !user_managed || is_in_edit_mode {
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

fn render_action_button<F>(
    icon: Icon,
    hovered_color: ColorU,
    base_color: ColorU,
    mouse_state: MouseStateHandle,
    on_click: F,
) -> Box<dyn Element>
where
    F: Fn(&mut EventContext) + Clone + 'static,
{
    Hoverable::new(mouse_state, move |state| {
        let icon_element = ConstrainedBox::new(
            icon.to_warpui_icon(if state.is_hovered() {
                hovered_color.into()
            } else {
                base_color.into()
            })
            .finish(),
        )
        .with_height(16.)
        .with_width(16.)
        .finish();
        Container::new(icon_element)
            .with_padding(Padding::uniform(2.))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| on_click(ctx))
    .finish()
}
