use std::collections::HashMap;

use session_sharing_protocol::common::{ParticipantId, Role, RoleRequestId};
use warp_core::features::FeatureFlag;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Flex, MainAxisAlignment, MouseStateHandle,
    ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use super::{BODY_PADDING, HEADER_FONT_SIZE, MODAL_PADDING, TEXT_FONT_SIZE};
use crate::appearance::Appearance;
use crate::terminal::shared_session::render_util::{
    ParticipantAvatarParams, non_hoverable_participant_avatar,
};
use crate::ui_components::blended_colors;

pub const BUTTON_HEIGHT: f32 = 32.;
pub const BUTTON_WIDTH: f32 = 75.;
pub const BUTTON_FONT_SIZE: f32 = 12.;

#[derive(Clone)]
struct MouseStateHandles {
    approve_button: MouseStateHandle,
    deny_button: MouseStateHandle,
}

// Struct that contains fields needed to
// render a role request.
#[derive(Clone)]
struct RoleRequestParams {
    participant_id: ParticipantId,
    firebase_uid: String,
    role: Role,
    avatar: ParticipantAvatarParams,
    button_mouse_states: MouseStateHandles,
}

#[derive(Debug)]
pub enum SharerResponseBodyAction {
    Approve {
        participant_id: ParticipantId,
        role_request_id: RoleRequestId,
        role: Role,
    },
    Deny {
        participant_id: ParticipantId,
        role_request_id: RoleRequestId,
    },
}

pub enum SharerResponseBodyEvent {
    Approve {
        participant_id: ParticipantId,
        role_request_id: RoleRequestId,
        role: Role,
    },
    Deny {
        participant_id: ParticipantId,
        role_request_id: RoleRequestId,
    },
    Close,
}

pub struct SharerResponseBody {
    role_requests: HashMap<RoleRequestId, RoleRequestParams>,
}

impl SharerResponseBody {
    pub fn new() -> Self {
        Self {
            role_requests: HashMap::new(),
        }
    }

    pub fn add_role_request(
        &mut self,
        participant_id: ParticipantId,
        firebase_uid: String,
        role_request_id: RoleRequestId,
        role: Role,
        params: ParticipantAvatarParams,
        ctx: &mut ViewContext<Self>,
    ) {
        let role_request = RoleRequestParams {
            participant_id: participant_id.clone(),
            firebase_uid: firebase_uid.clone(),
            role,
            avatar: params,
            button_mouse_states: MouseStateHandles {
                approve_button: Default::default(),
                deny_button: Default::default(),
            },
        };

        // Ensure there exists only one request per participant
        // by removing the previous request (if it exists)
        // If ACLs are enabled, we make sure there is only one request per user.
        if let Some(request_id) = self.role_requests.iter().find_map(|(request_id, params)| {
            let is_duplicate = if FeatureFlag::SessionSharingAcls.is_enabled() {
                params.firebase_uid == firebase_uid
            } else {
                params.participant_id == participant_id
            };
            is_duplicate.then_some(request_id.clone())
        }) {
            self.role_requests.remove(&request_id);
        }

        self.role_requests.insert(role_request_id, role_request);
        ctx.notify();
    }

    pub fn remove_role_request(
        &mut self,
        role_request_id: RoleRequestId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.role_requests.remove(&role_request_id);

        if self.role_requests.is_empty() {
            ctx.emit(SharerResponseBodyEvent::Close)
        }
        ctx.notify();
    }

