use itertools::Itertools;
use warp_editor::editor::NavigationKey;
use warpui::{
    elements::{
        ConstrainedBox, Container, DispatchEventResult, Element, Fill, Flex, ParentElement,
        ScrollStateHandle, Scrollable, ScrollableElement, ScrollbarWidth, Shrinkable, Text,
        UniformList, UniformListState, LEFT_PADDING as SCROLLABLE_LEFT_PADDING,
    },
    fonts::{Properties, Weight},
    AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, WeakViewHandle,
};

use crate::appearance::Appearance;
use crate::util::bindings::CustomAction;
use crate::voltron::{VoltronFeatureViewMeta, VoltronMetadata};
use crate::workflows::WorkflowType;
use crate::{
    cloud_object::model::persistence::CloudModel, workspaces::user_workspaces::UserWorkspaces,
};
use crate::{editor::Event as EditorEvent, send_telemetry_from_ctx};
use crate::{server::telemetry::TelemetryEvent, user_config::WarpConfig};
use crate::{
    themes::theme::{self, Blend, WarpTheme},
    user_config::WarpConfigUpdateEvent,
};
use fuzzy_match::{match_indices_case_insensitive, FuzzyMatchResult};
use std::collections::HashMap;
use std::ops::Deref;
#[cfg(feature = "local_fs")]
use std::path::PathBuf;
use std::sync::Arc;
use warp_core::ui::builder::UiBuilder;
use warp_core::ui::theme::color::internal_colors;
use warp_workflows::workflows as global_workflows;
use warpui::accessibility::{AccessibilityContent, WarpA11yRole};
use warpui::color::ColorU;
use warpui::elements::{
    Align, CrossAxisAlignment, EventHandler, Highlight, Hoverable, MainAxisSize, MouseStateHandle,
};
use warpui::keymap::FixedBinding;
use warpui::text_layout::TextStyle;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};

use super::{workflow::Workflow, WorkflowSource};

const SCROLLBAR_WIDTH: ScrollbarWidth = ScrollbarWidth::Auto;
const DESCRIPTION_MARGIN: f32 = 24.;

/// The padding top/bottom between each element in the workflow list.
const WORKFLOW_LIST_PADDING_Y: f32 = 10.;
/// The padding between the workflow title and subtext.
const WORKFLOW_LIST_PADDING_MIDDLE: f32 = 5.;

pub const WORKFLOW_SUBTEXT_FONT_SIZE: f32 = 14.0;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings(vec![
        FixedBinding::new("up", WorkflowsViewAction::Up, id!("WorkflowsView")),
        FixedBinding::new("down", WorkflowsViewAction::Down, id!("WorkflowsView")),
        FixedBinding::new(
            "right",
            WorkflowsViewAction::FocusEditor,
            id!("WorkflowsView"),
        ),
        FixedBinding::new("escape", WorkflowsViewAction::Close, id!("WorkflowsView")),
        FixedBinding::new(
            "enter",
            WorkflowsViewAction::FocusEditor,
            id!("WorkflowsView"),
        ),
    ]);
}

#[derive(Debug, Clone)]
pub enum WorkflowsViewAction {
    WorkflowItemClick { index: usize },
    Close,
    SetFocusedWorkflowType(WorkflowViewType),
    Up,
    Down,
    FocusEditor,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WorkflowViewType {
    All,
    LocalPersonal, // represents both local + personal cloud
    Project,
    Category { category_index: usize },
    Team,
}

/// A Workflow's tag, or `Untagged` if the Workflow is not tagged at all.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum WorkflowTag {
    Tagged { tag_name: String },
    Untagged,
}

impl WorkflowTag {
    fn tag_name(&self) -> Option<&str> {
        match self {
            WorkflowTag::Tagged { tag_name } => Some(tag_name.as_str()),
            WorkflowTag::Untagged => None,
        }
    }
}

