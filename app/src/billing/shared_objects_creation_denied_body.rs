use crate::localization;
use warpui::elements::{
    Container, CornerRadius, CrossAxisAlignment, Flex, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, Shrinkable, Text,
};
use warpui::fonts::Weight;
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::appearance::Appearance;
use crate::drive::DriveObjectType;
use crate::ui_components::blended_colors;
use crate::workspaces::workspace::{BillingMetadata, CustomerType};

const BUTTON_PADDING: f32 = 12.;
const BUTTON_FONT_SIZE: f32 = 14.;
const BUTTON_BORDER_RADIUS: f32 = 4.;

#[derive(Default)]
struct MouseStateHandles {
    button_mouse_state: MouseStateHandle,
}

pub struct SharedObjectsCreationDeniedBody {
    object_type: Option<DriveObjectType>,
    has_admin_permissions: bool,
    is_delinquent_due_to_payment_issue: bool,
    customer_type: CustomerType,
    button_mouse_states: MouseStateHandles,
}

#[derive(Debug, Clone, Copy)]
pub enum SharedObjectsCreationDeniedBodyAction {
    Upgrade,
    ManageBilling,
}

pub enum SharedObjectsCreationDeniedBodyEvent {
    Upgrade,
    ManageBilling,
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

impl SharedObjectsCreationDeniedBody {
    pub fn new(object_type: Option<DriveObjectType>) -> Self {
        Self {
            object_type,
            has_admin_permissions: false,
            is_delinquent_due_to_payment_issue: false,
            customer_type: Default::default(),
            button_mouse_states: Default::default(),
        }
    }

    pub fn update_state(
        &mut self,
        object_type: DriveObjectType,
        has_admin_permissions: bool,
        is_delinquent_due_to_payment_issue: bool,
        customer_type: CustomerType,
        ctx: &mut ViewContext<Self>,
    ) {
        self.object_type = Some(object_type);
        self.has_admin_permissions = has_admin_permissions;
        self.is_delinquent_due_to_payment_issue = is_delinquent_due_to_payment_issue;
        self.customer_type = customer_type;
        ctx.notify();
    }
}

impl Entity for SharedObjectsCreationDeniedBody {
    type Event = SharedObjectsCreationDeniedBodyEvent;
}

impl View for SharedObjectsCreationDeniedBody {
    fn ui_name() -> &'static str {
        "SharedObjectsCreationDeniedBody"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let is_stripe_paid_plan = BillingMetadata::is_stripe_paid_plan(self.customer_type);

        let shared_object_type = self
            .object_type
            .map(|object_type| object_type_label(app, object_type))
            .unwrap_or_else(|| text(app, "drive.object.lower.drive_objects"));

        let sub_header = match self.object_type {
            Some(object_type) => {
                let object_type = object_type_label(app, object_type);
                match (
                    self.is_delinquent_due_to_payment_issue,
                    self.has_admin_permissions,
                    self.customer_type,
                ) {
                    (true, true, _) => {
                        if is_stripe_paid_plan {
                            text_with(
                                app,
                                "drive.shared_objects_limit.description.delinquent.admin",
                                &[("{object_type}", &object_type)],
                            )
                        } else {
                            text_with(
                                app,
                                "drive.shared_objects_limit.description.delinquent.enterprise_admin",
                                &[("{object_type}", &object_type)],
                            )
                        }
                    }
                    (true, false, _) => text_with(
                        app,
                        "drive.shared_objects_limit.description.delinquent.member",
                        &[("{object_type}", &object_type)],
                    ),
                    (false, true, CustomerType::Prosumer) => text_with(
                        app,
                        "drive.shared_objects_limit.description.prosumer.admin",
                        &[("{object_type}", &object_type)],
                    ),
                    (false, false, CustomerType::Prosumer) => text_with(
                        app,
                        "drive.shared_objects_limit.description.prosumer.member",
                        &[("{object_type}", &object_type)],
                    ),
                    (false, true, _) => text_with(
                        app,
                        "drive.shared_objects_limit.description.free.admin",
                        &[("{object_type}", &object_type)],
                    ),
                    (false, false, _) => text_with(
                        app,
                        "drive.shared_objects_limit.description.free.member",
                        &[("{object_type}", &object_type)],
                    ),
                }
            }
            _ => {
                match (
                    self.is_delinquent_due_to_payment_issue,
                    self.has_admin_permissions,
                    self.customer_type,
                ) {
                    (true, true, _) => {
                        if is_stripe_paid_plan {
                            text_with(
                                app,
                                "drive.shared_objects_limit.description.delinquent.admin",
                                &[("{object_type}", &shared_object_type)],
                            )
                        } else {
                            text_with(
                                app,
                                "drive.shared_objects_limit.description.delinquent.enterprise_admin",
                                &[("{object_type}", &shared_object_type)],
                            )
                        }
                    }
                    (true, false, _) => text_with(
                        app,
                        "drive.shared_objects_limit.description.delinquent.member",
                        &[("{object_type}", &shared_object_type)],
                    ),
                    (false, true, CustomerType::Prosumer) => text_with(
                        app,
                        "drive.shared_objects_limit.description.prosumer.admin",
                        &[("{object_type}", &shared_object_type)],
                    ),
                    (false, false, CustomerType::Prosumer) => text_with(
                        app,
                        "drive.shared_objects_limit.description.prosumer.member",
                        &[("{object_type}", &shared_object_type)],
                    ),
                    (false, true, _) => text_with(
                        app,
                        "drive.shared_objects_limit.description.free.admin",
                        &[("{object_type}", &shared_object_type)],
                    ),
                    (false, false, _) => text_with(
                        app,
                        "drive.shared_objects_limit.description.free.member",
                        &[("{object_type}", &shared_object_type)],
                    ),
                }
            }
        };

