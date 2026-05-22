use std::sync::Arc;

use ordered_float::OrderedFloat;
use warpui::elements::{
    Border, Clipped, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, Highlight,
    MainAxisAlignment, MainAxisSize, ParentElement, Radius, Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::search::command_search::searcher::{AcceptedWorkflow, CommandSearchItemAction};
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;
use crate::search::workflows::fuzzy_match::FuzzyMatchWorkflowResult;
use crate::server::ids::SyncId;
use crate::ui_components::icons::Icon;
use crate::workflows::workflow::Workflow;
use crate::workflows::{CloudWorkflowModel, WorkflowSource, WorkflowType};

/// Holds workflow data for a `WorkflowSearchItem`, used to read workflow fields
/// during rendering and to produce an `AcceptedWorkflow` payload on selection.
///
/// Cloud workflows use a shared `Arc` pointer into CloudModel so the snapshot
/// avoids deep-cloning on every keystroke. Non-cloud workflows (local files,
/// AI-generated) don't live in CloudModel, so they must carry owned data.
#[derive(Clone, Debug)]
pub enum WorkflowIdentity {
    Cloud {
        id: SyncId,
        model: Arc<CloudWorkflowModel>,
    },
    Local(Box<WorkflowType>),
}

/// Struct designed to be the implementation of CommandSearchItem for workflows.
#[derive(Clone, Debug)]
pub struct WorkflowSearchItem {
    pub identity: WorkflowIdentity,
    pub source: WorkflowSource,
    pub fuzzy_matched_workflow: FuzzyMatchWorkflowResult,
}

impl WorkflowSearchItem {
    fn workflow_data(&self) -> &Workflow {
        match &self.identity {
            WorkflowIdentity::Cloud { model, .. } => &model.data,
            WorkflowIdentity::Local(workflow_type) => workflow_type.as_workflow(),
        }
    }

    fn render_name(&self, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .wrappable_text(
                self.workflow_data().name().to_owned(),
                /*soft_wrap=*/ true,
            )
            .with_style(UiComponentStyles {
                font_family_id: Some(appearance.ui_font_family()),
                font_color: Some(
                    appearance
                        .theme()
                        .main_text_color(appearance.theme().background())
                        .into(),
                ),
                font_size: Some(
                    appearance.monospace_font_size() * styles::TITLE_FONT_SIZE_SCALE_FACTOR,
                ),
                font_weight: Some(Weight::Bold),
                ..Default::default()
            })
            .build()
            .finish()
    }

    fn render_command(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(
            appearance
                .ui_builder()
                .paragraph(self.workflow_data().content().to_owned())
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.monospace_font_family()),
                    font_color: Some(theme.sub_text_color(theme.surface_2()).into()),
                    font_size: Some(appearance.monospace_font_size() * 0.85),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_background_color(theme.surface_2().into_solid())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_border(Border::all(1.).with_border_fill(theme.split_pane_border_color()))
        .with_uniform_padding(4.)
        .finish()
    }
}

impl SearchItem for WorkflowSearchItem {
    type Action = CommandSearchItemAction;

    /// Returns an text 'icon' containing the appropriate display abbreviation for the workflow's
    /// source.
    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                if self.workflow_data().is_agent_mode_workflow() {
                    Icon::Prompt
                } else {
                    Icon::Workflow
                }
                .to_warpui_icon(highlight_state.icon_fill(appearance))
                .finish(),
            )
            .with_width(appearance.monospace_font_size())
            .with_height(appearance.monospace_font_size())
            .finish(),
        )
        .with_margin_right(12.)
        .finish()
    }

    /// Renders the name of the workflow.
    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut name_text = Text::new_inline(
            self.workflow_data().name().to_owned(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(highlight_state.main_text_fill(appearance).into_solid());

        if let Some(name_match_result) = &self.fuzzy_matched_workflow.name_match_result {
            name_text = name_text.with_single_highlight(
                Highlight::new()
                    .with_properties(Properties::default().weight(Weight::Bold))
                    .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
                name_match_result.matched_indices.clone(),
            );
        }

        let mut content_text = Text::new_inline(
            self.workflow_data().content().to_owned(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size() - 2.,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid());

        if let Some(content_match_result) = &self.fuzzy_matched_workflow.content_match_result {
            content_text = content_text.with_single_highlight(
                Highlight::new()
                    .with_properties(Properties::default().weight(Weight::Bold))
                    .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
                content_match_result.matched_indices.clone(),
            );
        }

        let contents = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(name_text.finish())
            .with_child(content_text.finish())
            .finish();

        Clipped::new(Shrinkable::new(1., contents).finish()).finish()
    }

    fn render_details(&self, ctx: &AppContext) -> Option<Box<dyn Element>> {
        let appearance = Appearance::as_ref(ctx);
        let mut flex_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(self.render_name(appearance))
                    .with_margin_bottom(16.)
                    .finish(),
            );

        if let Some(description) = self.workflow_data().description() {
            let mut description_text = appearance
                .ui_builder()
                .paragraph(description.clone())
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_color: Some(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().surface_2())
                            .into(),
                    ),
                    font_size: Some(appearance.monospace_font_size()),
                    ..Default::default()
                });

            if let Some(description_match_result) =
                &self.fuzzy_matched_workflow.description_match_result
            {
                description_text = description_text.with_highlights(
                    description_match_result.matched_indices.clone(),
                    Highlight::new()
                        .with_properties(Properties::default().weight(Weight::Bold))
                        .with_foreground_color(
                            appearance
                                .theme()
                                .main_text_color(appearance.theme().background())
                                .into(),
                        ),
                );
            }

            flex_column.add_child(
                Container::new(description_text.build().finish())
                    .with_margin_bottom(16.)
                    .finish(),
            )
        }

        flex_column.add_child(self.render_command(appearance));

        Some(flex_column.finish())
    }

    /// The match score for a workflow is an average of the match scores
    /// against the name, command and description of the workflow.
    fn score(&self) -> OrderedFloat<f64> {
        self.fuzzy_matched_workflow.score()
    }

    fn accept_result(&self) -> CommandSearchItemAction {
        let accepted = match &self.identity {
            WorkflowIdentity::Cloud { id, .. } => AcceptedWorkflow::Cloud {
                id: *id,
                source: self.source,
            },
            WorkflowIdentity::Local(workflow_type) => AcceptedWorkflow::Local {
                workflow: workflow_type.clone(),
                source: self.source,
            },
        };
        CommandSearchItemAction::AcceptWorkflow(accepted)
    }

    fn execute_result(&self) -> CommandSearchItemAction {
        // Workflows typically require the user to provide values for command arguments, so we
        // can't execute the workflow directly and instead fallback to 'accept' the workflow
        // instead.
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Workflow: {}", self.workflow_data().name())
    }
}

mod styles {
    pub const TITLE_FONT_SIZE_SCALE_FACTOR: f32 = 1.12;
}
