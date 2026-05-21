use itertools::Itertools;
use lazy_static::lazy_static;
use std::{collections::HashSet, ops::Range};

use warpui::{
    elements::{
        Align, ConstrainedBox, Container, CornerRadius, Dismiss, Empty, Fill, Flex, ParentElement,
        Radius, SavePosition, ScrollStateHandle, Scrollable, ScrollableElement, Shrinkable,
        UniformList, UniformListState,
    },
    presenter::ChildView,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Element, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle, WeakViewHandle,
};

use crate::{
    appearance::Appearance,
    external_secrets::ExternalSecret,
    search::{
        external_secrets::{
            external_secret_data_source::ExternalSecretDataSource,
            searcher::{ExternalSecretSearchItemAction, ExternalSecretSearchMixer},
        },
        result_renderer::{QueryResultRenderer, QueryResultRendererStyles},
        search_bar::{SearchBar, SearchBarEvent, SearchBarState, SearchResultOrdering},
    },
};

lazy_static! {
    static ref QUERY_RESULT_RENDERER_STYLES: QueryResultRendererStyles =
        QueryResultRendererStyles {
            result_item_height_fn: |appearance| {
                styles::line_height_sensitive_vertical_padding(appearance)
                    + styles::name_font_size(appearance)
            },
            panel_drop_shadow: styles::panel_drop_shadow(),
            panel_corner_radius: CornerRadius::with_all(Radius::Pixels(styles::CORNER_RADIUS)),
            result_vertical_padding: 4.,
            ..Default::default()
        };
}

const DEFAULT_PLACEHOLDER_TEXT: &str = "Search for a secret";

pub struct ExternalSecretsMenu {
    scroll_state: ScrollStateHandle,
    list_state: UniformListState,
    search_bar: ViewHandle<SearchBar<ExternalSecretSearchItemAction>>,
    search_bar_state: ModelHandle<SearchBarState<ExternalSecretSearchItemAction>>,
    mixer: ModelHandle<ExternalSecretSearchMixer>,
    handle: WeakViewHandle<Self>,
}

#[derive(Clone, Debug)]
pub enum ExternalSecretsMenuAction {
    ResultClicked {
        result_index: usize,
        result_action: Box<ExternalSecretSearchItemAction>,
    },
    Close,
}

pub enum ExternalSecretsMenuEvent {
    ItemSelected {
        payload: Box<ExternalSecretSearchItemAction>,
    },
    Close,
    Open,
}

impl ExternalSecretsMenu {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let appearance = Appearance::as_ref(ctx);
        let ui_font_family = appearance.ui_font_family();

        let search_bar_state = ctx.add_model(|_| {
            SearchBarState::new(SearchResultOrdering::TopDown).run_query_on_buffer_empty()
        });

        ctx.observe(&search_bar_state, |_, _, ctx| {
            ctx.notify();
        });

        let mixer = ctx.add_model(|_| ExternalSecretSearchMixer::new());

        let search_bar = ctx.add_typed_action_view(|ctx| {
            SearchBar::new(
                mixer.clone(),
                search_bar_state.clone(),
                DEFAULT_PLACEHOLDER_TEXT,
                |result_index, result| {
                    QueryResultRenderer::new(
                        result,
                        format!("QueryResultRenderer:{result_index}"),
                        |result_index, result_action, event_ctx| {
                            event_ctx.dispatch_typed_action(
                                ExternalSecretsMenuAction::ResultClicked {
                                    result_index,
                                    result_action: Box::new(result_action),
                                },
                            )
                        },
                        *QUERY_RESULT_RENDERER_STYLES,
                    )
                },
                ctx,
            )
            .with_font_family(ui_font_family, ctx)
        });

        ctx.subscribe_to_view(&search_bar, |me, _handle, event, ctx| {
            me.handle_search_bar_event(event, ctx);
        });

        ctx.subscribe_to_model(&search_bar_state, |me, _handle, event, ctx| {
            me.handle_search_bar_event(event, ctx);
        });

