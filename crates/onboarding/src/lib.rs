// Onboarding library crate

/// Looks up a translation key using the user's chosen locale.
/// Falls back to the English (en) translation, then to the provided fallback string.
pub fn menu_label(key: &str, fallback: &str) -> &'static str {
    match i18n::lookup(key, i18n::current_locale()) {
        i18n::TranslationLookup::Found(v) => Box::leak(v.into_owned().into_boxed_str()),
        i18n::TranslationLookup::Missing => match i18n::lookup(key, "en") {
            i18n::TranslationLookup::Found(v) => Box::leak(v.into_owned().into_boxed_str()),
            i18n::TranslationLookup::Missing => Box::leak(fallback.to_string().into_boxed_str()),
        },
    }
}

mod agent_onboarding_view;
pub mod callout;
mod model;
pub mod slides;
pub mod telemetry;

/// The user's intention selected during onboarding slides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnboardingIntention {
    Terminal,
    AgentDrivenDevelopment,
}

impl std::fmt::Display for OnboardingIntention {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OnboardingIntention::AgentDrivenDevelopment => write!(f, "agent_driven"),
            OnboardingIntention::Terminal => write!(f, "terminal"),
        }
    }
}

pub use callout::{OnboardingCalloutView, OnboardingKeybindings};

/// User-facing descriptions of the AI features enabled when the agent intention is selected.
/// Shared by the intention slide's agent card checklist and the login slide's
/// skip-login confirmation dialog so the two always stay in sync.
pub fn ai_features() -> [&'static str; 6] {
    static FEATURES: std::sync::OnceLock<[&'static str; 6]> = std::sync::OnceLock::new();
    *FEATURES.get_or_init(|| {
        [
            menu_label(
                "onboarding.ai_features.frontier_models",
                "Use frontier and open-weight models with Warp Agent",
            ),
            menu_label(
                "onboarding.ai_features.cloud_handoff",
                "Hand off agent work to cloud agents",
            ),
            menu_label(
                "onboarding.ai_features.auto_fix",
                "Automatically diagnose and fix terminal errors",
            ),
            menu_label(
                "onboarding.ai_features.agentic_control",
                "Agentic control of long-running commands and TUIs",
            ),
            menu_label(
                "onboarding.ai_features.code_review",
                "Review code diffs and send comments directly to agents",
            ),
            menu_label(
                "onboarding.ai_features.remote_control",
                "Remote control for Claude Code, Codex, and other agents",
            ),
        ]
    })
}

/// User-facing names of the Warp Drive features enabled when the terminal
/// intention is selected with Warp Drive turned on. Shared by the login slide's
/// skip-login confirmation dialog so the list stays in sync with any future
/// surfaces that need it.
pub fn warp_drive_features() -> &'static [&'static str] {
    static FEATURES: std::sync::OnceLock<[&'static str; 2]> = std::sync::OnceLock::new();
    FEATURES
        .get_or_init(|| {
            [
                menu_label("onboarding.warp_drive_features.warp_drive", "Warp Drive"),
                menu_label(
                    "onboarding.warp_drive_features.session_sharing",
                    "Session Sharing",
                ),
            ]
        })
        .as_slice()
}

cfg_if::cfg_if! {
    if #[cfg(feature = "bin")] {
        mod telemetry_provider;
        pub use telemetry_provider::MockTelemetryContextProvider;
    }
}

pub mod components;
mod visuals;

/// The default mode for new sessions, chosen during onboarding.
/// Mapped to `DefaultSessionMode` at the application boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionDefault {
    #[default]
    Agent,
    Terminal,
}

impl std::fmt::Display for SessionDefault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionDefault::Agent => write!(f, "agent"),
            SessionDefault::Terminal => write!(f, "terminal"),
        }
    }
}

pub use agent_onboarding_view::{AgentOnboardingAction, AgentOnboardingEvent, AgentOnboardingView};
pub use model::{OnboardingAuthState, SelectedSettings, UICustomizationSettings};
pub use slides::ProjectOnboardingSettings;
pub use telemetry::OnboardingEvent;

pub fn init(app: &mut warpui_core::AppContext) {
    agent_onboarding_view::init(app);
    callout::init(app);
}
