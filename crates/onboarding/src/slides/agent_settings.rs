use ai::LLMId;
use warp_core::ui::icons::Icon;

/// Information about a model retained for compatibility with onboarding telemetry
/// and settings payloads.
#[derive(Clone, Debug)]
pub struct OnboardingModelInfo {
    pub id: LLMId,
    pub title: String,
    pub icon: Icon,
    pub is_default: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AgentAutonomy {
    Full,
    #[default]
    Partial,
    None,
}

impl std::fmt::Display for AgentAutonomy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentAutonomy::Full => write!(f, "full"),
            AgentAutonomy::Partial => write!(f, "partial"),
            AgentAutonomy::None => write!(f, "none"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentDevelopmentSettings {
    /// Retained for compatibility with the old built-in Agent onboarding payload.
    pub selected_model_id: LLMId,
    pub autonomy: Option<AgentAutonomy>,
    /// Whether the CLI agent toolbar is enabled.
    pub cli_agent_toolbar_enabled: bool,
    /// The default session mode chosen during onboarding.
    pub session_default: crate::SessionDefault,
    /// Whether Warp's built-in AI assistant is disabled.
    pub disable_oz: bool,
    /// Whether agent notifications are shown.
    pub show_agent_notifications: bool,
}

impl AgentDevelopmentSettings {
    pub fn new(default_model_id: LLMId) -> Self {
        Self {
            selected_model_id: default_model_id,
            autonomy: Some(AgentAutonomy::default()),
            cli_agent_toolbar_enabled: true,
            session_default: crate::SessionDefault::Terminal,
            disable_oz: true,
            show_agent_notifications: true,
        }
    }
}
