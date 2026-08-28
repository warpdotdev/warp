use ai::LLMId;
use warp_core::send_telemetry_from_ctx;
use warpui_core::{Entity, ModelContext};

use crate::OnboardingIntention;
use crate::slides::{AgentAutonomy, AgentDevelopmentSettings, OfferVariant, OnboardingModelInfo};
use crate::telemetry::OnboardingEvent;

/// UI customization settings chosen during the "Customize your UI" onboarding slide.
#[derive(Clone, Debug)]
pub struct UICustomizationSettings {
    pub use_vertical_tabs: bool,
    pub show_conversation_history: bool,
    pub show_project_explorer: bool,
    pub show_global_search: bool,
    pub show_warp_drive: bool,
    pub show_code_review_button: bool,
}

impl UICustomizationSettings {
    /// Defaults for agent-first development (all features enabled).
    pub fn agent_defaults() -> Self {
        Self {
            use_vertical_tabs: true,
            show_conversation_history: true,
            show_project_explorer: true,
            show_global_search: true,
            show_warp_drive: true,
            show_code_review_button: true,
        }
    }

    /// Defaults for terminal mode (all features disabled).
    pub fn terminal_defaults() -> Self {
        Self {
            use_vertical_tabs: false,
            show_conversation_history: false,
            show_project_explorer: false,
            show_global_search: false,
            show_warp_drive: false,
            show_code_review_button: false,
        }
    }

    /// Returns true if any tools-panel sub-setting visible for the given
    /// intention is enabled. In terminal mode the conversation-history chip is
    /// hidden, so it does not count.
    pub fn tools_panel_enabled(&self, intention: &OnboardingIntention) -> bool {
        let conversation_visible = matches!(intention, OnboardingIntention::AgentDrivenDevelopment);
        (conversation_visible && self.show_conversation_history)
            || self.show_project_explorer
            || self.show_global_search
            || self.show_warp_drive
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnboardingAuthState {
    LoggedOut,
    FreeUser,
    PayingUser,
}

#[derive(Clone, Debug)]
pub enum SelectedSettings {
    Terminal {
        ui_customization: Option<UICustomizationSettings>,
        cli_agent_toolbar_enabled: bool,
        show_agent_notifications: bool,
    },
    AgentDrivenDevelopment {
        agent_settings: AgentDevelopmentSettings,
        ui_customization: Option<UICustomizationSettings>,
    },
}

impl SelectedSettings {
    pub fn is_ai_enabled(&self) -> bool {
        match self {
            // Agent-driven development always means "I want AI" (including the
            // bring-your-own-agents `disable_oz` path). This reflects intent and
            // is used to decide that an account/login is required; whether AI is
            // actually enabled is applied later based on whether the user has an
            // account (see `apply_onboarding_settings`).
            SelectedSettings::AgentDrivenDevelopment { .. } => true,
            SelectedSettings::Terminal { .. } => false,
        }
    }

    pub fn is_warp_drive_enabled(&self) -> bool {
        match self {
            SelectedSettings::AgentDrivenDevelopment {
                ui_customization, ..
            } => ui_customization
                .as_ref()
                .map(|ui| ui.show_warp_drive)
                .unwrap_or(true),
            SelectedSettings::Terminal {
                ui_customization, ..
            } => ui_customization
                .as_ref()
                .map(|ui| ui.show_warp_drive)
                .unwrap_or(false),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OnboardingStep {
    Intro,
    Intention,
    AiSetup,
    Customize,
    Agent,
    AiAccess,
    ThirdParty,
    ThemePicker,
    PostAuthOffer,
}

/// The AI setup selected on the "Choose your AI setup" slide.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AiSetupChoice {
    #[default]
    WarpAgent,
    ThirdParty,
}

impl std::fmt::Display for AiSetupChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiSetupChoice::WarpAgent => write!(f, "warp_agent"),
            AiSetupChoice::ThirdParty => write!(f, "third_party"),
        }
    }
}

/// The access method selected on the "Choose how to access AI" slide (Warp Agent path).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AiAccessChoice {
    #[default]
    Subscription,
    SetUpLater,
}

