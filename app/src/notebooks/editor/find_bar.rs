use std::fmt::Write;
use std::time::Duration;

use async_channel::Sender;
use pathfinder_geometry::vector::vec2f;
use warp_editor::render::model::{AutoScrollMode, Decoration};
use warp_editor::search::{SearchEvent, Searcher};
use warpui::accessibility::{AccessibilityContent, ActionAccessibilityContent, WarpA11yRole};
use warpui::elements::{
    Border, ChildAnchor, Clipped, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Empty, Flex, MouseStateHandle, OffsetPositioning, ParentElement, PositionedElementAnchor,
    PositionedElementOffsetBounds, Radius, Rect, Shrinkable, Stack,
};
use warpui::platform::Cursor;
use warpui::presenter::ChildView;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::toggle_button::ToggleButton;
use warpui::{
    AppContext, BlurContext, Element, Entity, FocusContext, ModelHandle, SingletonEntity,
    TypedActionView, View, ViewContext, ViewHandle,
};

use super::model::NotebooksEditorModel;
use super::view::{EditorViewEvent, RichTextEditorView};
use crate::appearance::Appearance;
use crate::debounce::debounce;
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions, TextOptions};
use crate::ui_components::icons::Icon;
use crate::view_components::find::{
    CASE_SENSITIVE_LABEL, CASE_SENSITIVE_TOOLTIP, FIND_BAR_WIDTH, REGEX_TOGGLE_LABEL,
    REGEX_TOGGLE_TOOLTIP,
};

/// View for the find bar within a notebook.
pub struct FindBar {
    searcher: ModelHandle<Searcher>,
    editor_model: ModelHandle<NotebooksEditorModel>,
    query_editor: ViewHandle<EditorView>,
    query_change_tx: Sender<()>,
    button_handles: ButtonHandles,
}

#[derive(Default)]
struct ButtonHandles {
    regex_toggle: MouseStateHandle,
    case_sensitive_toggle: MouseStateHandle,
    next_match: MouseStateHandle,
    previous_match: MouseStateHandle,
    close: MouseStateHandle,
}

#[derive(Debug, Clone, Copy)]
pub enum FindBarEvent {
    Close,
    SearchDecorationsChanged,
}

#[derive(Debug, Clone, Copy)]
pub enum FindBarAction {
    ToggleRegex,
    ToggleCaseSensitive,
    FocusNextMatch,
    FocusPreviousMatch,
    Close,
}

const QUERY_DEBOUNCE_PERIOD: Duration = Duration::from_millis(20);

impl FindBar {
    pub fn new(
        editor_model: ModelHandle<NotebooksEditorModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let searcher = editor_model.update(ctx, |model, ctx| model.new_search(ctx));
        let query_editor = ctx.add_typed_action_view(|ctx| {
            EditorView::single_line(
                SingleLineEditorOptions {
                    // Ensure the search input font size is consistent with the button labels.
                    text: TextOptions::ui_font_size(Appearance::as_ref(ctx)),
                    ..Default::default()
                },
                ctx,
            )
        });
        ctx.subscribe_to_view(&query_editor, Self::handle_query_editor_event);

        let (tx, rx) = async_channel::unbounded();
        ctx.spawn_stream_local(
            debounce(QUERY_DEBOUNCE_PERIOD, rx),
            Self::handle_debounced_query_change,
            |_, _| {},
        );

        ctx.subscribe_to_model(&searcher, Self::handle_search_event);

        Self {
            searcher,
            editor_model,
            query_editor,
            query_change_tx: tx,
            button_handles: Default::default(),
        }
    }

    /// Whether or not the query editor is focused.
    pub fn query_editor_focused(&self, app: &AppContext) -> bool {
        self.query_editor.is_focused(app)
    }

    /// Decorations for the current find-bar search results.
    pub fn decorations(&self, ctx: &AppContext) -> Vec<Decoration> {
        self.searcher.as_ref(ctx).result_decorations()
    }

