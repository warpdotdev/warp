use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

/// Telemetry events for the onboarding flow.
#[derive(Clone, Debug, Serialize, Deserialize, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
#[strum_discriminants(name(OnboardingEventDiscriminant))]
pub enum OnboardingEvent {
    /// The onboarding flow was started.
    OnboardingStarted,
    /// A specific slide was viewed.
    SlideViewed { slide_name: String },
    /// A setting was changed during onboarding.
    SettingChanged { setting: String, value: String },
    /// The onboarding slides were completed.
    OnboardingSlidesCompleted {
        intention: String,
        model: Option<String>,
        autonomy: Option<String>,
        has_project_path: bool,
        /// How the user is accessing AI when intention is agent_driven:
        /// "warp_agent" or "third_party". None when intention is not agent_driven.
        ai_access: Option<String>,
    },
    /// The user clicked the "Get Started" button.
    GetStartedClicked,
    /// The user started folder selection.
    FolderSelectionStarted,
    /// The user selected a folder.
    FolderSelected,
    /// A callout was displayed.
    CalloutDisplayed { callout: String },
    /// The user clicked next on a callout.
    CalloutNext,
    /// The user completed the callout flow.
    CalloutCompleted { completion_type: String },
    /// The user navigated to the next slide.
    SlideNavigatedNext,
    /// The user navigated to the previous slide.
    SlideNavigatedBack,
    /// The user was shown the "Are you sure you don't want AI?" confirmation modal.
    NoAiConfirmationShown,
    /// The user confirmed they don't want AI in the confirmation modal.
    NoAiConfirmed,
    /// The user chose to keep AI ("Give me AI features") in the confirmation modal.
    NoAiConfirmationCancelled,
    /// The user clicked the "Upgrade" button on the "Customize your agent" slide.
    AgentSlideUpgradeClicked,
    /// The user clicked the "Log in" link on the welcome/intro slide.
    WelcomeLoginClicked,
}