impl std::fmt::Display for AiAccessChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiAccessChoice::Subscription => write!(f, "subscription"),
            AiAccessChoice::SetUpLater => write!(f, "set_up_later"),
        }
    }
}

/// Which opt-out entry point opened the "Are you sure you don't want AI?" modal.
/// Determines where "Give me AI features" routes the user on cancel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoAiConfirmationSource {
    /// Triggered from the intention slide via "Just use the terminal" + Next.
    Intention,
}

#[derive(Clone, Debug)]
pub(crate) enum OnboardingStateEvent {
    ModelsUpdated,
    SelectedSlideChanged,
    IntentionChanged,
    Completed,
    UpgradeRequested,
    AuthStateChanged,
    NoAiConfirmationChanged,
    /// The user can now use AI, so onboarding may advance past the offer slide.
    AiSellOfferSatisfied,
}

#[derive(Clone, Debug)]
pub(crate) struct OnboardingStateModel {
    step: OnboardingStep,
    intention: OnboardingIntention,
    agent_settings: AgentDevelopmentSettings,
    ui_customization: UICustomizationSettings,
    models: Vec<OnboardingModelInfo>,
    /// Whether the workspace enforces autonomy settings, hiding the user selection UI.
    workspace_enforces_autonomy: bool,
    /// The AI setup selected on the "Choose your AI setup" slide.
    ai_setup_choice: AiSetupChoice,
    /// The access method selected on the "Choose how to access AI" slide.
    ai_access_choice: AiAccessChoice,
    /// Auth / billing state of the user.
    auth_state: OnboardingAuthState,
    /// Which account-first offer is currently presented after authentication.
    offer_variant: Option<OfferVariant>,
    /// When set, the "Are you sure you don't want AI?" confirmation modal is
    /// shown; the value records which entry point triggered it.
    no_ai_confirmation: Option<NoAiConfirmationSource>,
    pricing_promotion_message: Option<String>,
}

impl OnboardingStateModel {
    /// Creates a new OnboardingStateModel.
    pub(crate) fn new(
        models: Vec<OnboardingModelInfo>,
        default_model_id: LLMId,
        workspace_enforces_autonomy: bool,
        auth_state: OnboardingAuthState,
    ) -> Self {
        Self {
            step: OnboardingStep::Intro,
            intention: OnboardingIntention::AgentDrivenDevelopment,
            agent_settings: AgentDevelopmentSettings::new(default_model_id),
            ui_customization: UICustomizationSettings::agent_defaults(),
            models,
            workspace_enforces_autonomy,
            ai_setup_choice: AiSetupChoice::default(),
            ai_access_choice: AiAccessChoice::default(),
            auth_state,
            offer_variant: None,
            no_ai_confirmation: None,
            pricing_promotion_message: None,
        }
    }

    pub(crate) fn auth_state(&self) -> OnboardingAuthState {
        self.auth_state
    }

    pub(crate) fn offer_variant(&self) -> Option<OfferVariant> {
        self.offer_variant
    }

    pub(crate) fn show_post_auth_offer(
        &mut self,
        variant: OfferVariant,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.step == OnboardingStep::PostAuthOffer {
            return;
        }
        self.offer_variant = Some(variant);
        self.set_step(OnboardingStep::PostAuthOffer, ctx);
    }

    pub(crate) fn set_auth_state(
        &mut self,
        auth_state: OnboardingAuthState,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.auth_state == auth_state {
            return;
        }
        self.auth_state = auth_state;
        ctx.emit(OnboardingStateEvent::AuthStateChanged);
    }