        Self {
            search_bar,
            search_bar_state,
            mixer,
            handle: ctx.handle(),
            scroll_state: Default::default(),
            list_state: Default::default(),
        }
    }

    pub fn setup(&mut self, secrets: Vec<ExternalSecret>, ctx: &mut ViewContext<Self>) {
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.reset(ctx);

            mixer.add_sync_source(ExternalSecretDataSource::new(secrets), HashSet::new());
            ctx.notify();
        });

        self.search_bar.update(ctx, |search_bar, ctx| {
            search_bar.reset(None, None, SearchResultOrdering::TopDown, ctx);
            ctx.notify();
        });

        ctx.emit(ExternalSecretsMenuEvent::Open);
    }

    pub fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(ExternalSecretsMenuEvent::Close);
    }

    fn handle_search_bar_event(
        &mut self,
        event: &SearchBarEvent<ExternalSecretSearchItemAction>,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SearchBarEvent::Close => self.close(ctx),
            SearchBarEvent::BufferCleared { .. } => {}
            SearchBarEvent::ResultAccepted { action, .. } => {
                self.handle_result_selected(action.clone(), ctx);
            }
            SearchBarEvent::ResultSelected { index } => {
                self.list_state.scroll_to(*index);
                ctx.notify();
            }
            SearchBarEvent::QueryFilterChanged { .. } => {}
            SearchBarEvent::SelectionUpdateInZeroState { .. } => {}
            SearchBarEvent::EnterInZeroState { .. } => {}
        }
    }

    fn handle_result_selected(
        &mut self,
        result_action: ExternalSecretSearchItemAction,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(ExternalSecretsMenuEvent::ItemSelected {
            payload: Box::new(result_action),
        });
        self.close(ctx);
    }

    fn render_no_results(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        // There are no results to display, so notify the user of that fact.
        let text = appearance
            .ui_builder()
            .span(crate::i18n::tr_static(app, "No results found."))
            .with_style(UiComponentStyles {
                font_size: Some(appearance.monospace_font_size()),
                font_family_id: Some(appearance.ui_font_family()),
                font_color: Some(appearance.theme().nonactive_ui_text_color().into()),
                ..Default::default()
            })
            .build()
            .finish();

        let vertical_padding = styles::line_height_sensitive_vertical_padding(appearance);
        Container::new(
            ConstrainedBox::new(Align::new(text).finish())
                // Make the height the same as a single item, but adjust by the panel padding so
                // the text is centered within the panel.
                .with_height(
                    appearance.monospace_font_size() + vertical_padding - styles::TOP_PADDING,
                )
                .finish(),
        )
        .with_margin_bottom(styles::TOP_PADDING)
        .finish()
    }

    fn render_present_results(
        &self,
        appearance: &Appearance,
        selected_index: usize,
        query_result_renderers: &[QueryResultRenderer<ExternalSecretSearchItemAction>],
    ) -> Box<dyn Element> {
        let view_handle = self.handle.clone();
        let build_items = move |range: Range<usize>, app: &AppContext| {
            let secrets_view = view_handle
                .upgrade(app)
                .expect("View handle should upgradeable.")
                .as_ref(app);
            let query_result_renderers = secrets_view
                .search_bar_state
                .as_ref(app)
                .query_result_renderers();
            match query_result_renderers {
                Some(query_result_renderers) => {
                    let query_result_iter = if range.end == 1 {
                        // Despite being upper-bound exclusive, taking a slice where
                        // the end of the range is out of bounds results in a panic.
                        query_result_renderers[range.start..].iter()
                    } else {
                        query_result_renderers[range.start..range.end].iter()
                    };
                    query_result_iter
                        .enumerate()
                        .map(|(result_index, result_renderer)| {
                            let result_index = result_index + range.start;
                            SavePosition::new(
                                result_renderer.render(
                                    result_index,
                                    result_index == selected_index,
                                    app,
                                ),
                                result_renderer.position_id.as_str(),
                            )
                            .finish()
                        })
                        .collect_vec()
                        .into_iter()
                }
                None => Vec::new().into_iter(),
            }
        };

        let scrollable_results = Scrollable::vertical(
            self.scroll_state.clone(),
            UniformList::new(
                self.list_state.clone(),
                query_result_renderers.len(),
                build_items,
            )
            .finish_scrollable(),
            styles::SCROLLBAR_WIDTH,
            appearance.theme().nonactive_ui_detail().into(),
            appearance.theme().active_ui_detail().into(),
            Fill::None,
        )
        .finish();

        ConstrainedBox::new(scrollable_results)
            .with_max_height(styles::VIEW_HEIGHT)
            .finish()
    }

    fn render_results(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let query_result_renderers = self.search_bar_state.as_ref(app).query_result_renderers();
        let selected_index = self.search_bar_state.as_ref(app).selected_index();
        match (query_result_renderers, selected_index) {
            (Some(query_result_renderers), _) if query_result_renderers.is_empty() => {
                self.render_no_results(appearance, app)
            }
            (Some(query_result_renderers), Some(selected_index)) => {
                self.render_present_results(appearance, selected_index, query_result_renderers)
            }
            _ => Empty::new().finish(),
        }
    }

    fn render_input_area(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(ChildView::new(&self.search_bar).finish())
            .with_background(styles::search_bar_overlay(appearance))
            .with_corner_radius(CornerRadius::with_top(Radius::Pixels(
                styles::CORNER_RADIUS,
            )))
            .with_border(styles::panel_border(appearance).with_sides(true, true, false, true))
            .with_uniform_padding(12.)
            .finish()
    }
}