impl WorkflowViewType {
    fn render(
        &self,
        selection_state: SelectionState,
        category_names: &[String],
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let bg_color = selection_state.background_color(appearance.theme());
        let font_weight = selection_state.font_weight();

        let workflow_type = *self;
        let mut container = Container::new(
            appearance
                .ui_builder()
                .span(self.as_str(category_names).to_string())
                .with_style(UiComponentStyles {
                    font_weight: Some(font_weight),
                    font_color: Some(appearance.theme().main_text_color(bg_color).into_solid()),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_padding_top(5.)
        .with_padding_bottom(5.)
        .with_padding_left(10.)
        .with_padding_right(10.);

        if selection_state.is_selected() {
            container = container.with_background(bg_color);
        }

        EventHandler::new(container.finish())
            .on_left_mouse_down(move |ctx, _, _| {
                ctx.dispatch_typed_action(WorkflowsViewAction::SetFocusedWorkflowType(
                    workflow_type,
                ));
                DispatchEventResult::StopPropagation
            })
            .finish()
    }

    fn as_str<'a>(&self, category_names: &'a [String]) -> &'a str {
        match self {
            WorkflowViewType::All => "All",
            WorkflowViewType::LocalPersonal => "My Workflows",
            WorkflowViewType::Project => "Repository Workflows",
            WorkflowViewType::Team => "Team Workflows",
            WorkflowViewType::Category { category_index, .. } => &category_names[*category_index],
        }
    }

    fn as_accessibility_contents(&self, category_names: &[String]) -> AccessibilityContent {
        let a11y_content = match self {
            WorkflowViewType::Category { .. } => {
                format!(
                    "Showing workflows with category {}",
                    self.as_str(category_names)
                )
            }
            WorkflowViewType::All => "Showing all workflows".into(),
            WorkflowViewType::LocalPersonal => "Showing my workflows".into(),
            WorkflowViewType::Project => "Showing project workflows".into(),
            WorkflowViewType::Team => "Showing team workflows".into(),
        };

        AccessibilityContent::new_without_help(a11y_content, WarpA11yRole::UserAction)
    }
}

fn render_workflow(
    workflow: &Workflow,
    appearance: &Appearance,
    selection_state: SelectionState,
    workflow_match_type: &WorkflowMatchType,
) -> Box<dyn Element> {
    let mut title = Text::new_inline(
        workflow.name().to_owned(),
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    );

    let theme = appearance.theme();
    let bg_color = selection_state.background_color(appearance.theme());

    let title_text_color = theme.main_text_color(bg_color).into_solid();

    title = title.with_color(title_text_color);

    if let WorkflowMatchType::Name { match_result } = workflow_match_type {
        let highlight = Highlight::new()
            .with_text_style(
                TextStyle::new()
                    .with_foreground_color(theme.main_text_color(bg_color).into_solid()),
            )
            .with_properties(Properties::default().weight(Weight::Bold));
        title = title.with_single_highlight(highlight, match_result.matched_indices.clone());
    }

    let subheader_color = theme.sub_text_color(bg_color).into_solid();
    let mut subtext = Text::new_inline(
        workflow.content().to_owned(),
        appearance.monospace_font_family(),
        WORKFLOW_SUBTEXT_FONT_SIZE,
    );
    subtext = subtext.with_color(subheader_color);

    if let WorkflowMatchType::Command { match_result } = workflow_match_type {
        let highlight = Highlight::new()
            .with_text_style(
                TextStyle::new()
                    .with_foreground_color(theme.main_text_color(bg_color).into_solid()),
            )
            .with_properties(Properties::default().weight(Weight::Bold));
        subtext = subtext.with_single_highlight(highlight, match_result.matched_indices.clone());
    }

    let workflow_row = Container::new(
        Flex::column()
            .with_child(
                Container::new(title.finish())
                    .with_padding_top(WORKFLOW_LIST_PADDING_Y)
                    .finish(),
            )
            .with_child(
                Container::new(subtext.finish())
                    .with_padding_top(WORKFLOW_LIST_PADDING_MIDDLE)
                    .with_padding_bottom(WORKFLOW_LIST_PADDING_Y)
                    .finish(),
            )
            .finish(),
    )
    .with_margin_left(DESCRIPTION_MARGIN)
    .finish();

    if selection_state.is_selected() {
        Container::new(workflow_row)
            .with_background(bg_color)
            .finish()
    } else {
        workflow_row
    }
}

pub enum CategoriesViewEvent {
    Close,
    WorkflowSelected {
        // use pointer to box to fix clippy error on size difference between variants
        workflow: Box<WorkflowType>,
        workflow_source: WorkflowSource,
    },
}

#[derive(Default)]
struct ScrollableListState {
    scroll_state: ScrollStateHandle,
    list_state: UniformListState,
}

type CategorizedWorkflows = HashMap<WorkflowTag, Vec<Arc<WorkflowType>>>;

#[derive(Default)]
struct LinkMouseStateHandles {
    documentation_link_handle: MouseStateHandle,
}

pub struct CategoriesView {
    handle: WeakViewHandle<Self>,
    workflow_list_state: ScrollableListState,
    workflows_types_list_state: ScrollableListState,
    workflows_by_source: HashMap<WorkflowSource, CategorizedWorkflows>,
    // The list of workflows that are actively being searched.
    active_workflows: Vec<(Arc<WorkflowType>, WorkflowSource)>,
    category_names: Vec<String>,
    selected_workflow_index: usize,
    workflows_mouse_state_handles: Vec<MouseStateHandle>,
    link_mouse_state_handles: LinkMouseStateHandles,
    selected_workflow_type: WorkflowViewType,
    focus_state: WorkflowsFocusState,
    search_term: String,
}

#[allow(dead_code)]
enum WorkflowsFocusState {
    Editor,
    WorkflowTypesSidebar,
}

#[derive(Copy, Clone)]
enum SelectionState {
    Unselected,
    Selected,
    SelectedAndFocused,
}

#[derive(Debug, PartialEq, Eq)]
enum WorkflowMatchType {
    Name { match_result: FuzzyMatchResult },
    Command { match_result: FuzzyMatchResult },
    Tag,
    Unmatched,
}

impl WorkflowMatchType {
    fn match_score(&self) -> i64 {
        match self {
            WorkflowMatchType::Name { match_result } => match_result.score,
            WorkflowMatchType::Command { match_result } => match_result.score,
            // For now a set of score of 0 for matches that aren't fuzzy matches.
            _ => 0,
        }
    }
}

impl SelectionState {
    fn new(is_selected: bool, is_focused: bool) -> Self {
        match (is_selected, is_focused) {
            (true, true) => SelectionState::SelectedAndFocused,
            (true, false) => SelectionState::Selected,
            _ => SelectionState::Unselected,
        }
    }

