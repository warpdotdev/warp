use warp_core::ui::appearance::Appearance;
use warpui::elements::{Container, CrossAxisAlignment, Element, Flex, ParentElement, Text};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use super::search_results_common::{
    render_collapsible_search_results, CollapsibleSearchResultsState,
};
use crate::ai::agent::icons::yellow_running_icon;
use crate::ai::agent::WebSearchStatus;
use crate::ai::blocklist::block::view_impl::WithContentItemSpacing;

pub enum WebSearchViewEvent {}

#[derive(Clone, Debug)]
pub enum WebSearchViewAction {
    ToggleExpanded,
}

pub struct WebSearchView {
    pub status: WebSearchStatus,
    pub collapsible: CollapsibleSearchResultsState,
}

impl WebSearchView {
    pub fn new(query: String) -> Self {
        Self {
            status: WebSearchStatus::Searching {
                query: if query.is_empty() { None } else { Some(query) },
            },
            collapsible: CollapsibleSearchResultsState::new(),
        }
    }

    pub fn set_status(&mut self, status: &WebSearchStatus) {
        self.status = status.clone();
    }

    fn render_loading(&self, query: &Option<String>, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let loading_icon = yellow_running_icon(appearance);

        let text = if let Some(q) = query {
            i18n::t("ai.web_search.searching_for").replace("{query}", q)
        } else {
            i18n::t("ai.web_search.searching")
        };

        super::search_results_common::render_loading_header(text, loading_icon, app)
    }

    fn render_success(
        &self,
        query: &str,
        pages: &[(String, String)],
        app: &AppContext,
    ) -> Box<dyn Element> {
        let title_text = if query.is_empty() {
            i18n::t("ai.web_search.searched")
        } else {
            i18n::t("ai.web_search.searched_for").replace("{query}", query)
        };

        let body = if self.collapsible.is_expanded {
            Some(self.render_urls_list(pages, app))
        } else {
            None
        };

        render_collapsible_search_results(
            title_text,
            pages.len(),
            &i18n::t("ai.search_results.urls"),
            &self.collapsible,
            body,
            |ctx| {
                ctx.dispatch_typed_action(WebSearchViewAction::ToggleExpanded);
            },
            app,
        )
    }

    fn render_urls_list(&self, pages: &[(String, String)], app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        for (url, title) in pages {
            let display_text = if title.is_empty() {
                url.clone()
            } else {
                format!("{title} ({url})")
            };

            let url_text = Text::new_inline(
                display_text,
                appearance.ui_font_family(),
                appearance.monospace_font_size() - 1.0,
            )
            .with_color(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().surface_1())
                    .into(),
            )
            .finish();

            column.add_child(Container::new(url_text).with_vertical_padding(2.).finish());
        }

        if pages.is_empty() {
            let no_results = Text::new_inline(
                i18n::t("ai.web_search.no_urls_found"),
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().surface_1())
                    .into(),
            )
            .finish();
            column.add_child(no_results);
        }

        column.finish()
    }
}

impl Entity for WebSearchView {
    type Event = WebSearchViewEvent;
}

impl TypedActionView for WebSearchView {
    type Action = WebSearchViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            WebSearchViewAction::ToggleExpanded => {
                self.collapsible.toggle_expanded();
                ctx.notify();
            }
        }
    }
}

impl View for WebSearchView {
    fn ui_name() -> &'static str {
        "WebSearchView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        match &self.status {
            WebSearchStatus::Searching { query } => self
                .render_loading(query, app)
                .with_agent_output_item_spacing(app)
                .finish(),
            WebSearchStatus::Success { query, pages } => self
                .render_success(query, pages, app)
                .with_agent_output_item_spacing(app)
                .finish(),
            WebSearchStatus::Error { query } => {
                // For now, render as if search completed with no results
                // TODO(advait): Add proper error rendering
                self.render_success(query, &[], app)
                    .with_agent_output_item_spacing(app)
                    .finish()
            }
        }
    }
}
