use std::ops::Range;
use std::sync::{Arc, RwLock};

use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Container, CornerRadius, CrossAxisAlignment, Element, Flex, FormattedTextElement,
    MainAxisAlignment, ParentElement, Radius, SelectableArea, SelectionHandle, Shrinkable,
};
use warpui::ui_components::components::UiComponentStyles;
use warpui::{
    AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
};

use super::search_results_common::{
    render_collapsible_search_results, CollapsibleSearchResultsState,
};
use crate::ai::agent::icons::yellow_running_icon;
use crate::ai::agent::FileContext;
use crate::ai::blocklist::action_model::AIActionStatus;
use crate::ai::blocklist::block::find::FindState;
use crate::ai::blocklist::block::secret_redaction::SecretRedactionState;
use crate::ai::blocklist::block::view_impl::output::{
    render_read_files_text, LinkActionConstructors, RenderContext, RenderReadFileArg,
};
use crate::ai::blocklist::block::view_impl::{FindContext, WithContentItemSpacing};
use crate::ai::blocklist::inline_action::inline_action_header::{
    INLINE_ACTION_HEADER_VERTICAL_PADDING, INLINE_ACTION_HORIZONTAL_PADDING,
};
use crate::ai::blocklist::inline_action::inline_action_icons::cancelled_icon;
use crate::ai::blocklist::TextLocation;
use crate::terminal::find::TerminalFindModel;
use crate::terminal::view::RichContentLink;
use crate::terminal::ShellLaunchData;
use crate::util::link_detection::{
    detect_links, DetectedLinkType, DetectedLinksState, LinkLocation,
};

pub enum SearchCodebaseViewEvent {
    OpenLinkTooltip {
        rich_content_link: RichContentLink,
    },
    #[cfg(feature = "local_fs")]
    OpenDetectedFilePath {
        absolute_path: std::path::PathBuf,
        line_and_column_num: Option<warp_util::path::LineAndColumnArg>,
    },
    TextSelected,
}

#[derive(Clone, Debug)]
pub enum SearchCodebaseViewAction {
    ToggleExpanded,
    OpenLink {
        link_range: Range<usize>,
        location: TextLocation,
    },
    ChangedHoverOnLink {
        link_range: Range<usize>,
        location: TextLocation,
        is_hovering: bool,
    },
    OpenLinkTooltip {
        location: TextLocation,
        link_range: Range<usize>,
    },
    SelectText,
}
pub struct SearchCodebaseView {
    shell_launch_data: Option<ShellLaunchData>,
    current_working_directory: Option<String>,
    detected_links_state: DetectedLinksState,
    secret_redaction_state: SecretRedactionState,
    find_model: ModelHandle<TerminalFindModel>,
    find_state: FindState,
    file_contexts: Vec<FileContext>,
    search_query: String,
    repo_name: Option<String>,
    collapsible: CollapsibleSearchResultsState,
    selection_handle: SelectionHandle,
    selected_text: Arc<RwLock<Option<String>>>,
    action_index: usize,
    status: Option<AIActionStatus>,
}

impl SearchCodebaseView {
    pub fn new(
        find_model: ModelHandle<TerminalFindModel>,
        file_contexts: Vec<FileContext>,
        search_query: String,
        repo_path: Option<String>,
        shell_launch_data: &Option<ShellLaunchData>,
        current_working_directory: &Option<String>,
        action_index: usize,
    ) -> Self {
        let repo_name = repo_path.as_ref().and_then(|path| {
            std::path::Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(|s| s.to_string())
        });

        Self {
            detected_links_state: DetectedLinksState::default(),
            secret_redaction_state: Default::default(),
            find_model,
            find_state: FindState::default(),
            file_contexts,
            search_query,
            repo_name,
            collapsible: CollapsibleSearchResultsState::new(),
            selection_handle: SelectionHandle::default(),
            selected_text: Arc::new(RwLock::new(None)),
            action_index,
            status: None,
            shell_launch_data: shell_launch_data.clone(),
            current_working_directory: current_working_directory.clone(),
        }
    }

    pub fn update_status(&mut self, status: Option<AIActionStatus>) {
        self.status = status;
    }

    pub fn update_render_read_file_args(
        &mut self,
        find_state: &FindState,
        file_contexts: Vec<FileContext>,
        status: Option<AIActionStatus>,
    ) {
        self.find_state = find_state.clone();
        self.file_contexts = file_contexts;
        self.status = status;

        self.detected_links_state.detected_links_by_location.clear();
        for (line_index, file_context) in self.file_contexts.iter().enumerate() {
            let text_location = TextLocation::Action {
                action_index: self.action_index,
                line_index,
            };
            let file_display = file_context.to_string();
            detect_links(
                &mut self.detected_links_state,
                &file_display,
                text_location,
                self.current_working_directory.as_ref(),
                self.shell_launch_data.as_ref(),
            );
        }
    }

