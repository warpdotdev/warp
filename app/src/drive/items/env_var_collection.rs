use itertools::Itertools;
use warp_core::context_flag::ContextFlag;
use warpui::elements::{Clipped, Container, Flex, MouseStateHandle, ParentElement};
use warpui::fonts::Weight;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, SingletonEntity};

use super::{WarpDriveItem, WarpDriveItemId};
use crate::appearance::Appearance;
use crate::cloud_object::model::actions::{ObjectActionType, ObjectActions};
use crate::cloud_object::CloudObjectMetadata;
use crate::drive::index::DriveIndexAction;
use crate::drive::{CloudObjectTypeAndId, DriveObjectType};
use crate::env_vars::{CloudEnvVarCollection, EnvVarValue};
use crate::themes::theme::Fill;

#[derive(Clone)]
pub struct WarpDriveEnvVarCollection {
    id: CloudObjectTypeAndId,
    env_var_collection: CloudEnvVarCollection,
}

impl WarpDriveEnvVarCollection {
    pub fn new(id: CloudObjectTypeAndId, env_var_collection: CloudEnvVarCollection) -> Self {
        Self {
            id,
            env_var_collection,
        }
    }
}

impl WarpDriveItem for WarpDriveEnvVarCollection {
    fn display_name(&self) -> Option<String> {
        self.env_var_collection.model().string_model.title.clone()
    }

    fn metadata(&self) -> Option<&CloudObjectMetadata> {
        Some(&self.env_var_collection.metadata)
    }

    fn object_type(&self) -> Option<DriveObjectType> {
        Some(DriveObjectType::EnvVarCollection)
    }

    fn secondary_icon(&self, _color: Option<Fill>) -> Option<Box<dyn Element>> {
        None
    }

    fn click_action(&self) -> Option<DriveIndexAction> {
        // If running the workflow is disabled (true for some web views),
        // we should just open the workflow instead.
        if !ContextFlag::RunWorkflow.is_enabled() {
            Some(DriveIndexAction::OpenObject(self.id))
        } else {
            Some(DriveIndexAction::RunObject(self.id))
        }
    }

    fn preview(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let title_text = self.env_var_collection.model().string_model.title.clone();
        let title_to_render = if let Some(title) = title_text {
            title
        } else {
            "Untitled".to_string()
        };
        let title = appearance
            .ui_builder()
            .wrappable_text(title_to_render, true)
            .with_style(UiComponentStyles {
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
            .finish();

        let mut text = Flex::column().with_child(Container::new(title).finish());

        if let Some(description) = self
            .env_var_collection
            .model()
            .string_model
            .description
            .clone()
        {
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

        let rows = self
            .env_var_collection
            .model()
            .string_model
            .vars
            .iter()
            .map(|var| {
                Clipped::new(
                    appearance
                        .ui_builder()
                        .label(match &var.value {
                            EnvVarValue::Constant(val) => format!("{}: {}", var.name, val),
                            EnvVarValue::Command(cmd) => format!("{}: {}", var.name, cmd.name),
                            EnvVarValue::Secret(sec) => {
                                format!("{}: {}", var.name, sec.get_display_name())
                            }
                        })
                        .with_style(UiComponentStyles {
                            font_family_id: Some(appearance.ui_font_family()),
                            font_size: Some(12.),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .finish()
            })
            .collect_vec();

        text.add_child(
            Container::new(Flex::column().with_children(rows).finish())
                .with_margin_top(8.)
                .finish(),
        );

        Some(text.finish())
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
        self.env_var_collection
            .metadata
            .pending_changes_statuses
            .render_icon(sync_queue_is_dequeueing, hover_state, appearance)
    }

    fn action_summary(&self, app: &AppContext) -> Option<String> {
        ObjectActions::as_ref(app)
            .get_action_history_summary_for_action_type(&self.id.uid(), ObjectActionType::Execute)
    }

    fn clone_box(&self) -> Box<dyn WarpDriveItem> {
        Box::new(self.clone())
    }
}
