//! Modal shown when the user clicks "Click here to paste your token from
//! the browser" on the onboarding agent-slide upgrade-prompt bar. Accepts a
//! pasted auth redirect URL and routes it through
//! `AuthManager::initialize_user_from_auth_payload`.
//!
//! This lives in the app crate (not the onboarding crate) because it reuses
//! `EditorView` for the text input, which the onboarding crate doesn't
//! depend on.
use crate::appearance::Appearance;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::auth_view_modal::AuthRedirectPayload;
use crate::auth::login_failure_notification::LoginFailureReason;
use crate::editor::{
    EditorView, InteractionState, SingleLineEditorOptions, TextColors, TextOptions,
};
use crate::server::server_api::auth::UserAuthenticationError;
use crate::themes::theme::Fill as ThemeFill;
use crate::util::bindings::CustomAction;

use pathfinder_color::ColorU;
use ui_components::{button, Component as _, Options as _};
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss, Fill,
    Flex, FormattedTextElement, HighlightedHyperlink, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, ParentElement, Radius, Shrinkable, Stack,
};
use warpui::fonts::Weight;
use warpui::keymap::{FixedBinding, Keystroke};
use warpui::text_layout::TextAlignment;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    actions::StandardAction, AppContext, Element, Entity, FocusContext, SingletonEntity,
    TypedActionView, View, ViewContext, ViewHandle,
};

const MODAL_WIDTH: f32 = 460.;
const AUTH_TOKEN_INPUT_BORDER_RADIUS: Radius = Radius::Pixels(4.);

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;
    app.register_fixed_bindings([
        FixedBinding::new(
            "enter",
            PasteAuthTokenModalAction::Confirm,
            id!(PasteAuthTokenModalView::ui_name()),
        ),
        FixedBinding::new(
            "escape",
            PasteAuthTokenModalAction::Cancel,
            id!(PasteAuthTokenModalView::ui_name()),
        ),
        FixedBinding::custom(
            CustomAction::Paste,
            PasteAuthTokenModalAction::PasteIntoEditor,
            "Paste",
            id!(PasteAuthTokenModalView::ui_name()),
        ),
        FixedBinding::standard(
            StandardAction::Paste,
            PasteAuthTokenModalAction::PasteIntoEditor,
            id!(PasteAuthTokenModalView::ui_name()),
        ),
    ]);

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))]
    app.register_fixed_bindings([FixedBinding::new(
        "cmdorctrl-v",
        PasteAuthTokenModalAction::PasteIntoEditor,
        id!(PasteAuthTokenModalView::ui_name()),
    )]);
}

#[derive(Clone, Copy, Debug)]
pub enum PasteAuthTokenModalAction {
    Confirm,
    Cancel,
    /// Cmd+V/Ctrl+V at the modal level — routes paste into the editor even
    /// when focus is still on the modal itself rather than the input.
    PasteIntoEditor,
}

#[derive(Clone, Debug)]
pub enum PasteAuthTokenModalEvent {
    Cancelled,
}

pub struct PasteAuthTokenModalView {
    auth_token_input: ViewHandle<EditorView>,
    cancel_button: button::Button,
    continue_button: button::Button,
    close_mouse_state: MouseStateHandle,
    last_failure_reason: Option<LoginFailureReason>,
    highlighted_hyperlink_state: HighlightedHyperlink,
}

