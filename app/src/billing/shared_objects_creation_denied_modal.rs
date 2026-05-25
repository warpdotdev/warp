use std::default::Default;

use warp_core::ui::appearance::Appearance;
use warpui::fonts::Weight;
use warpui::keymap::FixedBinding;
use warpui::presenter::ChildView;
use warpui::ui_components::components::{Coords, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use super::shared_objects_creation_denied_body::{
    SharedObjectsCreationDeniedBody, SharedObjectsCreationDeniedBodyEvent,
};
use crate::drive::cloud_object_styling::warp_drive_icon_color;
use crate::drive::DriveObjectType;
use crate::localization;
use crate::modal::{Modal, ModalEvent};
use crate::server::ids::ServerId;
use crate::themes::theme::Fill;
use crate::ui_components::icons::Icon;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::CustomerType;

pub struct SharedObjectsCreationDeniedModal {
    shared_objects_creation_denied_modal: ViewHandle<Modal<SharedObjectsCreationDeniedBody>>,
    team_uid: Option<ServerId>,
}

#[derive(Debug)]
pub enum SharedObjectsCreationDeniedModalAction {
    Close,
}

pub enum SharedObjectsCreationDeniedModalEvent {
    Close,
    TeamSettings,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        SharedObjectsCreationDeniedModalAction::Close,
        id!("SharedObjectsCreationDeniedModal"),
    )]);
}

fn text(app: &AppContext, key: &str) -> String {
    localization::text_for_app(app, key)
}

fn text_with(app: &AppContext, key: &str, replacements: &[(&str, &str)]) -> String {
    replacements
        .iter()
        .fold(text(app, key), |message, (placeholder, value)| {
            message.replace(placeholder, value)
        })
}

fn object_type_label(app: &AppContext, object_type: DriveObjectType) -> String {
    let key = match object_type {
        DriveObjectType::Notebook { .. } => "drive.object.lower.notebooks",
        DriveObjectType::Workflow => "drive.object.lower.workflows",
        DriveObjectType::Folder => "drive.object.lower.folders",
        DriveObjectType::EnvVarCollection => "drive.object.lower.environment_variables",
        DriveObjectType::AgentModeWorkflow => "drive.object.lower.agent_workflows",
        DriveObjectType::AIFact => "drive.object.lower.ai_fact",
        DriveObjectType::AIFactCollection => "drive.object.lower.rules",
        DriveObjectType::MCPServer => "drive.object.lower.mcp_server",
        DriveObjectType::MCPServerCollection => "drive.object.lower.mcp_servers",
    };
    text(app, key)
}

impl SharedObjectsCreationDeniedModal {
    pub fn new(object_type: Option<DriveObjectType>, ctx: &mut ViewContext<Self>) -> Self {
        let shared_objects_creation_denied_body = ctx.add_typed_action_view(
            |_ctx: &mut ViewContext<'_, SharedObjectsCreationDeniedBody>| {
                SharedObjectsCreationDeniedBody::new(object_type)
            },
        );

        ctx.subscribe_to_view(
            &shared_objects_creation_denied_body,
            move |me, _, event, ctx| {
                me.handle_shared_objects_creation_denied_body_event(event, ctx);
            },
        );

        let shared_objects_creation_denied_modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(
                Some(text(ctx, "drive.shared_objects_limit.title.default")),
                shared_objects_creation_denied_body,
                ctx,
            )
            .with_modal_style(UiComponentStyles {
                width: Some(355.),
                ..Default::default()
            })
            .with_header_style(UiComponentStyles {
                font_size: Some(16.),
                font_weight: Some(Weight::Bold),
                padding: Some(Coords {
                    top: 24.,
                    bottom: 16.,
                    left: 24.,
                    right: 24.,
                }),
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
            .with_dismiss_on_click()
        });
        ctx.subscribe_to_view(
            &shared_objects_creation_denied_modal,
            |me, _, event, ctx| match event {
                ModalEvent::Close => me.close(ctx),
            },
        );

