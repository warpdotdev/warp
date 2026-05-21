use warp_core::ui::builder::UiBuilder;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::color::darken;
use warpui::accessibility::{AccessibilityContent, WarpA11yRole};
use warpui::color::ColorU;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Fill, Flex, Icon,
    MouseStateHandle, ParentElement, Radius, Shrinkable,
};
use warpui::fonts::Weight;
use warpui::keymap::FixedBinding;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::appearance::Appearance;
use crate::modal::MODAL_CORNER_RADIUS;
use crate::util::color::lighten;

const MODAL_PADDING: f32 = 32.;

const AUTH_MODAL_GAP: f32 = 16.;
const BUTTON_ROW_GAP: f32 = 8.;
const ACTION_BUTTON_HEIGHT: f32 = 40.;
const ACTION_BUTTON_BORDER_WIDTH: f32 = 2.;
const ACTION_BUTTON_HORIZONTAL_PADDING: f32 = 8.;
const ACTION_BUTTON_FONT_SIZE: f32 = 14.;

const AUTH_OVERRIDE_DESCRIPTION: &str = "It looks like you logged into a Warp account through a web browser. If you continue, any personal Warp drive objects and preferences from this anonymous session with be permanently deleted.";
const AUTH_OVERRIDE_CONFIRMATION_WARNING: &str = "This cannot be undone.";
const AUTH_OVERRIDE_INITIAL_STEP_HEADER: &str = "New login detected";
const AUTH_OVERRIDE_CONFIRM_CONFIRMATION_STEP_HEADER: &str =
    "Delete personal Warp Drive objects and preferences?";
const AUTH_OVERRIDE_BULK_EXPORT_BUTTON_LABEL: &str = "Export your data";
const AUTH_OVERRIDE_BULK_EXPORT_DESCRIPTION: &str = " to import later.";
const AUTH_OVERRIDE_CANCEL_BUTTON_LABEL: &str = "Cancel";
const AUTH_OVERRIDE_CONTINUE_BUTTON_LABEL: &str = "Continue";

#[derive(Clone, Copy, Debug)]
pub enum AuthOverrideWarningBodyAction {
    Close,
    InitiateAllowLogin,
    ConfirmAllowLogin,
    BulkExport,
}

enum AuthOverrideConfirmationStep {
    Initial,
    ConfirmChangeUser,
}

#[derive(Default)]
struct MouseStateHandles {
    cancel_button_mouse_state_handle: MouseStateHandle,
    continue_button_mouse_state_handle: MouseStateHandle,
    export_button_mouse_state_handle: MouseStateHandle,
}

pub struct AuthOverrideWarningBody {
    mouse_state_handles: MouseStateHandles,
    confirmation_step: AuthOverrideConfirmationStep,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "enter",
        AuthOverrideWarningBodyAction::Close,
        id!("AuthOverrideWarningBody"),
    )]);
    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        AuthOverrideWarningBodyAction::Close,
        id!("AuthOverrideWarningBody"),
    )]);
}

impl AuthOverrideWarningBody {
    pub fn new() -> Self {
        AuthOverrideWarningBody {
            mouse_state_handles: Default::default(),
            confirmation_step: AuthOverrideConfirmationStep::Initial,
        }
    }

    pub fn reset(&mut self) {
        self.confirmation_step = AuthOverrideConfirmationStep::Initial;
    }

    fn render_header(&self, appearance: &Appearance, ui_builder: &UiBuilder) -> Box<dyn Element> {
        let header_styles = UiComponentStyles {
            font_family_id: Some(appearance.header_font_family()),
            font_color: Some(appearance.theme().active_ui_text_color().into()),
            font_size: Some(20.),
            font_weight: Some(Weight::Semibold),
            ..Default::default()
        };

        let text = match self.confirmation_step {
            AuthOverrideConfirmationStep::Initial => AUTH_OVERRIDE_INITIAL_STEP_HEADER,
            AuthOverrideConfirmationStep::ConfirmChangeUser => {
                AUTH_OVERRIDE_CONFIRM_CONFIRMATION_STEP_HEADER
            }
        };

        ui_builder
            .span(text)
            .with_soft_wrap()
            .with_style(header_styles)
            .build()
            .finish()
    }

