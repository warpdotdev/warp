//! Onboarding-specific AI types and conversions.

use ai::LLMId;
use onboarding::slides::OnboardingModelInfo;
use onboarding::OnboardingAuthState;
use warp_core::ui::icons::Icon;
use warpui::{AppContext, SingletonEntity};

use crate::auth::AuthStateProvider;
use crate::workspaces::user_workspaces::UserWorkspaces;

use super::llms::{DisableReason, LLMInfo, LLMPreferences};

impl From<&LLMInfo> for OnboardingModelInfo {
    fn from(llm: &LLMInfo) -> Self {
        Self {
            id: llm.id.clone(),
            title: llm.display_name.clone(),
            icon: llm.provider.icon().unwrap_or(Icon::Oz),
            requires_upgrade: matches!(llm.disable_reason, Some(DisableReason::RequiresUpgrade)),
            is_default: false,
        }
    }
}

pub fn build_onboarding_models(
    prefs: &LLMPreferences,
    app: &AppContext,
) -> (Vec<OnboardingModelInfo>, LLMId) {
    let default_id = prefs.get_default_base_model().id.clone();
    let models: Vec<OnboardingModelInfo> = prefs
        .get_base_llm_choices_for_agent_mode(app)
        .map(|llm| {
            let mut info = OnboardingModelInfo::from(llm);
            info.is_default = info.id == default_id;
            info
        })
        .collect();
    (models, default_id)
}

pub fn current_onboarding_auth_state(ctx: &AppContext) -> OnboardingAuthState {
    let auth_state = AuthStateProvider::as_ref(ctx).get();
    if auth_state.is_anonymous_or_logged_out() {
        return OnboardingAuthState::LoggedOut;
    }
    let is_on_paid_plan = UserWorkspaces::as_ref(ctx)
        .current_workspace()
        .map(|w| w.billing_metadata.is_user_on_paid_plan())
        .unwrap_or(false);
    if is_on_paid_plan {
        OnboardingAuthState::PayingUser
    } else {
        OnboardingAuthState::FreeUser
    }
}