        Self {
            shared_objects_creation_denied_modal,
            team_uid: None,
        }
    }

    pub fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(SharedObjectsCreationDeniedModalEvent::Close);
    }

    pub fn update_modal_state(
        &mut self,
        team_uid: ServerId,
        object_type: DriveObjectType,
        has_admin_permissions: bool,
        is_delinquent_due_to_payment_issue: bool,
        customer_type: CustomerType,
        ctx: &mut ViewContext<Self>,
    ) {
        let appearance = Appearance::as_ref(ctx);
        self.team_uid = Some(team_uid);
        let object_type_label = object_type_label(ctx, object_type);
        let title: Option<String> = if is_delinquent_due_to_payment_issue {
            Some(text_with(
                ctx,
                "drive.shared_objects_limit.title.restricted",
                &[("{object_type}", &object_type_label)],
            ))
        } else {
            Some(text_with(
                ctx,
                "drive.shared_objects_limit.title.limit_reached",
                &[("{object_type}", &object_type_label)],
            ))
        };
        let (icon, icon_color) = match object_type {
            DriveObjectType::Notebook { is_ai_document } => (
                Some(Icon::Notebook),
                Some(Fill::Solid(warp_drive_icon_color(
                    appearance,
                    DriveObjectType::Notebook { is_ai_document },
                ))),
            ),
            DriveObjectType::Workflow => (
                Some(Icon::Workflow),
                Some(Fill::Solid(warp_drive_icon_color(
                    appearance,
                    DriveObjectType::Workflow,
                ))),
            ),
            _ => (None, None),
        };
        self.shared_objects_creation_denied_modal
            .update(ctx, |modal, ctx| {
                modal.set_title(title);
                modal.set_header_icon(icon);
                modal.set_header_icon_color(icon_color);
                modal
                    .body()
                    .update(ctx, |shared_objects_creation_denied_body, ctx| {
                        shared_objects_creation_denied_body.update_state(
                            object_type,
                            has_admin_permissions,
                            is_delinquent_due_to_payment_issue,
                            customer_type,
                            ctx,
                        );
                    });
                ctx.notify();
            });
    }

    fn handle_shared_objects_creation_denied_body_event(
        &mut self,
        event: &SharedObjectsCreationDeniedBodyEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SharedObjectsCreationDeniedBodyEvent::Upgrade => match self.team_uid {
                // If team_uid is set, then open up the upgrade page for the team
                // directly.
                Some(team_uid) => {
                    ctx.open_url(UserWorkspaces::upgrade_link_for_team(team_uid).as_str());
                }
                // Otherwise redirect them to the team settings page.
                None => ctx.emit(SharedObjectsCreationDeniedModalEvent::TeamSettings),
            },
            SharedObjectsCreationDeniedBodyEvent::ManageBilling => match self.team_uid {
                // If team_uid is set, then open up the manage billing page for the team
                // directly. The actual logic that opens the billing portal url in the
                // browser is in the handle_model_event method of TeamsPageView.
                Some(team_uid) => {
                    UserWorkspaces::handle(ctx).update(ctx, move |user_workspaces, ctx| {
                        user_workspaces.generate_stripe_billing_portal_link(team_uid, ctx);
                    });
                }
                // Otherwise redirect them to the team settings page.
                None => ctx.emit(SharedObjectsCreationDeniedModalEvent::TeamSettings),
            },
        }
    }
}

impl Entity for SharedObjectsCreationDeniedModal {
    type Event = SharedObjectsCreationDeniedModalEvent;
}

impl View for SharedObjectsCreationDeniedModal {
    fn ui_name() -> &'static str {
        "SharedObjectsCreationDeniedModal"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.shared_objects_creation_denied_modal).finish()
    }
}

impl TypedActionView for SharedObjectsCreationDeniedModal {
    type Action = SharedObjectsCreationDeniedModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SharedObjectsCreationDeniedModalAction::Close => self.close(ctx),
        }
    }
}
