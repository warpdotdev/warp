use warpui::{
    elements::{Align, Container, CrossAxisAlignment, Flex, MouseStateHandle, ParentElement, Text},
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::appearance::Appearance;
use crate::auth::UserUid;
use crate::server::ids::ServerId;

pub struct TransferOwnershipConfirmationModal {
    cancel_mouse_state: MouseStateHandle,
    confirm_mouse_state: MouseStateHandle,
    new_owner_email: Option<String>,
    new_owner_uid: Option<UserUid>,
    team_uid: Option<ServerId>,
}

impl TransferOwnershipConfirmationModal {
    pub fn new() -> Self {
        Self {
            cancel_mouse_state: Default::default(),
            confirm_mouse_state: Default::default(),
            new_owner_email: None,
            new_owner_uid: None,
            team_uid: None,
        }
    }

    pub fn set_new_owner(&mut self, email: String, user_uid: UserUid, team_uid: ServerId) {
        self.new_owner_email = Some(email);
        self.new_owner_uid = Some(user_uid);
        self.team_uid = Some(team_uid);
    }
}

impl Entity for TransferOwnershipConfirmationModal {
    type Event = TransferOwnershipConfirmationEvent;
}

impl View for TransferOwnershipConfirmationModal {
    fn ui_name() -> &'static str {
        "TransferOwnershipConfirmationModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let email = self.new_owner_email.as_deref().unwrap_or_default();

        let description_text = Text::new(
            format!(
                "Are you sure you want to transfer team ownership to {}? You will no longer be the owner and will not be able to take any administrative actions for this team.",
                email
            ),
            appearance.ui_font_family(),
            14.,
        )
        .with_color(theme.sub_text_color(theme.surface_2()).into())
        .finish();

        let button_style = UiComponentStyles {
            font_size: Some(14.),
            padding: Some(Coords::uniform(8.).left(12.).right(12.)),
            ..Default::default()
        };

        let buttons_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                appearance
                    .ui_builder()
                    .button(ButtonVariant::Secondary, self.cancel_mouse_state.clone())
                    .with_text_label(crate::i18n::tr_static(app, "Cancel").to_string())
                    .with_style(button_style)
                    .build()
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(TransferOwnershipConfirmationAction::Cancel);
                    })
                    .finish(),
            )
            .with_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .button(ButtonVariant::Accent, self.confirm_mouse_state.clone())
                        .with_text_label(crate::i18n::tr_static(app, "Transfer").to_string())
                        .with_style(button_style)
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(TransferOwnershipConfirmationAction::Confirm);
                        })
                        .finish(),
                )
                .with_margin_left(12.)
                .finish(),
            )
            .finish();

        Flex::column()
            .with_child(
                Container::new(description_text)
                    .with_margin_bottom(24.)
                    .finish(),
            )
            .with_child(Align::new(buttons_row).right().finish())
            .finish()
    }
}

pub enum TransferOwnershipConfirmationEvent {
    Confirm {
        new_owner_uid: UserUid,
        team_uid: ServerId,
    },
    Cancel,
}

#[derive(Debug)]
pub enum TransferOwnershipConfirmationAction {
    Confirm,
    Cancel,
}

impl TypedActionView for TransferOwnershipConfirmationModal {
    type Action = TransferOwnershipConfirmationAction;

    fn handle_action(
        &mut self,
        action: &TransferOwnershipConfirmationAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            TransferOwnershipConfirmationAction::Confirm => {
                let (Some(new_owner_uid), Some(team_uid)) = (self.new_owner_uid, self.team_uid)
                else {
                    log::error!("Transfer ownership confirm button pressed with no new owner set");
                    return;
                };
                ctx.emit(TransferOwnershipConfirmationEvent::Confirm {
                    new_owner_uid,
                    team_uid,
                });
            }
            TransferOwnershipConfirmationAction::Cancel => {
                ctx.emit(TransferOwnershipConfirmationEvent::Cancel);
            }
        }
    }
}