impl Entity for ExternalSecretsMenu {
    type Event = ExternalSecretsMenuEvent;
}

impl View for ExternalSecretsMenu {
    fn ui_name() -> &'static str {
        "ExternalSecretsMenu"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.search_bar);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let panel_children = vec![
            self.render_input_area(appearance),
            Shrinkable::new(
                1.,
                Container::new(self.render_results(appearance, app))
                    .with_padding_top(styles::TOP_PADDING)
                    .with_background(styles::panel_background_fill(appearance))
                    .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(
                        styles::CORNER_RADIUS,
                    )))
                    .with_border(
                        styles::panel_border(appearance).with_sides(false, true, true, true),
                    )
                    .finish(),
            )
            .finish(),
        ];

        let panel_contents =
            ConstrainedBox::new(Flex::column().with_children(panel_children).finish())
                .with_max_width(styles::VIEW_WIDTH)
                .finish();

        Dismiss::new(
            Container::new(panel_contents)
                .with_drop_shadow(styles::panel_drop_shadow())
                .finish(),
        )
        .on_dismiss(|ctx, _app| {
            ctx.dispatch_typed_action(ExternalSecretsMenuAction::Close);
        })
        .finish()
    }
}

impl TypedActionView for ExternalSecretsMenu {
    type Action = ExternalSecretsMenuAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ExternalSecretsMenuAction::Close => self.close(ctx),
            ExternalSecretsMenuAction::ResultClicked { result_action, .. } => {
                self.handle_result_selected(*result_action.clone(), ctx)
            }
        }
    }
}

pub mod styles {
    use pathfinder_color::ColorU;
    use warpui::elements::{Border, DropShadow, ScrollbarWidth};

    use crate::{appearance::Appearance, themes::theme::Fill};

    pub const CORNER_RADIUS: f32 = 6.;
    pub const VIEW_WIDTH: f32 = 450.;
    pub const VIEW_HEIGHT: f32 = 450.;
    pub const TOP_PADDING: f32 = CORNER_RADIUS;
    pub const SCROLLBAR_WIDTH: ScrollbarWidth = ScrollbarWidth::Auto;

    /// Returns the `Fill` to be used as the background of the search results panel and details
    /// panel.
    pub fn panel_background_fill(appearance: &Appearance) -> Fill {
        appearance.theme().surface_2()
    }

    pub fn search_bar_overlay(appearance: &Appearance) -> Fill {
        appearance.theme().surface_1()
    }

    /// Returns the `DropShadow` for both the search results panel and details panel.
    pub fn panel_drop_shadow() -> DropShadow {
        DropShadow::new_with_standard_offset_and_spread(ColorU::new(0, 0, 0, 64))
    }

    /// Returns the baseline `Border` settings (not applied to any sides)
    pub fn panel_border(appearance: &Appearance) -> Border {
        Border::new(1.).with_border_fill(appearance.theme().surface_3())
    }

    /// Returns a vertical padding value that is sensitive to the user's line height setting. This
    /// value is used to determine the height of each result in the panel.
    pub fn line_height_sensitive_vertical_padding(appearance: &Appearance) -> f32 {
        appearance.line_height_ratio() * name_font_size(appearance)
    }

    /// The font size for the object name in search results.
    pub fn name_font_size(appearance: &Appearance) -> f32 {
        appearance.ui_font_size() + 2.
    }
}