impl PasteAuthTokenModalView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let auth_token_input = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let theme = appearance.theme();
            let bg_solid = theme.surface_2().into_solid();
            let default_color = ThemeFill::Solid(internal_colors::text_main(theme, bg_solid));
            let disabled_color = ThemeFill::Solid(internal_colors::text_disabled(theme, bg_solid));
            let hint_color = ThemeFill::Solid(internal_colors::text_sub(theme, bg_solid));
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions {
                        font_size_override: Some(12.),
                        font_family_override: Some(appearance.ui_font_family()),
                        text_colors_override: Some(TextColors {
                            default_color,
                            disabled_color,
                            hint_color,
                        }),
                        ..Default::default()
                    },
                    soft_wrap: false,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text(crate::i18n::tr_static(ctx, "Enter auth token"), ctx);
            editor
        });

        // When the editor sees an Enter/Paste/etc. commit, submit the current
        // buffer text upward. This matches the semantics of the inline editor
        // in `login_slide.rs`.
        ctx.subscribe_to_view(&auth_token_input, |me, _, event, ctx| {
            use crate::editor::Event::{AltEnter, CmdEnter, Enter, Paste, ShiftEnter};
            match event {
                AltEnter | CmdEnter | Enter | Paste | ShiftEnter => {
                    me.submit(ctx);
                }
                _ => {}
            };
            ctx.notify();
        });

        // Handle AuthFailed for attempts that originated from this modal: show
        // an inline error and re-enable the editor so the user can try again.
        ctx.subscribe_to_model(&AuthManager::handle(ctx), |me, _, event, ctx| {
            if let AuthManagerEvent::AuthFailed(err) = event {
                me.last_failure_reason = Some(match err {
                    UserAuthenticationError::InvalidStateParameter => {
                        LoginFailureReason::InvalidStateParameter
                    }
                    UserAuthenticationError::MissingStateParameter => {
                        LoginFailureReason::MissingStateParameter
                    }
                    UserAuthenticationError::DeniedAccessToken(_)
                    | UserAuthenticationError::UserAccountDisabled(_)
                    | UserAuthenticationError::Unexpected(_) => {
                        LoginFailureReason::FailedUserAuthentication
                    }
                });
                me.set_editor_enabled(true, ctx);
                ctx.notify();
            }
        });

        Self {
            auth_token_input,
            cancel_button: button::Button::default(),
            continue_button: button::Button::default(),
            close_mouse_state: MouseStateHandle::default(),
            last_failure_reason: None,
            highlighted_hyperlink_state: HighlightedHyperlink::default(),
        }
    }

    /// Disables the editor while the auth request is in flight. Re-enabled
    /// automatically on `AuthManagerEvent::AuthFailed` or on local parse
    /// failure in `submit`.
    fn set_editor_enabled(&mut self, is_enabled: bool, ctx: &mut ViewContext<Self>) {
        let state = if is_enabled {
            InteractionState::Editable
        } else {
            InteractionState::Disabled
        };
        self.auth_token_input
            .update(ctx, |editor, ctx| editor.set_interaction_state(state, ctx));
    }

    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self.auth_token_input.as_ref(ctx).buffer_text(ctx);
        if text.trim().is_empty() {
            return;
        }
        // Clear any previous error before the next attempt.
        self.last_failure_reason = None;
        self.set_editor_enabled(false, ctx);
        match AuthRedirectPayload::from_raw_url(text) {
            Ok(payload) => {
                AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                    auth_manager.initialize_user_from_auth_payload(payload, true, ctx);
                });
            }
            Err(error) => {
                log::error!("Failed to parse pasted auth URL: {error:#}");
                self.last_failure_reason =
                    Some(LoginFailureReason::InvalidRedirectUrl { was_pasted: true });
                self.set_editor_enabled(true, ctx);
                ctx.notify();
            }
        }
    }
}

impl Entity for PasteAuthTokenModalView {
    type Event = PasteAuthTokenModalEvent;
}

impl View for PasteAuthTokenModalView {
    fn ui_name() -> &'static str {
        "PasteAuthTokenModalView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            // Redirect focus to the editor so keystrokes immediately appear
            // in the input field.
            ctx.focus(&self.auth_token_input);
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let dialog_surface = theme.surface_1();
        let dialog_surface_solid = dialog_surface.into_solid();
        let border_color = internal_colors::neutral_4(theme);
        let input_bg = theme.surface_2();
        let input_bg_solid = input_bg.into_solid();
        let input_text_color: ColorU = internal_colors::text_main(theme, input_bg_solid);
        let ui_builder = appearance.ui_builder();

        let title = FormattedTextElement::from_str(
            "Paste your auth token below",
            appearance.ui_font_family(),
            16.,
        )
        .with_color(internal_colors::text_main(theme, dialog_surface_solid))
        .with_weight(Weight::Bold)
        .with_line_height_ratio(1.25)
        .finish();

        let close_button = ui_builder
            .close_button(24., self.close_mouse_state.clone())
            .build()
            .on_click(|ctx: &mut warpui::EventContext, _, _| {
                ctx.dispatch_typed_action(PasteAuthTokenModalAction::Cancel);
            })
            .finish();

        let title_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(Shrinkable::new(1., title).finish())
            .with_child(close_button)
            .finish();