    fn render_button_row(
        &self,
        role_request_id: RoleRequestId,
        role_request_params: RoleRequestParams,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let participant_id = role_request_params.participant_id.clone();
        let request_id = role_request_id.clone();
        let deny_button = Container::new(
            appearance
                .ui_builder()
                .button(
                    ButtonVariant::Outlined,
                    role_request_params.button_mouse_states.deny_button,
                )
                .with_centered_text_label(String::from("Deny"))
                .with_style(UiComponentStyles {
                    font_size: Some(BUTTON_FONT_SIZE),
                    font_weight: Some(Weight::Bold),
                    height: Some(BUTTON_HEIGHT),
                    width: Some(BUTTON_WIDTH),
                    ..Default::default()
                })
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(SharerResponseBodyAction::Deny {
                        participant_id: participant_id.clone(),
                        role_request_id: request_id.clone(),
                    })
                })
                .finish(),
        )
        .with_padding_right(BODY_PADDING)
        .finish();

        let participant_id = role_request_params.participant_id.clone();
        let request_id = role_request_id.clone();
        let role = role_request_params.role;
        let approve_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Outlined,
                role_request_params.button_mouse_states.approve_button,
            )
            .with_centered_text_label(String::from("Approve"))
            .with_style(UiComponentStyles {
                font_size: Some(BUTTON_FONT_SIZE),
                font_weight: Some(Weight::Bold),
                height: Some(BUTTON_HEIGHT),
                width: Some(BUTTON_WIDTH),
                ..Default::default()
            })
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(SharerResponseBodyAction::Approve {
                    participant_id: participant_id.clone(),
                    role_request_id: request_id.clone(),
                    role,
                })
            })
            .finish();

        Flex::row()
            .with_child(deny_button)
            .with_child(approve_button)
            .finish()
    }

    fn render_role_request(
        &self,
        role_request_id: RoleRequestId,
        role_request_params: RoleRequestParams,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let button_row =
            self.render_button_row(role_request_id, role_request_params.clone(), appearance);
        let avatar_params = role_request_params.avatar;

        let avatar = non_hoverable_participant_avatar(
            avatar_params.display_name.clone(),
            avatar_params.image_url,
            avatar_params.participant_color,
            avatar_params.is_muted,
            false,
            app,
        );

        let participant = ConstrainedBox::new(
            Flex::row()
                .with_child(
                    Container::new(avatar)
                        .with_padding_right(BODY_PADDING)
                        .finish(),
                )
                .with_child(
                    Text::new_inline(
                        avatar_params.display_name,
                        appearance.ui_font_family(),
                        TEXT_FONT_SIZE,
                    )
                    .with_color(blended_colors::text_main(
                        appearance.theme(),
                        appearance.theme().background(),
                    ))
                    .finish(),
                )
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::Start)
                .finish(),
        )
        .with_width(220.)
        .finish();

        Flex::row()
            .with_child(participant)
            .with_child(button_row)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .finish()
    }
}

impl Entity for SharerResponseBody {
    type Event = SharerResponseBodyEvent;
}

impl View for SharerResponseBody {
    fn ui_name() -> &'static str {
        "SharerResponseBody"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let header = "Edit Requests";
        let text1 = "This grants the ability to execute commands on your";
        let text2 = "behalf. Use with caution.";

        let text_body = Container::new(
            Flex::column()
                .with_child(
                    Container::new(
                        Text::new_inline(header, appearance.ui_font_family(), HEADER_FONT_SIZE)
                            .with_color(blended_colors::text_main(
                                appearance.theme(),
                                appearance.theme().background(),
                            ))
                            .with_style(Properties::default().weight(Weight::Bold))
                            .finish(),
                    )
                    .with_padding_bottom(BODY_PADDING)
                    .finish(),
                )
                .with_child(
                    Flex::column()
                        .with_child(
                            Text::new(text1, appearance.ui_font_family(), TEXT_FONT_SIZE)
                                .with_color(blended_colors::text_sub(
                                    appearance.theme(),
                                    appearance.theme().background(),
                                ))
                                .finish(),
                        )
                        .with_child(
                            Text::new(text2, appearance.ui_font_family(), TEXT_FONT_SIZE)
                                .with_color(blended_colors::text_sub(
                                    appearance.theme(),
                                    appearance.theme().background(),
                                ))
                                .finish(),
                        )
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .finish(),
                )
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .with_padding_bottom(MODAL_PADDING)
        .finish();

        let mut role_requests = Flex::column();
        for (i, (id, params)) in self.role_requests.iter().enumerate() {
            let mut role_request =
                self.render_role_request(id.clone(), params.clone(), appearance, app);
            // Don't add extra padding to the last element
            if i != self.role_requests.len() - 1 {
                role_request = Container::new(role_request)
                    .with_padding_bottom(BODY_PADDING)
                    .finish();
            }
            role_requests.add_child(role_request);
        }

        Flex::column()
            .with_child(text_body)
            .with_child(
                role_requests
                    .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
                    .finish(),
            )
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish()
    }
}

impl TypedActionView for SharerResponseBody {
    type Action = SharerResponseBodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SharerResponseBodyAction::Approve {
                participant_id,
                role_request_id,
                role,
            } => {
                ctx.emit(SharerResponseBodyEvent::Approve {
                    participant_id: participant_id.clone(),
                    role_request_id: role_request_id.clone(),
                    role: *role,
                });
                self.remove_role_request(role_request_id.clone(), ctx);
            }
            SharerResponseBodyAction::Deny {
                participant_id,
                role_request_id,
            } => {
                ctx.emit(SharerResponseBodyEvent::Deny {
                    participant_id: participant_id.clone(),
                    role_request_id: role_request_id.clone(),
                });
                self.remove_role_request(role_request_id.clone(), ctx);
            }
        }
    }
}
