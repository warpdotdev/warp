use std::time::Duration;

use ai::LLMId;
use instant::Instant;
use warp_core::features::FeatureFlag;
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
    AiSetupSlide, CustomizeUISlide, IntentionSlide, IntroSlide, IntroSlideEvent,
    OnboardingModelInfo, OnboardingSlide, ProjectSlide, ThemePickerSlide, ThemePickerSlideEvent,
    ThirdPartySlide,
};
use crate::telemetry::OnboardingEvent;
use crate::AI_FEATURES;

const APP_BECAME_ACTIVE_DEBOUNCE: Duration = Duration::from_secs(15);

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use ui_components::{button, Component as _, Options as _};
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::{Fill, WarpTheme};
use warp_core::ui::Icon;
use warpui_core::elements::{
    Align, CacheOption, ChildAnchor, Container, Dismiss, Empty, Image, OffsetPositioning,
    ParentAnchor, ParentElement, ParentOffsetBounds, Rect, Shrinkable, Stack,
};
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{FixedBinding, Keystroke};
use warpui_core::presenter::ChildView;
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
    third_party_slide: ViewHandle<ThirdPartySlide>,
    project_slide: ViewHandle<ProjectSlide>,
    skippable: bool,
    close_button: button::Button,
    no_ai_confirm_button: button::Button,
    no_ai_cancel_button: button::Button,
    no_ai_close_button: button::Button,
    last_model_refresh: Option<Instant>,
    last_auth_state: OnboardingAuthState,
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
        | AgentOnboardingAction::NoAiDismiss => {}
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
        agent_modality_enabled: bool,
        auth_state: OnboardingAuthState,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let onboarding_state = ctx.add_model(|_| {
            OnboardingStateModel::new(
                models,
                default_model_id,
                workspace_enforces_autonomy,
                agent_modality_enabled,
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

        let third_party_slide = {
            let onboarding_state = onboarding_state.clone();
            ctx.add_typed_action_view(move |ctx| ThirdPartySlide::new(onboarding_state, ctx))
        };

        let project_slide = {
            let onboarding_state = onboarding_state.clone();
            ctx.add_typed_action_view(move |_| ProjectSlide::new(onboarding_state))
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
            third_party_slide,
            project_slide,
            skippable,
            close_button: button::Button::default(),
            no_ai_confirm_button: button::Button::default(),
            no_ai_cancel_button: button::Button::default(),
            no_ai_close_button: button::Button::default(),
            last_model_refresh: None,
            last_auth_state: auth_state,
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

    /// Compatibility no-op for the removed built-in AI access slide.
    pub fn set_byok_status(
        &mut self,
        _key_count: usize,
        _endpoint_count: usize,
        _ctx: &mut ViewContext<Self>,
    ) {
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
        if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
            Self::preload_onboarding_images(ctx);
        }

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

    /// Reacts to a billing/auth transition.
    fn handle_auth_state_changed(&mut self, ctx: &mut ViewContext<Self>) {
        let new_state = self.onboarding_state.as_ref(ctx).auth_state();
        self.last_auth_state = new_state;
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
            OnboardingStep::ThirdParty => ChildView::new(&self.third_party_slide).finish(),
            OnboardingStep::Project => ChildView::new(&self.project_slide).finish(),
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
            OnboardingStep::ThirdParty => self.third_party_slide.update(ctx, |slide, ctx| {
                dispatch_onboarding_action_to_slide(slide, *action, ctx)
            }),
            OnboardingStep::Project => self.project_slide.update(ctx, |slide, ctx| {
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