    fn handle_query_editor_event(
        &mut self,
        _editor: ViewHandle<EditorView>,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Edited(_) => {
                let _ = self.query_change_tx.try_send(());
            }
            EditorEvent::Enter => {
                self.searcher
                    .update(ctx, |search, ctx| search.select_next_result(ctx));
            }
            EditorEvent::ShiftEnter | EditorEvent::AltEnter => {
                self.searcher
                    .update(ctx, |search, ctx| search.select_previous_result(ctx));
            }
            EditorEvent::Escape => ctx.emit(FindBarEvent::Close),
            _ => (),
        }
    }

    fn handle_debounced_query_change(&mut self, _event: (), ctx: &mut ViewContext<Self>) {
        let query = self.query_editor.as_ref(ctx).buffer_text(ctx);
        self.searcher
            .update(ctx, |searcher, ctx| searcher.set_query(query, ctx));
    }

    fn handle_search_event(
        &mut self,
        _model: ModelHandle<Searcher>,
        event: &SearchEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SearchEvent::Updated => {
                // We ask the parent view to update decorations instead of doing it ourselves. This
                // way, it can merge together decorations from multiple sources.
                ctx.emit(FindBarEvent::SearchDecorationsChanged);
                ctx.notify();
            }
            SearchEvent::SelectedResultChanged => {
                if let Some(autoscroll_match) = self.searcher.as_ref(ctx).selected_match_range() {
                    self.editor_model.as_ref(ctx).render_state().clone().update(
                        ctx,
                        |render_state, _ctx| {
                            render_state.request_autoscroll_to(
                                AutoScrollMode::ScrollOffsetsIntoViewport(autoscroll_match),
                            );
                        },
                    )
                }
                ctx.emit(FindBarEvent::SearchDecorationsChanged);
                ctx.notify();
            }
            SearchEvent::InvalidQuery => {
                // TODO: Show an error border?
            }
        }
    }

    /// Line height for the query editor.
    fn editor_height(&self, appearance: &Appearance, app: &AppContext) -> f32 {
        self.query_editor
            .as_ref(app)
            .line_height(app.font_cache(), appearance)
    }

    fn render_match_index(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let searcher = self.searcher.as_ref(app);
        if searcher.has_query() {
            let match_count = searcher.match_count();
            let text = if match_count == 0 {
                "No matches".to_string()
            } else {
                let mut text = String::new();
                match searcher.selected_match() {
                    Some(idx) => {
                        let _ = write!(&mut text, "{}", idx + 1);
                    }
                    None => text.push('?'),
                }
                text.push('/');
                let _ = write!(&mut text, "{match_count}");
                text
            };

            appearance.ui_builder().span(text).build().finish()
        } else {
            Empty::new().finish()
        }
    }

    /// Renders the separator between the query and search options.
    fn render_separator_line(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Rect::new()
                    .with_background(
                        appearance
                            .theme()
                            .foreground_button_color()
                            .with_opacity(20),
                    )
                    .finish(),
            )
            .with_width(1.)
            .with_height(self.editor_height(appearance, app) + 16.)
            .finish(),
        )
        .with_padding_left(12.)
        .with_padding_top(7.)
        .with_padding_bottom(7.)
        .finish()
    }

    fn render_action_button(
        &self,
        icon: Icon,
        action: FindBarAction,
        enabled: bool,
        mouse_state_handle: MouseStateHandle,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let size = self.editor_height(appearance, app);

        let base_styles = self
            .button_styles(appearance, app)
            // We have to add back in space for the padding, because Button applies its size
            // constraint around the padding and border.
            .set_width(size + 16.)
            .set_height(size + 16.);
        let mut button = appearance
            .ui_builder()
            .button(ButtonVariant::Text, mouse_state_handle)
            // The fill here doesn't matter, since it's overridden by the button text color.
            .with_icon_label(icon.to_warpui_icon(crate::themes::theme::Fill::white()))
            .with_style(base_styles)
            .with_hovered_styles(UiComponentStyles {
                background: Some(appearance.theme().foreground_button_color().into()),
                ..Default::default()
            })
            .with_disabled_styles(UiComponentStyles {
                font_color: Some(appearance.theme().nonactive_ui_text_color().into()),
                ..Default::default()
            });

        if !enabled {
            button = button.disabled();
        }

        let button = button
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(action);
            })
            .with_cursor(Cursor::PointingHand)
            .finish();

        Container::new(button)
            .with_vertical_padding(8.)
            .with_padding_left(4.)
            .finish()
    }

    /// Render a toggle button for one of the search options.
    #[allow(clippy::too_many_arguments)]
    fn render_toggle_button(
        &self,
        text: &str,
        tooltip: &str,
        action: FindBarAction,
        toggled_on: bool,
        mouse_state: MouseStateHandle,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let button = ToggleButton::new(mouse_state, self.button_styles(appearance, app))
            .with_label(text)
            .with_toggled_on(toggled_on)
            .with_hovered_styles(UiComponentStyles {
                background: Some(appearance.theme().foreground_button_color().into()),
                ..Default::default()
            })
            .with_toggled_on_styles(UiComponentStyles {
                background: Some(appearance.theme().find_bar_button_selection_color().into()),
                border_color: Some(appearance.theme().accent().into()),
                ..Default::default()
            })
            .with_tooltip(
                appearance
                    .ui_builder()
                    .tool_tip(tooltip.to_string())
                    .build()
                    .finish(),
            )
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action))
            .with_cursor(Cursor::PointingHand)
            .finish();

        Container::new(button)
            .with_vertical_padding(8.)
            .with_padding_left(4.)
            .finish()
    }

    /// Shared styles for find-bar buttons.
    fn button_styles(&self, appearance: &Appearance, app: &AppContext) -> UiComponentStyles {
        let size = self.editor_height(appearance, app);
        UiComponentStyles {
            width: Some(size),
            height: Some(size),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            border_width: Some(1.),
            padding: Some(Coords::uniform(7.)),
            font_size: Some(appearance.ui_font_size()),
            font_family_id: Some(appearance.ui_font_family()),
            font_color: Some(appearance.theme().active_ui_text_color().into_solid()),
            ..Default::default()
        }
    }
}

