use std::time::Duration;

use ai::LLMId;
use instant::Instant;
use warp_core::send_telemetry_from_ctx;
use warpui_core::assets::asset_cache::AssetSource;
use warpui_core::image_cache::ImageType;
use warpui_core::windowing::state::{ApplicationStage, StateEvent};
use warpui_core::windowing::WindowManager;

use crate::components::feature_optout_dialog::{render_feature_optout_dialog, FeatureOptOutDialog};
use crate::model::{
    OnboardingAuthState, OnboardingStateEvent, OnboardingStateModel, OnboardingStep,
    SelectedSettings,
};
use crate::slides::{
    AgentSlide, AiAccessSlide, AiAccessSlideEvent, AiSetupSlide, CustomizeUISlide, IntentionSlide,
    IntroSlide, IntroSlideEvent, OnboardingModelInfo, OnboardingSlide, ThemePickerSlide,
    ThemePickerSlideEvent, ThirdPartySlide,
};
use crate::telemetry::OnboardingEvent;
use crate::AI_FEATURES;

const APP_BECAME_ACTIVE_DEBOUNCE: Duration = Duration::from_secs(15);

const PLAN_ACTIVATED_TOAST_DURATION: Duration = Duration::from_secs(5);

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use ui_components::{button, Component as _, Options as _};
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::{Fill, WarpTheme};
use warp_core::ui::Icon;
use warpui_core::elements::{
    Align, CacheOption, ChildAnchor, ConstrainedBox, Container, CrossAxisAlignment, Dismiss, Empty,
    Flex, Image, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning,
    ParentAnchor, ParentElement, ParentOffsetBounds, Rect, Shrinkable, Stack,
};
use warpui_core::fonts::Weight;
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{FixedBinding, Keystroke};
use warpui_core::presenter::ChildView;
use warpui_core::ui_components::components::{UiComponent as _, UiComponentStyles};
use warpui_core::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View,
    ViewContext, ViewHandle,
};

#[derive(Clone, Debug)]
pub enum AgentOnboardingEvent {
    ThemeSelected {
        theme_name: String,
    },
    SyncWithOsToggled {
        enabled: bool,
    },
    OnboardingCompleted(SelectedSettings),
    OnboardingSkipped,
    LoginFromWelcomeRequested,
    /// Emitted when the user clicks the "Privacy Settings" link on the terminal
    /// intention theme slide. The variant name encodes that the event is only
    /// emitted from the terminal-intention theme slide; consumers (e.g. a
    /// `LoginSlideView` with `LoginSlideSource::PrivacySettingsFromTerminalIntentionTheme`)
    /// rely on that to select the right visual / back-routing behavior.
    PrivacySettingsFromTerminalThemeSlideRequested,
    UpgradeRequested,
    UpgradeCopyUrlRequested,
    UpgradePasteTokenFromClipboardRequested,
    AddApiKeyRequested,
    AddCustomEndpointRequested,
    /// Emitted when the app regains focus (e.g. user returns from the browser).
    /// The parent should refresh any stale data: available models, workspace/billing metadata, etc.
    AppBecameActive,
}

pub struct AgentOnboardingView {
    onboarding_state: ModelHandle<OnboardingStateModel>,
    intro_slide: ViewHandle<IntroSlide>,
    theme_picker_slide: ViewHandle<ThemePickerSlide>,
    intention_slide: ViewHandle<IntentionSlide>,
    ai_setup_slide: ViewHandle<AiSetupSlide>,
    customize_slide: ViewHandle<CustomizeUISlide>,
    agent_slide: ViewHandle<AgentSlide>,
    ai_access_slide: ViewHandle<AiAccessSlide>,
    third_party_slide: ViewHandle<ThirdPartySlide>,
    skippable: bool,
    close_button: button::Button,
    no_ai_confirm_button: button::Button,
    no_ai_cancel_button: button::Button,
    no_ai_close_button: button::Button,
    last_model_refresh: Option<Instant>,
    show_plan_activated_toast: bool,
    last_auth_state: OnboardingAuthState,
    plan_activated_close_mouse_state: MouseStateHandle,
}

