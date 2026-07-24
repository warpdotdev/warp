use std::collections::HashMap;
use std::default::Default;
use std::sync::Arc;

use pathfinder_geometry::vector::vec2f;
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    ChildAnchor, Empty, OffsetPositioning, PositionedElementAnchor, PositionedElementOffsetBounds,
};
use warpui::fonts::Weight;
use warpui::keymap::FixedBinding;
use warpui::presenter::ChildView;
use warpui::ui_components::components::{Coords, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::TelemetryEvent;
use crate::ai::agent::SuggestedAgentModeWorkflow;
use crate::modal::{Modal, ModalEvent};
use crate::pane_group::PaneEvent;
use crate::server::ids::SyncId;
use crate::ui_components::blended_colors;
use crate::workflows::workflow_view::{WorkflowView, WorkflowViewEvent};
use crate::workflows::{WorkflowSelectionSource, WorkflowSource, WorkflowType};
use crate::workspaces::user_workspaces::UserWorkspaces;

const SUGGESTED_PROMPT_MODAL_HEADER: &str = "Prompt";

/// A modal component for displaying and managing suggested agent mode workflows.
/// This component wraps a WorkflowView in a modal dialog with proper styling and
/// event handling.
#[derive(Debug, Clone, Default)]
pub struct SuggestedAgentModeWorkflowModal {
    modal: Option<ViewHandle<Modal<WorkflowView>>>,
    workflow_view: Option<ViewHandle<WorkflowView>>,
    workflow_and_id: Option<SuggestedAgentModeWorkflowAndId>,
}

#[derive(Debug, Clone)]
pub struct SuggestedAgentModeWorkflowAndId {
    pub workflow: SuggestedAgentModeWorkflow,
    pub sync_id: SyncId,
}

#[derive(Debug, Clone)]
pub enum SuggestedAgentModeWorkflowModalAction {
    /// Triggered when the modal should be cancelled/closed
    Cancel,
}

#[derive(Debug, Clone)]
pub enum SuggestedAgentModeWorkflowModalEvent {
    /// Emitted when the modal should be closed
    Close,
    /// Emitted when a new workflow is successfully created
    WorkflowCreated,
    /// Emitted when the workflow should be run
    RunWorkflow {
        workflow: Arc<WorkflowType>,
        source: Box<WorkflowSource>,
        argument_override: Option<HashMap<String, String>>,
        workflow_selection_source: WorkflowSelectionSource,
    },
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        SuggestedAgentModeWorkflowModalAction::Cancel,
        id!("SuggestedAgentModeWorkflowModal"),
    )]);
}