impl Entity for FindBar {
    type Event = FindBarEvent;
}

impl View for FindBar {
    fn ui_name() -> &'static str {
        "FindBar"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        let appearance = Appearance::as_ref(app);
        let searcher = self.searcher.as_ref(app);
        let theme = appearance.theme();
        let editor_height = self.editor_height(appearance, app);
        let has_matches = searcher.match_count() > 0;

        let find_icon = Container::new(
            ConstrainedBox::new(Icon::Find.to_warpui_icon(theme.active_ui_detail()).finish())
                .with_height(editor_height)
                .with_width(editor_height)
                .finish(),
        )
        .with_padding_left(12.)
        .with_padding_top(16.)
        .with_padding_bottom(16.)
        .finish();

        let find_editor = Container::new(
            ConstrainedBox::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_children([
                        Shrinkable::new(
                            1.,
                            Clipped::new(ChildView::new(&self.query_editor).finish()).finish(),
                        )
                        .finish(),
                        self.render_match_index(appearance, app),
                    ])
                    .finish(),
            )
            .with_height(editor_height)
            .finish(),
        )
        .with_padding_left(8.)
        .with_vertical_padding(16.)
        .finish();

        let find_box = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children([
                find_icon,
                Shrinkable::new(1., find_editor).finish(),
                self.render_separator_line(appearance, app),
                self.render_action_button(
                    Icon::ChevronUp,
                    FindBarAction::FocusPreviousMatch,
                    has_matches,
                    self.button_handles.previous_match.clone(),
                    appearance,
                    app,
                ),
                self.render_action_button(
                    Icon::ChevronDown,
                    FindBarAction::FocusNextMatch,
                    has_matches,
                    self.button_handles.next_match.clone(),
                    appearance,
                    app,
                ),
                self.render_toggle_button(
                    REGEX_TOGGLE_LABEL,
                    REGEX_TOGGLE_TOOLTIP,
                    FindBarAction::ToggleRegex,
                    searcher.is_regex(),
                    self.button_handles.regex_toggle.clone(),
                    appearance,
                    app,
                ),
                self.render_toggle_button(
                    CASE_SENSITIVE_LABEL,
                    CASE_SENSITIVE_TOOLTIP,
                    FindBarAction::ToggleCaseSensitive,
                    searcher.is_case_sensitive(),
                    self.button_handles.case_sensitive_toggle.clone(),
                    appearance,
                    app,
                ),
                self.render_action_button(
                    Icon::X,
                    FindBarAction::Close,
                    true,
                    self.button_handles.close.clone(),
                    appearance,
                    app,
                ),
            ]);

        let container = Container::new(
            ConstrainedBox::new(find_box.finish())
                .with_width(FIND_BAR_WIDTH)
                .finish(),
        )
        .with_padding_right(14.)
        .with_background(theme.surface_2())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_border(Border::all(1.).with_border_fill(theme.outline()))
        .finish();

        Container::new(container)
            .with_padding_top(10.)
            .with_padding_right(20.)
            .finish()
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        // Enable auto-selection so that new search results automatically select the
        // nearest match from the cursor, avoiding a "?" in the result counter.
        self.searcher.update(ctx, |searcher, _ctx| {
            searcher.set_auto_select(true);
        });

        if focus_ctx.is_self_focused() {
            self.query_editor
                .update(ctx, |editor, ctx| editor.select_all(ctx));
            ctx.focus(&self.query_editor);

            // If reopening with cached results but no selection, select the nearest match.
            let should_select = {
                let searcher = self.searcher.as_ref(ctx);
                searcher.match_count() > 0 && searcher.selected_match().is_none()
            };
            if should_select {
                self.searcher
                    .update(ctx, |searcher, ctx| searcher.select_next_from_cursor(ctx));
            }

            // If there's a cached previous search, show the results.
            ctx.emit(FindBarEvent::SearchDecorationsChanged);
            ctx.notify();
        }
    }

    fn on_blur(&mut self, _blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        // Check if focus moved to the query editor (a child of this view).
        let focused_view_id = ctx.focused_view_id(ctx.window_id());
        let is_focus_within = focused_view_id == Some(self.query_editor.id());

        if !is_focus_within {
            self.searcher.update(ctx, |searcher, ctx| {
                searcher.clear_selected_result(ctx);
                searcher.set_auto_select(false);
            });
            ctx.notify();
        }
    }
}

