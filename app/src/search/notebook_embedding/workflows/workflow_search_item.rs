use ordered_float::OrderedFloat;
use warpui::{
    elements::{
        Clipped, ConstrainedBox, Container, CrossAxisAlignment, Flex, Highlight, MainAxisAlignment,
        MainAxisSize, ParentElement, Shrinkable, Text,
    },
    fonts::{Properties, Weight},
    ui_components::components::{Coords, UiComponent, UiComponentStyles},
    AppContext, Element, SingletonEntity,
};

use crate::search::result_renderer::ItemHighlightState;
use crate::{
    appearance::Appearance,
    cloud_object::CloudObject,
    drive::{cloud_object_styling::warp_drive_icon_color, DriveObjectType},
    search::{
        item::IconLocation,
        notebook_embedding::{
            embedded_fuzzy_match::FuzzyMatchEmbeddedObjectResult,
            searcher::EmbeddingSearchItemAction, view::styles,
        },
    },
    themes::theme::Fill,
    ui_components::icons::Icon,
};
use crate::{search::item::SearchItem, workflows::CloudWorkflow};

/// The size of the object type icons, in pixels.
const ICON_SIZE: f32 = 16.;

/// Struct designed to be the implementation of CommandSearchItem for workflows.
#[derive(Clone, Debug)]
pub struct WorkflowSearchItem {
    pub cloud_workflow: CloudWorkflow,
    pub fuzzy_matched_workflow: FuzzyMatchEmbeddedObjectResult,
    /// Whether or not this workflow is accessible to all users that have access to the object
    /// being embedded into.
    pub is_accessible: bool,
}

impl SearchItem for WorkflowSearchItem {
    type Action = EmbeddingSearchItemAction;

    /// Returns an text 'icon' containing the appropriate display abbreviation for the workflow's
    /// source.
    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let (icon, icon_color) = if self.cloud_workflow.model().data.is_agent_mode_workflow() {
            (
                Icon::Prompt,
                warp_drive_icon_color(appearance, DriveObjectType::AgentModeWorkflow),
            )
        } else {
            (
                Icon::Workflow,
                warp_drive_icon_color(appearance, DriveObjectType::Workflow),
            )
        };

        Container::new(
            ConstrainedBox::new(icon.to_warpui_icon(icon_color.into()).finish())
                .with_width(ICON_SIZE)
                .with_height(ICON_SIZE)
                .finish(),
        )
        .with_margin_right(12.)
        .finish()
    }

    fn icon_location(&self, appearance: &Appearance) -> IconLocation {
        let name_size = styles::name_font_size(appearance) * appearance.line_height_ratio();
        IconLocation::Top {
            margin_top: name_size - ICON_SIZE,
        }
    }

    /// Renders the name of the workflow.
    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let workflow = &self.cloud_workflow.model().data;
        let appearance = Appearance::as_ref(app);

        let mut name_text = Text::new_inline(
            workflow.name().to_owned(),
            appearance.ui_font_family(),
            styles::name_font_size(appearance),
        )
        .with_color(highlight_state.main_text_fill(appearance).into_solid());

        if let Some(name_match_result) = &self.fuzzy_matched_workflow.name_match_result {
            name_text = name_text.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                name_match_result.matched_indices.clone(),
            );
        }

        let name = if self.is_accessible {
            name_text.finish()
        } else {
            let name_text = name_text.finish();
            let warning_font_size = appearance.ui_font_size() - 4.;
            let warning_text = appearance
                .ui_builder()
                .span(crate::i18n::tr_static(app, "Not visible to other users"))
                .with_style(UiComponentStyles {
                    font_size: Some(warning_font_size),
                    margin: Some(Coords::uniform(0.).left(4.)),
                    ..Default::default()
                })
                .build()
                .finish();
            let warning_icon = ConstrainedBox::new(
                Icon::Warning
                    .to_warpui_icon(appearance.theme().ui_warning_color().into())
                    .finish(),
            )
            .with_width(warning_font_size)
            .with_height(warning_font_size)
            .finish();
            let warning = Flex::row()
                .with_children([warning_icon, warning_text])
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish();
            Flex::row()
                .with_children([name_text, warning])
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .finish()
        };

        let mut breadcrumb_text = Text::new_inline(
            self.cloud_workflow.breadcrumbs(app),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(breadcrumb_text_fill(highlight_state, appearance).into_solid());

        if let Some(command_match_result) = &self.fuzzy_matched_workflow.breadcrumb_match_result {
            breadcrumb_text = breadcrumb_text.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                command_match_result.matched_indices.clone(),
            );
        }

        let contents = Flex::column()
            .with_child(name)
            .with_child(
                Container::new(breadcrumb_text.finish())
                    .with_padding_top(2.)
                    .finish(),
            )
            .finish();

        Clipped::new(Shrinkable::new(1., contents).finish()).finish()
    }

    fn render_details(&self, _ctx: &AppContext) -> Option<Box<dyn Element>> {
        None
    }

    /// The match score for a workflow is an average of the match scores
    /// against the name, command and description of the workflow.
    fn score(&self) -> OrderedFloat<f64> {
        self.fuzzy_matched_workflow.score()
    }

    fn accept_result(&self) -> EmbeddingSearchItemAction {
        EmbeddingSearchItemAction::AcceptWorkflow(self.cloud_workflow.id)
    }

    fn execute_result(&self) -> EmbeddingSearchItemAction {
        // Workflows typically require the user to provide values for command arguments, so we
        // can't execute the workflow directly and instead fallback to 'accept' the workflow
        // instead.
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        let workflow = &self.cloud_workflow.model().data;

        format!("Workflow: {}", workflow.name())
    }
}

/// The fill to be used for the search result's breadcrumbs.
fn breadcrumb_text_fill(highlight_state: ItemHighlightState, appearance: &Appearance) -> Fill {
    let theme = appearance.theme();
    match highlight_state {
        ItemHighlightState::Selected { .. } => {
            theme.disabled_text_color(theme.accent().with_opacity(80))
        }
        ItemHighlightState::Hovered | ItemHighlightState::Default => {
            theme.disabled_text_color(theme.surface_2())
        }
    }
}
