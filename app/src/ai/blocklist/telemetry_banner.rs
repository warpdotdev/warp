use crate::{
    settings_view::SettingsSection,
    terminal::view::TerminalAction,
    ui_components::{buttons::icon_button, icons::Icon},
    workspaces::{user_workspaces::UserWorkspaces, workspace::UgcCollectionEnablementSetting},
    Appearance, FeatureFlag, WorkspaceAction,
};
use warpui::{
    elements::{
        ConstrainedBox, Container, CrossAxisAlignment, Flex, MainAxisAlignment, MainAxisSize,
        MouseStateHandle, ParentElement, Shrinkable, Text,
    },
    platform::Cursor,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, Entity, SingletonEntity, View, ViewContext,
};

const TITLE_EXISTING_USERS: &str = "We've updated our telemetry policy.";
const TITLE_NEW_USERS: &str = "Help improve Warp.";
const DESCRIPTION: &str = "We may collect certain console interactions to improve Warp's AI capabilities. You can opt out any time.";
const PRIVACY_URL: &str = "https://warp.dev/privacy";

#[derive(Default, Debug, Clone)]
pub struct TelemetryBanner {
    pub is_onboarded: bool,
    pub learn_more_mouse_state: MouseStateHandle,
    pub privacy_settings_mouse_state: MouseStateHandle,
    pub close_button_mouse_state: MouseStateHandle,
}

impl TelemetryBanner {
    pub fn new(is_onboarded: bool, _ctx: &mut ViewContext<Self>) -> Self {
        Self {
            is_onboarded,
            learn_more_mouse_state: Default::default(),
            privacy_settings_mouse_state: Default::default(),
            close_button_mouse_state: Default::default(),
        }
    }
}

impl View for TelemetryBanner {
    fn ui_name() -> &'static str {
        "TelemetryBanner"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder();

        let title = if self.is_onboarded {
            TITLE_EXISTING_USERS
        } else {
            TITLE_NEW_USERS
        };

        let left = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        Icon::Info
                            .to_warpui_icon(theme.active_ui_text_color())
                            .finish(),
                    )
                    .with_height(20.)
                    .with_width(20.)
                    .finish(),
                )
                .with_margin_right(8.)
                .finish(),
            )
            .with_child(
                Shrinkable::new(
                    1.,
                    Flex::column()
                        .with_main_axis_alignment(MainAxisAlignment::Start)
                        .with_child(
                            Text::new(title, ui_builder.ui_font_family(), 14.)
                                .with_color(theme.active_ui_text_color().into_solid())
                                .finish(),
                        )
                        .with_child(
                            Text::new(DESCRIPTION, ui_builder.ui_font_family(), 12.)
                                .with_color(theme.nonactive_ui_text_color().into_solid())
                                .soft_wrap(true)
                                .finish(),
                        )
                        .finish(),
                )
                .finish(),
            )
            .finish();

        let right = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    ui_builder
                        .button(ButtonVariant::Text, self.learn_more_mouse_state.clone())
                        .with_text_label(crate::i18n::tr_static(app, "Learn more").into())
                        .with_style(UiComponentStyles {
                            height: Some(24.),
                            padding: Some(Coords {
                                left: 8.,
                                right: 8.,
                                ..Default::default()
                            }),
                            ..Default::default()
                        })
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(WorkspaceAction::OpenLink(
                                PRIVACY_URL.to_string(),
                            ));
                            ctx.dispatch_typed_action(
                                TerminalAction::HideTelemetryBannerPermanently,
                            );
                        })
                        .finish(),
                )
                .with_margin_right(4.)
                .finish(),
            )
            .with_child(
                Container::new(
                    ui_builder
                        .button(
                            ButtonVariant::Outlined,
                            self.privacy_settings_mouse_state.clone(),
                        )
                        .with_text_label(
                            crate::i18n::tr_static(app, "Manage privacy settings").into(),
                        )
                        .with_style(UiComponentStyles {
                            ..Default::default()
                        })
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(WorkspaceAction::ShowSettingsPage(
                                SettingsSection::Privacy,
                            ));
                            ctx.dispatch_typed_action(
                                TerminalAction::HideTelemetryBannerPermanently,
                            );
                        })
                        .finish(),
                )
                .with_margin_left(4.)
                .with_margin_right(12.)
                .finish(),
            )
            .with_child(
                icon_button(
                    appearance,
                    Icon::X,
                    false,
                    self.close_button_mouse_state.clone(),
                )
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(TerminalAction::HideTelemetryBannerPermanently);
                })
                .with_cursor(Cursor::PointingHand)
                .finish(),
            )
            .finish();

        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(Shrinkable::new(1., left).finish())
                .with_child(right)
                .finish(),
        )
        .with_background(theme.surface_1())
        .with_uniform_padding(12.)
        .finish()
    }
}

impl Entity for TelemetryBanner {
    type Event = ();
}

/// Returns `true` if we should collect UGC (user-generated content) telemetry for AI features.
///
/// This should apply to telemetry events that include user-generated content, like queries or
/// outputs, but need not be checked for regular metadata telemetry events.
///
/// For example, a metadata event that records if a user toggled Pair/Dispatch mode does not
/// require this check, but an event that logs the input buffer for natural language detection
/// _does_ need to check this.
pub fn should_collect_ai_ugc_telemetry(app: &AppContext, is_telemetry_enabled: bool) -> bool {
    match UserWorkspaces::as_ref(app).get_ugc_collection_enablement_setting() {
        UgcCollectionEnablementSetting::Disable => false,
        UgcCollectionEnablementSetting::Enable => true,
        UgcCollectionEnablementSetting::RespectUserSetting => {
            (FeatureFlag::GlobalAIAnalyticsCollection.is_enabled()
                // Do NOT remove this check. Unlike the send telemetry macro,
                // UploadBlock endpoint does not automatically check user's telemetry setting.
                && is_telemetry_enabled)
                || FeatureFlag::AgentModeAnalytics.is_enabled()
        }
    }
}