    pub(crate) fn settings(&self) -> SelectedSettings {
        let ui_customization = Some(self.ui_customization.clone());

        match &self.intention {
            OnboardingIntention::Terminal => SelectedSettings::Terminal {
                ui_customization,
                cli_agent_toolbar_enabled: self.agent_settings.cli_agent_toolbar_enabled,
                show_agent_notifications: self.agent_settings.show_agent_notifications,
            },
            OnboardingIntention::AgentDrivenDevelopment => {
                SelectedSettings::AgentDrivenDevelopment {
                    agent_settings: AgentDevelopmentSettings {
                        selected_model_id: self.agent_settings.selected_model_id.clone(),
                        autonomy: if self.workspace_enforces_autonomy {
                            None
                        } else {
                            self.agent_settings.autonomy
                        },
                        cli_agent_toolbar_enabled: self.agent_settings.cli_agent_toolbar_enabled,
                        session_default: self.agent_settings.session_default,
                        disable_oz: self.agent_settings.disable_oz,
                        // Agent intention always has notifications enabled (no toggle shown).
                        show_agent_notifications: true,
                    },
                    ui_customization,
                }
            }
        }
    }

    pub(crate) fn step(&self) -> OnboardingStep {
        self.step
    }

    pub(crate) fn intention(&self) -> &OnboardingIntention {
        &self.intention
    }

    pub(crate) fn agent_settings(&self) -> &AgentDevelopmentSettings {
        &self.agent_settings
    }

    pub(crate) fn workspace_enforces_autonomy(&self) -> bool {
        self.workspace_enforces_autonomy
    }

    pub(crate) fn ai_setup_choice(&self) -> AiSetupChoice {
        self.ai_setup_choice
    }

    pub(crate) fn ai_access_choice(&self) -> AiAccessChoice {
        self.ai_access_choice
    }