#[derive(Clone, Copy, Debug)]
pub enum AgentOnboardingAction {
    UpKey,
    DownKey,
    LeftKey,
    RightKey,
    TabKey,
    EnterKey,
    CmdOrCtrlEnterKey,
    Escape,
    NoAiConfirm,
    NoAiCancel,
    NoAiDismiss,
    DismissPlanActivatedToast,
}

fn dispatch_onboarding_action_to_slide<V: OnboardingSlide>(
    slide: &mut V,
    action: AgentOnboardingAction,
    ctx: &mut ViewContext<V>,
) {
    match action {
        AgentOnboardingAction::UpKey => slide.on_up(ctx),
        AgentOnboardingAction::DownKey => slide.on_down(ctx),
        AgentOnboardingAction::LeftKey => slide.on_left(ctx),
        AgentOnboardingAction::RightKey => slide.on_right(ctx),
        AgentOnboardingAction::TabKey => slide.on_tab(ctx),
        AgentOnboardingAction::EnterKey => slide.on_enter(ctx),
        AgentOnboardingAction::CmdOrCtrlEnterKey => slide.on_cmd_or_ctrl_enter(ctx),
        AgentOnboardingAction::Escape => slide.on_escape(ctx),
        // Parent-level actions are handled by the parent view, never routed to a slide.
        AgentOnboardingAction::NoAiConfirm
        | AgentOnboardingAction::NoAiCancel
        | AgentOnboardingAction::NoAiDismiss
        | AgentOnboardingAction::DismissPlanActivatedToast => {}
    }
}

impl AgentOnboardingView {
    /// Creates a new AgentOnboardingView.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        theme_picker_themes: [WarpTheme; 4],
        skippable: bool,
        models: Vec<OnboardingModelInfo>,
        default_model_id: LLMId,
        workspace_enforces_autonomy: bool,
        auth_state: OnboardingAuthState,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let onboarding_state = ctx.add_model(|_| {
            OnboardingStateModel::new(
                models,
                default_model_id,
                workspace_enforces_autonomy,
                auth_state,
            )
        });
        ctx.subscribe_to_model(&onboarding_state, |me, _model, event, ctx| {
            // Re-render when slide selection changes.
            if !ctx.is_self_or_child_focused() {
                ctx.focus_self();
            }
            ctx.notify();

            match event {
                OnboardingStateEvent::Completed => {
                    me.handle_onboarding_completed(ctx);
                }
                OnboardingStateEvent::UpgradeRequested => {
                    ctx.emit(AgentOnboardingEvent::UpgradeRequested);
                }
                OnboardingStateEvent::AuthStateChanged => {
                    me.handle_auth_state_changed(ctx);
                }
                OnboardingStateEvent::ModelsUpdated
                | OnboardingStateEvent::SelectedSlideChanged
                | OnboardingStateEvent::IntentionChanged
                | OnboardingStateEvent::NoAiConfirmationChanged => {}
            }
        });

        let intro_slide = {
            let onboarding_state = onboarding_state.clone();
            ctx.add_typed_action_view(move |_| IntroSlide::new(onboarding_state))
        };

        ctx.subscribe_to_view(&intro_slide, |_me, _view, event, ctx| match event {
            IntroSlideEvent::LoginRequested => {
                ctx.emit(AgentOnboardingEvent::LoginFromWelcomeRequested);
            }
        });

        let theme_picker_slide = {
            let themes = theme_picker_themes.clone();
            let onboarding_state = onboarding_state.clone();
            ctx.add_typed_action_view(move |ctx| {
                ThemePickerSlide::new(themes.clone(), onboarding_state, ctx)
            })
        };

        let intention_slide = {
            let onboarding_state = onboarding_state.clone();
            ctx.add_typed_action_view(move |_| IntentionSlide::new(onboarding_state))
        };

        let ai_setup_slide = {
            let onboarding_state = onboarding_state.clone();
            ctx.add_typed_action_view(move |_| AiSetupSlide::new(onboarding_state))
        };

