use std::cell::Cell;

use onboarding::components::feature_optout_dialog::{
    render_feature_optout_dialog, FeatureOptOutDialog,
};
use onboarding::slides::{layout, slide_content};
use onboarding::{OnboardingIntention, WARP_DRIVE_FEATURES};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use ui_components::{button, Component as _, Options as _};
use warp_core::features::FeatureFlag;
use warp_core::safe_error;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::Icon;
use warpui::actions::StandardAction;
use warpui::clipboard::ClipboardContent;
use warpui::elements::{
    Align, CacheOption, ChildAnchor, ClippedScrollStateHandle, Container, CornerRadius,
    CrossAxisAlignment, Dismiss, Fill, Flex, FormattedTextElement, HighlightedHyperlink, Image,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Shrinkable, Stack,
};
use warpui::fonts::Weight;
use warpui::keymap::{FixedBinding, Keystroke};
use warpui::text_layout::TextAlignment;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, UpdateModel, View,
    ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::auth_view_modal::AuthRedirectPayload;
use crate::auth::auth_view_shared_helpers::{
    render_privacy_settings_toggles, PrivacySettingsActions, PrivacySettingsHandles,
};
use crate::auth::login_failure_notification::{self, LoginFailureReason};
use crate::editor::{EditorView, SingleLineEditorOptions, TextColors, TextOptions};
use crate::server::telemetry::{LoginEventSource, TelemetryEvent};
use crate::settings::PrivacySettings;
use crate::themes::theme::Fill as ThemeFill;
use crate::util::bindings::CustomAction;
use crate::{send_telemetry_from_ctx, send_telemetry_sync_from_ctx};

const TOS_URL: &str = "https://www.warp.dev/terms-of-service";

// ---------------------------------------------------------------------------
// Init (keybindings)
// ---------------------------------------------------------------------------

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "enter",
            LoginSlideAction::Enter,
            id!(LoginSlideView::ui_name()),
        ),
        FixedBinding::new(
            "cmdorctrl-enter",
            LoginSlideAction::ShowSkipDialog,
            id!(LoginSlideView::ui_name()),
        ),
        FixedBinding::new(
            "escape",
            LoginSlideAction::DismissOverlayOrBack,
            id!(LoginSlideView::ui_name()),
        ),
        FixedBinding::custom(
            CustomAction::Paste,
            LoginSlideAction::PasteAuthUrl,
            "Paste",
            id!(LoginSlideView::ui_name()),
        ),
        FixedBinding::standard(
            StandardAction::Paste,
            LoginSlideAction::PasteAuthUrl,
            id!(LoginSlideView::ui_name()),
        ),
    ]);

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))]
    app.register_fixed_bindings([FixedBinding::new(
        "cmdorctrl-v",
        LoginSlideAction::PasteAuthUrl,
        id!(LoginSlideView::ui_name()),
    )]);
}

// ---------------------------------------------------------------------------
// Actions & Events
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub enum LoginSlideAction {
    Enter,
    ShowSkipDialog,
    ConfirmSkip,
    LoginFromSkipDialog,
    DismissDialog,
    DismissOverlayOrBack,
    Back,
    BackToSelectAuthPathway,
    CopyLoginUrl,
    EnterToken,
    ShowPrivacySettings,
    HideOverlay,
    ToggleTelemetry,
    ToggleCrashReporting,
    ToggleCloudConversationStorage,
    DismissNotification,
    PasteAuthUrl,
}

#[derive(Clone, Debug)]
pub enum LoginSlideEvent {
    BackToOnboarding,
    LoginLaterConfirmed,
}

/// How the user arrived at the login slide. Controls which step is shown first
/// and how "Back" is routed when the user backs out of the privacy-settings step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginSlideSource {
    /// Reached via the normal onboarding flow (e.g. agent intention requires an account).
    OnboardingFlow,
    /// Reached via the "Log in" link on the intro / welcome slide.
    LoginExistingUserFromWelcome,
    /// Reached via the "Privacy Settings" link on the terminal-intention theme slide.
    /// Starts directly in the privacy settings step and routes Back to onboarding.
    PrivacySettingsFromTerminalIntentionTheme,
}

// ---------------------------------------------------------------------------
// Login step
// ---------------------------------------------------------------------------

enum LoginStep {
    SelectAuthPathway,
    BrowserOpen,
    PrivacySettings,
}

// ---------------------------------------------------------------------------
// Overlay
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug)]
enum LoginSlideOverlay {
    SkipDialog,
}

