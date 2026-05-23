use std::default::Default;
use std::path::PathBuf;

use warpui::fonts::Weight;
use warpui::keymap::FixedBinding;
use warpui::platform::{FilePickerConfiguration, FileType};
use warpui::presenter::ChildView;
use warpui::ui_components::components::{Coords, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::modal::Modal;
use crate::themes::theme::ThemeKind;
use crate::themes::theme_creator_body::{
    ThemeCreatorBody, ThemeCreatorBodyAction, ThemeCreatorBodyEvent,
};
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;

const THEME_CREATOR_MODAL_HEADER: &str = "Create new theme from image";

pub struct ThemeCreatorModal {
    theme_creator_modal: ViewHandle<Modal<ThemeCreatorBody>>,
}

#[derive(Debug)]
pub enum ThemeCreatorModalAction {
    Cancel,
}

pub enum ThemeCreatorModalEvent {
    Close,
    SetCustomTheme { theme: ThemeKind },
    ShowErrorToast { message: String },
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        ThemeCreatorModalAction::Cancel,
        id!("ThemeCreatorModal"),
    )]);
}

impl ThemeCreatorModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let theme_creator_body =
            ctx.add_typed_action_view(|ctx: &mut ViewContext<'_, ThemeCreatorBody>| {
                ThemeCreatorBody::new(ctx)
            });

        ctx.subscribe_to_view(&theme_creator_body, move |me, _, event, ctx| {
            me.handle_theme_creator_body_event(event, ctx);
        });

        let theme_creator_modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(
                Some(THEME_CREATOR_MODAL_HEADER.to_string()),
                theme_creator_body,
                ctx,
            )
            .with_modal_style(UiComponentStyles {
                width: Some(600.),
                height: Some(300.),
                ..Default::default()
            })
            .with_header_style(UiComponentStyles {
                padding: Some(Coords {
                    top: 24.,
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
            theme_creator_modal,
        }
    }

    pub fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(ThemeCreatorModalEvent::Close);
    }

    pub fn cancel(&mut self, ctx: &mut ViewContext<Self>) {
        self.theme_creator_modal.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |theme_creator_body, ctx| {
                theme_creator_body.cancel(ctx);
            });
        });
    }

    pub fn open_file_picker(&mut self, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        let theme_creator_body_id = self
            .theme_creator_modal
            .read(ctx, |modal, _ctx| modal.body().id());
        ctx.open_file_picker(
            move |result, ctx| match result {
                Ok(paths) => {
                    if let Some(path_string) = paths.into_iter().next() {
                        ctx.dispatch_typed_action_for_view(
                            window_id,
                            theme_creator_body_id,
                            &ThemeCreatorBodyAction::HandleImageSelected(PathBuf::from(
                                path_string,
                            )),
                        );
                    } else {
                        ctx.dispatch_typed_action_for_view(
                            window_id,
                            theme_creator_body_id,
                            &ThemeCreatorBodyAction::FilePickerCancelled,
                        );
                    }
                }
                Err(err) => {
                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::error(format!("{err}")),
                            window_id,
                            ctx,
                        );
                    });
                    ctx.dispatch_typed_action_for_view(
                        window_id,
                        theme_creator_body_id,
                        &ThemeCreatorBodyAction::FilePickerCancelled,
                    );
                }
            },
            FilePickerConfiguration::new().set_allowed_file_types(vec![FileType::Image]),
        );
    }

    fn handle_theme_creator_body_event(
        &mut self,
        event: &ThemeCreatorBodyEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ThemeCreatorBodyEvent::Close => {
                self.close(ctx);
            }
            ThemeCreatorBodyEvent::OpenFilePicker => {
                self.open_file_picker(ctx);
            }
            ThemeCreatorBodyEvent::SetCustomTheme { theme } => {
                ctx.emit(ThemeCreatorModalEvent::SetCustomTheme {
                    theme: theme.clone(),
                });
            }
            ThemeCreatorBodyEvent::ShowErrorToast { message } => {
                ctx.emit(ThemeCreatorModalEvent::ShowErrorToast {
                    message: message.clone(),
                });
            }
        }
    }
}

impl Entity for ThemeCreatorModal {
    type Event = ThemeCreatorModalEvent;
}

impl View for ThemeCreatorModal {
    fn ui_name() -> &'static str {
        "ThemeCreatorModal"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.theme_creator_modal).finish()
    }
}

impl TypedActionView for ThemeCreatorModal {
    type Action = ThemeCreatorModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ThemeCreatorModalAction::Cancel => self.cancel(ctx),
        }
    }
}