impl TypedActionView for FindBar {
    type Action = FindBarAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            FindBarAction::ToggleRegex => {
                self.searcher
                    .update(ctx, |search, ctx| search.set_regex(!search.is_regex(), ctx));
                ctx.notify();
            }
            FindBarAction::ToggleCaseSensitive => {
                self.searcher.update(ctx, |search, ctx| {
                    search.set_case_sensitive(!search.is_case_sensitive(), ctx)
                });
                ctx.notify();
            }
            FindBarAction::FocusNextMatch => {
                self.searcher
                    .update(ctx, |search, ctx| search.select_next_result(ctx));
            }
            FindBarAction::FocusPreviousMatch => {
                self.searcher
                    .update(ctx, |search, ctx| search.select_previous_result(ctx));
            }
            FindBarAction::Close => {
                ctx.emit(FindBarEvent::Close);
            }
        }
    }

    fn action_accessibility_contents(
        &mut self,
        action: &Self::Action,
        ctx: &mut ViewContext<Self>,
    ) -> ActionAccessibilityContent {
        let text = match action {
            FindBarAction::ToggleRegex => {
                if self.searcher.as_ref(ctx).is_regex() {
                    "Enable regex search"
                } else {
                    "Disable regex search"
                }
            }
            FindBarAction::ToggleCaseSensitive => {
                if self.searcher.as_ref(ctx).is_case_sensitive() {
                    "Enable case-sensitive search"
                } else {
                    "Disable case-sensitive search"
                }
            }
            FindBarAction::FocusNextMatch => "Focus next match",
            FindBarAction::FocusPreviousMatch => "Focus previous match",
            FindBarAction::Close => "Close find bar",
        };
        Some(AccessibilityContent::new_without_help(
            text,
            WarpA11yRole::UserAction,
        ))
        .into()
    }
}

/// State for embedding a find bar in a rich-text editor.
pub struct FindBarState {
    bar_view: ViewHandle<FindBar>,
    is_open: bool,
    parent_position: String,
}

impl FindBarState {
    pub fn new(
        parent_position: String,
        model: ModelHandle<NotebooksEditorModel>,
        ctx: &mut ViewContext<RichTextEditorView>,
    ) -> Self {
        let bar_view = ctx.add_typed_action_view(|ctx| FindBar::new(model, ctx));
        Self {
            parent_position,
            bar_view,
            is_open: false,
        }
    }

    pub fn view(&self) -> &ViewHandle<FindBar> {
        &self.bar_view
    }

    /// Whether or not the find bar is focused.
    pub fn is_focused(&self, app: &AppContext) -> bool {
        self.bar_view.is_focused(app) || self.bar_view.as_ref(app).query_editor_focused(app)
    }

    /// Decorations to highlight find-bar matches.
    pub fn decorations(&self, app: &AppContext) -> Vec<Decoration> {
        if self.is_open {
            self.bar_view.as_ref(app).decorations(app)
        } else {
            Vec::new()
        }
    }

    /// Render the find bar, if open.
    pub fn render(&self, stack: &mut Stack) {
        if self.is_open {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.bar_view).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    self.parent_position.clone(),
                    vec2f(-4., -4.),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::TopRight,
                    ChildAnchor::TopRight,
                ),
            )
        }
    }

    /// Open and focus the find bar.
    pub fn show(&mut self, ctx: &mut ViewContext<RichTextEditorView>) {
        self.is_open = true;
        ctx.focus(&self.bar_view);
        ctx.emit(EditorViewEvent::OpenedFindBar);
        ctx.notify();
    }

    /// Hide the find bar. If search matches were highlighted, the parent view should clear them.
    pub fn hide(&mut self, ctx: &mut ViewContext<RichTextEditorView>) {
        self.is_open = false;
        ctx.focus_self();
        ctx.notify();
    }
}
