use instant::Instant;
use serde_json::{Value, json};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

use super::TuiLoginPhase;
use crate::server::server_api::auth::UserAuthenticationError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AuthenticationEntrypoint {
    OpenBrowser,
    CopyUrl,
}

impl AuthenticationEntrypoint {
    fn as_str(self) -> &'static str {
        match self {
            Self::OpenBrowser => "open_browser",
            Self::CopyUrl => "copy_url",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Journey {
    InitialLogin,
    PostLogout,
}

impl Journey {
    fn as_str(self) -> &'static str {
        match self {
            Self::InitialLogin => "initial_login",
            Self::PostLogout => "post_logout",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AuthenticationAttempt {
    Initial,
    Retry,
}

impl AuthenticationAttempt {
    fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Retry => "retry",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BrowserLaunchTrigger {
    Initial,
    Retry,
    PostLogout,
}

impl BrowserLaunchTrigger {
    fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Retry => "retry",
            Self::PostLogout => "post_logout",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Succeeded,
    Failed,
}

impl Outcome {
    fn from_succeeded(succeeded: bool) -> Self {
        if succeeded {
            Self::Succeeded
        } else {
            Self::Failed
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AuthenticationFailureStage {
    DeviceCodeRequest,
    Authentication,
}

impl AuthenticationFailureStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::DeviceCodeRequest => "device_code_request",
            Self::Authentication => "authentication",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AuthenticationFailureReason {
    DeniedAccessToken,
    UserAccountDisabled,
    InvalidState,
    MissingState,
    DeviceCodeRequestTimeout,
    Unexpected,
}

impl AuthenticationFailureReason {
    fn from_error(error: &UserAuthenticationError) -> Self {
        match error {
            UserAuthenticationError::DeniedAccessToken(_) => Self::DeniedAccessToken,
            UserAuthenticationError::UserAccountDisabled(_) => Self::UserAccountDisabled,
            UserAuthenticationError::InvalidStateParameter => Self::InvalidState,
            UserAuthenticationError::MissingStateParameter => Self::MissingState,
            UserAuthenticationError::DeviceCodeRequestTimedOut { .. } => {
                Self::DeviceCodeRequestTimeout
            }
            UserAuthenticationError::Unexpected(_) => Self::Unexpected,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::DeniedAccessToken => "denied_access_token",
            Self::UserAccountDisabled => "user_account_disabled",
            Self::InvalidState => "invalid_state",
            Self::MissingState => "missing_state",
            Self::DeviceCodeRequestTimeout => "device_code_request_timeout",
            Self::Unexpected => "unexpected",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AbandonmentPhase {
    Welcome,
    RequestingLink,
    WaitingForLogin,
    BrowserOpenFailed,
    AuthenticationFailed,
}

impl AbandonmentPhase {
    pub(super) fn from_login_phase(phase: &TuiLoginPhase) -> Option<Self> {
        match phase {
            TuiLoginPhase::SignedOutWelcome => Some(Self::Welcome),
            TuiLoginPhase::AwaitingLogin { browser_url: None } => Some(Self::RequestingLink),
            TuiLoginPhase::AwaitingLogin {
                browser_url: Some(_),
            } => Some(Self::WaitingForLogin),
            TuiLoginPhase::BrowserOpenFailed { .. } => Some(Self::BrowserOpenFailed),
            TuiLoginPhase::Failed { .. } => Some(Self::AuthenticationFailed),
            TuiLoginPhase::LoggedIn => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Welcome => "welcome",
            Self::RequestingLink => "requesting_link",
            Self::WaitingForLogin => "waiting_for_login",
            Self::BrowserOpenFailed => "browser_open_failed",
            Self::AuthenticationFailed => "authentication_failed",
        }
    }
}

#[derive(Debug, PartialEq, Eq, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub(super) enum TuiOnboardingTelemetryEvent {
    AuthenticationStarted {
        journey: Journey,
        entrypoint: AuthenticationEntrypoint,
        attempt: AuthenticationAttempt,
    },
    DeviceAuthorizationReady,
    BrowserLaunch {
        journey: Journey,
        trigger: BrowserLaunchTrigger,
        outcome: Outcome,
    },
    LoginUrlCopied {
        outcome: Outcome,
    },
    AuthenticationFailed {
        journey: Journey,
        stage: AuthenticationFailureStage,
        reason: AuthenticationFailureReason,
        duration_ms: u64,
    },
    Abandoned {
        journey: Journey,
        phase: AbandonmentPhase,
        duration_ms: u64,
    },
    Completed {
        journey: Journey,
        duration_ms: u64,
    },
}

impl TelemetryEvent for TuiOnboardingTelemetryEvent {
    fn name(&self) -> &'static str {
        TuiOnboardingTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            Self::AuthenticationStarted {
                journey,
                entrypoint,
                attempt,
            } => Some(json!({
                "journey": journey.as_str(),
                "entrypoint": entrypoint.as_str(),
                "attempt": attempt.as_str(),
            })),
            Self::DeviceAuthorizationReady => None,
            Self::BrowserLaunch {
                journey,
                trigger,
                outcome,
            } => Some(json!({
                "journey": journey.as_str(),
                "trigger": trigger.as_str(),
                "outcome": outcome.as_str(),
            })),
            Self::LoginUrlCopied { outcome } => Some(json!({
                "outcome": outcome.as_str(),
            })),
            Self::AuthenticationFailed {
                journey,
                stage,
                reason,
                duration_ms,
            } => Some(json!({
                "journey": journey.as_str(),
                "stage": stage.as_str(),
                "reason": reason.as_str(),
                "duration_ms": duration_ms,
            })),
            Self::Abandoned {
                journey,
                phase,
                duration_ms,
            } => Some(json!({
                "journey": journey.as_str(),
                "phase": phase.as_str(),
                "duration_ms": duration_ms,
            })),
            Self::Completed {
                journey,
                duration_ms,
            } => Some(json!({
                "journey": journey.as_str(),
                "duration_ms": duration_ms,
            })),
        }
    }

    fn description(&self) -> &'static str {
        TuiOnboardingTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        TuiOnboardingTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        false
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for TuiOnboardingTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::AuthenticationStarted => "TUI.Onboarding.AuthenticationStarted",
            Self::DeviceAuthorizationReady => "TUI.Onboarding.DeviceAuthorizationReady",
            Self::BrowserLaunch => "TUI.Onboarding.BrowserLaunch",
            Self::LoginUrlCopied => "TUI.Onboarding.LoginUrlCopied",
            Self::AuthenticationFailed => "TUI.Onboarding.AuthenticationFailed",
            Self::Abandoned => "TUI.Onboarding.Abandoned",
            Self::Completed => "TUI.Onboarding.Completed",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::AuthenticationStarted => "TUI browser authentication started",
            Self::DeviceAuthorizationReady => "TUI device authorization URL became available",
            Self::BrowserLaunch => "TUI attempted to launch the authentication browser",
            Self::LoginUrlCopied => "TUI attempted to copy the authentication URL",
            Self::AuthenticationFailed => "TUI authentication failed",
            Self::Abandoned => "User exited while the TUI authentication UI was visible",
            Self::Completed => "TUI displayed the terminal after interactive authentication",
        }
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Always
    }
}

warp_core::register_telemetry_event!(TuiOnboardingTelemetryEvent);

struct ActiveFlow {
    journey: Journey,
    started_at: Instant,
    attempt_started_at: Option<Instant>,
    attempts: usize,
    device_authorization_ready: bool,
}

impl ActiveFlow {
    fn new(journey: Journey, started_at: Instant) -> Self {
        Self {
            journey,
            started_at,
            attempt_started_at: None,
            attempts: 0,
            device_authorization_ready: false,
        }
    }

    fn browser_launch_trigger(&self) -> BrowserLaunchTrigger {
        if self.attempts > 1 {
            BrowserLaunchTrigger::Retry
        } else {
            match self.journey {
                Journey::InitialLogin => BrowserLaunchTrigger::Initial,
                Journey::PostLogout => BrowserLaunchTrigger::PostLogout,
            }
        }
    }
}

pub(super) struct TuiOnboardingTelemetry {
    flow: Option<ActiveFlow>,
}

impl TuiOnboardingTelemetry {
    pub(super) fn new(logged_in: bool) -> Self {
        Self {
            flow: (!logged_in).then(|| ActiveFlow::new(Journey::InitialLogin, Instant::now())),
        }
    }

    pub(super) fn authentication_started(
        &mut self,
        entrypoint: AuthenticationEntrypoint,
    ) -> TuiOnboardingTelemetryEvent {
        let now = Instant::now();
        let flow = self
            .flow
            .get_or_insert_with(|| ActiveFlow::new(Journey::InitialLogin, now));
        let attempt = if flow.attempts == 0 {
            AuthenticationAttempt::Initial
        } else {
            AuthenticationAttempt::Retry
        };
        flow.attempts += 1;
        flow.attempt_started_at = Some(now);
        flow.device_authorization_ready = false;
        TuiOnboardingTelemetryEvent::AuthenticationStarted {
            journey: flow.journey,
            entrypoint,
            attempt,
        }
    }

    pub(super) fn post_logout_authentication_started(&mut self) -> TuiOnboardingTelemetryEvent {
        self.flow = Some(ActiveFlow::new(Journey::PostLogout, Instant::now()));
        self.authentication_started(AuthenticationEntrypoint::OpenBrowser)
    }

    pub(super) fn device_authorization_ready(&mut self) -> Option<TuiOnboardingTelemetryEvent> {
        let flow = self.flow.as_mut()?;
        flow.attempt_started_at?;
        if flow.device_authorization_ready {
            return None;
        }
        flow.device_authorization_ready = true;
        Some(TuiOnboardingTelemetryEvent::DeviceAuthorizationReady)
    }

    pub(super) fn browser_launch(&self, succeeded: bool) -> Option<TuiOnboardingTelemetryEvent> {
        let flow = self.flow.as_ref()?;
        flow.attempt_started_at?;
        Some(TuiOnboardingTelemetryEvent::BrowserLaunch {
            journey: flow.journey,
            trigger: flow.browser_launch_trigger(),
            outcome: Outcome::from_succeeded(succeeded),
        })
    }

    pub(super) fn login_url_copied(&self, succeeded: bool) -> Option<TuiOnboardingTelemetryEvent> {
        self.flow.as_ref()?;
        Some(TuiOnboardingTelemetryEvent::LoginUrlCopied {
            outcome: Outcome::from_succeeded(succeeded),
        })
    }

    pub(super) fn authentication_failed(
        &mut self,
        error: &UserAuthenticationError,
    ) -> Option<TuiOnboardingTelemetryEvent> {
        let flow = self.flow.as_mut()?;
        let attempt_started_at = flow.attempt_started_at.take()?;
        let stage = if flow.device_authorization_ready {
            AuthenticationFailureStage::Authentication
        } else {
            AuthenticationFailureStage::DeviceCodeRequest
        };
        Some(TuiOnboardingTelemetryEvent::AuthenticationFailed {
            journey: flow.journey,
            stage,
            reason: AuthenticationFailureReason::from_error(error),
            duration_ms: elapsed_ms(attempt_started_at),
        })
    }

    pub(super) fn abandoned(
        &mut self,
        phase: AbandonmentPhase,
    ) -> Option<TuiOnboardingTelemetryEvent> {
        let flow = self.flow.take()?;
        Some(TuiOnboardingTelemetryEvent::Abandoned {
            journey: flow.journey,
            phase,
            duration_ms: elapsed_ms(flow.started_at),
        })
    }

    pub(super) fn completed(&mut self) -> Option<TuiOnboardingTelemetryEvent> {
        let flow = self.flow.take()?;
        Some(TuiOnboardingTelemetryEvent::Completed {
            journey: flow.journey,
            duration_ms: elapsed_ms(flow.started_at),
        })
    }
}

fn elapsed_ms(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