    pub(crate) fn set_ai_setup_choice(
        &mut self,
        choice: AiSetupChoice,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.ai_setup_choice == choice {
            return;
        }
        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "ai_setup".to_string(),
                value: choice.to_string(),
            },
            ctx
        );
        self.ai_setup_choice = choice;
        self.agent_settings.disable_oz = matches!(choice, AiSetupChoice::ThirdParty);
        ctx.notify();
    }

    pub(crate) fn set_ai_access_choice(
        &mut self,
        choice: AiAccessChoice,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.ai_access_choice == choice {
            return;
        }
        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "ai_access".to_string(),
                value: choice.to_string(),
            },
            ctx
        );
        self.ai_access_choice = choice;
        ctx.notify();
    }

    pub(crate) fn pricing_promotion_message(&self) -> Option<&str> {
        self.pricing_promotion_message.as_deref()
    }

    pub(crate) fn set_pricing_promotion_message(
        &mut self,
        message: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.pricing_promotion_message == message {
            return;
        }
        self.pricing_promotion_message = message;
        ctx.notify();
    }

    /// Reports whether the user can make an AI request. The AI-sell offer
    /// exists to get the user AI usage, so observing that they now have it is
    /// the whole completion condition — a plan or one-time credits, bought
    /// through any call to action.
    pub(crate) fn on_credit_availability_observed(
        &mut self,
        available: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if !available || !self.is_showing_ai_sell_offer() {
            return;
        }
        self.finish_ai_sell_offer(ctx);
    }

    /// A web checkout reported success through the desktop hand-off. The grant
    /// can lag the redirect, so the hand-off itself is trusted rather than
    /// waiting for an availability read. Returns whether an AI-sell offer
    /// consumed the signal.
    pub(crate) fn on_checkout_succeeded(&mut self, ctx: &mut ModelContext<Self>) -> bool {
        if !self.is_showing_ai_sell_offer() {
            return false;
        }
        self.finish_ai_sell_offer(ctx);
        true
    }

    /// Whether an onboarding screen whose purpose is to sell AI usage is on
    /// screen. The head-start offer is excluded: it ships with AI usage already
    /// on the account, so availability there says nothing about whether the
    /// user has made their choice yet.
    fn is_showing_ai_sell_offer(&self) -> bool {
        self.step == OnboardingStep::PostAuthOffer
            && self.offer_variant.is_some_and(OfferVariant::sells_ai_usage)
    }

    /// Reports that the user can now use AI, so onboarding moves past the offer.
    fn finish_ai_sell_offer(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.emit(OnboardingStateEvent::AiSellOfferSatisfied);
        ctx.notify();
    }

    pub(crate) fn no_ai_confirmation(&self) -> Option<NoAiConfirmationSource> {
        self.no_ai_confirmation
    }

    /// Shows the "Are you sure you don't want AI?" confirmation modal, recording
    /// which opt-out entry point triggered it so cancel can route appropriately.
    pub(crate) fn request_no_ai_confirmation(
        &mut self,
        source: NoAiConfirmationSource,
        ctx: &mut ModelContext<Self>,
    ) {
        send_telemetry_from_ctx!(OnboardingEvent::NoAiConfirmationShown, ctx);
        self.no_ai_confirmation = Some(source);
        ctx.emit(OnboardingStateEvent::NoAiConfirmationChanged);
        ctx.notify();
    }

    /// "I don't want AI": commit to the terminal-only path (AI features off) and
    /// continue the flow there, so declining AI never dead-ends onboarding.
    pub(crate) fn confirm_no_ai(&mut self, ctx: &mut ModelContext<Self>) {
        send_telemetry_from_ctx!(OnboardingEvent::NoAiConfirmed, ctx);
        self.no_ai_confirmation = None;
        self.set_intention(OnboardingIntention::Terminal, ctx);
        self.set_step(OnboardingStep::Customize, ctx);
    }

    /// "Give me AI features": abort the opt-out. The only trigger is the
    /// intention slide's "Just use the terminal", which is an explicit request
    /// for AI, so route onto the AI path.
    pub(crate) fn cancel_no_ai(&mut self, ctx: &mut ModelContext<Self>) {
        send_telemetry_from_ctx!(OnboardingEvent::NoAiConfirmationCancelled, ctx);
        match self.no_ai_confirmation.take() {
            Some(NoAiConfirmationSource::Intention) => {
                self.set_intention(OnboardingIntention::AgentDrivenDevelopment, ctx);
                self.set_step(OnboardingStep::AiSetup, ctx);
            }
            None => {
                ctx.emit(OnboardingStateEvent::NoAiConfirmationChanged);
                ctx.notify();
            }
        }
    }

    /// Closes the confirmation modal without changing the user's path (ESC / X).
    pub(crate) fn dismiss_no_ai(&mut self, ctx: &mut ModelContext<Self>) {
        if self.no_ai_confirmation.take().is_some() {
            ctx.emit(OnboardingStateEvent::NoAiConfirmationChanged);
            ctx.notify();
        }
    }

    pub fn ui_customization(&self) -> &UICustomizationSettings {
        &self.ui_customization
    }

    pub(crate) fn set_use_vertical_tabs(&mut self, value: bool, ctx: &mut ModelContext<Self>) {
        if self.ui_customization.use_vertical_tabs == value {
            return;
        }
        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "tab_styling".to_string(),
                value: if value { "vertical" } else { "horizontal" }.to_string(),
            },
            ctx
        );
        self.ui_customization.use_vertical_tabs = value;
        ctx.notify();
    }

    pub(crate) fn set_tools_panel_enabled(&mut self, enabled: bool, ctx: &mut ModelContext<Self>) {
        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "tools_panel".to_string(),
                value: if enabled { "enabled" } else { "disabled" }.to_string(),
            },
            ctx
        );
        self.ui_customization.show_conversation_history = enabled;
        self.ui_customization.show_project_explorer = enabled;
        self.ui_customization.show_global_search = enabled;
        self.ui_customization.show_warp_drive = enabled;
        ctx.notify();
    }

    pub(crate) fn set_show_conversation_history(
        &mut self,
        value: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.ui_customization.show_conversation_history == value {
            return;
        }
        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "conversation_history".to_string(),
                value: value.to_string(),
            },
            ctx
        );
        self.ui_customization.show_conversation_history = value;
        ctx.notify();
    }

    pub(crate) fn set_show_project_explorer(&mut self, value: bool, ctx: &mut ModelContext<Self>) {
        if self.ui_customization.show_project_explorer == value {
            return;
        }
        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "project_explorer".to_string(),
                value: value.to_string(),
            },
            ctx
        );
        self.ui_customization.show_project_explorer = value;
        ctx.notify();
    }

    pub(crate) fn set_show_global_search(&mut self, value: bool, ctx: &mut ModelContext<Self>) {
        if self.ui_customization.show_global_search == value {
            return;
        }
        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "global_search".to_string(),
                value: value.to_string(),
            },
            ctx
        );
        self.ui_customization.show_global_search = value;
        ctx.notify();
    }

    pub(crate) fn set_show_warp_drive(&mut self, value: bool, ctx: &mut ModelContext<Self>) {
        if self.ui_customization.show_warp_drive == value {
            return;
        }
        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "warp_drive".to_string(),
                value: value.to_string(),
            },
            ctx
        );
        self.ui_customization.show_warp_drive = value;
        ctx.notify();
    }

    pub(crate) fn set_cli_agent_toolbar_enabled(
        &mut self,
        value: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.agent_settings.cli_agent_toolbar_enabled == value {
            return;
        }
        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "cli_agent_toolbar".to_string(),
                value: if value { "enabled" } else { "disabled" }.to_string(),
            },
            ctx
        );
        self.agent_settings.cli_agent_toolbar_enabled = value;
        ctx.notify();
    }

    pub(crate) fn set_show_agent_notifications(
        &mut self,
        value: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.agent_settings.show_agent_notifications == value {
            return;
        }
        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "show_agent_notifications".to_string(),
                value: if value { "enabled" } else { "disabled" }.to_string(),
            },
            ctx
        );
        self.agent_settings.show_agent_notifications = value;
        ctx.notify();
    }

    pub(crate) fn set_show_code_review_button(
        &mut self,
        value: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.ui_customization.show_code_review_button == value {
            return;
        }
        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "code_review".to_string(),
                value: if value { "enabled" } else { "disabled" }.to_string(),
            },
            ctx
        );
        self.ui_customization.show_code_review_button = value;
        ctx.notify();
    }

    pub(crate) fn set_disable_oz(&mut self, value: bool, ctx: &mut ModelContext<Self>) {
        if self.agent_settings.disable_oz == value {
            return;
        }
        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "disable_oz".to_string(),
                value: value.to_string(),
            },
            ctx
        );
        self.agent_settings.disable_oz = value;
        ctx.notify();
    }

    pub(crate) fn set_workspace_enforces_autonomy(
        &mut self,
        value: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.workspace_enforces_autonomy == value {
            return;
        }
        self.workspace_enforces_autonomy = value;
        ctx.notify();
    }

    pub(crate) fn models(&self) -> &Vec<OnboardingModelInfo> {
        &self.models
    }

    fn set_intention(&mut self, intention: OnboardingIntention, ctx: &mut ModelContext<Self>) {
        if self.intention == intention {
            return;
        }

        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "intention".to_string(),
                value: intention.to_string(),
            },
            ctx
        );

        self.intention = intention;
        // Reset UI customization to defaults for the new intention.
        self.ui_customization = match intention {
            OnboardingIntention::AgentDrivenDevelopment => {
                UICustomizationSettings::agent_defaults()
            }
            OnboardingIntention::Terminal => UICustomizationSettings::terminal_defaults(),
        };
        // Reset notifications default based on intention.
        self.agent_settings.show_agent_notifications =
            matches!(intention, OnboardingIntention::AgentDrivenDevelopment);
        ctx.emit(OnboardingStateEvent::IntentionChanged);
        ctx.notify();
    }

    pub(crate) fn set_intention_terminal(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_intention(OnboardingIntention::Terminal, ctx);
    }

    pub(crate) fn set_intention_agent_driven_development(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_intention(OnboardingIntention::AgentDrivenDevelopment, ctx);
    }

    pub(crate) fn request_upgrade(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.emit(OnboardingStateEvent::UpgradeRequested);
    }

    pub(crate) fn on_user_selected_model(&mut self, model_id: LLMId, ctx: &mut ModelContext<Self>) {
        if self.agent_settings.selected_model_id == model_id {
            return;
        }

        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "model".to_string(),
                value: model_id.to_string(),
            },
            ctx
        );

        self.agent_settings.selected_model_id = model_id;
        ctx.notify();
    }

    /// Updates the list of available models.
    pub(crate) fn set_models(
        &mut self,
        models: Vec<OnboardingModelInfo>,
        default_model_id: LLMId,
        ctx: &mut ModelContext<Self>,
    ) {
        use warp_core::features::FeatureFlag;

        // If the user is past the agent slide, don't change the agent model from underneath them.
        // ThemePicker comes after the agent slides, so it must also be guarded.
        let is_past_agent_slide = if FeatureFlag::AccountFirstOnboarding.is_enabled() {
            matches!(
                self.step,
                OnboardingStep::Customize
                    | OnboardingStep::ThemePicker
                    | OnboardingStep::PostAuthOffer
            )
        } else {
            matches!(
                self.step,
                OnboardingStep::ThirdParty
                    | OnboardingStep::ThemePicker
                    | OnboardingStep::PostAuthOffer
            )
        };
        if is_past_agent_slide {
            return;
        }

        self.agent_settings.selected_model_id = default_model_id.clone();

        self.models = models;
        ctx.emit(OnboardingStateEvent::ModelsUpdated);
        ctx.notify();
    }

    pub(crate) fn set_agent_autonomy(
        &mut self,
        autonomy: AgentAutonomy,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.workspace_enforces_autonomy || self.agent_settings.autonomy == Some(autonomy) {
            return;
        }

        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "autonomy".to_string(),
                value: autonomy.to_string(),
            },
            ctx
        );

        self.agent_settings.autonomy = Some(autonomy);
        ctx.notify();
    }

    fn send_completion_telemetry(&self, ctx: &mut ModelContext<Self>) {
        if warp_core::features::FeatureFlag::AccountFirstOnboarding.is_enabled() {
            send_telemetry_from_ctx!(
                OnboardingEvent::OnboardingSlidesCompleted {
                    intention: "account_first".to_string(),
                    model: None,
                    autonomy: None,
                    has_project_path: false,
                    ai_access: None,
                },
                ctx
            );
            return;
        }
        let (intention, model, autonomy, ai_access) = match &self.intention {
            OnboardingIntention::Terminal => (self.intention.to_string(), None, None, None),
            OnboardingIntention::AgentDrivenDevelopment => (
                self.intention.to_string(),
                Some(self.agent_settings.selected_model_id.to_string()),
                self.agent_settings.autonomy.map(|x| x.to_string()),
                Some(self.ai_setup_choice.to_string()),
            ),
        };

        send_telemetry_from_ctx!(
            OnboardingEvent::OnboardingSlidesCompleted {
                intention,
                model,
                autonomy,
                has_project_path: false,
                ai_access,
            },
            ctx
        );
    }

    pub(crate) fn complete(&mut self, ctx: &mut ModelContext<Self>) {
        if warp_core::features::FeatureFlag::AccountFirstOnboarding.is_enabled() {
            self.send_account_first_action("next", ctx);
        }
        self.send_completion_telemetry(ctx);
        ctx.emit(OnboardingStateEvent::Completed);
        ctx.notify();
    }

    pub(crate) fn back(&mut self, ctx: &mut ModelContext<Self>) {
        use warp_core::features::FeatureFlag;
        let account_first = FeatureFlag::AccountFirstOnboarding.is_enabled();
        let agent_intention = matches!(self.intention, OnboardingIntention::AgentDrivenDevelopment);

        let prev = if account_first {
            match self.step {
                OnboardingStep::Intro => None,
                OnboardingStep::Customize => Some(OnboardingStep::Intro),
                OnboardingStep::ThemePicker => Some(OnboardingStep::Customize),
                OnboardingStep::PostAuthOffer => Some(OnboardingStep::ThemePicker),
                OnboardingStep::Intention
                | OnboardingStep::AiSetup
                | OnboardingStep::Agent
                | OnboardingStep::AiAccess
                | OnboardingStep::ThirdParty => Some(OnboardingStep::Intro),
            }
        } else {
            match self.step {
                OnboardingStep::Intro => None,
                OnboardingStep::Intention => Some(OnboardingStep::Intro),
                OnboardingStep::AiSetup => Some(OnboardingStep::Intention),
                OnboardingStep::Customize => {
                    if agent_intention {
                        match self.ai_setup_choice {
                            AiSetupChoice::WarpAgent => Some(OnboardingStep::AiAccess),
                            AiSetupChoice::ThirdParty => Some(OnboardingStep::ThirdParty),
                        }
                    } else {
                        Some(OnboardingStep::Intention)
                    }
                }
                OnboardingStep::AiAccess => Some(OnboardingStep::Agent),
                OnboardingStep::Agent => Some(OnboardingStep::AiSetup),
                OnboardingStep::ThirdParty => Some(OnboardingStep::AiSetup),
                OnboardingStep::ThemePicker => Some(OnboardingStep::Customize),
                OnboardingStep::PostAuthOffer => None,
            }
        };

        if let Some(prev) = prev {
            if account_first {
                self.send_account_first_action("back", ctx);
            }
            send_telemetry_from_ctx!(OnboardingEvent::SlideNavigatedBack, ctx);
            self.set_step(prev, ctx);
        }
    }

    pub(crate) fn next(&mut self, ctx: &mut ModelContext<Self>) {
        use warp_core::features::FeatureFlag;
        let account_first = FeatureFlag::AccountFirstOnboarding.is_enabled();
        let is_last_step = matches!(
            self.step,
            OnboardingStep::ThemePicker | OnboardingStep::PostAuthOffer
        );
        if !is_last_step {
            send_telemetry_from_ctx!(OnboardingEvent::SlideNavigatedNext, ctx);
        }

        if account_first {
            if !matches!(
                self.step,
                OnboardingStep::Intro | OnboardingStep::PostAuthOffer
            ) {
                self.send_account_first_action("next", ctx);
            }
            match self.step {
                OnboardingStep::Intro => self.set_step(OnboardingStep::Customize, ctx),
                OnboardingStep::Customize => self.set_step(OnboardingStep::ThemePicker, ctx),
                OnboardingStep::ThemePicker => {}
                OnboardingStep::PostAuthOffer => {}
                OnboardingStep::Intention
                | OnboardingStep::AiSetup
                | OnboardingStep::Agent
                | OnboardingStep::AiAccess
                | OnboardingStep::ThirdParty => self.set_step(OnboardingStep::Intro, ctx),
            }
        } else {
            match self.step {
                OnboardingStep::Intro => self.set_step(OnboardingStep::Intention, ctx),
                OnboardingStep::Intention => match self.intention {
                    OnboardingIntention::Terminal => self.set_step(OnboardingStep::Customize, ctx),
                    OnboardingIntention::AgentDrivenDevelopment => {
                        self.set_step(OnboardingStep::AiSetup, ctx)
                    }
                },
                OnboardingStep::AiSetup => match self.ai_setup_choice {
                    AiSetupChoice::WarpAgent => self.set_step(OnboardingStep::Agent, ctx),
                    AiSetupChoice::ThirdParty => self.set_step(OnboardingStep::ThirdParty, ctx),
                },
                OnboardingStep::Customize => self.set_step(OnboardingStep::ThemePicker, ctx),
                OnboardingStep::Agent => self.set_step(OnboardingStep::AiAccess, ctx),
                OnboardingStep::AiAccess => self.set_step(OnboardingStep::Customize, ctx),
                OnboardingStep::ThirdParty => {
                    if matches!(self.intention, OnboardingIntention::AgentDrivenDevelopment) {
                        self.set_step(OnboardingStep::Customize, ctx)
                    } else {
                        self.set_step(OnboardingStep::ThemePicker, ctx)
                    }
                }
                OnboardingStep::ThemePicker => {}
                OnboardingStep::PostAuthOffer => {}
            }
        }
    }

    pub(crate) fn set_step(&mut self, step: OnboardingStep, ctx: &mut ModelContext<Self>) {
        if self.step == step {
            return;
        }

        self.step = step;

        let account_first = warp_core::features::FeatureFlag::AccountFirstOnboarding.is_enabled();
        let slide_name = match step {
            OnboardingStep::Intro => {
                if account_first {
                    "welcome"
                } else {
                    "intro"
                }
            }
            OnboardingStep::PostAuthOffer => self
                .offer_variant
                .expect("offer variant is selected before entering the post-auth offer")
                .slide_name(),
            OnboardingStep::ThemePicker => "theme_picker",
            OnboardingStep::Intention => "intention",
            OnboardingStep::AiSetup => "ai_setup",
            OnboardingStep::AiAccess => "ai_access",
            OnboardingStep::Customize => "customize",
            OnboardingStep::Agent => "agent",
            OnboardingStep::ThirdParty => "third_party",
        };
        send_telemetry_from_ctx!(
            OnboardingEvent::SlideViewed {
                slide_name: slide_name.to_string(),
            },
            ctx
        );

        ctx.emit(OnboardingStateEvent::SelectedSlideChanged);
        ctx.notify();
    }

    /// The `(step_index, step_count)` shown by the bottom-nav progress dots for the
    /// current step, intention, and flow variant.
    pub(crate) fn progress(&self) -> (usize, usize) {
        use warp_core::features::FeatureFlag;
        if FeatureFlag::AccountFirstOnboarding.is_enabled() {
            return match self.step {
                OnboardingStep::Intro
                | OnboardingStep::Intention
                | OnboardingStep::AiSetup
                | OnboardingStep::Agent
                | OnboardingStep::AiAccess
                | OnboardingStep::ThirdParty => (0, 3),
                OnboardingStep::Customize => (0, 3),
                OnboardingStep::ThemePicker => (1, 3),
                OnboardingStep::PostAuthOffer => (0, 0),
            };
        }

        let is_terminal = matches!(self.intention, OnboardingIntention::Terminal);

        // The Warp Agent path has the extra "Choose how to access AI" step, so it
        // is one longer than the third-party-agent path.
        let is_warp_agent_path =
            !is_terminal && matches!(self.ai_setup_choice, AiSetupChoice::WarpAgent);
        let step_count = if is_terminal {
            3
        } else if is_warp_agent_path {
            6
        } else {
            5
        };
        let step_index = match self.step {
            OnboardingStep::Intro | OnboardingStep::Intention => 0,
            OnboardingStep::AiSetup => 1,
            OnboardingStep::Agent => 2,
            OnboardingStep::AiAccess => 3,
            OnboardingStep::Customize => {
                if is_terminal {
                    1
                } else if is_warp_agent_path {
                    4
                } else {
                    3
                }
            }
            OnboardingStep::ThirdParty => 2,
            OnboardingStep::ThemePicker => step_count - 1,
            OnboardingStep::PostAuthOffer => 0,
        };
        (step_index, step_count)
    }

    fn send_account_first_action(&self, action: &str, ctx: &mut ModelContext<Self>) {
        let slide_name = match self.step {
            OnboardingStep::Intro => "welcome",
            OnboardingStep::Customize => "customize",
            OnboardingStep::ThemePicker => "theme_picker",
            OnboardingStep::Intention => "intention",
            OnboardingStep::AiSetup => "ai_setup",
            OnboardingStep::Agent => "agent",
            OnboardingStep::AiAccess => "ai_access",
            OnboardingStep::ThirdParty => "third_party",
            OnboardingStep::PostAuthOffer => self
                .offer_variant
                .expect("offer variant is selected before entering the post-auth offer")
                .slide_name(),
        };
        send_telemetry_from_ctx!(
            OnboardingEvent::OnboardingAction {
                slide_name: slide_name.to_string(),
                action: action.to_string(),
                account_class: None,
            },
            ctx
        );
    }
}

impl Entity for OnboardingStateModel {
    type Event = OnboardingStateEvent;
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
