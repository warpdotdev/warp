use crate::appearance::Appearance;
use crate::search::mixer::SearchMixer;
use crate::search::search_bar::{
    CreateQueryResultRendererFn, SearchBar, SearchBarEvent, SearchBarState, SearchResultOrdering,
};
use crate::search::QueryFilter;
use itertools::Itertools;
use std::marker::PhantomData;
use std::ops::Range;
use warpui::elements::{
    ConstrainedBox, Container, Empty, Flex, ParentElement, SavePosition, ScrollStateHandle,
    Scrollable, ScrollableElement, ScrollbarWidth, Text, UniformList, UniformListState,
};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    Action, AppContext, Element, Entity, ModelHandle, SingletonEntity, View, ViewContext,
    ViewHandle, WeakViewHandle,
};

use super::styles::{ESTIMATED_RESULT_HEIGHT, MAX_DISPLAYED_RESULT_COUNT};

const HEADER_HORIZONTAL_PADDING: f32 = 16.;
const HEADER_VERTICAL_PADDING: f32 = 4.;

#[derive(Clone, Copy)]
pub struct SearchResultsMenuConfig {
    pub max_displayed_result_count: usize,
    pub estimated_result_height: f32,
    pub max_search_results: usize,
}

impl SearchResultsMenuConfig {
    // Add 0.5 of the result to make sure it's clear the menu is scrollable.
    pub fn max_height(&self) -> f32 {
        self.estimated_result_height * (self.max_displayed_result_count as f32 + 0.5)
    }
}

impl Default for SearchResultsMenuConfig {
    fn default() -> Self {
        Self {
            max_displayed_result_count: MAX_DISPLAYED_RESULT_COUNT,
            estimated_result_height: ESTIMATED_RESULT_HEIGHT,
            max_search_results: 100,
        }
    }
}

pub enum SearchResultsViewEvent<T: Action + Clone> {
    ResultAccepted(T),
}

pub struct SearchResultsMenuView<T: Action + Clone> {
    // Search results components (following AIContextMenu pattern)
    search_bar: ViewHandle<SearchBar<T>>,
    search_bar_state: ModelHandle<SearchBarState<T>>,
    scroll_state: ScrollStateHandle,
    uniform_list_state: UniformListState,
    handle: WeakViewHandle<Self>,
    config: SearchResultsMenuConfig,
    phantom: PhantomData<T>,
}

