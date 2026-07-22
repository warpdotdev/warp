use warp_core::ui::Icon;
use warpui::elements::{
    ChildView, ConstrainedBox, CrossAxisAlignment, Flex, MainAxisAlignment, MainAxisSize,
    ParentElement, Shrinkable, SizeConstraintCondition, SizeConstraintSwitch, Text,
};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use super::inline_action_icons::icon_size;
use crate::Appearance;
use crate::ai::blocklist::view_util::error_color;
use crate::ui_components::blended_colors;
use crate::view_components::action_button::{ActionButton, ButtonSize, NakedTheme, PrimaryTheme};

#[derive(Clone, Debug)]
pub enum GeminiEnterpriseCredentialsErrorAction {
    RefreshCredentials,
    OpenSettings,
}

#[derive(Clone, Debug)]
pub enum GeminiEnterpriseCredentialsErrorEvent {
    RefreshCredentials,
    OpenSettings,
}

pub struct GeminiEnterpriseCredentialsErrorView {
    refresh_button: ViewHandle<ActionButton>,
    manage_button: ViewHandle<ActionButton>,
}

impl GeminiEnterpriseCredentialsErrorView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let refresh_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Refresh credentials", PrimaryTheme)
                .with_size(ButtonSize::InlineActionHeader)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(
                        GeminiEnterpriseCredentialsErrorAction::RefreshCredentials,
                    )
                })
        });
        let manage_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Manage", NakedTheme)
                .with_size(ButtonSize::InlineActionHeader)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(GeminiEnterpriseCredentialsErrorAction::OpenSettings)
                })
        });

        Self {
            refresh_button,
            manage_button,
        }
    }
}

impl Entity for GeminiEnterpriseCredentialsErrorView {
    type Event = GeminiEnterpriseCredentialsErrorEvent;
}

impl View for GeminiEnterpriseCredentialsErrorView {
    fn ui_name() -> &'static str {
        "GeminiEnterpriseCredentialsErrorView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let make_header = || {
            Flex::row()
                .with_spacing(8.)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    ConstrainedBox::new(
                        Icon::AlertTriangle
                            .to_warpui_icon(error_color(theme).into())
                            .finish(),
                    )
                    .with_width(icon_size(app))
                    .with_height(icon_size(app))
                    .finish(),
                )
                .with_child(
                    Text::new(
                        "Gemini Enterprise credentials expired or invalid".to_string(),
                        appearance.ui_font_family(),
                        14.,
                    )
                    .with_color(error_color(theme))
                    .with_selectable(false)
                    .finish(),
                )
                .finish()
        };

        let make_detail = || {
            Text::new(
                "Warp couldn't authenticate with Google Cloud. Refresh your Gemini Enterprise credentials, then retry the request.".to_string(),
                appearance.ui_font_family(),
                14.,
            )
            .with_color(blended_colors::text_sub(theme, theme.surface_1()))
            .with_selectable(false)
            .finish()
        };

        let make_buttons = || {
            Flex::row()
                .with_spacing(8.)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(ChildView::new(&self.manage_button).finish())
                .with_child(ChildView::new(&self.refresh_button).finish())
                .finish()
        };

        let wide_layout = Flex::column()
            .with_spacing(12.)
            .with_child(make_header())
            .with_child(
                Flex::row()
                    .with_spacing(8.)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(Shrinkable::new(1., make_detail()).finish())
                    .with_child(make_buttons())
                    .finish(),
            )
            .finish();

        let narrow_layout = Flex::column()
            .with_spacing(12.)
            .with_child(make_header())
            .with_child(make_detail())
            .with_child(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::End)
                    .with_child(make_buttons())
                    .finish(),
            )
            .finish();

        SizeConstraintSwitch::new(
            wide_layout,
            vec![(
                SizeConstraintCondition::WidthLessThan(600. * appearance.monospace_ui_scalar()),
                narrow_layout,
            )],
        )
        .finish()
    }
}

impl TypedActionView for GeminiEnterpriseCredentialsErrorView {
    type Action = GeminiEnterpriseCredentialsErrorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            GeminiEnterpriseCredentialsErrorAction::RefreshCredentials => {
                ctx.emit(GeminiEnterpriseCredentialsErrorEvent::RefreshCredentials);
            }
            GeminiEnterpriseCredentialsErrorAction::OpenSettings => {
                ctx.emit(GeminiEnterpriseCredentialsErrorEvent::OpenSettings);
            }
        }
    }
}
