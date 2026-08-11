use warp_core::context_flag::ContextFlag;
use warpui::elements::{Container, Flex, MouseStateHandle, ParentElement};
use warpui::fonts::Weight;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, SingletonEntity};

use super::{WarpDriveItem, WarpDriveItemId};
use crate::appearance::Appearance;
use crate::cloud_object::CloudObjectMetadata;
use crate::cloud_object::model::actions::{ObjectActionType, ObjectActions};
use crate::drive::index::DriveIndexAction;
use crate::drive::{CloudObjectTypeAndId, DriveObjectType};
use crate::themes::theme::Fill;
use crate::workflows::{CloudWorkflow, WorkflowViewMode};

#[derive(Clone)]
pub struct WarpDriveWorkflow {
    id: CloudObjectTypeAndId,
    workflow: CloudWorkflow,
}

impl WarpDriveWorkflow {
    pub fn new(id: CloudObjectTypeAndId, workflow: CloudWorkflow) -> Self {
        Self { id, workflow }
    }
}

impl WarpDriveItem for WarpDriveWorkflow {
    fn display_name(&self) -> Option<String> {
        if self.workflow.model().data.name().is_empty() {
            None
        } else {
            Some(self.workflow.model().data.name().to_owned())
        }
    }

    fn metadata(&self) -> Option<&CloudObjectMetadata> {
        Some(&self.workflow.metadata)
    }

    fn object_type(&self) -> Option<DriveObjectType> {
        if self.workflow.model().data.is_agent_mode_workflow() {
            Some(DriveObjectType::AgentModeWorkflow)
        } else {
            Some(DriveObjectType::Workflow)
        }
    }

    fn secondary_icon(&self, _color: Option<Fill>) -> Option<Box<dyn Element>> {
        None
    }

    fn click_action(&self) -> Option<DriveIndexAction> {
        if !ContextFlag::RunWorkflow.is_enabled() {
            // If we are in a context where we can't run workflows, open it in view mode
            // by default
            Some(DriveIndexAction::OpenWorkflowInPane {
                cloud_object_type_and_id: self.id,
                open_mode: WorkflowViewMode::View,
            })
        } else {
            Some(DriveIndexAction::RunObject(self.id))
        }
    }

    fn preview(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let mut modal =
            Flex::column().with_cross_axis_alignment(warpui::elements::CrossAxisAlignment::Stretch);

        let mut text = Flex::column()
            .with_child(Container::new(self.render_workflow_name(appearance)).finish());

        if let Some(description) = self.workflow.model().data.description() {
            let description_text = appearance
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
                    font_size: Some(12.),
                    ..Default::default()
                });

            text.add_child(
                Container::new(description_text.build().finish())
                    .with_margin_top(4.)
                    .finish(),
            )
        }

        let content = self.render_workflow_content(appearance);

        modal.add_children([
            Container::new(text.finish())
                .with_margin_bottom(12.)
                .finish(),
            content,
        ]);

        Some(modal.finish())
    }

    fn warp_drive_id(&self) -> WarpDriveItemId {
        WarpDriveItemId::Object(self.id)
    }

    fn sync_status_icon(
        &self,
        sync_queue_is_dequeueing: bool,
        hover_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        self.workflow.metadata.pending_changes_statuses.render_icon(
            sync_queue_is_dequeueing,
            hover_state,
            appearance,
        )
    }

    fn action_summary(&self, app: &AppContext) -> Option<String> {
        ObjectActions::as_ref(app)
            .get_action_history_summary_for_action_type(&self.id.uid(), ObjectActionType::Execute)
    }

    fn clone_box(&self) -> Box<dyn WarpDriveItem> {
        Box::new(self.clone())
    }
}

impl WarpDriveWorkflow {
    fn render_workflow_name(&self, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .wrappable_text(self.workflow.model().data.name().to_owned(), true)
            .with_style(UiComponentStyles {
                font_family_id: Some(appearance.ui_font_family()),
                font_color: Some(
                    appearance
                        .theme()
                        .main_text_color(appearance.theme().background())
                        .into(),
                ),
                font_size: Some(14.),
                font_weight: Some(Weight::Bold),
                ..Default::default()
            })
            .build()
            .finish()
    }

    fn render_workflow_content(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(
            appearance
                .ui_builder()
                .paragraph(self.workflow.model().data.content().to_owned())
                .with_style(UiComponentStyles {
                    font_family_id: Some(if self.workflow.model().data.is_agent_mode_workflow() {
                        appearance.ui_font_family()
                    } else {
                        appearance.monospace_font_family()
                    }),
                    font_color: Some(theme.main_text_color(theme.surface_2()).into()),
                    font_size: Some(12.),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .finish()
    }
}