    fn render_finished(
        &self,
        _appearance: &Appearance,
        file_contexts: &[FileContext],
        app: &AppContext,
    ) -> Box<dyn Element> {
        let title_text = if let Some(repo_name) = &self.repo_name {
            format!("Searched for \"{}\" in {}", self.search_query, repo_name)
        } else {
            format!("Searched for \"{}\"", self.search_query)
        };

        let body = if self.collapsible.is_expanded {
            Some(self.render_results_body(file_contexts, app))
        } else {
            None
        };

        render_collapsible_search_results(
            title_text,
            file_contexts.len(),
            "results",
            &self.collapsible,
            body,
            |ctx| {
                ctx.dispatch_typed_action(SearchCodebaseViewAction::ToggleExpanded);
            },
            app,
        )
    }

    fn render_results_body(
        &self,
        file_contexts: &[FileContext],
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let render_read_file_args = RenderReadFileArg::new(
            RenderContext {
                shell_launch_data: self.shell_launch_data.as_ref(),
                current_working_directory: self.current_working_directory.as_ref(),
                detected_links_state: &self.detected_links_state,
                secret_redaction_state: &self.secret_redaction_state,
            },
            self.find_model
                .as_ref(app)
                .is_find_bar_open()
                .then_some(FindContext {
                    model: self.find_model.as_ref(app),
                    state: &self.find_state,
                }),
            self.selection_handle.is_selecting(),
            LinkActionConstructors {
                construct_open_link_action: |link_range, location| {
                    SearchCodebaseViewAction::OpenLink {
                        link_range,
                        location,
                    }
                },
                construct_open_link_tooltip_action: |link_range, location| {
                    SearchCodebaseViewAction::OpenLinkTooltip {
                        link_range,
                        location,
                    }
                },
                construct_changed_hover_on_link_action: |link_range, location, is_hovering| {
                    SearchCodebaseViewAction::ChangedHoverOnLink {
                        link_range,
                        location,
                        is_hovering,
                    }
                },
            },
        );

        let files: Box<dyn Element> = if file_contexts.is_empty() {
            let no_results_style = UiComponentStyles {
                font_family_id: Some(appearance.ui_font_family()),
                font_size: Some(appearance.monospace_font_size()),
                ..Default::default()
            };
            self.render_formatted_text("No results found".to_string(), no_results_style, appearance)
        } else {
            render_read_files_text(
                render_read_file_args,
                file_contexts.iter().map(|fc| fc.to_string()),
                app,
                appearance,
                self.action_index,
            )
            .finish()
        };

        let selected_text = self.selected_text.clone();
        SelectableArea::new(
            self.selection_handle.clone(),
            #[allow(clippy::unwrap_used)]
            move |selection_args, _, _| {
                *selected_text.write().unwrap() = selection_args.selection;
            },
            files,
        )
        .on_selection_updated(|ctx, _| {
            ctx.dispatch_typed_action(SearchCodebaseViewAction::SelectText);
        })
        .finish()
    }

    fn create_header_row(&self) -> Flex {
        Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
    }

    fn create_header_text_style(
        &self,
        appearance: &Appearance,
        header_background: warp_core::ui::theme::Fill,
    ) -> UiComponentStyles {
        UiComponentStyles {
            font_family_id: Some(appearance.ui_font_family()),
            font_size: Some(appearance.monospace_font_size()),
            font_color: Some(
                appearance
                    .theme()
                    .main_text_color(header_background)
                    .into_solid(),
            ),
            ..Default::default()
        }
    }

    fn create_header_container(
        &self,
        header_row: Box<dyn Element>,
        header_background: warp_core::ui::theme::Fill,
        corner_radius: CornerRadius,
        app: &AppContext,
    ) -> Box<dyn Element> {
        Container::new(header_row)
            .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_vertical_padding(INLINE_ACTION_HEADER_VERTICAL_PADDING)
            .with_background(header_background)
            .with_corner_radius(corner_radius)
            .finish()
            .with_agent_output_item_spacing(app)
            .finish()
    }

    fn render_simple_header(&self, text: String, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let header_background = theme.surface_2();
        let text_style = self.create_header_text_style(appearance, header_background);

        let mut header_row = self.create_header_row();

        let title_element = self.render_formatted_text(text, text_style, appearance);

        header_row.add_child(Shrinkable::new(1.0, title_element).finish());

        self.create_header_container(
            header_row.finish(),
            header_background,
            CornerRadius::with_all(Radius::Pixels(8.)),
            app,
        )
    }

    fn render_header(
        &self,
        _appearance: &Appearance,
        text: String,
        icon: warpui::elements::Icon,
        app: &AppContext,
    ) -> Box<dyn Element> {
        super::search_results_common::render_loading_header(text, icon, app)
    }

    pub fn clear_link_tooltip(&mut self, ctx: &mut ViewContext<Self>) {
        self.detected_links_state.link_location_open_tooltip = None;
        ctx.notify();
    }

    pub fn clear_selection(&mut self, _ctx: &mut ViewContext<Self>) {
        self.selection_handle.clear();
    }