        let mut body = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(
                    Text::new(sub_header, appearance.ui_font_family(), 14.)
                        .with_color(blended_colors::text_sub(
                            appearance.theme(),
                            appearance.theme().background(),
                        ))
                        .finish(),
                )
                .finish(),
            );

        // Only render an action button if:
        // 1. the team is delinquent + user is an admin + the team is on a stripe paid plan
        // OR
        // 2. if the team is not delinquent.
        // In the case where the team is delinquent and user is NOT an admin, or if the
        // team is delinquent but the team is not on a stripe paid plan, we don't render
        // any action button.
        if self.is_delinquent_due_to_payment_issue
            && self.has_admin_permissions
            && is_stripe_paid_plan
        {
            body.add_child(
                Container::new(
                    Flex::row()
                        .with_child(
                            Shrinkable::new(
                                0.5,
                                self.render_button(
                                    appearance,
                                    text(app, "drive.action.manage_billing"),
                                    self.button_mouse_states.button_mouse_state.clone(),
                                    SharedObjectsCreationDeniedBodyAction::ManageBilling,
                                ),
                            )
                            .finish(),
                        )
                        .with_main_axis_size(MainAxisSize::Max)
                        .finish(),
                )
                .with_margin_top(24.)
                .finish(),
            )
        } else if !self.is_delinquent_due_to_payment_issue {
            body.add_child(
                Container::new(
                    Flex::row()
                        .with_child(
                            Shrinkable::new(
                                0.5,
                                self.render_button(
                                    appearance,
                                    text(app, "drive.action.compare_plans"),
                                    self.button_mouse_states.button_mouse_state.clone(),
                                    SharedObjectsCreationDeniedBodyAction::Upgrade,
                                ),
                            )
                            .finish(),
                        )
                        .with_main_axis_size(MainAxisSize::Max)
                        .finish(),
                )
                .with_margin_top(24.)
                .finish(),
            )
        }

        body.finish()
    }
}

impl SharedObjectsCreationDeniedBody {
    fn render_button(
        &self,
        appearance: &Appearance,
        label: String,
        mouse_state: MouseStateHandle,
        action: SharedObjectsCreationDeniedBodyAction,
    ) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .button(ButtonVariant::Accent, mouse_state)
            .with_centered_text_label(label)
            .with_style(UiComponentStyles {
                font_size: Some(BUTTON_FONT_SIZE),
                font_weight: Some(Weight::Semibold),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(BUTTON_BORDER_RADIUS))),
                padding: Some(Coords::uniform(BUTTON_PADDING)),
                ..Default::default()
            })
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action))
            .finish()
    }
}

impl TypedActionView for SharedObjectsCreationDeniedBody {
    type Action = SharedObjectsCreationDeniedBodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SharedObjectsCreationDeniedBodyAction::Upgrade => {
                ctx.emit(SharedObjectsCreationDeniedBodyEvent::Upgrade)
            }
            SharedObjectsCreationDeniedBodyAction::ManageBilling => {
                ctx.emit(SharedObjectsCreationDeniedBodyEvent::ManageBilling)
            }
        }
    }
}
