use settings::Setting as _;
use warp_core::ui::Icon;
use warpui::elements::{
    ChildView, ConstrainedBox, Container, CrossAxisAlignment, Flex, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, ParentElement, Shrinkable, SizeConstraintCondition,
    SizeConstraintSwitch, Text,
};
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, EventContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::report_if_error;
use crate::Appearance;

use crate::settings::{AISettings, AISettingsChangedEvent};
use crate::ui_components::blended_colors;
use crate::view_components::action_button::{ActionButton, ButtonSize, NakedTheme, PrimaryTheme};

use super::inline_action_icons::icon_size;
use crate::ai::blocklist::view_util::error_color;

#[derive(Clone, Debug)]
pub enum AwsBedrockCredentialsErrorAction {
    RunLoginCommand,
    Configure,
    ToggleAutoLogin,
}

#[derive(Clone, Debug)]
pub enum AwsBedrockCredentialsErrorEvent {
    RunLoginCommand,
    ConfigureLoginCommand,
}

pub struct AwsBedrockCredentialsErrorView {
    model_name: String,
    login_command: String,

    /// Whether auto-login was triggered, which shows a simpler "running command" message.
    auto_login_triggered: bool,

    // Run button
    run_button: ViewHandle<ActionButton>,

    // Configure button
    configure_button: ViewHandle<ActionButton>,

    // Auto-login checkbox
    auto_login_checkbox_handle: MouseStateHandle,
}

impl AwsBedrockCredentialsErrorView {
    pub fn new(
        model_name: String,
        login_command: String,
        auto_login_triggered: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // Run button
        let run_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(
                crate::i18n::tr_static(ctx, "Refresh AWS Credentials"),
                PrimaryTheme,
            )
            .with_size(ButtonSize::InlineActionHeader)
            .on_click(|ctx| {
                ctx.dispatch_typed_action(AwsBedrockCredentialsErrorAction::RunLoginCommand)
            })
        });

        // Configure button
        let configure_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Configure"), NakedTheme)
                .with_size(ButtonSize::InlineActionHeader)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AwsBedrockCredentialsErrorAction::Configure)
                })
        });

        // Subscribe to AISettings changes to update checkbox state
        ctx.subscribe_to_model(&AISettings::handle(ctx), |_me, _, event, ctx| {
            if matches!(event, AISettingsChangedEvent::AwsBedrockAutoLogin { .. }) {
                ctx.notify();
            }
        });

        Self {
            model_name,
            login_command,
            auto_login_triggered,
            run_button,
            configure_button,
            auto_login_checkbox_handle: MouseStateHandle::default(),
        }
    }
}

impl Entity for AwsBedrockCredentialsErrorView {
    type Event = AwsBedrockCredentialsErrorEvent;
}

impl View for AwsBedrockCredentialsErrorView {
    fn ui_name() -> &'static str {
        "AwsBedrockCredentialsErrorView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // If auto-login was triggered, show a simple "running command" message
        if self.auto_login_triggered {
            return Flex::row()
                .with_spacing(8.)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Text::new(
                        format!("Running `{}`...", self.login_command),
                        appearance.ui_font_family(),
                        14.,
                    )
                    .with_color(blended_colors::text_sub(theme, theme.surface_1()))
                    .with_selectable(false)
                    .finish(),
                )
                .finish();
        }

        let auto_login_enabled = *AISettings::as_ref(app).aws_bedrock_auto_login.value();

        // Helper closures to create elements (since Box<dyn Element> can't be cloned)
        let make_alert_icon = || {
            ConstrainedBox::new(
                Icon::AlertTriangle
                    .to_warpui_icon(error_color(theme).into())
                    .finish(),
            )
            .with_width(icon_size(app))
            .with_height(icon_size(app))
            .finish()
        };

        let make_alert_text = || {
            Text::new(
                "AWS credentials expired or missing",
                appearance.ui_font_family(),
                14.,
            )
            .with_color(error_color(theme))
            .with_selectable(false)
            .finish()
        };

        let make_detail_text = || {
            Text::new(
                format!(
                    "Failed to authenticate with AWS Bedrock when using {}. \
                     Run `{}` to refresh credentials.",
                    self.model_name, self.login_command
                ),
                appearance.ui_font_family(),
                14.,
            )
            .with_color(blended_colors::text_sub(theme, theme.surface_1()))
            .with_selectable(false)
            .finish()
        };

        let make_buttons_row = || {
            Flex::row()
                .with_spacing(8.)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(ChildView::new(&self.configure_button).finish())
                .with_child(ChildView::new(&self.run_button).finish())
                .finish()
        };

        let make_checkbox_row = || {
            let checkbox = Container::new(
                appearance
                    .ui_builder()
                    .checkbox(self.auto_login_checkbox_handle.clone(), None)
                    .check(auto_login_enabled)
                    .build()
                    .on_click(|ctx: &mut EventContext, _, _| {
                        ctx.dispatch_typed_action(AwsBedrockCredentialsErrorAction::ToggleAutoLogin)
                    })
                    .finish(),
            )
            .with_margin_left(-4.)
            .finish();

            let checkbox_label = Text::new(
                "Always run automatically",
                appearance.ui_font_family(),
                appearance.monospace_font_size() - 1.,
            )
            .with_color(blended_colors::text_sub(theme, theme.surface_1()))
            .with_selectable(false)
            .finish();

            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(checkbox)
                .with_child(Container::new(checkbox_label).with_margin_left(4.).finish())
                .finish()
        };

        let make_header_row = || {
            Flex::row()
                .with_spacing(8.)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(make_alert_icon())
                .with_child(make_alert_text())
                .finish()
        };

        // Wide layout: detail text on left, checkbox + buttons on right (same row)
        let wide_layout = Flex::column()
            .with_spacing(12.)
            .with_child(make_header_row())
            .with_child(
                Flex::row()
                    .with_spacing(8.)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(Shrinkable::new(1., make_detail_text()).finish())
                    .with_child(
                        Flex::row()
                            .with_spacing(8.)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(make_checkbox_row())
                            .with_child(make_buttons_row())
                            .finish(),
                    )
                    .finish(),
            )
            .finish();

        // Narrow layout: detail text, then checkbox + buttons row
        let narrow_layout = Flex::column()
            .with_spacing(12.)
            .with_child(make_header_row())
            .with_child(make_detail_text())
            .with_child(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(make_checkbox_row())
                    .with_child(make_buttons_row())
                    .finish(),
            )
            .finish();

        // Switch threshold - when width is less than this, use narrow layout
        let threshold = 600.0 * appearance.monospace_ui_scalar();

        SizeConstraintSwitch::new(
            wide_layout,
            vec![(
                SizeConstraintCondition::WidthLessThan(threshold),
                narrow_layout,
            )],
        )
        .finish()
    }
}

impl TypedActionView for AwsBedrockCredentialsErrorView {
    type Action = AwsBedrockCredentialsErrorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AwsBedrockCredentialsErrorAction::RunLoginCommand => {
                ctx.emit(AwsBedrockCredentialsErrorEvent::RunLoginCommand);
            }
            AwsBedrockCredentialsErrorAction::Configure => {
                ctx.emit(AwsBedrockCredentialsErrorEvent::ConfigureLoginCommand);
            }
            AwsBedrockCredentialsErrorAction::ToggleAutoLogin => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let current = *settings.aws_bedrock_auto_login.value();
                    report_if_error!(settings.aws_bedrock_auto_login.set_value(!current, ctx));
                });
                ctx.notify();
            }
        }
    }
}