    pub fn selected_text(&self, _ctx: &AppContext) -> Option<String> {
        self.selected_text.read().unwrap().clone()
    }

    fn render_formatted_text(
        &self,
        text: String,
        style: UiComponentStyles,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let font_family = style
            .font_family_id
            .unwrap_or_else(|| appearance.ui_font_family());
        let font_size = style.font_size.unwrap_or_else(|| appearance.ui_font_size());
        let text_color = style.font_color.unwrap_or_else(|| {
            appearance
                .theme()
                .main_text_color(appearance.theme().surface_1())
                .into_solid()
        });

        FormattedTextElement::from_str(text, font_family, font_size)
            .with_color(text_color)
            .finish()
    }
}

impl Entity for SearchCodebaseView {
    type Event = SearchCodebaseViewEvent;
}

impl TypedActionView for SearchCodebaseView {
    type Action = SearchCodebaseViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SearchCodebaseViewAction::ToggleExpanded => {
                self.collapsible.toggle_expanded();
                ctx.notify();
            }
            SearchCodebaseViewAction::OpenLink {
                link_range,
                location,
            } => {
                if self
                    .detected_links_state
                    .link_at(location, link_range)
                    .is_some()
                {
                    match self.detected_links_state.link_at(location, link_range) {
                        Some(DetectedLinkType::Url(link)) => {
                            ctx.open_url(link);
                        }
                        #[cfg(feature = "local_fs")]
                        Some(DetectedLinkType::FilePath {
                            absolute_path,
                            line_and_column_num,
                        }) => ctx.emit(SearchCodebaseViewEvent::OpenDetectedFilePath {
                            absolute_path: absolute_path.clone(),
                            line_and_column_num: *line_and_column_num,
                        }),
                        None => (),
                    }
                }
            }
            SearchCodebaseViewAction::ChangedHoverOnLink {
                link_range,
                location,
                is_hovering,
            } => {
                self.detected_links_state.update_hovered_link(
                    *is_hovering,
                    self.selection_handle.is_selecting(),
                    link_range,
                    location,
                );
            }
            SearchCodebaseViewAction::OpenLinkTooltip {
                link_range,
                location,
            } => {
                if let Some(link_type) = self.detected_links_state.link_at(location, link_range) {
                    let rich_content_link: RichContentLink = match link_type {
                        DetectedLinkType::Url(link) => RichContentLink::Url(link.clone()),
                        #[cfg(feature = "local_fs")]
                        DetectedLinkType::FilePath {
                            absolute_path,
                            line_and_column_num,
                        } => RichContentLink::FilePath {
                            absolute_path: absolute_path.to_owned(),
                            line_and_column_num: *line_and_column_num,
                            target_override: None,
                        },
                    };

                    self.detected_links_state.link_location_open_tooltip = Some(LinkLocation {
                        link_range: link_range.clone(),
                        location: *location,
                    });

                    ctx.emit(SearchCodebaseViewEvent::OpenLinkTooltip { rich_content_link });
                    ctx.notify();
                }
            }
            SearchCodebaseViewAction::SelectText => {
                ctx.emit(SearchCodebaseViewEvent::TextSelected);
            }
        }
    }
}

impl View for SearchCodebaseView {
    fn ui_name() -> &'static str {
        "SearchCodebaseView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        match &self.status {
            Some(
                AIActionStatus::Preprocessing
                | AIActionStatus::Queued
                | AIActionStatus::RunningAsync,
            ) => {
                let loading_text = if let Some(repo_name) = &self.repo_name {
                    format!("Searching for \"{}\" in {}", self.search_query, repo_name)
                } else {
                    format!("Searching codebase for \"{}\"", self.search_query)
                };
                let loading_icon = yellow_running_icon(appearance);
                self.render_header(appearance, loading_text, loading_icon, app)
                    .with_agent_output_item_spacing(app)
                    .finish()
            }
            Some(AIActionStatus::Finished(result)) if result.result.is_cancelled() => {
                let cancelled_text = if let Some(repo_name) = &self.repo_name {
                    format!(
                        "Search for \"{}\" in {} cancelled",
                        self.search_query, repo_name
                    )
                } else {
                    format!("Search for \"{}\" cancelled", self.search_query)
                };
                let cancelled_icon = cancelled_icon(appearance);
                self.render_header(appearance, cancelled_text, cancelled_icon, app)
                    .with_agent_output_item_spacing(app)
                    .finish()
            }
            Some(AIActionStatus::Finished(_)) => self
                .render_finished(appearance, &self.file_contexts, app)
                .with_agent_output_item_spacing(app)
                .finish(),
            _ => {
                let text = if let Some(repo_name) = &self.repo_name {
                    format!(
                        "Searched codebase for \"{}\" in {}",
                        self.search_query, repo_name
                    )
                } else {
                    format!("Searched codebase for \"{}\"", self.search_query)
                };
                self.render_simple_header(text, app)
                    .with_agent_output_item_spacing(app)
                    .finish()
            }
        }
    }
}
