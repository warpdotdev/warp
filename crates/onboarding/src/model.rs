use ai::LLMId;
use warp_core::send_telemetry_from_ctx;
use warpui::{Entity, ModelContext};

use crate::slides::{
    AgentAutonomy, AgentDevelopmentSettings, OnboardingModelInfo, ProjectOnboardingSettings,
};
use crate::telemetry::OnboardingEvent;
use crate::OnboardingIntention;

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
        project_settings: ProjectOnboardingSettings,
        ui_customization: Option<UICustomizationSettings>,
    },
}

impl SelectedSettings {
    pub fn is_ai_enabled(&self) -> bool {
        use warp_core::features::FeatureFlag;
        match self {
            SelectedSettings::AgentDrivenDevelopment { agent_settings, .. } => {
                !agent_settings.disable_oz
            }
            SelectedSettings::Terminal { .. } => {
                // With old onboarding (no OpenWarpNewSettingsModes), Terminal
                // intent still leaves AI enabled; with new onboarding,
                // Terminal intent explicitly disables AI.
                !FeatureFlag::OpenWarpNewSettingsModes.is_enabled()
            }
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
    Customize,
    Agent,
    ThirdParty,
    Project,
    ThemePicker,
}

#[derive(Clone, Debug)]
pub(crate) enum OnboardingStateEvent {
    ModelsUpdated,
    SelectedSlideChanged,
    IntentionChanged,
    Completed,
    UpgradeRequested,
    AuthStateChanged,
}

#[derive(Clone, Debug)]
pub(crate) struct OnboardingStateModel {
    step: OnboardingStep,
    intention: OnboardingIntention,
    agent_settings: AgentDevelopmentSettings,
    project_settings: ProjectOnboardingSettings,
    ui_customization: UICustomizationSettings,
    models: Vec<OnboardingModelInfo>,
    /// Whether the workspace enforces autonomy settings, hiding the user selection UI.
    workspace_enforces_autonomy: bool,
    /// Whether the AgentView feature flag is enabled.
    agent_modality_enabled: bool,
    /// Whether the user is in the FreeUserNoAi experiment group (and is free tier).
    /// When true, the Agent Driven Development option on the intention slide is locked
    /// behind an upgrade CTA.
    free_user_no_ai_experiment: bool,
    /// Yearly price per month in USD cents for the agent plan badge.
    /// When `None`, falls back to a hardcoded default ($18/mo).
    agent_price_cents: Option<i32>,
    /// Auth / billing state of the user.
    auth_state: OnboardingAuthState,
}

impl OnboardingStateModel {
    /// Creates a new OnboardingStateModel.
    pub(crate) fn new(
        models: Vec<OnboardingModelInfo>,
        default_model_id: LLMId,
        workspace_enforces_autonomy: bool,
        agent_modality_enabled: bool,
        free_user_no_ai_experiment: bool,
        agent_price_cents: Option<i32>,
        auth_state: OnboardingAuthState,
    ) -> Self {
        Self {
            step: OnboardingStep::Intro,
            intention: OnboardingIntention::AgentDrivenDevelopment,
            agent_settings: AgentDevelopmentSettings::new(default_model_id),
            project_settings: ProjectOnboardingSettings::default(),
            ui_customization: UICustomizationSettings::agent_defaults(),
            models,
            workspace_enforces_autonomy,
            agent_modality_enabled,
            free_user_no_ai_experiment,
            agent_price_cents,
            auth_state,
        }
    }

    pub(crate) fn auth_state(&self) -> OnboardingAuthState {
        self.auth_state
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
        use warp_core::features::FeatureFlag;
        let ui_customization = if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
            Some(self.ui_customization.clone())
        } else {
            None
        };

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
                    project_settings: self.project_settings.clone(),
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

    pub(crate) fn project_settings(&self) -> &ProjectOnboardingSettings {
        &self.project_settings
    }

    pub(crate) fn workspace_enforces_autonomy(&self) -> bool {
        self.workspace_enforces_autonomy
    }

    pub(crate) fn agent_modality_enabled(&self) -> bool {
        self.agent_modality_enabled
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

    pub(crate) fn free_user_no_ai_experiment(&self) -> bool {
        self.free_user_no_ai_experiment
    }

    pub(crate) fn agent_price_badge(&self) -> String {
        const DEFAULT_AGENT_PRICE_CENTS: i32 = 1800;
        let cents = self.agent_price_cents.unwrap_or(DEFAULT_AGENT_PRICE_CENTS);
        format!("Starting at ${}/mo", cents / 100)
    }

    pub(crate) fn set_agent_price_cents(
        &mut self,
        cents: Option<i32>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.agent_price_cents == cents {
            return;
        }
        self.agent_price_cents = cents;
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

    pub(crate) fn set_free_user_no_ai_experiment(
        &mut self,
        value: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.free_user_no_ai_experiment == value {
            return;
        }
        self.free_user_no_ai_experiment = value;
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

    pub(crate) fn is_model_disabled(&self, model_id: &LLMId) -> bool {
        self.models
            .iter()
            .find(|m| &m.id == model_id)
            .is_some_and(|m| m.requires_upgrade)
    }

    pub(crate) fn request_upgrade(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.emit(OnboardingStateEvent::UpgradeRequested);
    }

    pub(crate) fn on_user_selected_model(&mut self, model_id: LLMId, ctx: &mut ModelContext<Self>) {
        if self.agent_settings.selected_model_id == model_id {
            return;
        }

        if self.is_model_disabled(&model_id) {
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
        // When the new settings modes flag is on, ThemePicker comes after the agent slides
        // so it must also be guarded.
        let is_past_agent_slide = if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
            matches!(
                self.step,
                OnboardingStep::ThirdParty | OnboardingStep::ThemePicker
            )
        } else {
            matches!(self.step, OnboardingStep::Project)
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

    pub(crate) fn set_project_selected_local_folder(
        &mut self,
        path: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        if path.is_some() {
            send_telemetry_from_ctx!(OnboardingEvent::FolderSelected, ctx);
        }
        self.project_settings = ProjectOnboardingSettings::from_path(path);
        ctx.notify();
    }

    pub(crate) fn toggle_project_initialize_projects_automatically(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) {
        if let ProjectOnboardingSettings::Project {
            initialize_projects_automatically,
            ..
        } = &mut self.project_settings
        {
            let new_value = !*initialize_projects_automatically;
            send_telemetry_from_ctx!(
                OnboardingEvent::SettingChanged {
                    setting: "initialize_project".to_string(),
                    value: new_value.to_string(),
                },
                ctx
            );
            *initialize_projects_automatically = new_value;
            ctx.notify();
        }
    }

    fn send_completion_telemetry(&self, ctx: &mut ModelContext<Self>) {
        let (intention, model, autonomy) = match &self.intention {
            OnboardingIntention::Terminal => (self.intention.to_string(), None, None),
            OnboardingIntention::AgentDrivenDevelopment => (
                self.intention.to_string(),
                Some(self.agent_settings.selected_model_id.to_string()),
                self.agent_settings.autonomy.map(|x| x.to_string()),
            ),
        };

        let has_project_path = matches!(
            self.project_settings,
            ProjectOnboardingSettings::Project { .. }
        );

        send_telemetry_from_ctx!(
            OnboardingEvent::OnboardingSlidesCompleted {
                intention,
                model,
                autonomy,
                has_project_path,
            },
            ctx
        );
    }

    pub(crate) fn complete(&mut self, ctx: &mut ModelContext<Self>) {
        self.send_completion_telemetry(ctx);
        ctx.emit(OnboardingStateEvent::Completed);
        ctx.notify();
    }

    pub(crate) fn back(&mut self, ctx: &mut ModelContext<Self>) {
        use warp_core::features::FeatureFlag;
        let theme_picker_last = FeatureFlag::OpenWarpNewSettingsModes.is_enabled();

        let prev = if theme_picker_last {
            match self.step {
                OnboardingStep::Intro => None,
                OnboardingStep::Intention => Some(OnboardingStep::Intro),
                OnboardingStep::Customize => Some(OnboardingStep::Intention),
                OnboardingStep::Agent => Some(OnboardingStep::Customize),
                OnboardingStep::ThirdParty => match self.intention {
                    OnboardingIntention::Terminal => Some(OnboardingStep::Customize),
                    OnboardingIntention::AgentDrivenDevelopment => Some(OnboardingStep::Agent),
                },
                OnboardingStep::Project => Some(OnboardingStep::ThirdParty),
                OnboardingStep::ThemePicker => Some(OnboardingStep::ThirdParty),
            }
        } else {
            match self.step {
                OnboardingStep::Intro => None,
                OnboardingStep::ThemePicker => Some(OnboardingStep::Intro),
                OnboardingStep::Intention => Some(OnboardingStep::ThemePicker),
                OnboardingStep::Customize => None,
                OnboardingStep::ThirdParty => None,
                OnboardingStep::Agent => Some(OnboardingStep::Intention),
                OnboardingStep::Project => Some(OnboardingStep::Agent),
            }
        };

        if let Some(prev) = prev {
            send_telemetry_from_ctx!(OnboardingEvent::SlideNavigatedBack, ctx);
            self.set_step(prev, ctx);
        }
    }

    pub(crate) fn next(&mut self, ctx: &mut ModelContext<Self>) {
        use warp_core::features::FeatureFlag;
        let theme_picker_last = FeatureFlag::OpenWarpNewSettingsModes.is_enabled();

        let is_last_step = if theme_picker_last {
            matches!(self.step, OnboardingStep::ThemePicker)
        } else {
            matches!(self.step, OnboardingStep::Project)
        };
        if !is_last_step {
            send_telemetry_from_ctx!(OnboardingEvent::SlideNavigatedNext, ctx);
        }

        if theme_picker_last {
            match self.step {
                OnboardingStep::Intro => self.set_step(OnboardingStep::Intention, ctx),
                OnboardingStep::Intention => self.set_step(OnboardingStep::Customize, ctx),
                OnboardingStep::Customize => match self.intention {
                    OnboardingIntention::Terminal => self.set_step(OnboardingStep::ThirdParty, ctx),
                    OnboardingIntention::AgentDrivenDevelopment => {
                        self.set_step(OnboardingStep::Agent, ctx)
                    }
                },
                OnboardingStep::Agent => self.set_step(OnboardingStep::ThirdParty, ctx),
                OnboardingStep::ThirdParty => self.set_step(OnboardingStep::ThemePicker, ctx),
                OnboardingStep::Project => self.set_step(OnboardingStep::ThemePicker, ctx),
                OnboardingStep::ThemePicker => {}
            }
        } else {
            match self.step {
                OnboardingStep::Intro => self.set_step(OnboardingStep::ThemePicker, ctx),
                OnboardingStep::ThemePicker => self.set_step(OnboardingStep::Intention, ctx),
                OnboardingStep::Intention => self.set_step(OnboardingStep::Agent, ctx),
                OnboardingStep::Customize => {}
                OnboardingStep::ThirdParty => {}
                OnboardingStep::Agent => self.set_step(OnboardingStep::Project, ctx),
                OnboardingStep::Project => {}
            }
        }
    }

    pub(crate) fn set_step(&mut self, step: OnboardingStep, ctx: &mut ModelContext<Self>) {
        if self.step == step {
            return;
        }

        self.step = step;

        match step {
            OnboardingStep::Intro => {
                send_telemetry_from_ctx!(
                    OnboardingEvent::SlideViewed {
                        slide_name: "intro".to_string(),
                    },
                    ctx
                );
            }
            OnboardingStep::ThemePicker => {
                send_telemetry_from_ctx!(
                    OnboardingEvent::SlideViewed {
                        slide_name: "theme_picker".to_string(),
                    },
                    ctx
                );
            }
            OnboardingStep::Intention => {
                send_telemetry_from_ctx!(
                    OnboardingEvent::SlideViewed {
                        slide_name: "intention".to_string(),
                    },
                    ctx
                );
            }
            OnboardingStep::Customize => {
                send_telemetry_from_ctx!(
                    OnboardingEvent::SlideViewed {
                        slide_name: "customize".to_string(),
                    },
                    ctx
                );
            }
            OnboardingStep::Agent => {
                send_telemetry_from_ctx!(
                    OnboardingEvent::SlideViewed {
                        slide_name: "agent".to_string(),
                    },
                    ctx
                );
            }
            OnboardingStep::ThirdParty => {
                send_telemetry_from_ctx!(
                    OnboardingEvent::SlideViewed {
                        slide_name: "third_party".to_string(),
                    },
                    ctx
                );
            }
            OnboardingStep::Project => {
                send_telemetry_from_ctx!(
                    OnboardingEvent::SlideViewed {
                        slide_name: "project".to_string(),
                    },
                    ctx
                );
            }
        }

        ctx.emit(OnboardingStateEvent::SelectedSlideChanged);
        ctx.notify();
    }
}

impl Entity for OnboardingStateModel {
    type Event = OnboardingStateEvent;
}
