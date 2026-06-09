mod hoa_onboarding_flow;
mod tab_config_step;
mod welcome_banner;

pub use hoa_onboarding_flow::{init, HoaOnboardingFlow, HoaOnboardingFlowEvent, HoaOnboardingStep};
use warp_core::user_preferences::GetUserPreferences;
use warpui::AppContext;

const HAS_COMPLETED_HOA_ONBOARDING_KEY: &str = "HasCompletedHOAOnboarding";

pub fn has_completed_hoa_onboarding(ctx: &AppContext) -> bool {
    ctx.private_user_preferences()
        .read_value(HAS_COMPLETED_HOA_ONBOARDING_KEY)
        .unwrap_or_default()
        .and_then(|s| serde_json::from_str::<bool>(&s).ok())
        .unwrap_or(false)
}

pub fn mark_hoa_onboarding_completed(ctx: &AppContext) {
    let _ = ctx.private_user_preferences().write_value(
        HAS_COMPLETED_HOA_ONBOARDING_KEY,
        serde_json::to_string(&true).expect("bool serializes to JSON"),
    );
}