        let customize_slide = {
            let onboarding_state = onboarding_state.clone();
            ctx.add_typed_action_view(move |ctx| CustomizeUISlide::new(onboarding_state, ctx))
        };

        ctx.subscribe_to_view(&theme_picker_slide, |me, _view, event, ctx| {
            me.handle_theme_picker_slide_event(event, ctx);
        });

        let agent_slide = {
            let onboarding_state = onboarding_state.clone();
            ctx.add_typed_action_view(move |ctx| AgentSlide::new(onboarding_state, ctx))
        };

        let ai_access_slide = {
            let onboarding_state = onboarding_state.clone();
            ctx.add_typed_action_view(move |_| AiAccessSlide::new(onboarding_state))
        };

        ctx.subscribe_to_view(&ai_access_slide, |_me, _view, event, ctx| match event {
            AiAccessSlideEvent::AddApiKeyRequested => {
                ctx.emit(AgentOnboardingEvent::AddApiKeyRequested);
            }
            AiAccessSlideEvent::AddCustomEndpointRequested => {
                ctx.emit(AgentOnboardingEvent::AddCustomEndpointRequested);
            }
            AiAccessSlideEvent::CopyUpgradeUrlRequested => {
                ctx.emit(AgentOnboardingEvent::UpgradeCopyUrlRequested);
            }
            AiAccessSlideEvent::PasteAuthTokenFromClipboardRequested => {
                ctx.emit(AgentOnboardingEvent::UpgradePasteTokenFromClipboardRequested);
            }
        });

        let third_party_slide = {
            let onboarding_state = onboarding_state.clone();
            ctx.add_typed_action_view(move |ctx| ThirdPartySlide::new(onboarding_state, ctx))
        };

        // When the app regains focus (e.g. user returning from the upgrade page in the
        // browser), notify the parent to refresh models and workspace/billing metadata.
        // Debounced to avoid excessive API calls from rapid alt-tabbing.
        ctx.subscribe_to_model(&WindowManager::handle(ctx), |me, _wm, event, ctx| {
            let StateEvent::ValueChanged { current, previous } = event;
            if previous.stage != ApplicationStage::Active
                && current.stage == ApplicationStage::Active
            {
                let now = Instant::now();
                let should_refresh = me
                    .last_model_refresh
                    .is_none_or(|last| now.duration_since(last) >= APP_BECAME_ACTIVE_DEBOUNCE);
                if should_refresh {
                    me.last_model_refresh = Some(now);
                    ctx.emit(AgentOnboardingEvent::AppBecameActive);
                }
            }
        });