    fn font_weight(&self) -> Weight {
        if self.is_selected() {
            Weight::Bold
        } else {
            Weight::Normal
        }
    }

    fn background_color(&self, theme: &WarpTheme) -> theme::Fill {
        match self {
            SelectionState::Unselected => theme.surface_2(),
            SelectionState::Selected => theme.surface_2().blend(&theme.accent_overlay()),
            SelectionState::SelectedAndFocused => theme.accent(),
        }
    }

    fn is_selected(&self) -> bool {
        match self {
            SelectionState::Unselected => false,
            SelectionState::Selected | SelectionState::SelectedAndFocused => true,
        }
    }
}

/// A workflow with all necessary information to render.
struct WorkflowForRender<'a> {
    workflow_type: &'a WorkflowType,
    workflow_source: WorkflowSource,
    mouse_state_handle: &'a MouseStateHandle,
    workflow_match: WorkflowMatchType,
}

impl CategoriesView {
    pub fn new(
        local_workflows: impl IntoIterator<Item = Workflow>,
        app_workflows: impl IntoIterator<Item = Workflow>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let mut categorized_workflows = HashMap::new();
        categorized_workflows.insert(
            WorkflowSource::Global,
            Self::categorize_workflows(
                global_workflows()
                    .into_iter()
                    .map(Workflow::from) // convert from the public-facing Workflow type to the warp-internal Workflow type
                    .map(WorkflowType::Local)
                    .map(Arc::new),
            ),
        );
        categorized_workflows.insert(
            WorkflowSource::Local,
            Self::categorize_workflows(
                local_workflows
                    .into_iter()
                    .map(WorkflowType::Local)
                    .map(Arc::new),
            ),
        );
        categorized_workflows.insert(WorkflowSource::Project, HashMap::new());
        categorized_workflows.insert(
            WorkflowSource::App,
            Self::categorize_workflows(
                app_workflows
                    .into_iter()
                    .map(WorkflowType::Local)
                    .map(Arc::new),
            ),
        );

        // Notify if there were changes to the team workflows, so we can reload
        let user_workspaces = UserWorkspaces::handle(ctx);
        ctx.observe(&user_workspaces, |_, _, ctx| {
            ctx.notify();
        });

        ctx.subscribe_to_model(&WarpConfig::handle(ctx), |me, _, event, ctx| {
            if let WarpConfigUpdateEvent::LocalUserWorkflows = event {
                me.update_workflows(ctx);
            }
        });

        let mut workflows_view = Self {
            handle: ctx.handle(),
            workflow_list_state: Default::default(),
            workflows_types_list_state: Default::default(),
            workflows_by_source: categorized_workflows,
            active_workflows: Default::default(),
            selected_workflow_index: 0,
            workflows_mouse_state_handles: Default::default(),
            link_mouse_state_handles: Default::default(),
            selected_workflow_type: WorkflowViewType::All,
            focus_state: WorkflowsFocusState::Editor,
            category_names: Default::default(),
            search_term: String::new(),
        };
        workflows_view.compute_active_workflows(ctx);
        workflows_view.compute_category_names();

        workflows_view
    }

    #[cfg(feature = "integration_tests")]
    pub fn local_workflows(&self) -> impl Iterator<Item = &Arc<WorkflowType>> {
        self.workflows_by_source[&WorkflowSource::Local]
            .values()
            .flatten()
    }

    #[cfg(feature = "integration_tests")]
    pub fn project_workflows(&self) -> impl Iterator<Item = &Arc<WorkflowType>> {
        self.workflows_by_source[&WorkflowSource::Project]
            .values()
            .flatten()
    }

    #[cfg(feature = "local_fs")]
    fn on_project_workflows_loaded(
        &mut self,
        workflows: Vec<Workflow>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !workflows.is_empty() {
            let new_project_workflows = Self::categorize_workflows(
                workflows.into_iter().map(WorkflowType::Local).map(Arc::new),
            );
            if self.workflows_by_source.get(&WorkflowSource::Project)
                != Some(&new_project_workflows)
            {
                self.workflows_by_source
                    .insert(WorkflowSource::Project, new_project_workflows);
                self.selected_workflow_index = 0;
                self.compute_active_workflows(ctx);
                ctx.notify();
            }
        } else if !self
            .workflows_by_source
            .get(&WorkflowSource::Project)
            .is_none_or(HashMap::is_empty)
        {
            // Reset state if there were project workflows previously, but no longer are.
            self.workflows_by_source
                .entry(WorkflowSource::Project)
                .or_default()
                .clear();
            self.selected_workflow_index = 0;
            self.compute_active_workflows(ctx);
            ctx.notify();
        }
    }

    #[cfg(feature = "local_fs")]
    fn load_project_workflows(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        let _ = ctx.spawn(
            async move {
                // TODO(CORE-1372): This should probably be delegating to the
                // `LocalWorkflows` singleton model to load and cache the
                // project workflows at the given path.
                super::local_workflows::load_project_workflows(&path)
            },
            Self::on_project_workflows_loaded,
        );
    }