/// Why the login slide is being shown, which drives its copy. All three paths
/// need an account: Terminal+Drive for cloud sync, and the Warp-agent and
/// third-party paths because Warp's AI features run on a Warp account. Skipping
/// therefore defers sign-in and leaves the gated features off until the user
/// creates an account.
enum LoginPurpose {
    WarpAgent,
    WarpDrive,
    ThirdParty,
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

const AUTH_TOKEN_INPUT_BORDER_RADIUS: Radius = Radius::Pixels(4.);

pub struct LoginSlideView {
    /// Whether this path wants AI (agent intent) vs. not (terminal intention).
    /// Used to gate the cloud-conversation-storage toggle and AI wording in the
    /// privacy settings step. This reflects intent, not the final state: AI runs
    /// on a Warp account, so skipping login leaves it off even when this is true.
    /// The actual `AISettings` value is written when settings are applied.
    ai_enabled: bool,
    /// Whether the user chose third-party (BYO) agents during onboarding. Drives
    /// the agent-path login copy ("Create an account" for third-party vs. "Get
    /// started with AI" for Warp Agent); it does not affect whether AI is
    /// enabled, which depends on the user creating an account.
    uses_third_party_agents: bool,
    /// Onboarding intention selected by the user, used to render Drive-focused
    /// copy on the Terminal+Drive path. On the login slide, `intention ==
    /// OnboardingIntention::Terminal` is equivalent to "Terminal+Drive":
    /// `RootView` only routes Terminal-intent users here when Warp Drive is
    /// enabled.
    intention: OnboardingIntention,
    theme_visual_path: &'static str,
    step: LoginStep,
    active_overlay: Option<LoginSlideOverlay>,
    last_login_failure_reason: Option<LoginFailureReason>,
    source: LoginSlideSource,

    // Auth token input (browser-open step)
    auth_token_input: ViewHandle<EditorView>,
    show_auth_token_input: bool,

    // Buttons
    back_button: button::Button,
    skip_button: button::Button,
    login_button: button::Button,
    browser_back_button: button::Button,
    done_button: button::Button,
    dialog_login_button: button::Button,
    dialog_skip_button: button::Button,
    dialog_close_button: button::Button,

    // Mouse states for links
    tos_mouse_state: MouseStateHandle,
    privacy_settings_mouse_state: MouseStateHandle,
    copy_url_mouse_state: MouseStateHandle,
    enter_token_mouse_state: MouseStateHandle,

    // Privacy settings overlay (shared with AuthViewBody)
    privacy_settings_handles: PrivacySettingsHandles,

    scroll_state: ClippedScrollStateHandle,
    close_login_notification_mouse_state: MouseStateHandle,
    highlighted_hyperlink_state: HighlightedHyperlink,
}

/// All image paths used by the login slide visual. These mirror the set in
/// `ThemePickerSlide::VISUAL_IMAGE_PATHS` so the login slide can keep showing
/// the same themed right panel the user was looking at on the theme slide.
const VISUAL_IMAGE_PATHS: &[&str] = &[
    // Terminal intention
    "async/png/onboarding/terminal_intention/theme/theme_phenomenon_vertical.png",
    "async/png/onboarding/terminal_intention/theme/theme_phenomenon_horizontal.png",
    "async/png/onboarding/terminal_intention/theme/theme_dark_vertical.png",
    "async/png/onboarding/terminal_intention/theme/theme_dark_horizontal.png",
    "async/png/onboarding/terminal_intention/theme/theme_light_vertical.png",
    "async/png/onboarding/terminal_intention/theme/theme_light_horizontal.png",
    "async/png/onboarding/terminal_intention/theme/theme_adeberry_vertical.png",
    "async/png/onboarding/terminal_intention/theme/theme_adeberry_horizontal.png",
    // Agent intention
    "async/png/onboarding/agent_intention/theme/theme_phenomenon_vertical.png",
    "async/png/onboarding/agent_intention/theme/theme_phenomenon_horizontal.png",
    "async/png/onboarding/agent_intention/theme/theme_dark_vertical.png",
    "async/png/onboarding/agent_intention/theme/theme_dark_horizontal.png",
    "async/png/onboarding/agent_intention/theme/theme_light_vertical.png",
    "async/png/onboarding/agent_intention/theme/theme_light_horizontal.png",
    "async/png/onboarding/agent_intention/theme/theme_adeberry_vertical.png",
    "async/png/onboarding/agent_intention/theme/theme_adeberry_horizontal.png",
];

fn resolve_visual_path(
    intention: OnboardingIntention,
    theme_name: &str,
    use_vertical_tabs: bool,
) -> &'static str {
    let intention_dir = match intention {
        OnboardingIntention::AgentDrivenDevelopment => "agent_intention",
        OnboardingIntention::Terminal => "terminal_intention",
    };
    let name_key = match theme_name {
        "Phenomenon" => "phenomenon",
        "Dark" => "dark",
        "Light" => "light",
        "Adeberry" => "adeberry",
        _ => "dark",
    };
    let orientation = if use_vertical_tabs {
        "vertical"
    } else {
        "horizontal"
    };
    VISUAL_IMAGE_PATHS
        .iter()
        .find(|p| p.contains(intention_dir) && p.contains(name_key) && p.contains(orientation))
        .unwrap_or(&VISUAL_IMAGE_PATHS[0])
}

impl LoginSlideView {
    /// Whether the auth token input editor is currently rendered and should be focusable.
    /// This is only true on the BrowserOpen step after the user clicks to paste their token.
    pub fn is_auth_token_input_visible(&self) -> bool {
        matches!(self.step, LoginStep::BrowserOpen) && self.show_auth_token_input
    }

