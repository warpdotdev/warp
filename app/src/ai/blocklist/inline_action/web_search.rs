use warp_core::ui::appearance::Appearance;
use warpui::elements::{Container, CrossAxisAlignment, Element, Flex, ParentElement, Text};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use super::search_results_common::{
    CollapsibleSearchResultsState, render_collapsible_search_results,
};
use crate::ai::agent::WebSearchStatus;
use crate::ai::agent::icons::{failed_icon, yellow_running_icon};
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
        // The proto error status carries no query; preserve the prior one for the header.
        self.status = match status {
            WebSearchStatus::Error { query } if query.is_empty() => WebSearchStatus::Error {
                query: self.current_query(),
            },
            other => other.clone(),
        };
    }

    fn current_query(&self) -> String {
        match &self.status {
            WebSearchStatus::Searching { query } => query.clone().unwrap_or_default(),
            WebSearchStatus::Success { query, .. } => query.clone(),
            WebSearchStatus::Error { query } => query.clone(),
        }
    }

    fn render_loading(&self, query: &Option<String>, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let loading_icon = yellow_running_icon(appearance);

        let text = if let Some(q) = query {
            format!("Searching the web for \"{q}\"")
        } else {
            "Searching the web".to_string()
        };

        super::search_results_common::render_status_header(text, loading_icon, app)
    }

    fn render_success(
        &self,
        query: &str,
        pages: &[(String, String)],
        app: &AppContext,
    ) -> Box<dyn Element> {
        let title_text = if query.is_empty() {
            "Searched the web".to_string()
        } else {
            format!("Searched the web for \"{query}\"")
        };

        let body = if self.collapsible.is_expanded {
            Some(self.render_urls_list(pages, app))
        } else {
            None
        };

        render_collapsible_search_results(
            title_text,
            pages.len(),
            "URLs",
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
                "No URLs found".to_string(),
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
                let appearance = Appearance::as_ref(app);
                let text = if query.is_empty() {
                    "Web search failed".to_string()
                } else {
                    format!("Web search failed for \"{query}\"")
                };
                super::search_results_common::render_status_header(
                    text,
                    failed_icon(appearance),
                    app,
                )
                .with_agent_output_item_spacing(app)
                .finish()
            }
        }
    }
}