        let subtitle_color = internal_colors::text_sub(theme, dialog_surface_solid);
        let subtitle = FormattedTextElement::from_str(
            "Paste your auth token from the browser to get complete login.",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(subtitle_color)
        .with_weight(Weight::Normal)
        .with_alignment(TextAlignment::Left)
        .with_line_height_ratio(1.2)
        .finish();

        let input = ui_builder
            .text_input(self.auth_token_input.clone())
            .with_style(UiComponentStyles {
                background: Some(input_bg.into()),
                border_width: Some(1.),
                border_color: Some(Fill::Solid(border_color)),
                border_radius: Some(CornerRadius::with_all(AUTH_TOKEN_INPUT_BORDER_RADIUS)),
                font_color: Some(input_text_color),
                padding: Some(Coords {
                    top: 12.,
                    bottom: 12.,
                    left: 16.,
                    right: 16.,
                }),
                ..Default::default()
            })
            .build()
            .finish();

        let mut body = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(subtitle)
                    .with_margin_top(8.)
                    .with_margin_bottom(16.)
                    .finish(),
            )
            .with_child(input);

        if let Some(reason) = &self.last_failure_reason {
            let error_text = FormattedTextElement::new(
                reason.to_formatted_text(),
                14.,
                appearance.ui_font_family(),
                appearance.monospace_font_family(),
                theme.ui_error_color(),
                self.highlighted_hyperlink_state.clone(),
            )
            .register_default_click_handlers(|url, _, ctx| {
                ctx.open_url(&url.url);
            })
            .finish();
            body = body.with_child(Container::new(error_text).with_margin_top(8.).finish());
        }

        let body = body.finish();

        let cancel_button = self.cancel_button.render(
            appearance,
            button::Params {
                content: button::Content::Label(crate::i18n::tr_static(app, "Cancel").into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(PasteAuthTokenModalAction::Cancel);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let enter = Keystroke::parse("enter").unwrap_or_default();
        let continue_button = self.continue_button.render(
            appearance,
            button::Params {
                content: button::Content::Label(crate::i18n::tr_static(app, "Continue").into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    keystroke: Some(enter),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(PasteAuthTokenModalAction::Confirm);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let footer = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(cancel_button)
                .with_child(
                    Container::new(continue_button)
                        .with_margin_left(8.)
                        .finish(),
                )
                .finish(),
        )
        .with_border(Border::top(1.).with_border_color(border_color))
        .with_horizontal_padding(24.)
        .with_vertical_padding(12.)
        .finish();

        let dialog = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(title_row)
                    .with_horizontal_padding(24.)
                    .with_padding_top(24.)
                    .with_padding_bottom(12.)
                    .finish(),
            )
            .with_child(
                Container::new(body)
                    .with_horizontal_padding(24.)
                    .with_padding_bottom(16.)
                    .finish(),
            )
            .with_child(footer)
            .finish();

        let modal = ConstrainedBox::new(
            Container::new(dialog)
                .with_background(dialog_surface)
                .with_border(Border::all(1.).with_border_color(border_color))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .finish(),
        )
        .with_width(MODAL_WIDTH)
        .finish();

        // Dim backdrop with click-to-dismiss behavior (matches the mockup).
        let mut stack = Stack::new();
        stack.add_child(
            Container::new(warpui::elements::Empty::new().finish())
                .with_background_color(ColorU::new(0, 0, 0, 179))
                .finish(),
        );
        stack.add_child(
            Dismiss::new(Align::new(modal).finish())
                .on_dismiss(|ctx, _app| {
                    ctx.dispatch_typed_action(PasteAuthTokenModalAction::Cancel);
                })
                .finish(),
        );
        stack.finish()
    }
}

impl TypedActionView for PasteAuthTokenModalView {
    type Action = PasteAuthTokenModalAction;

    fn handle_action(&mut self, action: &PasteAuthTokenModalAction, ctx: &mut ViewContext<Self>) {
        match action {
            PasteAuthTokenModalAction::Confirm => {
                self.submit(ctx);
            }
            PasteAuthTokenModalAction::Cancel => {
                ctx.emit(PasteAuthTokenModalEvent::Cancelled);
            }
            PasteAuthTokenModalAction::PasteIntoEditor => {
                self.auth_token_input
                    .update(ctx, |editor, ctx| editor.paste(ctx));
            }
        }
    }
}