    fn render_warning_icon(&self, appearance: &Appearance) -> Box<dyn Element> {
        let color = match self.confirmation_step {
            AuthOverrideConfirmationStep::Initial => {
                appearance.theme().terminal_colors().normal.yellow
            }
            AuthOverrideConfirmationStep::ConfirmChangeUser => {
                appearance.theme().terminal_colors().normal.red
            }
        };
        ConstrainedBox::new(
            Container::new(Icon::new("bundled/svg/alert-triangle.svg", color).finish())
                .with_background(appearance.theme().surface_1())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
                .with_horizontal_padding(11.)
                .finish(),
        )
        .with_width(64.)
        .with_height(64.)
        .finish()
    }

    fn render_warning_description(
        &self,
        appearance: &Appearance,
        ui_builder: &UiBuilder,
    ) -> Vec<Box<dyn Element>> {
        let muted_styles = UiComponentStyles {
            font_color: Some(
                appearance
                    .theme()
                    .sub_text_color(appearance.theme().background())
                    .into(),
            ),
            ..Default::default()
        };

        match self.confirmation_step {
            AuthOverrideConfirmationStep::Initial => {
                let description = Container::new(
                    ui_builder
                        .paragraph(AUTH_OVERRIDE_DESCRIPTION)
                        .with_style(muted_styles)
                        .build()
                        .finish(),
                )
                .with_margin_top(AUTH_MODAL_GAP)
                .finish();

                let export = Container::new(
                    Flex::row()
                        .with_child(
                            ui_builder
                                .link(
                                    AUTH_OVERRIDE_BULK_EXPORT_BUTTON_LABEL.into(),
                                    None,
                                    Some(Box::new(|ctx| {
                                        ctx.dispatch_typed_action(
                                            AuthOverrideWarningBodyAction::BulkExport,
                                        );
                                    })),
                                    self.mouse_state_handles
                                        .export_button_mouse_state_handle
                                        .clone(),
                                )
                                .soft_wrap(false)
                                .build()
                                .finish(),
                        )
                        .with_child(
                            ui_builder
                                .span(AUTH_OVERRIDE_BULK_EXPORT_DESCRIPTION)
                                .with_style(muted_styles)
                                .build()
                                .finish(),
                        )
                        .finish(),
                )
                .with_margin_top(AUTH_MODAL_GAP)
                .with_margin_bottom(AUTH_MODAL_GAP)
                .finish();

                vec![description, export]
            }
            AuthOverrideConfirmationStep::ConfirmChangeUser => {
                let confirmation = Container::new(
                    ui_builder
                        .paragraph(AUTH_OVERRIDE_CONFIRMATION_WARNING)
                        .with_style(muted_styles)
                        .build()
                        .finish(),
                )
                .with_margin_top(AUTH_MODAL_GAP)
                .with_margin_bottom(AUTH_MODAL_GAP)
                .finish();

                vec![confirmation]
            }
        }
    }