        Self {
            onboarding_state,
            intro_slide,
            theme_picker_slide,
            intention_slide,
            ai_setup_slide,
            customize_slide,
            agent_slide,
            ai_access_slide,
            third_party_slide,
            skippable,
            close_button: button::Button::default(),
            no_ai_confirm_button: button::Button::default(),
            no_ai_cancel_button: button::Button::default(),
            no_ai_close_button: button::Button::default(),
            last_model_refresh: None,
            show_plan_activated_toast: false,
            last_auth_state: auth_state,
            plan_activated_close_mouse_state: MouseStateHandle::default(),
        }
    }

    /// Updates the list of available models.
    pub fn set_onboarding_models(
        &mut self,
        models: Vec<OnboardingModelInfo>,
        default_model_id: LLMId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.onboarding_state.update(ctx, |state, ctx| {
            state.set_models(models, default_model_id, ctx);
        });
        ctx.notify();
    }

    pub fn set_workspace_enforces_autonomy(&mut self, value: bool, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |state, ctx| {
            state.set_workspace_enforces_autonomy(value, ctx);
        });
        ctx.notify();
    }

    pub fn set_auth_state(&mut self, auth_state: OnboardingAuthState, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |state, ctx| {
            state.set_auth_state(auth_state, ctx);
        });
        ctx.notify();
    }

    /// Updates how many BYOK provider keys and custom endpoints the user has
    /// configured. This drives the AI-access slide's "connected" status line and
    /// gates "Next" on the bring-your-own path.
    pub fn set_byok_status(
        &mut self,
        key_count: usize,
        endpoint_count: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        self.ai_access_slide.update(ctx, |slide, ctx| {
            slide.set_byok_status(key_count, endpoint_count, ctx);
        });
    }

    /// The current `use_vertical_tabs` value on the onboarding UI customization.
    /// This reflects the intention's default (agent = vertical, terminal = horizontal)
    /// and any change the user made on the customize slide, and is what the
    /// theme slide uses to pick its right-panel image.
    pub fn use_vertical_tabs(&self, ctx: &AppContext) -> bool {
        self.onboarding_state
            .as_ref(ctx)
            .ui_customization()
            .use_vertical_tabs
    }

    pub fn start_onboarding(&self, ctx: &mut ViewContext<Self>) {
        // Focus the onboarding view so key bindings (Enter, arrow keys, etc.) are routed here
        // instead of to other views (e.g. the editor).
        ctx.focus_self();

        // Preload customize-slide images so they're ready when the user reaches that slide.
        Self::preload_onboarding_images(ctx);

        send_telemetry_from_ctx!(OnboardingEvent::OnboardingStarted, ctx);
        send_telemetry_from_ctx!(
            OnboardingEvent::SlideViewed {
                slide_name: "intro".to_string(),
            },
            ctx
        );
    }

    /// Eagerly loads all onboarding slide images into the asset cache
    /// so they display instantly when the user navigates between slides.
    fn preload_onboarding_images(ctx: &mut ViewContext<Self>) {
        let asset_cache = warpui_core::assets::asset_cache::AssetCache::as_ref(ctx);
        // Preload the shared background image used on all right panels.
        asset_cache.load_asset::<ImageType>(AssetSource::Bundled {
            path: crate::slides::layout::ONBOARDING_BG_PATH,
        });
        for path in IntentionSlide::VISUAL_IMAGE_PATHS {
            asset_cache.load_asset::<ImageType>(AssetSource::Bundled { path });
        }
        for path in AiSetupSlide::VISUAL_IMAGE_PATHS {
            asset_cache.load_asset::<ImageType>(AssetSource::Bundled { path });
        }
        for path in AiAccessSlide::VISUAL_IMAGE_PATHS {
            asset_cache.load_asset::<ImageType>(AssetSource::Bundled { path });
        }
        for path in CustomizeUISlide::VISUAL_IMAGE_PATHS {
            asset_cache.load_asset::<ImageType>(AssetSource::Bundled { path });
        }
        for path in ThirdPartySlide::VISUAL_IMAGE_PATHS {
            asset_cache.load_asset::<ImageType>(AssetSource::Bundled { path });
        }
        for path in ThemePickerSlide::VISUAL_IMAGE_PATHS {
            asset_cache.load_asset::<ImageType>(AssetSource::Bundled { path });
        }
        // Agent slide reuses customize_vertical_tabs / customize_horizontal_tabs
        // which are already in CustomizeUISlide::VISUAL_IMAGE_PATHS.
    }

    fn render_no_ai_dialog(&self, appearance: &Appearance) -> Box<dyn Element> {
        let escape = Keystroke::parse("escape").unwrap_or_default();
        let close_button = self.no_ai_close_button.render(
            appearance,
            button::Params {
                content: button::Content::Icon(Icon::X),
                theme: &button::themes::Naked,
                options: button::Options {
                    keystroke: Some(escape),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(AgentOnboardingAction::NoAiDismiss);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let cancel_button = self.no_ai_cancel_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Give me AI features".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(AgentOnboardingAction::NoAiCancel);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let enter = Keystroke::parse("enter").unwrap_or_default();
        let confirm_button = self.no_ai_confirm_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("I don't want AI".into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    keystroke: Some(enter),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(AgentOnboardingAction::NoAiConfirm);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        render_feature_optout_dialog(
            appearance,
            FeatureOptOutDialog {
                title: "Are you sure you don't want AI?",
                body: "Warp is better with AI. By continuing, you won't have access to any of the \
                       following features:",
                features: AI_FEATURES,
                close_button,
                cancel_button,
                confirm_button,
            },
        )
    }

    fn handle_onboarding_completed(&mut self, ctx: &mut ViewContext<Self>) {
        let settings = self.onboarding_state.as_ref(ctx).settings();
        ctx.emit(AgentOnboardingEvent::OnboardingCompleted(settings));
    }

    /// Reacts to a billing/auth transition. When the user becomes a paying user
    /// we show a success toast; if they're still on the AI-access slide we also
    /// advance them, since selecting a plan was the remaining action there.
    fn handle_auth_state_changed(&mut self, ctx: &mut ViewContext<Self>) {
        let new_state = self.onboarding_state.as_ref(ctx).auth_state();
        let became_paying = new_state == OnboardingAuthState::PayingUser
            && self.last_auth_state != OnboardingAuthState::PayingUser;
        self.last_auth_state = new_state;
        if !became_paying {
            return;
        }

        let on_ai_access = self.onboarding_state.as_ref(ctx).step() == OnboardingStep::AiAccess;
        if on_ai_access {
            self.onboarding_state
                .update(ctx, |model, ctx| model.next(ctx));
        }

        self.show_plan_activated_toast = true;
        let _ = ctx.spawn(
            warpui_core::r#async::Timer::after(PLAN_ACTIVATED_TOAST_DURATION),
            |me: &mut Self, _, ctx| {
                if me.show_plan_activated_toast {
                    me.show_plan_activated_toast = false;
                    ctx.notify();
                }
            },
        );
    }

    /// Green success pill shown after billing succeeds. Hosted at the view level
    /// (not the slide) so it survives the auto-advance off the AI-access slide.
    fn render_plan_activated_toast(&self, appearance: &Appearance) -> Box<dyn Element> {
        const TOAST_MIN_HEIGHT: f32 = 40.;
        const ICON_SIZE: f32 = 14.;
        const CLOSE_SIZE: f32 = 16.;
        const FONT_SIZE: f32 = 12.;

        let theme = appearance.theme();
        let toast_bg: Fill = theme.ansi_fg_green().into();
        let text_color: ColorU = theme.font_color(toast_bg.into_solid()).into();
        let ui_builder = appearance.ui_builder();

        let check_icon = ConstrainedBox::new(Box::new(
            Icon::CheckSkinny.to_warpui_icon(Fill::Solid(text_color)),
        ))
        .with_width(ICON_SIZE)
        .with_height(ICON_SIZE)
        .finish();

        let text = ui_builder
            .span("Plan successfully activated!")
            .with_style(UiComponentStyles {
                font_color: Some(text_color),
                font_size: Some(FONT_SIZE),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            })
            .build()
            .finish();

        let close_button = ui_builder
            .close_button(CLOSE_SIZE, self.plan_activated_close_mouse_state.clone())
            .with_style(UiComponentStyles {
                font_color: Some(text_color),
                ..Default::default()
            })
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(AgentOnboardingAction::DismissPlanActivatedToast);
            })
            .finish();

        let left = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(check_icon)
            .with_child(Container::new(text).with_margin_left(8.).finish())
            .finish();

        let row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(left)
            .with_child(close_button)
            .finish();

        ConstrainedBox::new(
            Container::new(row)
                .with_background(toast_bg)
                .with_horizontal_padding(16.)
                .finish(),
        )
        .with_min_height(TOAST_MIN_HEIGHT)
        .finish()
    }

    fn handle_theme_picker_slide_event(
        &mut self,
        event: &ThemePickerSlideEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ThemePickerSlideEvent::ThemeSelected { theme_name } => {
                ctx.emit(AgentOnboardingEvent::ThemeSelected {
                    theme_name: theme_name.clone(),
                });
            }
            ThemePickerSlideEvent::SyncWithOsToggled { enabled } => {
                ctx.emit(AgentOnboardingEvent::SyncWithOsToggled { enabled: *enabled });
            }
            ThemePickerSlideEvent::PrivacySettingsRequested => {
                ctx.emit(AgentOnboardingEvent::PrivacySettingsFromTerminalThemeSlideRequested);
            }
        }
    }
}