    pub fn load_cloud_workflows(&mut self, ctx: &mut ViewContext<Self>) {
        let user_workspaces = UserWorkspaces::as_ref(ctx);
        let cloud_model = CloudModel::as_ref(ctx);

        for space in user_workspaces.all_user_spaces(ctx) {
            let workflows_in_space = cloud_model.active_workflows_in_space(space, ctx);
            let new_workflows_in_space = Self::categorize_workflows(
                // Don't include AI workflows in Voltron.
                workflows_in_space
                    .into_iter()
                    .filter(|workflow| !workflow.model().data.is_agent_mode_workflow())
                    .map(|w| Arc::new(WorkflowType::Cloud(Box::new(w.clone())))),
            );
            self.workflows_by_source
                .insert(space.into(), new_workflows_in_space);
        }

        self.selected_workflow_index = 0;
        self.compute_active_workflows(ctx);
        ctx.notify();
    }

    /// Given an iterator of a Vector workflows, constructs a `Vector` of `Workflow` and
    /// `WorkflowSource` pairs.
    fn create_workflow_source_pair<'a>(
        workflows: impl IntoIterator<Item = &'a Vec<Arc<WorkflowType>>>,
        workflow_type: WorkflowSource,
    ) -> Vec<(Arc<WorkflowType>, WorkflowSource)> {
        workflows
            .into_iter()
            .flatten()
            .map(|workflow| (workflow.clone(), workflow_type))
            .collect()
    }

    fn compute_category_names(&mut self) {
        let new_category_names = self
            .workflows_by_source
            .values()
            .flatten()
            .filter_map(|(workflow_tag, _)| workflow_tag.tag_name())
            .map(ToOwned::to_owned)
            .sorted()
            .dedup()
            .collect();

        // If the category names have changed, update the selected_workflow_type, as it depends on
        // indices within the list of category names.
        if new_category_names != self.category_names {
            self.category_names = new_category_names;
            self.selected_workflow_type = WorkflowViewType::All;
        }
    }

    fn compute_active_workflows(&mut self, ctx: &mut ViewContext<Self>) {
        self.active_workflows = match &self.selected_workflow_type {
            WorkflowViewType::All => self
                .workflows_by_source
                .iter()
                .flat_map(|(workflow_source, categorized_workflows)| {
                    Self::create_workflow_source_pair(
                        categorized_workflows.values(),
                        *workflow_source,
                    )
                })
                .collect(),
            WorkflowViewType::Project => self
                .workflows_by_source
                .get(&WorkflowSource::Project)
                .map(|categorized_workflows| {
                    Self::create_workflow_source_pair(
                        categorized_workflows.values(),
                        WorkflowSource::Project,
                    )
                })
                .unwrap_or_default(),
            WorkflowViewType::Team => {
                // TODO: this only assumes one team
                let team_uid = UserWorkspaces::as_ref(ctx).current_team_uid();
                if let Some(team_uid) = team_uid {
                    self.workflows_by_source
                        .get(&WorkflowSource::Team { team_uid })
                        .map(|categorized_workflows| {
                            Self::create_workflow_source_pair(
                                categorized_workflows.values(),
                                WorkflowSource::Team { team_uid },
                            )
                        })
                        .unwrap_or_default()
                } else {
                    Default::default()
                }
            }
            WorkflowViewType::LocalPersonal => {
                let local = self.workflows_by_source.get(&WorkflowSource::Local).map(
                    |categorized_workflows| {
                        Self::create_workflow_source_pair(
                            categorized_workflows.values(),
                            WorkflowSource::Local,
                        )
                    },
                );
                let personal_cloud = self
                    .workflows_by_source
                    .get(&WorkflowSource::PersonalCloud)
                    .map(|categorized_workflows| {
                        Self::create_workflow_source_pair(
                            categorized_workflows.values(),
                            WorkflowSource::PersonalCloud,
                        )
                    });
                // Append the two options of vectors
                let result = local.and_then(|v1| {
                    personal_cloud.map(|v2| {
                        let mut joined_vec = v1;
                        joined_vec.extend(v2);
                        joined_vec
                    })
                });
                result.unwrap_or_default()
            }
            WorkflowViewType::Category { category_index } => self
                .category_names
                .get(*category_index)
                .map_or(Default::default(), |category_name| {
                    let workflow_tag = WorkflowTag::Tagged {
                        tag_name: category_name.to_owned(),
                    };

                    self.workflows_by_source
                        .iter()
                        .filter_map(|(workflow_source, categorized_workflows)| {
                            categorized_workflows.get(&workflow_tag).map(|workflows| {
                                Self::create_workflow_source_pair([workflows], *workflow_source)
                            })
                        })
                        .flatten()
                        .collect()
                }),
        };

        // Keep the workflows as a `Vec` so we only have to order and dedupe we have recompute the
        // active workflows.
        self.active_workflows
            .sort_by(|(workflow_a, _), (workflow_b, _)| {
                unicase::UniCase::ascii(workflow_a.as_workflow().name())
                    .cmp(&unicase::UniCase::ascii(workflow_b.as_workflow().name()))
            });
        self.active_workflows.dedup();

        self.workflows_mouse_state_handles = self
            .active_workflows
            .iter()
            .map(|_| Default::default())
            .collect();

        // Reset the selected workflow index as the underlying workflows have changed.
        self.selected_workflow_index = 0;
        self.workflow_highlighted(ctx);
    }

    fn workflow_by_filtered_index(&self, index: usize) -> Option<(&WorkflowType, WorkflowSource)> {
        self.filtered_workflows()
            .nth(index)
            .map(|filtered_workflows| {
                (
                    filtered_workflows.workflow_type,
                    filtered_workflows.workflow_source,
                )
            })
    }

    /// Resets all necessary state within the view.
    pub fn reset(&mut self, ctx: &mut ViewContext<Self>) {
        self.workflow_list_state = Default::default();
        self.workflows_types_list_state = Default::default();
        self.selected_workflow_type = WorkflowViewType::All;
        self.focus_state = WorkflowsFocusState::Editor;
        self.compute_active_workflows(ctx);

        ctx.notify();
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        self.reset(ctx);
        ctx.emit(CategoriesViewEvent::Close);
    }

    /// Categorizes an iterator of Workflows by a Workflow's tag.
    fn categorize_workflows(
        workflows: impl IntoIterator<Item = Arc<WorkflowType>>,
    ) -> HashMap<WorkflowTag, Vec<Arc<WorkflowType>>> {
        let mut categories_map = HashMap::new();
        workflows.into_iter().for_each(|workflow| {
            if workflow
                .as_workflow()
                .tags()
                .is_none_or(|tags| tags.is_empty())
            {
                categories_map
                    .entry(WorkflowTag::Untagged)
                    .or_insert_with(Vec::new)
                    .push(workflow)
            } else if let Some(tags) = workflow.as_workflow().tags() {
                tags.iter().for_each(|tag| {
                    categories_map
                        .entry(WorkflowTag::Tagged {
                            tag_name: tag.to_owned(),
                        })
                        .or_insert_with(Vec::new)
                        .push(workflow.clone())
                })
            }
        });
        categories_map
    }

    fn workflow_highlighted(&self, ctx: &mut ViewContext<Self>) {
        if let Some(workflow_for_render) =
            self.filtered_workflows().nth(self.selected_workflow_index)
        {
            let a11y_content_text = format!(
                "Selected {} {}",
                workflow_for_render.workflow_type.as_workflow().name(),
                workflow_for_render.workflow_type.as_workflow().content()
            );
            ctx.emit_a11y_content(AccessibilityContent::new_without_help(
                a11y_content_text,
                WarpA11yRole::MenuItemRole,
            ));
        }
    }

    fn editor_up(&mut self, ctx: &mut ViewContext<Self>) {
        if self.selected_workflow_index > 0 {
            self.selected_workflow_index -= 1;
            self.workflow_highlighted(ctx);

            self.workflow_list_state
                .list_state
                .scroll_to(self.selected_workflow_index);
            ctx.notify();
        }
    }

    fn editor_down(&mut self, ctx: &mut ViewContext<Self>) {
        let filtered_workflows_count = self.filtered_workflows().count();

        if self.selected_workflow_index < filtered_workflows_count.saturating_sub(1) {
            self.selected_workflow_index += 1;
            self.workflow_highlighted(ctx);

            self.workflow_list_state
                .list_state
                .scroll_to(self.selected_workflow_index);
            ctx.notify();
        }
    }

    fn render_empty_list_placeholder(&self, appearance: &Appearance) -> Box<dyn Element> {
        let no_workflows_text =
            CategoriesView::text_label("No matching workflows found.", appearance);

        let mut workflow_documentation_link_text =
            Flex::row().with_child(CategoriesView::text_label("Try ", appearance));

        workflow_documentation_link_text.add_child(
            appearance
                .ui_builder()
                .link(
                    "creating your own workflow".into(),
                    Some(
                        "https://docs.warp.dev/knowledge-and-collaboration/warp-drive/workflows"
                            .into(),
                    ),
                    None,
                    self.link_mouse_state_handles
                        .documentation_link_handle
                        .clone(),
                )
                .soft_wrap(false)
                .with_style(UiComponentStyles {
                    font_size: Some(WORKFLOW_SUBTEXT_FONT_SIZE),
                    ..Default::default()
                })
                .build()
                .finish(),
        );

        let flex_column = Flex::column()
            .with_children([no_workflows_text, workflow_documentation_link_text.finish()])
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        Align::new(flex_column.finish()).finish()
    }

    fn editor_enter(&mut self, ctx: &mut ViewContext<Self>) {
        self.select_workflow_item(self.selected_workflow_index, ctx);
    }

    fn select_workflow_item(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some((workflow, workflow_type)) = self.workflow_by_filtered_index(index) {
            ctx.emit(CategoriesViewEvent::WorkflowSelected {
                workflow: Box::new(workflow.clone()),
                workflow_source: workflow_type,
            });

            self.close(ctx);
        }
    }

    /// Determines if a the given `text` is contained within the `Workflow`. If the text fuzzy
    /// matches either the Workflow name or command, the corresponding match type with the higher
    /// score is returned. If neither are matched, `WorkflowMatchType::Tags` is returned if the
    /// `text` is a substring of one of the Workflow tags, otherwise `WorkflowMatch::Unmatched` is
    /// returned.
    fn matches_workflow(workflow: &Arc<WorkflowType>, text: &str) -> WorkflowMatchType {
        let workflow_name_match = match_indices_case_insensitive(
            &workflow.as_workflow().name().to_ascii_lowercase(),
            text,
        );
        let workflow_content_match = match_indices_case_insensitive(
            &workflow.as_workflow().content().to_ascii_lowercase(),
            text,
        );

        match (workflow_name_match, workflow_content_match) {
            (Some(name_match_result), Some(content_match_result)) => {
                if name_match_result.score >= content_match_result.score {
                    WorkflowMatchType::Name {
                        match_result: name_match_result,
                    }
                } else {
                    WorkflowMatchType::Command {
                        match_result: content_match_result,
                    }
                }
            }
            (Some(match_result), _) => WorkflowMatchType::Name { match_result },
            (_, Some(match_result)) => WorkflowMatchType::Command { match_result },
            (_, _) => {
                // Neither the name of the command were matched. Check if any tags match.
                let contains_tag = workflow.as_workflow().tags().is_some_and(|tags| {
                    tags.iter()
                        .any(|tag| tag.to_ascii_lowercase().contains(text))
                });

                if contains_tag {
                    WorkflowMatchType::Tag
                } else {
                    WorkflowMatchType::Unmatched
                }
            }
        }
    }

    fn filtered_workflows(&self) -> impl Iterator<Item = WorkflowForRender<'_>> {
        self.active_workflows
            .iter()
            .zip(self.workflows_mouse_state_handles.iter())
            .filter_map(move |((workflow, workflow_type), mouse_state_handle)| {
                if self.search_term.is_empty() {
                    Some((
                        WorkflowMatchType::Unmatched,
                        workflow,
                        workflow_type,
                        mouse_state_handle,
                    ))
                } else {
                    match Self::matches_workflow(workflow, &self.search_term) {
                        WorkflowMatchType::Unmatched => None,
                        other => Some((other, workflow, workflow_type, mouse_state_handle)),
                    }
                }
            })
            .sorted_by(|(match_type_1, _, _, _), (match_type_2, _, _, _)| {
                match_type_2.match_score().cmp(&match_type_1.match_score())
            })
            .map(
                |(match_type, workflow, workflow_type, mouse_state_handle)| WorkflowForRender {
                    workflow_type: workflow,
                    workflow_source: *workflow_type,
                    mouse_state_handle,
                    workflow_match: match_type,
                },
            )
    }

    fn workflow_types_label(
        text: impl Into<String>,
        font_color: Option<ColorU>,
        ui_builder: &UiBuilder,
    ) -> Box<dyn Element> {
        ui_builder
            .span(text.into())
            .with_style(UiComponentStyles {
                font_color,
                ..Default::default()
            })
            .build()
            .with_padding_bottom(4.)
            .finish()
    }

    fn text_label(text: impl Into<String>, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .span(text.into())
            .with_style(UiComponentStyles {
                font_size: Some(WORKFLOW_SUBTEXT_FONT_SIZE),
                ..Default::default()
            })
            .build()
            .with_padding_bottom(4.)
            .finish()
    }

    /// Renders the list of workflow types as a left sidebar.
    fn render_workflow_types_sidebar(&self, appearance: &Appearance) -> Box<dyn Element> {
        let workflows_type_sidebar_focused =
            matches!(self.focus_state, WorkflowsFocusState::WorkflowTypesSidebar);

        let workflow_types = vec![
            WorkflowViewType::All,
            WorkflowViewType::LocalPersonal,
            WorkflowViewType::Team,
            WorkflowViewType::Project,
        ];

        let mut workflow_types_list = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_children(workflow_types.into_iter().map(|workflow_type| {
                let selection_state = SelectionState::new(
                    self.selected_workflow_type == workflow_type,
                    workflows_type_sidebar_focused,
                );
                Container::new(workflow_type.render(
                    selection_state,
                    self.category_names.deref(),
                    appearance,
                ))
                .with_padding_right(SCROLLBAR_WIDTH.as_f32() + SCROLLABLE_LEFT_PADDING)
                .with_padding_top(WORKFLOW_LIST_PADDING_Y)
                .finish()
            }));

        let theme = appearance.theme();
        workflow_types_list.add_child(
            Container::new(Self::workflow_types_label(
                "Categories",
                Some(theme.sub_text_color(theme.surface_2()).into_solid()),
                appearance.ui_builder(),
            ))
            .with_padding_top(9.)
            .with_padding_left(5.)
            .finish(),
        );

        let handle = self.handle.clone();
        let categories_list = UniformList::new(
            self.workflows_types_list_state.list_state.clone(),
            self.category_names.len(),
            move |range, app| {
                let view = handle
                    .upgrade(app)
                    .expect("view handle should upgradeable")
                    .as_ref(app);
                let appearance = Appearance::as_ref(app);

                view.category_names
                    .iter()
                    .enumerate()
                    .skip(range.start)
                    .take(range.end - range.start)
                    .map(|(index, _)| {
                        let workflow_type = WorkflowViewType::Category {
                            category_index: index,
                        };

                        let selection_state = SelectionState::new(
                            workflow_type == view.selected_workflow_type,
                            workflows_type_sidebar_focused,
                        );

                        workflow_type.render(
                            selection_state,
                            view.category_names.deref(),
                            appearance,
                        )
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
            },
        );

        let scrollable_list = Scrollable::vertical(
            self.workflows_types_list_state.scroll_state.clone(),
            categories_list.finish_scrollable(),
            SCROLLBAR_WIDTH,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            Fill::None, // Leave the background transparent
        )
        .finish();

        workflow_types_list.add_child(Shrinkable::new(1., scrollable_list).finish());

        Container::new(workflow_types_list.finish())
            .with_background(theme.surface_2())
            .finish()
    }

    fn render_workflow_list(&self, appearance: &Appearance) -> Box<dyn Element> {
        let workflows: Vec<_> = self
            .filtered_workflows()
            .map(|workflow_with_highlight| {
                (
                    workflow_with_highlight.workflow_match,
                    workflow_with_highlight.workflow_type.clone(),
                    workflow_with_highlight.mouse_state_handle.clone(),
                )
            })
            .collect();

        if workflows.is_empty() {
            return self.render_empty_list_placeholder(appearance);
        }

        let selected_index = self.selected_workflow_index;
        let is_workflows_list_focused = matches!(self.focus_state, WorkflowsFocusState::Editor);

        let list = UniformList::new(
            self.workflow_list_state.list_state.clone(),
            workflows.len(),
            move |range, app| {
                let appearance = Appearance::as_ref(app);

                workflows
                    .iter()
                    .enumerate()
                    .skip(range.start)
                    .take(range.end - range.start)
                    .map(
                        |(index, (workflow_match_type, workflow, mouse_state_handle))| {
                            let selection_state = SelectionState::new(
                                index == selected_index,
                                is_workflows_list_focused,
                            );
                            let workflow = render_workflow(
                                workflow.as_workflow(),
                                appearance,
                                selection_state,
                                workflow_match_type,
                            );

                            Hoverable::new(mouse_state_handle.clone(), |_| workflow)
                                .on_click(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(
                                        WorkflowsViewAction::WorkflowItemClick { index },
                                    );
                                })
                                .finish()
                        },
                    )
                    .collect::<Vec<_>>()
                    .into_iter()
            },
        );

        Container::new(
            Scrollable::vertical(
                self.workflow_list_state.scroll_state.clone(),
                list.finish_scrollable(),
                SCROLLBAR_WIDTH,
                appearance.theme().nonactive_ui_detail().into(),
                appearance.theme().active_ui_detail().into(),
                Fill::None, // Leave the background transparent
            )
            .finish(),
        )
        .with_background(internal_colors::neutral_3(appearance.theme()))
        .with_padding_top(WORKFLOW_LIST_PADDING_Y)
        .with_padding_bottom(WORKFLOW_LIST_PADDING_Y)
        .finish()
    }

    fn set_focused_workflow_type(
        &mut self,
        workflow_type: &WorkflowViewType,
        ctx: &mut ViewContext<CategoriesView>,
    ) {
        if workflow_type != &self.selected_workflow_type {
            self.selected_workflow_type = *workflow_type;
            self.compute_active_workflows(ctx);

            if let WorkflowViewType::Category { category_index, .. } = &self.selected_workflow_type
            {
                self.workflows_types_list_state
                    .list_state
                    .scroll_to(*category_index);
            } else {
                self.workflows_types_list_state.list_state.scroll_to(0);
            }

            ctx.emit_a11y_content(
                self.selected_workflow_type
                    .as_accessibility_contents(self.category_names.deref()),
            );
        }

        ctx.notify();
    }

    fn focus_editor(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus_state = WorkflowsFocusState::Editor;
        ctx.notify();
    }

    /// Determines the next `WorkflowType` that should be selected.
    /// Note: this is currently unused because the editor is always focused
    /// and increments the list position on editor_up.
    fn increment_focused_workflow_type(&mut self, ctx: &mut ViewContext<Self>) {
        let next = match &self.selected_workflow_type {
            WorkflowViewType::All => WorkflowViewType::LocalPersonal,
            WorkflowViewType::LocalPersonal => WorkflowViewType::Team,
            WorkflowViewType::Team => WorkflowViewType::Project,
            WorkflowViewType::Project if self.category_names.is_empty() => {
                WorkflowViewType::Project
            }
            WorkflowViewType::Project => WorkflowViewType::Category { category_index: 0 },
            WorkflowViewType::Category { category_index } => WorkflowViewType::Category {
                category_index: (category_index + 1).min(self.category_names.len() - 1),
            },
        };
        self.set_focused_workflow_type(&next, ctx);
    }

    /// Determines the previous `WorkflowType` that should be selected.
    /// Note: this is currently unused because the editor is always focused
    /// and decrements the list position on editor_down.
    fn decrement_focused_workflow_type(&mut self, ctx: &mut ViewContext<Self>) {
        // If the user is already on the topmost focused workflow--focus the editor.
        if matches!(self.selected_workflow_type, WorkflowViewType::All) {
            self.focus_editor(ctx);
        } else {
            let previous = match &self.selected_workflow_type {
                WorkflowViewType::All => WorkflowViewType::All,
                WorkflowViewType::LocalPersonal => WorkflowViewType::All,
                WorkflowViewType::Team => WorkflowViewType::LocalPersonal,
                WorkflowViewType::Project => WorkflowViewType::Team,
                WorkflowViewType::Category { category_index, .. } if *category_index == 0 => {
                    WorkflowViewType::Project
                }
                WorkflowViewType::Category { category_index } => WorkflowViewType::Category {
                    category_index: *category_index - 1,
                },
            };
            self.set_focused_workflow_type(&previous, ctx);
        }
    }

    fn update_workflows(&mut self, ctx: &mut ViewContext<Self>) {
        let workflows = WarpConfig::as_ref(ctx)
            .local_user_workflows()
            .iter()
            .map(Clone::clone)
            .map(WorkflowType::Local)
            .map(Arc::new);
        self.workflows_by_source
            .insert(WorkflowSource::Local, Self::categorize_workflows(workflows));
        self.selected_workflow_index = 0;

        self.compute_active_workflows(ctx);
        self.compute_category_names();

        ctx.notify();
    }
}