    fn render_buttons(&self, appearance: &Appearance, ui_builder: &UiBuilder) -> Box<dyn Element> {
        let button_color = appearance.theme().accent().into();

        let button_styles = UiComponentStyles {
            font_size: Some(ACTION_BUTTON_FONT_SIZE),
            font_family_id: Some(appearance.ui_font_family()),
            font_weight: Some(Weight::Bold),
            background: Some(Fill::Solid(button_color)),
            border_width: Some(ACTION_BUTTON_BORDER_WIDTH),
            border_color: Some(Fill::Solid(ColorU::transparent_black())),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            padding: Some(Coords {
                top: 0.,
                bottom: 0.,
                left: ACTION_BUTTON_HORIZONTAL_PADDING,
                right: ACTION_BUTTON_HORIZONTAL_PADDING,
            }),
            height: Some(ACTION_BUTTON_HEIGHT),
            ..Default::default()
        };

        let hover_button_style = UiComponentStyles {
            border_color: Some(Fill::Solid(lighten(button_color))),
            ..button_styles
        };

        let click_button_style = UiComponentStyles {
            background: Some(Fill::Solid(darken(button_color))),
            ..hover_button_style
        };

        let outline_color: ColorU = appearance.theme().accent().into();

        let outline_button_styles = UiComponentStyles {
            font_size: Some(ACTION_BUTTON_FONT_SIZE),
            font_family_id: Some(appearance.ui_font_family()),
            font_weight: Some(Weight::Bold),
            border_width: Some(2.),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            padding: Some(Coords {
                top: 0.,
                bottom: 0.,
                left: ACTION_BUTTON_HORIZONTAL_PADDING,
                right: ACTION_BUTTON_HORIZONTAL_PADDING,
            }),
            height: Some(ACTION_BUTTON_HEIGHT),
            ..Default::default()
        };

        let outline_hover_button_style = UiComponentStyles {
            border_color: Some(outline_color.into()),
            font_color: Some(outline_color),
            ..outline_button_styles
        };

        let outline_click_button_style = UiComponentStyles {
            border_color: Some(Fill::Solid(darken(outline_color))),
            ..outline_hover_button_style
        };

        let cancel_button = ui_builder
            .button_with_custom_styles(
                ButtonVariant::Accent,
                self.mouse_state_handles
                    .cancel_button_mouse_state_handle
                    .clone(),
                button_styles,
                Some(hover_button_style),
                Some(click_button_style),
                None,
            )
            .with_centered_text_label(AUTH_OVERRIDE_CANCEL_BUTTON_LABEL.into())
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(AuthOverrideWarningBodyAction::Close);
            })
            .finish();

        let continue_action = match self.confirmation_step {
            AuthOverrideConfirmationStep::Initial => {
                AuthOverrideWarningBodyAction::InitiateAllowLogin
            }
            AuthOverrideConfirmationStep::ConfirmChangeUser => {
                AuthOverrideWarningBodyAction::ConfirmAllowLogin
            }
        };
        let continue_button = ui_builder
            .button_with_custom_styles(
                ButtonVariant::Outlined,
                self.mouse_state_handles
                    .continue_button_mouse_state_handle
                    .clone(),
                outline_button_styles,
                Some(outline_hover_button_style),
                Some(outline_click_button_style),
                None,
            )
            .with_centered_text_label(AUTH_OVERRIDE_CONTINUE_BUTTON_LABEL.into())
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(continue_action);
            })
            .finish();

        Flex::row()
            .with_child(
                Shrinkable::new(
                    1.,
                    Container::new(continue_button)
                        .with_margin_right(BUTTON_ROW_GAP)
                        .finish(),
                )
                .finish(),
            )
            .with_child(Shrinkable::new(1., cancel_button).finish())
            .finish()
    }
}

pub enum AuthOverrideWarningBodyEvent {
    Close,
    AllowLogin,
    BulkExport,
}

impl Entity for AuthOverrideWarningBody {
    type Event = AuthOverrideWarningBodyEvent;
}

impl TypedActionView for AuthOverrideWarningBody {
    type Action = AuthOverrideWarningBodyAction;

    fn handle_action(
        &mut self,
        action: &AuthOverrideWarningBodyAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            AuthOverrideWarningBodyAction::Close => {
                ctx.emit(AuthOverrideWarningBodyEvent::Close);
            }
            AuthOverrideWarningBodyAction::InitiateAllowLogin => {
                self.confirmation_step = AuthOverrideConfirmationStep::ConfirmChangeUser;
                ctx.notify();
            }
            AuthOverrideWarningBodyAction::ConfirmAllowLogin => {
                ctx.emit(AuthOverrideWarningBodyEvent::AllowLogin);
            }
            AuthOverrideWarningBodyAction::BulkExport => {
                ctx.emit(AuthOverrideWarningBodyEvent::BulkExport);
            }
        }
    }
}

impl View for AuthOverrideWarningBody {
    fn ui_name() -> &'static str {
        "AuthOverrideWarningBody"
    }

    fn accessibility_contents(&self, _: &AppContext) -> Option<AccessibilityContent> {
        Some(AccessibilityContent::new(
            "New login detected",
            "Warp has detected a new login from a web browser. Press escape to cancel and continue using Warp without login.",
            WarpA11yRole::HelpRole,
        ))
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus_self();
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let ui_builder = appearance.ui_builder();

        let logo_row = Container::new(self.render_warning_icon(appearance))
            .with_margin_bottom(AUTH_MODAL_GAP)
            .finish();

        let content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(logo_row)
            .with_child(self.render_header(appearance, ui_builder))
            .with_children(self.render_warning_description(appearance, ui_builder))
            .with_child(self.render_buttons(appearance, ui_builder))
            .finish();

        Container::new(content)
            .with_background(
                appearance
                    .theme()
                    .background()
                    .blend(&appearance.theme().surface_1().with_opacity(50)),
            )
            .with_corner_radius(CornerRadius::with_all(MODAL_CORNER_RADIUS))
            .with_uniform_padding(MODAL_PADDING)
            .finish()
    }
}
