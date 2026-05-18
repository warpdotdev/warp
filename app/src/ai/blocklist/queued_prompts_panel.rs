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

use crate::ai::blocklist::{QueuedQueryEvent, QueuedQueryId, QueuedQueryModel};
use crate::appearance::Appearance;
use crate::editor::{
    EditorOptions, EditorView, Event as EditorEvent, PropagateAndNoOpEscapeKey,
    PropagateAndNoOpNavigationKeys, PropagateHorizontalNavigationKeys, TextOptions,
};
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::TelemetryEvent;
use crate::ui_components::icons::Icon;
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
            .with_icon(Icon::Pencil)
            .with_tooltip("Edit queued prompt")
            .with_size(ButtonSize::XSmall)
            .on_click(move |ctx| {
                ctx.dispatch_typed_action(QueuedPromptsPanelAction::StartEditingRow(query_id));
            })
    });
    let delete_button = ctx.add_typed_action_view(move |_| {
        ActionButton::new("", NakedTheme)
            .with_icon(Icon::Trash)
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
    /// Cached view id; used to namespace per-row `SavePosition` ids so live-reorder lookups are
    /// scoped to this panel even if multiple panels share a window.
    view_id: EntityId,
    queued_query_model: ModelHandle<QueuedQueryModel>,
    /// Reusable editor for whichever row is currently in edit mode.
    /// Created once and reused across edit sessions to avoid view churn.
    edit_editor: ViewHandle<EditorView>,
    edit_editor_is_single_logical_line: bool,
    edit_editor_scroll_state: ClippedScrollStateHandle,
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
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let edit_editor = build_edit_editor(ctx);

        ctx.subscribe_to_view(&edit_editor, |me, _, event, ctx| {
            me.handle_edit_editor_event(event, ctx);
        });

        ctx.subscribe_to_model(&queued_query_model, Self::handle_queued_query_event);

        Self {
            view_id: ctx.view_id(),
            queued_query_model,
            edit_editor,
            edit_editor_is_single_logical_line: true,
            edit_editor_scroll_state: Default::default(),
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
            QueuedQueryEvent::Removed { query_id } => {
                self.row_states.remove(query_id);
                if self.dragging_query_id == Some(*query_id) {
                    self.clear_drag_state();
                }
            }
            QueuedQueryEvent::EditEntered { query_id } => {
                let initial_text = self
                    .queued_query_model
                    .as_ref(ctx)
                    .queue()
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
            QueuedQueryEvent::Cleared => {
                self.row_states.clear();
                self.clear_drag_state();
            }
            QueuedQueryEvent::Appended { query_id } => {
                self.row_states
                    .entry(*query_id)
                    .or_insert_with(|| build_row_state(*query_id, ctx));
            }
            QueuedQueryEvent::Replaced { .. }
            | QueuedQueryEvent::Reordered
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
        self.queued_query_model.as_ref(ctx).editing_row()
    }

    fn commit_edit(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(query_id) = self.editing_row_id(ctx) else {
            return;
        };
        let origin = self
            .queued_query_model
            .as_ref(ctx)
            .queue()
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

    /// Visibility predicate used by the host to decide whether to render the panel.
    pub fn should_render(&self, ctx: &AppContext) -> bool {
        if !FeatureFlag::QueueSlashCommand.is_enabled() {
            return false;
        }
        self.queued_query_model.as_ref(ctx).has_queue()
    }
}

impl TypedActionView for QueuedPromptsPanelView {
    type Action = QueuedPromptsPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            QueuedPromptsPanelAction::ToggleCollapsed => {
                let collapsed = !self.queued_query_model.as_ref(ctx).is_collapsed();
                self.queued_query_model.update(ctx, |model, ctx| {
                    model.set_collapsed(collapsed, ctx);
                });
                send_telemetry_from_ctx!(
                    TelemetryEvent::QueuedPromptPanelCollapseToggled { collapsed },
                    ctx
                );
                ctx.emit(QueuedPromptsPanelEvent::CollapseToggled { collapsed });
            }
            QueuedPromptsPanelAction::StartEditingRow(query_id) => {
                let query_id = *query_id;
                self.queued_query_model.update(ctx, |model, ctx| {
                    model.enter_edit_mode(query_id, ctx);
                });
                ctx.emit(QueuedPromptsPanelEvent::RowEditEntered { query_id });
            }
            QueuedPromptsPanelAction::DeleteRow(query_id) => {
                let query_id = *query_id;
                let removed = self
                    .queued_query_model
                    .update(ctx, |model, ctx| model.remove_by_id(query_id, ctx));
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
                        text: removed.text().to_owned(),
                    });
                }
            }
            QueuedPromptsPanelAction::StartDrag(query_id) => {
                let query_id = *query_id;
                // If the row is in edit mode, cancel that edit so dragging is unambiguous.
                let editing = self.queued_query_model.as_ref(ctx).editing_row();
                if editing == Some(query_id) {
                    self.queued_query_model.update(ctx, |model, ctx| {
                        model.cancel_edit(ctx);
                    });
                }
                let from_index = self
                    .queued_query_model
                    .as_ref(ctx)
                    .queue()
                    .iter()
                    .position(|q| q.id() == query_id);
                self.dragging_query_id = Some(query_id);
                self.drag_start_index = from_index;
                ctx.notify();
            }
            // On every `on_drag` tick, compare the dragged row's midpoint against neighbor row
            // midpoints and swap with the neighbor when the threshold is crossed. This produces
            // live, single-step reordering as the user drags so the queue visibly reflows under
            // the cursor.
            QueuedPromptsPanelAction::DragMoved { rect } => {
                let rect = *rect;
                let Some(source_id) = self.dragging_query_id else {
                    return;
                };
                let panel_view_id = ctx.view_id();
                let queue_len = self.queued_query_model.as_ref(ctx).queue().len();
                let Some(current_index) = self
                    .queued_query_model
                    .as_ref(ctx)
                    .queue()
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
                    model.reorder(source_id, new_index, ctx);
                });
                ctx.notify();
            }
            QueuedPromptsPanelAction::DropEnd => {
                let Some(source_id) = self.dragging_query_id.take() else {
                    return;
                };
                let from_index = self.drag_start_index.take();
                let queue = self.queued_query_model.as_ref(ctx).queue();
                let to_index = queue.iter().position(|q| q.id() == source_id);
                let origin = to_index.map(|idx| queue[idx].origin());
                // Only emit reorder telemetry/event if the row's index actually changed during the drag.
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
                        ctx.emit(QueuedPromptsPanelEvent::RowReordered {
                            query_id: source_id,
                            from_index,
                            to_index,
                        });
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

        let appearance = Appearance::as_ref(app);
        let queue_model = self.queued_query_model.as_ref(app);
        let queue: Vec<_> = queue_model.queue().to_vec();
        let collapsed = queue_model.is_collapsed();
        let editing_row_id = queue_model.editing_row();

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
    // Register the editor as a child so focus events bubble through the panel.
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
            Icon::DragIndicator
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