impl Entity for CategoriesView {
    type Event = CategoriesViewEvent;
}

impl TypedActionView for CategoriesView {
    type Action = WorkflowsViewAction;

    fn handle_action(&mut self, action: &WorkflowsViewAction, ctx: &mut ViewContext<Self>) {
        match action {
            WorkflowsViewAction::WorkflowItemClick { index } => {
                self.select_workflow_item(*index, ctx)
            }
            WorkflowsViewAction::Close => self.close(ctx),
            WorkflowsViewAction::SetFocusedWorkflowType(sidebar_item) => {
                self.set_focused_workflow_type(sidebar_item, ctx);
                self.focus_editor(ctx);
            }
            WorkflowsViewAction::Up => {
                self.decrement_focused_workflow_type(ctx);
            }
            WorkflowsViewAction::Down => {
                self.increment_focused_workflow_type(ctx);
            }
            WorkflowsViewAction::FocusEditor => {
                self.focus_editor(ctx);
            }
        }
    }
}

impl View for CategoriesView {
    fn ui_name() -> &'static str {
        "WorkflowsView"
    }

    fn accessibility_contents(&self, _: &AppContext) -> Option<AccessibilityContent> {
        Some(AccessibilityContent::new(
            "Workflows",
            "Search or use arrow up and arrow down keys to navigate and find a workflow. Use enter to confirm the workflow and esc to quit.",
            WarpA11yRole::MenuRole,
        ))
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        Flex::column()
            .with_child(
                ConstrainedBox::new(
                    Shrinkable::new(
                        1.,
                        Flex::row()
                            .with_children([
                                // The workflow types sidebar.
                                ConstrainedBox::new(self.render_workflow_types_sidebar(appearance))
                                    .with_width(150.)
                                    .finish(),
                                // The list of workflows.
                                Shrinkable::new(1., self.render_workflow_list(appearance)).finish(),
                            ])
                            .with_main_axis_size(MainAxisSize::Max)
                            .finish(),
                    )
                    .finish(),
                )
                .with_max_height(400.)
                .finish(),
            )
            .finish()
    }
}