impl Entity for AgentOnboardingView {
    type Event = AgentOnboardingEvent;
}

impl View for AgentOnboardingView {
    fn ui_name() -> &'static str {
        "AgentOnboardingView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut stack = Stack::new();

        if let Some(img) = theme.background_image() {
            // Render the image behind everything.
            stack.add_child(
                Shrinkable::new(
                    1.,
                    Image::new(img.source(), CacheOption::Original)
                        .cover()
                        .finish(),
                )
                .finish(),
            );

            // Overlay the theme background so the image shows through at img.opacity.
            let overlay_opacity = (100u8).saturating_sub(img.opacity);
            stack.add_child(
                Rect::new()
                    .with_background(theme.background().with_opacity(overlay_opacity))
                    .finish(),
            );
        } else {
            stack.add_child(
                Container::new(Empty::new().finish())
                    .with_background(theme.background())
                    .finish(),
            );
        }

        let selected_slide = self.onboarding_state.as_ref(app).step();
        let slide = match selected_slide {
            OnboardingStep::Intro => ChildView::new(&self.intro_slide).finish(),
            OnboardingStep::ThemePicker => ChildView::new(&self.theme_picker_slide).finish(),
            OnboardingStep::Intention => ChildView::new(&self.intention_slide).finish(),
            OnboardingStep::AiSetup => ChildView::new(&self.ai_setup_slide).finish(),
            OnboardingStep::Customize => ChildView::new(&self.customize_slide).finish(),
            OnboardingStep::Agent => ChildView::new(&self.agent_slide).finish(),
            OnboardingStep::AiAccess => ChildView::new(&self.ai_access_slide).finish(),
            OnboardingStep::ThirdParty => ChildView::new(&self.third_party_slide).finish(),
        };

