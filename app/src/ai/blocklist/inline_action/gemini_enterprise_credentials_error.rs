use ai::api_keys::{ApiKeyManager, GeapCredentialsState};
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
    refresh_requested: bool,
    refresh_succeeded: bool,
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
        ctx.subscribe_to_model(&ApiKeyManager::handle(ctx), |view, manager, _event, ctx| {
            if !view.refresh_requested {
                return;
            }

            match manager.as_ref(ctx).geap_credentials_state().clone() {
                GeapCredentialsState::Refreshing { .. } => {
                    view.update_refresh_button("Refreshing...", true, ctx);
                }
                GeapCredentialsState::Loaded {
                    ref credentials, ..
                } if !credentials.needs_refresh() => {
                    view.refresh_succeeded = true;
                    view.update_refresh_button("Credentials refreshed", true, ctx);
                }
                GeapCredentialsState::Missing
                | GeapCredentialsState::Disabled
                | GeapCredentialsState::Unconfigured
                | GeapCredentialsState::Loaded { .. }
                | GeapCredentialsState::Failed { .. } => {
                    view.refresh_requested = false;
                    view.refresh_succeeded = false;
                    view.update_refresh_button("Try again", false, ctx);
                }
            }
            ctx.notify();
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
            refresh_requested: false,
            refresh_succeeded: false,
        }
    }

    fn update_refresh_button(
        &self,
        label: &'static str,
        disabled: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.refresh_button.update(ctx, |button, ctx| {
            button.set_label(label, ctx);
            button.set_disabled(disabled, ctx);
        });
    }

    pub fn reset(&mut self, ctx: &mut ViewContext<Self>) {
        self.refresh_requested = false;
        self.refresh_succeeded = false;
        self.update_refresh_button("Refresh credentials", false, ctx);
        ctx.notify();
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

        let title = if self.refresh_succeeded {
            "Gemini Enterprise credentials refreshed"
        } else if self.refresh_requested {
            "Refreshing Gemini Enterprise credentials..."
        } else {
            "Gemini Enterprise credentials expired or invalid"
        };
        let detail = if self.refresh_succeeded {
            "Your credentials are ready. Retry the request to continue."
        } else if self.refresh_requested {
            "Warp is refreshing your Google Cloud credentials."
        } else {
            "Warp couldn't authenticate with Google Cloud. Refresh your Gemini Enterprise credentials, then retry the request."
        };
        let header_color = if self.refresh_succeeded {
            theme.ansi_fg_green()
        } else {
            error_color(theme)
        };

        let make_header = || {
            Flex::row()
                .with_spacing(8.)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    ConstrainedBox::new(if self.refresh_succeeded {
                        Icon::Check.to_warpui_icon(header_color.into()).finish()
                    } else {
                        Icon::AlertTriangle
                            .to_warpui_icon(header_color.into())
                            .finish()
                    })
                    .with_width(icon_size(app))
                    .with_height(icon_size(app))
                    .finish(),
                )
                .with_child(
                    Text::new(title.to_string(), appearance.ui_font_family(), 14.)
                        .with_color(header_color)
                        .with_selectable(false)
                        .finish(),
                )
                .finish()
        };

        let make_detail = || {
            Text::new(detail.to_string(), appearance.ui_font_family(), 14.)
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
                self.refresh_requested = true;
                self.refresh_succeeded = false;
                self.update_refresh_button("Refreshing...", true, ctx);
                ctx.emit(GeminiEnterpriseCredentialsErrorEvent::RefreshCredentials);
                ctx.notify();
            }
            GeminiEnterpriseCredentialsErrorAction::OpenSettings => {
                ctx.emit(GeminiEnterpriseCredentialsErrorEvent::OpenSettings);
            }
        }
    }
}