impl VoltronFeatureViewMeta for CategoriesView {
    fn editor_placeholder_text(&self) -> &'static str {
        "Search workflows"
    }

    fn custom_action() -> Option<CustomAction> {
        Some(CustomAction::Workflows)
    }

    // Unused variables allowed when no local filesystem as `metadata` arg
    // is unused.
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    fn on_load(&mut self, metadata: VoltronMetadata, ctx: &mut ViewContext<Self>) {
        #[cfg(feature = "local_fs")]
        if let Some(active_path) = metadata.active_session_path_if_local {
            self.load_project_workflows(active_path, ctx);
        }

        self.load_cloud_workflows(ctx);

        send_telemetry_from_ctx!(TelemetryEvent::OpenWorkflowSearch, ctx);
        self.search_term = String::new();
        ctx.notify();
    }

    fn handle_editor_event(
        &mut self,
        event: &EditorEvent,
        current_editor_text: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        self.search_term = current_editor_text.to_string();
        match event {
            EditorEvent::Edited(_) => {
                self.selected_workflow_index = 0;
                self.workflow_highlighted(ctx);
                self.workflow_list_state.list_state.scroll_to(0);
                ctx.notify();
            }
            EditorEvent::Navigate(NavigationKey::Up) => self.editor_up(ctx),
            EditorEvent::Navigate(NavigationKey::Down) => self.editor_down(ctx),
            EditorEvent::Enter => self.editor_enter(ctx),
            EditorEvent::Escape => self.close(ctx),
            EditorEvent::Activate => {
                self.focus_state = WorkflowsFocusState::Editor;
                ctx.notify();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "categories_tests.rs"]
mod tests;