        stack.add_child(slide);

        if self.skippable {
            let esc = Keystroke::parse("escape").unwrap_or_default();

            let close_button = self.close_button.render(
                appearance,
                button::Params {
                    content: button::Content::Label("Skip".into()),
                    theme: &button::themes::Naked,
                    options: button::Options {
                        size: button::Size::Small,
                        keystroke: Some(esc),
                        on_click: Some(Box::new(|ctx, _app, _pos| {
                            ctx.dispatch_typed_action(AgentOnboardingAction::Escape);
                        })),
                        ..button::Options::default(appearance)
                    },
                },
            );

            stack.add_positioned_child(
                close_button,
                OffsetPositioning::offset_from_parent(
                    vec2f(-24., 24.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopRight,
                    ChildAnchor::TopRight,
                ),
            );
        }

        if self
            .onboarding_state
            .as_ref(app)
            .no_ai_confirmation()
            .is_some()
        {
            let dialog = self.render_no_ai_dialog(appearance);
            stack.add_child(
                Rect::new()
                    .with_background(Fill::Solid(ColorU::black()).with_opacity(60))
                    .finish(),
            );
            stack.add_child(
                Dismiss::new(Align::new(dialog).finish())
                    .on_dismiss(|ctx, _app| {
                        ctx.dispatch_typed_action(AgentOnboardingAction::NoAiDismiss);
                    })
                    .finish(),
            );
        }

        if self.show_plan_activated_toast {
            stack.add_child(
                Align::new(self.render_plan_activated_toast(appearance))
                    .bottom_center()
                    .finish(),
            );
        }

        stack.finish()
    }
}

impl TypedActionView for AgentOnboardingView {
    type Action = AgentOnboardingAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        if self
            .onboarding_state
            .as_ref(ctx)
            .no_ai_confirmation()
            .is_some()
        {
            match action {
                AgentOnboardingAction::NoAiConfirm | AgentOnboardingAction::EnterKey => {
                    self.onboarding_state
                        .update(ctx, |model, ctx| model.confirm_no_ai(ctx));
                }
                AgentOnboardingAction::NoAiCancel => {
                    self.onboarding_state
                        .update(ctx, |model, ctx| model.cancel_no_ai(ctx));
                }
                AgentOnboardingAction::NoAiDismiss | AgentOnboardingAction::Escape => {
                    self.onboarding_state
                        .update(ctx, |model, ctx| model.dismiss_no_ai(ctx));
                }
                _ => {}
            }
            return;
        }