    pub fn new(
        ai_enabled: bool,
        uses_third_party_agents: bool,
        theme_name: &str,
        use_vertical_tabs: bool,
        intention: OnboardingIntention,
        source: LoginSlideSource,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let auth_manager = AuthManager::handle(ctx);
        ctx.subscribe_to_model(&auth_manager, |me, _, event, ctx| {
            me.handle_auth_manager_event(event, ctx);
        });

        let auth_token_input = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let text_color = ThemeFill::Solid(ColorU::black());
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions {
                        font_size_override: Some(12.),
                        font_family_override: Some(appearance.ui_font_family()),
                        text_colors_override: Some(TextColors {
                            default_color: text_color,
                            disabled_color: text_color.with_opacity(20),
                            hint_color: text_color.with_opacity(40),
                        }),
                        ..Default::default()
                    },
                    soft_wrap: false,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text("Auth Token", ctx);
            editor
        });

        ctx.subscribe_to_view(&auth_token_input, |me, _, event, ctx| {
            use crate::editor::Event::{AltEnter, CmdEnter, Enter, Paste, ShiftEnter};
            match event {
                AltEnter | CmdEnter | Enter | Paste | ShiftEnter => {
                    let text = me.auth_token_input.as_ref(ctx).buffer_text(ctx);
                    me.handle_pasted_auth_url(text, ctx);
                }
                _ => {}
            };
            ctx.notify();
        });

        Self {
            ai_enabled,
            uses_third_party_agents,
            intention,
            theme_visual_path: resolve_visual_path(intention, theme_name, use_vertical_tabs),
            step: match source {
                LoginSlideSource::OnboardingFlow => LoginStep::SelectAuthPathway,
                LoginSlideSource::LoginExistingUserFromWelcome => LoginStep::BrowserOpen,
                LoginSlideSource::PrivacySettingsFromTerminalIntentionTheme => {
                    LoginStep::PrivacySettings
                }
            },
            active_overlay: None,
            last_login_failure_reason: None,
            source,
            auth_token_input,
            show_auth_token_input: false,
            back_button: button::Button::default(),
            skip_button: button::Button::default(),
            login_button: button::Button::default(),
            browser_back_button: button::Button::default(),
            done_button: button::Button::default(),
            dialog_login_button: button::Button::default(),
            dialog_skip_button: button::Button::default(),
            dialog_close_button: button::Button::default(),
            tos_mouse_state: MouseStateHandle::default(),
            privacy_settings_mouse_state: MouseStateHandle::default(),
            copy_url_mouse_state: MouseStateHandle::default(),
            enter_token_mouse_state: MouseStateHandle::default(),
            privacy_settings_handles: PrivacySettingsHandles::default(),
            scroll_state: ClippedScrollStateHandle::new(),
            close_login_notification_mouse_state: MouseStateHandle::default(),
            highlighted_hyperlink_state: HighlightedHyperlink::default(),
        }
    }

    // ------------------------------------------------------------------
    // Auth manager
    // ------------------------------------------------------------------

    fn handle_auth_manager_event(&mut self, event: &AuthManagerEvent, ctx: &mut ViewContext<Self>) {
        match event {
            AuthManagerEvent::AuthFailed(err) => {
                use crate::server::server_api::auth::UserAuthenticationError;
                if let UserAuthenticationError::InvalidStateParameter = err {
                    self.last_login_failure_reason =
                        Some(LoginFailureReason::InvalidStateParameter);
                } else if let UserAuthenticationError::MissingStateParameter = err {
                    self.last_login_failure_reason =
                        Some(LoginFailureReason::MissingStateParameter);
                } else {
                    self.last_login_failure_reason =
                        Some(LoginFailureReason::FailedUserAuthentication);
                }
            }
            AuthManagerEvent::CreateAnonymousUserFailed => {
                self.last_login_failure_reason = Some(LoginFailureReason::FailedUserAuthentication);
            }
            AuthManagerEvent::MintCustomTokenFailed(_) => {
                self.last_login_failure_reason = Some(LoginFailureReason::FailedMintCustomToken);
            }
            _ => {}
        }
        ctx.notify();
    }

    fn handle_pasted_auth_url(&mut self, pasted_url: String, ctx: &mut ViewContext<Self>) {
        match AuthRedirectPayload::from_raw_url(pasted_url) {
            Ok(redirect_payload) => {
                AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                    auth_manager.initialize_user_from_auth_payload(redirect_payload, true, ctx);
                });
            }
            Err(error) => {
                safe_error!(
                    safe: ("Failed to parse AuthRedirectPayload from redirect URL"),
                    full: ("Failed to parse AuthRedirectPayload from redirect URL: {error:#}")
                );
                self.last_login_failure_reason =
                    Some(LoginFailureReason::InvalidRedirectUrl { was_pasted: true });
            }
        }
        ctx.notify();
    }

    fn handle_login_later(&mut self, ctx: &mut ViewContext<Self>) {
        // Send synchronously since this is an important event in the sign up funnel and we
        // don't want to lose events if the user quits before the event queue is flushed.
        send_telemetry_sync_from_ctx!(
            TelemetryEvent::LoginLaterConfirmationButtonClicked {
                source: LoginEventSource::OnboardingSlide,
            },
            ctx
        );
        if FeatureFlag::SkipFirebaseAnonymousUser.is_enabled() {
            AuthManager::handle(ctx).update(ctx, |_, ctx| {
                ctx.emit(AuthManagerEvent::SkippedLogin);
            });
        } else {
            AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                auth_manager.create_anonymous_user(None, ctx);
            });
        }
        ctx.emit(LoginSlideEvent::LoginLaterConfirmed);
    }

    /// Starts the browser sign-up flow. Shared by the Continue button and the
    /// skip dialog's cancel button.
    fn start_login(&mut self, ctx: &mut ViewContext<Self>) {
        send_telemetry_from_ctx!(
            TelemetryEvent::LoginButtonClicked {
                source: LoginEventSource::OnboardingSlide,
            },
            ctx
        );
        self.last_login_failure_reason = None;
        self.step = LoginStep::BrowserOpen;
        AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
            let sign_up_url = auth_manager.sign_up_url();
            ctx.open_url(&sign_up_url);
        });
        ctx.notify();
    }

    // ------------------------------------------------------------------
    // Rendering — main layout
    // ------------------------------------------------------------------

    fn render_content(
        &self,
        appearance: &Appearance,
        app: &AppContext,
        editor_rendered: &Cell<bool>,
    ) -> Box<dyn Element> {
        match self.step {
            LoginStep::SelectAuthPathway => {
                let children = self.render_select_auth_content(appearance);
                let bottom_nav = self.render_select_auth_bottom_nav(appearance);
                slide_content::onboarding_slide_content(
                    children,
                    bottom_nav,
                    self.scroll_state.clone(),
                    appearance,
                )
            }
            LoginStep::BrowserOpen => {
                let children = self.render_browser_open_content(appearance, editor_rendered);
                let bottom_nav = self.render_browser_open_bottom_nav(appearance);
                slide_content::onboarding_slide_content(
                    children,
                    bottom_nav,
                    self.scroll_state.clone(),
                    appearance,
                )
            }
            LoginStep::PrivacySettings => {
                let children = self.render_privacy_settings_content(appearance, app);
                let bottom_nav = self.render_privacy_settings_bottom_nav(appearance);
                slide_content::onboarding_slide_content(
                    children,
                    bottom_nav,
                    self.scroll_state.clone(),
                    appearance,
                )
            }
        }
    }

    // ------------------------------------------------------------------
    // Step 1: Select auth pathway
    // ------------------------------------------------------------------

    /// Disclaimer prefix shown before the "Privacy Settings" link. AI is
    /// dropped from the wording on paths that don't enable AI (e.g.
    /// Terminal+Drive), since there are no AI features to opt out of there.
    fn privacy_disclaimer_prefix(&self) -> &'static str {
        if self.ai_enabled {
            "If you'd like to opt out of analytics and AI features, you can adjust your "
        } else {
            "If you'd like to opt out of analytics, you can adjust your "
        }
    }

    fn login_purpose(&self) -> LoginPurpose {
        match self.intention {
            OnboardingIntention::Terminal => LoginPurpose::WarpDrive,
            OnboardingIntention::AgentDrivenDevelopment => {
                if self.uses_third_party_agents {
                    LoginPurpose::ThirdParty
                } else {
                    LoginPurpose::WarpAgent
                }
            }
        }
    }

    fn render_select_auth_content(&self, appearance: &Appearance) -> Vec<Box<dyn Element>> {
        let theme = appearance.theme();
        let sub_text_color = internal_colors::text_sub(theme, theme.background().into_solid());
        let ui_builder = appearance.ui_builder();

        let (title_text, subtitle_text) = match self.login_purpose() {
            LoginPurpose::WarpDrive => (
                "Get started with Warp Drive",
                "Connect your account to save and share notebooks, workflows, and more across devices.",
            ),
            LoginPurpose::WarpAgent => (
                "Get started with AI",
                "Connect your account to enable AI-powered planning, coding, and automation.",
            ),
            LoginPurpose::ThirdParty => (
                "Create an account",
                "Create a Warp account to enable AI-powered planning, coding, and automations.",
            ),
        };
        let title = FormattedTextElement::from_str(title_text, appearance.ui_font_family(), 36.)
            .with_color(internal_colors::text_main(
                theme,
                theme.background().into_solid(),
            ))
            .with_weight(Weight::Medium)
            .with_alignment(TextAlignment::Left)
            .finish();

        let subtitle =
            FormattedTextElement::from_str(subtitle_text, appearance.ui_font_family(), 16.)
                .with_color(sub_text_color)
                .with_weight(Weight::Normal)
                .with_alignment(TextAlignment::Left)
                .with_line_height_ratio(1.0)
                .finish();

        // TOS and Privacy links
        let disclaimer_styles = UiComponentStyles {
            font_color: Some(sub_text_color),
            font_size: Some(12.),
            ..Default::default()
        };

        let tos_line = Flex::row()
            .with_child(
                ui_builder
                    .span("By continuing, you agree to Warp's ")
                    .with_style(disclaimer_styles)
                    .build()
                    .finish(),
            )
            .with_child(
                ui_builder
                    .link(
                        "Terms of Service".into(),
                        Some(TOS_URL.into()),
                        None,
                        self.tos_mouse_state.clone(),
                    )
                    .soft_wrap(false)
                    .with_style(UiComponentStyles {
                        font_size: Some(12.),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish();

        let privacy_line = Flex::row()
            .with_child(
                ui_builder
                    .span(self.privacy_disclaimer_prefix())
                    .with_style(disclaimer_styles)
                    .build()
                    .finish(),
            )
            .with_child(
                ui_builder
                    .link(
                        "Privacy Settings".into(),
                        None,
                        Some(Box::new(|ctx| {
                            ctx.dispatch_typed_action(LoginSlideAction::ShowPrivacySettings);
                        })),
                        self.privacy_settings_mouse_state.clone(),
                    )
                    .soft_wrap(false)
                    .with_style(UiComponentStyles {
                        font_size: Some(12.),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish();

        let disclaimers = Container::new(
            Flex::column()
                .with_child(privacy_line)
                .with_child(Container::new(tos_line).with_margin_top(8.).finish())
                .finish(),
        )
        .with_margin_top(24.)
        .finish();

        let header = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(title)
            .with_child(Container::new(subtitle).with_margin_top(16.).finish())
            .with_child(disclaimers)
            .finish();

        vec![header]
    }

    fn render_select_auth_bottom_nav(&self, appearance: &Appearance) -> Box<dyn Element> {
        let back_button = self.back_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Back".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(LoginSlideAction::Back);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let cmd_enter = Keystroke::parse("cmdorctrl-enter").unwrap_or_default();
        let skip_label = match self.login_purpose() {
            LoginPurpose::WarpDrive => "Disable Warp Drive",
            LoginPurpose::WarpAgent => "Skip for now",
            LoginPurpose::ThirdParty => "Skip for now",
        };
        let skip_button = self.skip_button.render(
            appearance,
            button::Params {
                content: button::Content::Label(skip_label.into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    keystroke: Some(cmd_enter),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(LoginSlideAction::ShowSkipDialog);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let enter = Keystroke::parse("enter").unwrap_or_default();
        let login_button = self.login_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Continue".into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    keystroke: Some(enter),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(LoginSlideAction::Enter);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let right_buttons = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(skip_button)
            .with_child(Container::new(login_button).with_margin_left(4.).finish())
            .finish();

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(back_button)
            .with_child(right_buttons)
            .finish()
    }

    // ------------------------------------------------------------------
    // Step 2: Browser open
    // ------------------------------------------------------------------

    fn render_browser_open_content(
        &self,
        appearance: &Appearance,
        editor_rendered: &Cell<bool>,
    ) -> Vec<Box<dyn Element>> {
        let theme = appearance.theme();
        let sub_text_color = internal_colors::text_sub(theme, theme.background().into_solid());
        let ui_builder = appearance.ui_builder();

        let sub_text_styles = UiComponentStyles {
            font_color: Some(sub_text_color),
            ..Default::default()
        };

        let title = FormattedTextElement::from_str(
            "Sign in on your browser to continue",
            appearance.ui_font_family(),
            36.,
        )
        .with_color(internal_colors::text_main(
            theme,
            theme.background().into_solid(),
        ))
        .with_weight(Weight::Medium)
        .with_alignment(TextAlignment::Left)
        .finish();

        let hint = Flex::column()
            .with_child(
                Flex::row()
                    .with_child(
                        ui_builder
                            .span("If your browser hasn't launched, ")
                            .with_style(sub_text_styles)
                            .build()
                            .finish(),
                    )
                    .with_child(
                        ui_builder
                            .link(
                                "copy the URL".into(),
                                None,
                                Some(Box::new(|ctx| {
                                    ctx.dispatch_typed_action(LoginSlideAction::CopyLoginUrl);
                                })),
                                self.copy_url_mouse_state.clone(),
                            )
                            .soft_wrap(false)
                            .build()
                            .finish(),
                    )
                    .with_child(
                        ui_builder
                            .span(" and open")
                            .with_style(sub_text_styles)
                            .build()
                            .finish(),
                    )
                    .finish(),
            )
            .with_child(
                ui_builder
                    .span("the page manually.")
                    .with_style(sub_text_styles)
                    .build()
                    .finish(),
            )
            .finish();

        // Auth token: show either the "Click here" link or the input box.
        // When showing the input, we use `editor_rendered` (a Cell<bool> passed
        // from render()) so the ChildView is only created on the FIRST call of
        // this closure. static_left calls the left-content closure twice (for
        // narrow and wide layouts); creating two ChildViews for the same editor
        // breaks focus/event dispatch.
        let auth_token: Box<dyn Element> = if self.show_auth_token_input {
            if editor_rendered.get() {
                // Second call (two-column layout, the default): render the real editor.
                ui_builder
                    .text_input(self.auth_token_input.clone())
                    .with_style(UiComponentStyles {
                        background: Some(Fill::Solid(ColorU::white())),
                        border_width: Some(0.),
                        border_radius: Some(CornerRadius::with_all(AUTH_TOKEN_INPUT_BORDER_RADIUS)),
                        padding: Some(Coords {
                            top: 12.,
                            bottom: 12.,
                            left: 16.,
                            right: 16.,
                        }),
                        margin: Some(Coords {
                            top: 8.,
                            bottom: 0.,
                            left: 0.,
                            right: 0.,
                        }),
                        ..Default::default()
                    })
                    .build()
                    .finish()
            } else {
                // First call (narrow layout fallback): placeholder.
                editor_rendered.set(true);
                Container::new(warpui::elements::Empty::new().finish())
                    .with_padding_top(12.)
                    .with_padding_bottom(12.)
                    .with_padding_left(16.)
                    .with_padding_right(16.)
                    .with_margin_top(8.)
                    .finish()
            }
        } else {
            Flex::row()
                .with_child(
                    ui_builder
                        .link(
                            "Click here to paste your token from the browser".into(),
                            None,
                            Some(Box::new(|ctx| {
                                ctx.dispatch_typed_action(LoginSlideAction::EnterToken);
                            })),
                            self.enter_token_mouse_state.clone(),
                        )
                        .soft_wrap(false)
                        .build()
                        .finish(),
                )
                .finish()
        };

        let header = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(title)
            .with_child(Container::new(hint).with_margin_top(16.).finish())
            .with_child(Container::new(auth_token).with_margin_top(16.).finish())
            .finish();

        vec![header]
    }

    fn render_browser_open_bottom_nav(&self, appearance: &Appearance) -> Box<dyn Element> {
        let back_button = self.browser_back_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Back".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(LoginSlideAction::BackToSelectAuthPathway);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(back_button)
            .finish()
    }

    // ------------------------------------------------------------------
    // Step 3: Privacy settings (inline in left column)
    // ------------------------------------------------------------------

    fn render_privacy_settings_content(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Vec<Box<dyn Element>> {
        let theme = appearance.theme();

        let title =
            FormattedTextElement::from_str("Privacy Settings", appearance.ui_font_family(), 36.)
                .with_color(internal_colors::text_main(
                    theme,
                    theme.background().into_solid(),
                ))
                .with_weight(Weight::Medium)
                .with_alignment(TextAlignment::Left)
                .finish();

        let actions = PrivacySettingsActions {
            toggle_telemetry: LoginSlideAction::ToggleTelemetry,
            toggle_crash_reporting: LoginSlideAction::ToggleCrashReporting,
            toggle_cloud_conversation_storage: LoginSlideAction::ToggleCloudConversationStorage,
            hide_overlay: LoginSlideAction::HideOverlay,
        };

        let toggles = render_privacy_settings_toggles(
            appearance,
            app,
            &self.privacy_settings_handles,
            &actions,
            self.ai_enabled,
        );

        vec![title, Container::new(toggles).with_margin_top(24.).finish()]
    }

    fn render_privacy_settings_bottom_nav(&self, appearance: &Appearance) -> Box<dyn Element> {
        let back_button = self.done_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Back".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(LoginSlideAction::HideOverlay);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(back_button)
            .finish()
    }

    // ------------------------------------------------------------------
    // Visual
    // ------------------------------------------------------------------

    fn render_visual(&self) -> Box<dyn Element> {
        let path = self.theme_visual_path;
        layout::onboarding_right_panel_with_bg(path, layout::FOREGROUND_LAYOUT_DEFAULT)
    }

    // ------------------------------------------------------------------
    // Rendering — skip confirmation dialog
    // ------------------------------------------------------------------

    fn render_skip_dialog(&self, appearance: &Appearance) -> Box<dyn Element> {
        let (title, body, features, cancel_label): (
            &'static str,
            &'static str,
            &'static [&'static str],
            &'static str,
        ) = match self.login_purpose() {
            LoginPurpose::WarpDrive => (
                "Are you sure you want to disable Warp Drive?",
                "Warp Drive lets you save workflows and knowledge across devices and share them with your team. By continuing, you won't have access to the following features:",
                WARP_DRIVE_FEATURES,
                "Enable Warp Drive",
            ),
            LoginPurpose::WarpAgent | LoginPurpose::ThirdParty => (
                "Continue without signing in?",
                "Without an account, you won't have access to Warp's AI features. Sign in anytime to unlock agents and other AI features.",
                &[],
                "Sign in",
            ),
        };

        // Close button with ESC keyboard-shortcut badge.
        let escape = Keystroke::parse("escape").unwrap_or_default();
        let close_button = self.dialog_close_button.render(
            appearance,
            button::Params {
                content: button::Content::Icon(Icon::X),
                theme: &button::themes::Naked,
                options: button::Options {
                    keystroke: Some(escape),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(LoginSlideAction::DismissDialog);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let cancel_button = self.dialog_login_button.render(
            appearance,
            button::Params {
                content: button::Content::Label(cancel_label.into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(LoginSlideAction::LoginFromSkipDialog);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let dialog_enter = Keystroke::parse("enter").unwrap_or_default();
        let confirm_button = self.dialog_skip_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Skip for now".into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    keystroke: Some(dialog_enter),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(LoginSlideAction::ConfirmSkip);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        render_feature_optout_dialog(
            appearance,
            FeatureOptOutDialog {
                title,
                body,
                features,
                close_button,
                cancel_button,
                confirm_button,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Entity / View / TypedActionView
// ---------------------------------------------------------------------------

impl Entity for LoginSlideView {
    type Event = LoginSlideEvent;
}

impl View for LoginSlideView {
    fn ui_name() -> &'static str {
        "LoginSlideView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut stack = Stack::new();

        // Background (same as onboarding parent)
        if let Some(img) = theme.background_image() {
            stack.add_child(
                Shrinkable::new(
                    1.,
                    Image::new(img.source(), CacheOption::Original)
                        .cover()
                        .finish(),
                )
                .finish(),
            );
            let overlay_opacity = (100u8).saturating_sub(img.opacity);
            stack.add_child(
                warpui::elements::Rect::new()
                    .with_background(theme.background().with_opacity(overlay_opacity))
                    .finish(),
            );
        } else {
            stack.add_child(
                Container::new(warpui::elements::Empty::new().finish())
                    .with_background(theme.background())
                    .finish(),
            );
        }

        // Two-column slide layout
        // static_left calls the left closure twice (narrow + wide). We use a
        // Cell<bool> so the editor ChildView is only created once.
        let editor_rendered = Cell::new(false);
        let slide = layout::static_left(
            || self.render_content(appearance, app, &editor_rendered),
            || self.render_visual(),
        );
        stack.add_child(slide);

        // Skip dialog overlay
        if matches!(self.active_overlay, Some(LoginSlideOverlay::SkipDialog)) {
            let dialog = self.render_skip_dialog(appearance);
            stack.add_child(
                warpui::elements::Rect::new()
                    .with_background(
                        warp_core::ui::theme::Fill::Solid(ColorU::black()).with_opacity(60),
                    )
                    .finish(),
            );
            let centered = Align::new(dialog).finish();
            stack.add_child(
                Dismiss::new(centered)
                    .on_dismiss(|ctx, _app| {
                        ctx.dispatch_typed_action(LoginSlideAction::DismissDialog);
                    })
                    .finish(),
            );
        }

        // Login failure notification
        if let Some(login_failure_reason) = &self.last_login_failure_reason {
            let notification = login_failure_notification::render(
                login_failure_reason,
                self.close_login_notification_mouse_state.clone(),
                self.highlighted_hyperlink_state.clone(),
                LoginSlideAction::DismissNotification,
                app,
            );
            stack.add_positioned_overlay_child(
                notification,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 40.),
                    ParentOffsetBounds::ParentBySize,
                    ParentAnchor::TopMiddle,
                    ChildAnchor::TopMiddle,
                ),
            );
        }

        stack.finish()
    }
}

impl TypedActionView for LoginSlideView {
    type Action = LoginSlideAction;

    fn handle_action(&mut self, action: &LoginSlideAction, ctx: &mut ViewContext<Self>) {
        match action {
            LoginSlideAction::Enter => {
                // When the skip dialog is open, Enter should confirm skip instead.
                if self.active_overlay.is_some() {
                    self.active_overlay = None;
                    self.handle_login_later(ctx);
                    return;
                }
                // Otherwise Enter is log in
                self.start_login(ctx);
            }
            LoginSlideAction::ShowSkipDialog => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::LoginLaterButtonClicked {
                        source: LoginEventSource::OnboardingSlide,
                    },
                    ctx
                );
                self.active_overlay = Some(LoginSlideOverlay::SkipDialog);
                ctx.notify();
            }
            LoginSlideAction::ConfirmSkip => {
                self.active_overlay = None;
                self.handle_login_later(ctx);
            }
            LoginSlideAction::LoginFromSkipDialog => {
                self.active_overlay = None;
                self.start_login(ctx);
            }
            LoginSlideAction::DismissDialog => {
                self.active_overlay = None;
                ctx.notify();
            }
            LoginSlideAction::DismissOverlayOrBack => {
                if self.active_overlay.is_some() {
                    self.active_overlay = None;
                    ctx.notify();
                } else if matches!(self.step, LoginStep::PrivacySettings) {
                    match self.source {
                        LoginSlideSource::PrivacySettingsFromTerminalIntentionTheme => {
                            ctx.emit(LoginSlideEvent::BackToOnboarding);
                        }
                        LoginSlideSource::OnboardingFlow
                        | LoginSlideSource::LoginExistingUserFromWelcome => {
                            self.step = LoginStep::SelectAuthPathway;
                            ctx.focus_self();
                            ctx.notify();
                        }
                    }
                } else if matches!(self.step, LoginStep::BrowserOpen) {
                    // PrivacySettingsFromTerminalIntentionTheme starts on the
                    // privacy-settings step and should never transition into the
                    // select-auth-pathway step. If this branch is ever reached
                    // for that source, route back to onboarding instead.
                    match self.source {
                        LoginSlideSource::LoginExistingUserFromWelcome
                        | LoginSlideSource::PrivacySettingsFromTerminalIntentionTheme => {
                            ctx.emit(LoginSlideEvent::BackToOnboarding);
                        }
                        LoginSlideSource::OnboardingFlow => {
                            self.step = LoginStep::SelectAuthPathway;
                            ctx.focus_self();
                            ctx.notify();
                        }
                    }
                } else {
                    ctx.emit(LoginSlideEvent::BackToOnboarding);
                }
            }
            LoginSlideAction::Back => {
                ctx.emit(LoginSlideEvent::BackToOnboarding);
            }
            LoginSlideAction::BackToSelectAuthPathway => match self.source {
                // PrivacySettingsFromTerminalIntentionTheme only ever shows the
                // privacy-settings step; treat "back" the same as login-from-
                // welcome and return to onboarding rather than falling through
                // to a step this source was designed to skip.
                LoginSlideSource::LoginExistingUserFromWelcome
                | LoginSlideSource::PrivacySettingsFromTerminalIntentionTheme => {
                    ctx.emit(LoginSlideEvent::BackToOnboarding);
                }
                LoginSlideSource::OnboardingFlow => {
                    self.step = LoginStep::SelectAuthPathway;
                    ctx.focus_self();
                    ctx.notify();
                }
            },
            LoginSlideAction::CopyLoginUrl => {
                AuthManager::handle(ctx).update(ctx, |auth_manager, inner_ctx| {
                    let sign_in_url = auth_manager.sign_in_url();
                    inner_ctx.clipboard().write(ClipboardContent {
                        plain_text: sign_in_url.clone(),
                        paths: Some(vec![sign_in_url]),
                        ..Default::default()
                    });
                });
            }
            LoginSlideAction::EnterToken => {
                self.auth_token_input
                    .update(ctx, |editor, ctx| editor.paste(ctx));
                self.show_auth_token_input = true;
                ctx.notify();
            }
            LoginSlideAction::ShowPrivacySettings => {
                send_telemetry_sync_from_ctx!(
                    TelemetryEvent::OpenAuthPrivacySettings {
                        source: LoginEventSource::OnboardingSlide,
                    },
                    ctx
                );
                self.step = LoginStep::PrivacySettings;
                ctx.notify();
            }
            LoginSlideAction::HideOverlay => {
                // "Done" button in privacy settings returns to the auth pathway step,
                // except when the user entered the slide via the terminal-intention theme slide's
                // Privacy Settings link — in that case Back returns to the onboarding view.
                self.active_overlay = None;
                match self.source {
                    LoginSlideSource::PrivacySettingsFromTerminalIntentionTheme => {
                        ctx.emit(LoginSlideEvent::BackToOnboarding);
                    }
                    LoginSlideSource::OnboardingFlow
                    | LoginSlideSource::LoginExistingUserFromWelcome => {
                        self.step = LoginStep::SelectAuthPathway;
                        ctx.focus_self();
                        ctx.notify();
                    }
                }
            }
            LoginSlideAction::ToggleTelemetry => {
                let handle = PrivacySettings::handle(ctx);
                ctx.update_model(&handle, |settings, ctx| {
                    settings.set_is_telemetry_enabled(!settings.is_telemetry_enabled, ctx);
                });
                ctx.notify();
            }
            LoginSlideAction::ToggleCrashReporting => {
                let handle = PrivacySettings::handle(ctx);
                ctx.update_model(&handle, |settings, ctx| {
                    settings
                        .set_is_crash_reporting_enabled(!settings.is_crash_reporting_enabled, ctx);
                });
                ctx.notify();
            }
            LoginSlideAction::ToggleCloudConversationStorage => {
                let handle = PrivacySettings::handle(ctx);
                ctx.update_model(&handle, |settings, ctx| {
                    settings.set_is_cloud_conversation_storage_enabled(
                        !settings.is_cloud_conversation_storage_enabled,
                        ctx,
                    );
                });
                ctx.notify();
            }
            LoginSlideAction::DismissNotification => {
                self.last_login_failure_reason = None;
                ctx.notify();
            }
            LoginSlideAction::PasteAuthUrl => {
                self.last_login_failure_reason = None;
                let clipboard_content = ctx.clipboard().read();
                if !clipboard_content.plain_text.is_empty() {
                    self.handle_pasted_auth_url(clipboard_content.plain_text, ctx);
                }
            }
        }
    }
}