impl SuggestedAgentModeWorkflowModal {
    pub fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(SuggestedAgentModeWorkflowModalEvent::Close);
    }

    pub fn open_workflow(
        &mut self,
        workflow_and_id: &SuggestedAgentModeWorkflowAndId,
        ctx: &mut ViewContext<Self>,
    ) {
        let workflow_view = ctx.add_typed_action_view(|ctx| {
            let mut workflow_view = WorkflowView::new_in_suggestion_dialog(ctx);
            if let Some(owner) = UserWorkspaces::as_ref(ctx)
                .space_to_owner(crate::cloud_object::Space::Personal, ctx)
            {
                workflow_view.open_new_workflow(
                    Some(workflow_and_id.workflow.name.clone()),
                    Some(workflow_and_id.workflow.prompt.clone()),
                    owner,
                    None,
                    true,
                    workflow_and_id.sync_id,
                    ctx,
                );
            }
            workflow_view
        });

        let workflow_view_handle = workflow_view.clone();
        ctx.subscribe_to_view(&workflow_view, move |me, _, event, ctx| {
            me.handle_workflow_view_event(event, ctx);
        });

        let appearance = Appearance::as_ref(ctx);
        let background = blended_colors::neutral_2(appearance.theme());

        let modal = ctx.add_typed_action_view(|ctx| {
            let mut modal = Modal::new(
                Some(SUGGESTED_PROMPT_MODAL_HEADER.to_string()),
                workflow_view_handle,
                ctx,
            )
            .with_modal_style(UiComponentStyles {
                width: Some(810.),
                background: Some(background.into()),
                ..Default::default()
            })
            .with_header_style(UiComponentStyles {
                padding: Some(Coords {
                    top: 8.,
                    bottom: 0.,
                    left: 24.,
                    right: 24.,
                }),
                font_size: Some(16.),
                font_weight: Some(Weight::Bold),
                ..Default::default()
            })
            .with_body_style(UiComponentStyles {
                padding: Some(Coords {
                    top: 0.,
                    bottom: 24.,
                    left: 24.,
                    right: 24.,
                }),
                ..Default::default()
            })
            .with_background_opacity(100)
            .with_dismiss_on_click();
            modal.set_offset_positioning(OffsetPositioning::offset_from_save_position_element(
                format!(
                    "agent_mode_workflow_position_{}",
                    workflow_and_id.workflow.logging_id
                ),
                vec2f(0., 0.),
                PositionedElementOffsetBounds::WindowByPosition,
                PositionedElementAnchor::TopLeft,
                ChildAnchor::BottomLeft,
            ));
            modal
        });

        ctx.subscribe_to_view(&modal, |me, _, event, ctx| {
            me.handle_modal_event(event, ctx);
        });

        ctx.focus(&workflow_view);
        ctx.notify();

        self.workflow_and_id = Some(workflow_and_id.clone());
        self.modal = Some(modal);
        self.workflow_view = Some(workflow_view);
    }

    fn handle_workflow_view_event(
        &mut self,
        event: &WorkflowViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            WorkflowViewEvent::Pane(PaneEvent::Close) => {
                self.close(ctx);
            }
            WorkflowViewEvent::CreatedWorkflow(created_workflow_id) => {
                if let Some(SuggestedAgentModeWorkflowAndId { sync_id, workflow }) =
                    &self.workflow_and_id
                    && sync_id == created_workflow_id
                {
                    ctx.emit(SuggestedAgentModeWorkflowModalEvent::WorkflowCreated);
                    send_telemetry_from_ctx!(
                        TelemetryEvent::AISuggestedAgentModeWorkflowAdded {
                            logging_id: workflow.logging_id.clone(),
                        },
                        ctx
                    );
                }
                self.close(ctx);
            }
            WorkflowViewEvent::RunWorkflow {
                workflow,
                source,
                argument_override,
            } => {
                ctx.emit(SuggestedAgentModeWorkflowModalEvent::RunWorkflow {
                    workflow: workflow.clone(),
                    source: Box::new(*source),
                    argument_override: argument_override.clone(),
                    workflow_selection_source: WorkflowSelectionSource::WorkflowView,
                });
                self.close(ctx);
            }
            _ => {}
        }
    }

    fn handle_modal_event(&mut self, event: &ModalEvent, ctx: &mut ViewContext<Self>) {
        match event {
            ModalEvent::Close => {
                self.close(ctx);
            }
        }
    }
}

impl Entity for SuggestedAgentModeWorkflowModal {
    type Event = SuggestedAgentModeWorkflowModalEvent;
}

impl View for SuggestedAgentModeWorkflowModal {
    fn ui_name() -> &'static str {
        "SuggestedAgentModeWorkflowModal"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        if let Some(modal) = &self.modal {
            ChildView::new(modal).finish()
        } else {
            log::warn!("SuggestedAgentModeWorkflowModal has not been initialized");
            Empty::new().finish()
        }
    }
}

impl TypedActionView for SuggestedAgentModeWorkflowModal {
    type Action = SuggestedAgentModeWorkflowModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SuggestedAgentModeWorkflowModalAction::Cancel => self.close(ctx),
        }
    }
}
