use std::default::Default;

use warpui::fonts::Weight;
use warpui::keymap::FixedBinding;
use warpui::presenter::ChildView;
use warpui::ui_components::components::{Coords, UiComponentStyles};
use warpui::{AppContext, Element, Entity, TypedActionView, View, ViewContext, ViewHandle};

use crate::modal::Modal;
use crate::themes::theme::ThemeKind;
use crate::themes::theme_deletion_body::{ThemeDeletionBody, ThemeDeletionBodyEvent};

const THEME_DELETION_MODAL_HEADER: &str = "Are you sure you want to delete this theme?";

pub struct ThemeDeletionModal {
    theme_deletion_modal: ViewHandle<Modal<ThemeDeletionBody>>,
}

#[derive(Debug)]
pub enum ThemeDeletionModalAction {
    Cancel,
}

pub enum ThemeDeletionModalEvent {
    Close,
    ShowErrorToast { message: String },
    DeleteCurrentTheme,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        ThemeDeletionModalAction::Cancel,
        id!("ThemeDeletionModal"),
    )]);
}

impl ThemeDeletionModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let theme_deletion_body =
            ctx.add_typed_action_view(|_ctx: &mut ViewContext<'_, ThemeDeletionBody>| {
                ThemeDeletionBody::new()
            });

        ctx.subscribe_to_view(&theme_deletion_body, move |me, _, event, ctx| {
            me.handle_theme_deletion_body_event(event, ctx);
        });

        let theme_deletion_modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(
                Some(THEME_DELETION_MODAL_HEADER.to_string()),
                theme_deletion_body,
                ctx,
            )
            .with_modal_style(UiComponentStyles {
                width: Some(460.),
                height: Some(132.),
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
                height: Some(0.),
                ..Default::default()
            })
            .with_background_opacity(100)
            .with_dismiss_on_click()
            .close_modal_button_disabled()
        });

        Self {
            theme_deletion_modal,
        }
    }

    pub fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(ThemeDeletionModalEvent::Close);
    }

    pub fn set_theme_kind(&mut self, theme_kind: ThemeKind, ctx: &mut ViewContext<Self>) {
        self.theme_deletion_modal.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |theme_deletion_body, _ctx| {
                theme_deletion_body.set_theme_kind(theme_kind);
            });
        });
    }

    fn handle_theme_deletion_body_event(
        &mut self,
        event: &ThemeDeletionBodyEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ThemeDeletionBodyEvent::Close => {
                self.close(ctx);
            }
            ThemeDeletionBodyEvent::ShowErrorToast { message } => {
                ctx.emit(ThemeDeletionModalEvent::ShowErrorToast {
                    message: message.clone(),
                });
            }
            ThemeDeletionBodyEvent::DeleteCurrentTheme => {
                ctx.emit(ThemeDeletionModalEvent::DeleteCurrentTheme)
            }
        }
    }
}

impl Entity for ThemeDeletionModal {
    type Event = ThemeDeletionModalEvent;
}

impl View for ThemeDeletionModal {
    fn ui_name() -> &'static str {
        "ThemeDeletionModal"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.theme_deletion_modal).finish()
    }
}

impl TypedActionView for ThemeDeletionModal {
    type Action = ThemeDeletionModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ThemeDeletionModalAction::Cancel => self.close(ctx),
        }
    }
}