impl<T: Action + Clone> SearchResultsMenuView<T> {
    pub fn new(
        config: SearchResultsMenuConfig,
        mixer: ModelHandle<SearchMixer<T>>,
        query_result_renderer: CreateQueryResultRendererFn<T>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // Set up SearchBar system (following AIContextMenu pattern)
        let search_bar_state = ctx.add_model(|_ctx| {
            SearchBarState::new(SearchResultOrdering::TopDown)
                .with_max_results(config.max_search_results)
                .run_query_on_buffer_empty()
        });

        ctx.observe(&search_bar_state, |_, _, ctx| {
            ctx.notify();
        });

        let search_bar = ctx.add_typed_action_view(|ctx| {
            SearchBar::new(
                mixer.clone(),
                search_bar_state.clone(),
                "", // No placeholder for slash commands
                query_result_renderer,
                ctx,
            )
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
            scroll_state: Default::default(),
            uniform_list_state: Default::default(),
            handle: ctx.handle(),
            config,
            phantom: PhantomData,
        }
    }

    /// Returns the mixer handle from the search bar.
    pub fn mixer(&self, app: &AppContext) -> ModelHandle<SearchMixer<T>> {
        self.search_bar.as_ref(app).mixer()
    }

    pub fn render_results(
        &self,
        selected_index: Option<usize>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let view_handle = self.handle.clone();

        let build_items = move |range: Range<usize>, app: &AppContext| {
            let search_results_menu = view_handle
                .upgrade(app)
                .expect("View handle should be upgradeable.");
            let search_results_menu_ref = search_results_menu.as_ref(app);
            let query_result_renderers = search_results_menu_ref
                .search_bar_state
                .as_ref(app)
                .query_result_renderers();

            match query_result_renderers {
                Some(query_result_renderers) => {
                    let query_result_iter = if range.end == 1 {
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
                                    selected_index == Some(result_index),
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

        let item_count = self
            .search_bar_state
            .as_ref(app)
            .query_result_renderers()
            .map(|renderers| renderers.len())
            .unwrap_or(0);

        let max_height = self.config.max_height();
        ConstrainedBox::new(
            Scrollable::vertical(
                self.scroll_state.clone(),
                UniformList::new(self.uniform_list_state.clone(), item_count, build_items)
                    .finish_scrollable(),
                ScrollbarWidth::Auto,
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                warpui::elements::Fill::None,
            )
            .with_overlayed_scrollbar()
            .finish(),
        )
        .with_max_height(max_height)
        .finish()
    }

    pub fn render_no_results(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        Container::new(
            Text::new(
                "No results found",
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(theme.main_text_color(theme.background()).into_solid())
            .finish(),
        )
        .with_uniform_padding(10.0) // Use a reasonable default padding
        .finish()
    }

    pub fn scroll_selected_index_into_view(&self, index: usize, ctx: &mut ViewContext<Self>) {
        self.uniform_list_state.scroll_to(index);
        ctx.notify();
    }

    pub fn select_current_item(&self, ctx: &mut ViewContext<Self>) {
        self.search_bar.update(ctx, |search_bar, ctx| {
            search_bar.select_current_item(ctx);
        });
    }

    pub fn select_prev(&self, ctx: &mut ViewContext<Self>) {
        self.search_bar.update(ctx, |search_bar, ctx| {
            search_bar.up(ctx);
        });
    }

    pub fn select_next(&self, ctx: &mut ViewContext<Self>) {
        self.search_bar.update(ctx, |search_bar, ctx| {
            search_bar.down(ctx);
        });
    }

    // TODO(moira): refactor to shared component with AIContextMenu
    fn handle_search_bar_event(&mut self, event: &SearchBarEvent<T>, ctx: &mut ViewContext<Self>) {
        match event {
            SearchBarEvent::ResultSelected { index } => {
                self.scroll_selected_index_into_view(*index, ctx);
            }
            SearchBarEvent::ResultAccepted { action, .. } => {
                ctx.emit(SearchResultsViewEvent::ResultAccepted(action.clone()));
                ctx.notify();
            }
            SearchBarEvent::Close => {
                // For generic search results, we don't handle close directly.
                // The parent view should handle this through the search bar subscription.
                ctx.notify();
            }
            // All other events we can ignore
            _ => {}
        }
    }

    /// Update the menu based on the filter text - switches between zero state and search results
    /// Returns whether there are match results.
    pub fn update_search_filter(&mut self, filter_text: &str, ctx: &mut ViewContext<Self>) {
        self.search_bar.update(ctx, |search_bar, ctx| {
            search_bar.set_query(filter_text.to_string(), ctx);
        });
        ctx.notify();
    }

    pub fn has_results(&self, ctx: &AppContext) -> bool {
        self.search_bar_state
            .as_ref(ctx)
            .query_result_renderers()
            .map(|results| !results.is_empty())
            .unwrap_or_default()
    }

    pub fn set_active_query_filter(
        &mut self,
        filter: Option<QueryFilter>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.search_bar.update(ctx, |view, ctx| {
            view.set_query_filter(
                filter.map(|filter| (filter, filter.filter_atom().primary_text)),
                ctx,
            )
        });
        ctx.notify();
    }

    /// Render the search results using ScrollableMenu
    // TODO(moira): refactor to shared component with AIContextMenu
    pub fn render_search_results(&self, app: &AppContext) -> Box<dyn Element> {
        let state = self.search_bar_state.as_ref(app);
        let selected_index = state.selected_index();
        let query_result_renderers = state.query_result_renderers();

        let active_filter = state.active_query_filter();
        let appearance = Appearance::as_ref(app);

        let mut column = Flex::column();

        if let Some(title) = active_filter.and_then(renderable_title_name) {
            column.add_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .span(title)
                        .with_style(UiComponentStyles {
                            font_color: Some(
                                appearance
                                    .theme()
                                    .sub_text_color(appearance.theme().background())
                                    .into(),
                            ),
                            font_size: Some(12.),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_padding_bottom(HEADER_VERTICAL_PADDING)
                .with_horizontal_padding(HEADER_HORIZONTAL_PADDING)
                .finish(),
            );
        }

        column.add_child(match query_result_renderers {
            Some(query_result_renderers) if query_result_renderers.is_empty() => {
                self.render_no_results(app)
            }
            Some(_query_result_renderers) => self.render_results(selected_index, app),
            None => Empty::new().finish(),
        });

        column.finish()
    }
}

impl<T: Action + Clone> Entity for SearchResultsMenuView<T> {
    type Event = SearchResultsViewEvent<T>;
}

impl<T: Action + Clone> View for SearchResultsMenuView<T> {
    fn ui_name() -> &'static str {
        "SearchResultsMenuView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.render_search_results(app)
    }
}

fn renderable_title_name(query_filter: QueryFilter) -> Option<&'static str> {
    if matches!(query_filter, QueryFilter::AgentModeWorkflows) {
        return Some("Prompts");
    }

    None
}