        if matches!(action, AgentOnboardingAction::Escape) && self.skippable {
            ctx.emit(AgentOnboardingEvent::OnboardingSkipped);
            return;
        }

        if matches!(action, AgentOnboardingAction::DismissPlanActivatedToast) {
            self.show_plan_activated_toast = false;
            ctx.notify();
            return;
        }

        let selected_slide = self.onboarding_state.as_ref(ctx).step();

        match selected_slide {
            OnboardingStep::Intro => self.intro_slide.update(ctx, |slide, ctx| {
                dispatch_onboarding_action_to_slide(slide, *action, ctx)
            }),
            OnboardingStep::ThemePicker => self.theme_picker_slide.update(ctx, |slide, ctx| {
                dispatch_onboarding_action_to_slide(slide, *action, ctx)
            }),
            OnboardingStep::Intention => self.intention_slide.update(ctx, |slide, ctx| {
                dispatch_onboarding_action_to_slide(slide, *action, ctx)
            }),
            OnboardingStep::AiSetup => self.ai_setup_slide.update(ctx, |slide, ctx| {
                dispatch_onboarding_action_to_slide(slide, *action, ctx)
            }),
            OnboardingStep::Customize => self.customize_slide.update(ctx, |slide, ctx| {
                dispatch_onboarding_action_to_slide(slide, *action, ctx)
            }),
            OnboardingStep::Agent => self.agent_slide.update(ctx, |slide, ctx| {
                dispatch_onboarding_action_to_slide(slide, *action, ctx)
            }),
            OnboardingStep::AiAccess => self.ai_access_slide.update(ctx, |slide, ctx| {
                dispatch_onboarding_action_to_slide(slide, *action, ctx)
            }),
            OnboardingStep::ThirdParty => self.third_party_slide.update(ctx, |slide, ctx| {
                dispatch_onboarding_action_to_slide(slide, *action, ctx)
            }),
        }
    }
}

pub fn init(app: &mut AppContext) {
    app.register_fixed_bindings([
        FixedBinding::new(
            "up",
            AgentOnboardingAction::UpKey,
            id!(AgentOnboardingView::ui_name()),
        ),
        FixedBinding::new(
            "down",
            AgentOnboardingAction::DownKey,
            id!(AgentOnboardingView::ui_name()),
        ),
        FixedBinding::new(
            "left",
            AgentOnboardingAction::LeftKey,
            id!(AgentOnboardingView::ui_name()),
        ),
        FixedBinding::new(
            "right",
            AgentOnboardingAction::RightKey,
            id!(AgentOnboardingView::ui_name()),
        ),
        FixedBinding::new(
            "tab",
            AgentOnboardingAction::TabKey,
            id!(AgentOnboardingView::ui_name()),
        ),
        FixedBinding::new(
            "enter",
            AgentOnboardingAction::EnterKey,
            id!(AgentOnboardingView::ui_name()),
        ),
        FixedBinding::new(
            "numpadenter",
            AgentOnboardingAction::EnterKey,
            id!(AgentOnboardingView::ui_name()),
        ),
        FixedBinding::new(
            "cmdorctrl-enter",
            AgentOnboardingAction::CmdOrCtrlEnterKey,
            id!(AgentOnboardingView::ui_name()),
        ),
        FixedBinding::new(
            "cmdorctrl-numpadenter",
            AgentOnboardingAction::CmdOrCtrlEnterKey,
            id!(AgentOnboardingView::ui_name()),
        ),
        FixedBinding::new(
            "escape",
            AgentOnboardingAction::Escape,
            id!(AgentOnboardingView::ui_name()),
        ),
    ]);
}