impl TelemetryEvent for OnboardingEvent {
    fn name(&self) -> &'static str {
        match self {
            OnboardingEvent::OnboardingStarted => "onboarding_started",
            OnboardingEvent::SlideViewed { .. } => "onboarding_slide_viewed",
            OnboardingEvent::SettingChanged { .. } => "onboarding_setting_changed",
            OnboardingEvent::OnboardingSlidesCompleted { .. } => "onboarding_slides_completed",
            OnboardingEvent::GetStartedClicked => "onboarding_get_started_clicked",
            OnboardingEvent::FolderSelectionStarted => "onboarding_folder_selection_started",
            OnboardingEvent::FolderSelected => "onboarding_folder_selected",
            OnboardingEvent::CalloutDisplayed { .. } => "onboarding_callout_displayed",
            OnboardingEvent::CalloutNext => "onboarding_callout_next",
            OnboardingEvent::CalloutCompleted { .. } => "onboarding_callout_completed",
            OnboardingEvent::SlideNavigatedNext => "onboarding_slide_navigated_next",
            OnboardingEvent::SlideNavigatedBack => "onboarding_slide_navigated_back",
            OnboardingEvent::NoAiConfirmationShown => "onboarding_no_ai_confirmation_shown",
            OnboardingEvent::NoAiConfirmed => "onboarding_no_ai_confirmed",
            OnboardingEvent::NoAiConfirmationCancelled => "onboarding_no_ai_confirmation_cancelled",
            OnboardingEvent::AgentSlideUpgradeClicked => "onboarding_agent_slide_upgrade_clicked",
            OnboardingEvent::WelcomeLoginClicked => "onboarding_welcome_login_clicked",
        }
    }

    fn payload(&self) -> Option<Value> {
        match self {
            OnboardingEvent::OnboardingStarted => None,
            OnboardingEvent::SlideViewed { slide_name } => Some(json!({
                "slide_name": slide_name,
            })),
            OnboardingEvent::SettingChanged { setting, value } => Some(json!({
                "setting": setting,
                "value": value,
            })),
            OnboardingEvent::OnboardingSlidesCompleted {
                intention,
                model,
                autonomy,
                has_project_path,
                ai_access,
            } => Some(json!({
                "intention": intention,
                "model": model,
                "autonomy": autonomy,
                "has_project_path": has_project_path,
                "ai_access": ai_access,
            })),
            OnboardingEvent::GetStartedClicked => None,
            OnboardingEvent::FolderSelectionStarted => None,
            OnboardingEvent::FolderSelected => None,
            OnboardingEvent::CalloutDisplayed { callout } => Some(json!({
                "callout": callout,
            })),
            OnboardingEvent::CalloutNext => None,
            OnboardingEvent::CalloutCompleted { completion_type } => Some(json!({
                "completion_type": completion_type,
            })),
            OnboardingEvent::SlideNavigatedNext => None,
            OnboardingEvent::SlideNavigatedBack => None,
            OnboardingEvent::NoAiConfirmationShown => None,
            OnboardingEvent::NoAiConfirmed => None,
            OnboardingEvent::NoAiConfirmationCancelled => None,
            OnboardingEvent::AgentSlideUpgradeClicked => None,
            OnboardingEvent::WelcomeLoginClicked => None,
        }
    }

    fn description(&self) -> &'static str {
        match self {
            OnboardingEvent::OnboardingStarted => "User started the onboarding flow",
            OnboardingEvent::SlideViewed { .. } => "User viewed a slide in the onboarding flow",
            OnboardingEvent::SettingChanged { .. } => "User changed a setting during onboarding",
            OnboardingEvent::OnboardingSlidesCompleted { .. } => {
                "User completed the onboarding slides"
            }
            OnboardingEvent::GetStartedClicked => "User clicked the Get Started button",
            OnboardingEvent::FolderSelectionStarted => "User started folder selection",
            OnboardingEvent::FolderSelected => "User selected a folder",
            OnboardingEvent::CalloutDisplayed { .. } => "A callout was displayed to the user",
            OnboardingEvent::CalloutNext => "User clicked next on a callout",
            OnboardingEvent::CalloutCompleted { .. } => "User completed the callout flow",
            OnboardingEvent::SlideNavigatedNext => "User navigated to the next slide",
            OnboardingEvent::SlideNavigatedBack => "User navigated to the previous slide",
            OnboardingEvent::NoAiConfirmationShown => "User was shown the no-AI confirmation modal",
            OnboardingEvent::NoAiConfirmed => {
                "User confirmed they don't want AI in the confirmation modal"
            }
            OnboardingEvent::NoAiConfirmationCancelled => {
                "User chose to keep AI in the confirmation modal"
            }
            OnboardingEvent::AgentSlideUpgradeClicked => {
                "User clicked the Upgrade button on the Customize your agent slide"
            }
            OnboardingEvent::WelcomeLoginClicked => {
                "User clicked the Log in link on the welcome/intro slide"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Always
    }

    fn contains_ugc(&self) -> bool {
        false
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for OnboardingEventDiscriminant {
    fn name(&self) -> &'static str {
        match self {
            OnboardingEventDiscriminant::OnboardingStarted => "onboarding_started",
            OnboardingEventDiscriminant::SlideViewed => "onboarding_slide_viewed",
            OnboardingEventDiscriminant::SettingChanged => "onboarding_setting_changed",
            OnboardingEventDiscriminant::OnboardingSlidesCompleted => "onboarding_slides_completed",
            OnboardingEventDiscriminant::GetStartedClicked => "onboarding_get_started_clicked",
            OnboardingEventDiscriminant::FolderSelectionStarted => {
                "onboarding_folder_selection_started"
            }
            OnboardingEventDiscriminant::FolderSelected => "onboarding_folder_selected",
            OnboardingEventDiscriminant::CalloutDisplayed => "onboarding_callout_displayed",
            OnboardingEventDiscriminant::CalloutNext => "onboarding_callout_next",
            OnboardingEventDiscriminant::CalloutCompleted => "onboarding_callout_completed",
            OnboardingEventDiscriminant::SlideNavigatedNext => "onboarding_slide_navigated_next",
            OnboardingEventDiscriminant::SlideNavigatedBack => "onboarding_slide_navigated_back",
            OnboardingEventDiscriminant::NoAiConfirmationShown => {
                "onboarding_no_ai_confirmation_shown"
            }
            OnboardingEventDiscriminant::NoAiConfirmed => "onboarding_no_ai_confirmed",
            OnboardingEventDiscriminant::NoAiConfirmationCancelled => {
                "onboarding_no_ai_confirmation_cancelled"
            }
            OnboardingEventDiscriminant::AgentSlideUpgradeClicked => {
                "onboarding_agent_slide_upgrade_clicked"
            }
            OnboardingEventDiscriminant::WelcomeLoginClicked => "onboarding_welcome_login_clicked",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            OnboardingEventDiscriminant::OnboardingStarted => "User started the onboarding flow",
            OnboardingEventDiscriminant::SlideViewed => {
                "User viewed a slide in the onboarding flow"
            }
            OnboardingEventDiscriminant::SettingChanged => {
                "User changed a setting during onboarding"
            }
            OnboardingEventDiscriminant::OnboardingSlidesCompleted => {
                "User completed the onboarding slides"
            }
            OnboardingEventDiscriminant::GetStartedClicked => "User clicked the Get Started button",
            OnboardingEventDiscriminant::FolderSelectionStarted => "User started folder selection",
            OnboardingEventDiscriminant::FolderSelected => "User selected a folder",
            OnboardingEventDiscriminant::CalloutDisplayed => "A callout was displayed to the user",
            OnboardingEventDiscriminant::CalloutNext => "User clicked next on a callout",
            OnboardingEventDiscriminant::CalloutCompleted => "User completed the callout flow",
            OnboardingEventDiscriminant::SlideNavigatedNext => "User navigated to the next slide",
            OnboardingEventDiscriminant::SlideNavigatedBack => {
                "User navigated to the previous slide"
            }
            OnboardingEventDiscriminant::NoAiConfirmationShown => {
                "User was shown the no-AI confirmation modal"
            }
            OnboardingEventDiscriminant::NoAiConfirmed => {
                "User confirmed they don't want AI in the confirmation modal"
            }
            OnboardingEventDiscriminant::NoAiConfirmationCancelled => {
                "User chose to keep AI in the confirmation modal"
            }
            OnboardingEventDiscriminant::AgentSlideUpgradeClicked => {
                "User clicked the Upgrade button on the Customize your agent slide"
            }
            OnboardingEventDiscriminant::WelcomeLoginClicked => {
                "User clicked the Log in link on the welcome/intro slide"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Always
    }
}

warp_core::register_telemetry_event!(OnboardingEvent);
