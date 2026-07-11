use warp_core::features::FeatureFlag;
use warp_core::settings::ToggleableSetting as _;
use warp_errors::report_if_error;
use warpui::elements::{
    Container, Element, Flex, MouseStateHandle, ParentElement, Shrinkable, Text,
};
use warpui::fonts::Weight;
use warpui::keymap::ContextPredicate;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{
    id, Action, AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use super::settings_page::{
    render_body_item, AdditionalInfo, MatchData, PageType, SettingsPageMeta,
    SettingsPageViewHandle, SettingsWidget,
};
use super::{
    flags, LocalOnlyIconState, SettingActionPairContexts, SettingActionPairDescriptions,
    SettingsAction, SettingsSection, ToggleSettingActionPair, ToggleState,
};
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
use crate::drive::settings::WarpDriveSettings;

#[derive(Debug, Clone)]
pub enum WarpDriveSettingsPageAction {
    ToggleShowWarpDrive,
    SignUp,
    OpenUrl(String),
}

pub fn init_actions_from_parent_view<T: Action + Clone>(
    app: &mut AppContext,
    context: &ContextPredicate,
    builder: fn(SettingsAction) -> T,
) {
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![ToggleSettingActionPair::custom(
            SettingActionPairDescriptions::new("Enable Warp Drive", "Disable Warp Drive"),
            builder(SettingsAction::WarpDrive(
                WarpDriveSettingsPageAction::ToggleShowWarpDrive,
            )),
            SettingActionPairContexts::new(
                context.clone() & !id!(flags::ENABLE_WARP_DRIVE) & !id!("IsAnonymousUser"),
                context.clone() & id!(flags::ENABLE_WARP_DRIVE) & !id!("IsAnonymousUser"),
            ),
            None,
        )
        .with_enabled(|| FeatureFlag::OpenWarpNewSettingsModes.is_enabled())],
        app,
    );
}

pub enum WarpDriveSettingsPageEvent {
    SignUp,
}

pub struct WarpDriveSettingsPageView {
    page: PageType<Self>,
}

impl WarpDriveSettingsPageView {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            page: PageType::new_uncategorized(
                vec![
                    Box::new(WarpDriveHeaderWidget::default()),
                    Box::new(WarpDriveToggleWidget::default()),
                ],
                None,
            ),
        }
    }
}

impl Entity for WarpDriveSettingsPageView {
    type Event = WarpDriveSettingsPageEvent;
}

impl TypedActionView for WarpDriveSettingsPageView {
    type Action = WarpDriveSettingsPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            WarpDriveSettingsPageAction::ToggleShowWarpDrive => {
                WarpDriveSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.enable_warp_drive.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            WarpDriveSettingsPageAction::SignUp => {
                ctx.emit(WarpDriveSettingsPageEvent::SignUp);
            }
            WarpDriveSettingsPageAction::OpenUrl(url) => {
                ctx.open_url(url.as_str());
            }
        }
    }
}

impl View for WarpDriveSettingsPageView {
    fn ui_name() -> &'static str {
        "WarpDrivePage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl SettingsPageMeta for WarpDriveSettingsPageView {
    fn section() -> SettingsSection {
        SettingsSection::WarpDrive
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        FeatureFlag::OpenWarpNewSettingsModes.is_enabled()
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<WarpDriveSettingsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<WarpDriveSettingsPageView>) -> Self {
        SettingsPageViewHandle::WarpDrive(view_handle)
    }
}

#[derive(Default)]
struct WarpDriveHeaderWidget {
    sign_up_button: MouseStateHandle,
}

impl SettingsWidget for WarpDriveHeaderWidget {
    type View = WarpDriveSettingsPageView;

    fn search_terms(&self) -> &str {
        "warp drive sign up"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        FeatureFlag::SkipFirebaseAnonymousUser.is_enabled()
            && AuthStateProvider::as_ref(app)
                .get()
                .is_anonymous_or_logged_out()
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();

        let message = Container::new(
            Text::new_inline(
                "To use Warp Drive, please create an account.".to_string(),
                appearance.ui_font_family(),
                14.,
            )
            .with_color(
                appearance
                    .theme()
                    .sub_text_color(appearance.theme().surface_2())
                    .into_solid(),
            )
            .finish(),
        )
        .with_margin_right(16.)
        .finish();

        let button = Container::new(
            ui_builder
                .button(ButtonVariant::Accent, self.sign_up_button.clone())
                .with_style(UiComponentStyles {
                    font_size: Some(14.),
                    font_weight: Some(Weight::Semibold),
                    border_radius: Some(warpui::elements::CornerRadius::with_all(
                        warpui::elements::Radius::Pixels(4.),
                    )),
                    padding: Some(Coords {
                        top: 8.,
                        bottom: 8.,
                        left: 24.,
                        right: 24.,
                    }),
                    ..Default::default()
                })
                .with_text_label("Sign up".to_owned())
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(WarpDriveSettingsPageAction::SignUp);
                })
                .finish(),
        )
        .finish();

        Container::new(
            Flex::row()
                .with_cross_axis_alignment(warpui::elements::CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(1., message).finish())
                .with_child(button)
                .finish(),
        )
        .with_padding_bottom(15.)
        .finish()
    }
}

#[derive(Default)]
struct WarpDriveToggleWidget {
    switch_state: SwitchStateHandle,
    info_icon_mouse_state: MouseStateHandle,
}

impl SettingsWidget for WarpDriveToggleWidget {
    type View = WarpDriveSettingsPageView;

    fn search_terms(&self) -> &str {
        "warp drive tools panel command palette search workflows prompts notebooks environment variables"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let settings = WarpDriveSettings::as_ref(app);
        let is_anonymous_or_logged_out = FeatureFlag::SkipFirebaseAnonymousUser.is_enabled()
            && AuthStateProvider::as_ref(app)
                .get()
                .is_anonymous_or_logged_out();

        render_body_item::<WarpDriveSettingsPageAction>(
            "Warp Drive".into(),
            Some(AdditionalInfo {
                mouse_state: self.info_icon_mouse_state.clone(),
                on_click_action: Some(WarpDriveSettingsPageAction::OpenUrl(
                    "https://docs.warp.dev/knowledge-and-collaboration/warp-drive".to_string(),
                )),
                secondary_text: None,
                tooltip_override_text: None,
            }),
            LocalOnlyIconState::Hidden,
            if is_anonymous_or_logged_out {
                ToggleState::Disabled
            } else {
                ToggleState::Enabled
            },
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*settings.enable_warp_drive && !is_anonymous_or_logged_out)
                .with_disabled(is_anonymous_or_logged_out)
                .build()
                .on_click(move |ctx, _, _| {
                    if !is_anonymous_or_logged_out {
                        ctx.dispatch_typed_action(
                            WarpDriveSettingsPageAction::ToggleShowWarpDrive,
                        );
                    }
                })
                .finish(),
            Some("Warp Drive is a workspace in your terminal where you can save Workflows, Notebooks, Prompts, and Environment Variables for personal use or to share with a team.".into()),
        )
    }
}
