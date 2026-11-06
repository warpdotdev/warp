//! Telemetry for the `warp-tui` front-end.
use std::ffi::{OsStr, OsString};

use serde_json::{Value, json};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};
const MAX_TERM_PROGRAM_CHARS: usize = 64;

#[derive(Clone, Copy, Debug)]
enum TuiHostMultiplexer {
    None,
    Tmux,
    Screen,
    Zellij,
}

impl TuiHostMultiplexer {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Tmux => "tmux",
            Self::Screen => "screen",
            Self::Zellij => "zellij",
        }
    }
}

#[derive(Debug)]
pub(crate) struct TuiStartupTelemetryEvent {
    term_program: Option<String>,
    multiplexer: TuiHostMultiplexer,
}

impl TuiStartupTelemetryEvent {
    pub(crate) fn from_environment() -> Self {
        Self {
            term_program: sanitize_term_program(std::env::var_os("TERM_PROGRAM")),
            multiplexer: detect_multiplexer(
                std::env::var_os("TMUX").as_deref(),
                std::env::var_os("STY").as_deref(),
                std::env::var_os("ZELLIJ").as_deref(),
                std::env::var_os("ZELLIJ_SESSION_NAME").as_deref(),
            ),
        }
    }
}

fn sanitize_term_program(value: Option<OsString>) -> Option<String> {
    let value = value?.into_string().ok()?;
    let sanitized = value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_TERM_PROGRAM_CHARS)
        .collect::<String>();
    (!sanitized.is_empty()).then_some(sanitized)
}

fn detect_multiplexer(
    tmux: Option<&OsStr>,
    screen: Option<&OsStr>,
    zellij: Option<&OsStr>,
    zellij_session_name: Option<&OsStr>,
) -> TuiHostMultiplexer {
    let is_present = |value: Option<&OsStr>| value.is_some_and(|value| !value.is_empty());
    if is_present(tmux) {
        TuiHostMultiplexer::Tmux
    } else if is_present(screen) {
        TuiHostMultiplexer::Screen
    } else if is_present(zellij) || is_present(zellij_session_name) {
        TuiHostMultiplexer::Zellij
    } else {
        TuiHostMultiplexer::None
    }
}

impl TelemetryEvent for TuiStartupTelemetryEvent {
    fn name(&self) -> &'static str {
        "TUI.Startup"
    }

    fn payload(&self) -> Option<Value> {
        Some(json!({
            "term_program": self.term_program,
            "multiplexer": self.multiplexer.as_str(),
        }))
    }

    fn description(&self) -> &'static str {
        "The headless Warp TUI is launched"
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Always
    }

    fn contains_ugc(&self) -> bool {
        false
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        std::iter::once(Box::new(Self {
            term_program: None,
            multiplexer: TuiHostMultiplexer::None,
        }) as Box<dyn TelemetryEventDesc>)
    }
}

impl TelemetryEventDesc for TuiStartupTelemetryEvent {
    fn name(&self) -> &'static str {
        "TUI.Startup"
    }

    fn description(&self) -> &'static str {
        "The headless Warp TUI is launched"
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Always
    }
}

warp_core::register_telemetry_event!(TuiStartupTelemetryEvent);

#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub(crate) enum TuiConversationMenuTelemetryEvent {
    Opened,
    ItemSelected,
}

impl TelemetryEvent for TuiConversationMenuTelemetryEvent {
    fn name(&self) -> &'static str {
        TuiConversationMenuTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        None
    }

    fn description(&self) -> &'static str {
        TuiConversationMenuTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        TuiConversationMenuTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        false
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for TuiConversationMenuTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::Opened => "TUI.ConversationMenu.Opened",
            Self::ItemSelected => "TUI.ConversationMenu.ItemSelected",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::Opened => "The conversation menu opened in the headless Warp TUI",
            Self::ItemSelected => "A conversation-menu item was selected in the headless Warp TUI",
        }
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Always
    }
}

warp_core::register_telemetry_event!(TuiConversationMenuTelemetryEvent);

#[derive(Clone, Copy, Debug)]
pub(crate) enum TuiConversationRestoreTelemetryState {
    Started,
    Succeeded,
    Failed,
    Cancelled,
}

impl TuiConversationRestoreTelemetryState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TuiConversationRestoreTelemetryTarget {
    Local,
    Server,
}

impl TuiConversationRestoreTelemetryTarget {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Server => "server",
        }
    }
}

#[derive(Debug)]
pub(crate) struct TuiConversationRestoreTelemetryEvent {
    pub state: TuiConversationRestoreTelemetryState,
    pub target: TuiConversationRestoreTelemetryTarget,
}

impl TelemetryEvent for TuiConversationRestoreTelemetryEvent {
    fn name(&self) -> &'static str {
        "TUI.ConversationRestore"
    }

    fn payload(&self) -> Option<Value> {
        Some(json!({
            "state": self.state.as_str(),
            "target": self.target.as_str(),
        }))
    }

    fn description(&self) -> &'static str {
        "A conversation-list restore changed lifecycle state in the headless Warp TUI"
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Always
    }

    fn contains_ugc(&self) -> bool {
        false
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        std::iter::once(Box::new(Self {
            state: TuiConversationRestoreTelemetryState::Started,
            target: TuiConversationRestoreTelemetryTarget::Local,
        }) as Box<dyn TelemetryEventDesc>)
    }
}

impl TelemetryEventDesc for TuiConversationRestoreTelemetryEvent {
    fn name(&self) -> &'static str {
        "TUI.ConversationRestore"
    }

    fn description(&self) -> &'static str {
        "A conversation-list restore changed lifecycle state in the headless Warp TUI"
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Always
    }
}

warp_core::register_telemetry_event!(TuiConversationRestoreTelemetryEvent);

/// Health signals for the TUI auto-updater. Sent when the outcome of a
/// background update check *changes* (not on every poll), so repeated
/// `up_to_date` checks or repeated failures don't spam events.
#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub(crate) enum TuiAutoupdateTelemetryEvent {
    /// A background update check completed.
    CheckCompleted {
        /// `"up_to_date"`, `"installed"`, `"pending_restart"`,
        /// `"update_available"`, or `"locked"`.
        outcome: &'static str,
        /// The relevant version: the running version when up to date, or the
        /// newly installed, staged, or available version.
        version: Option<String>,
    },
    /// A background update check failed (e.g. network or install errors).
    CheckFailed { error: String },
}

impl TelemetryEvent for TuiAutoupdateTelemetryEvent {
    fn name(&self) -> &'static str {
        TuiAutoupdateTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            TuiAutoupdateTelemetryEvent::CheckCompleted { outcome, version } => Some(json!({
                "outcome": outcome,
                "version": version,
            })),
            TuiAutoupdateTelemetryEvent::CheckFailed { error } => Some(json!({
                "error": error,
            })),
        }
    }

    fn description(&self) -> &'static str {
        TuiAutoupdateTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        TuiAutoupdateTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        match self {
            TuiAutoupdateTelemetryEvent::CheckCompleted { .. } => false,
            // Error messages can embed install paths (which include the
            // user's home directory).
            TuiAutoupdateTelemetryEvent::CheckFailed { .. } => true,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for TuiAutoupdateTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            TuiAutoupdateTelemetryEventDiscriminants::CheckCompleted => {
                "TUI Autoupdate Check Completed"
            }
            TuiAutoupdateTelemetryEventDiscriminants::CheckFailed => "TUI Autoupdate Check Failed",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            TuiAutoupdateTelemetryEventDiscriminants::CheckCompleted => {
                "A warp-tui background update check completed with a new outcome"
            }
            TuiAutoupdateTelemetryEventDiscriminants::CheckFailed => {
                "A warp-tui background update check failed"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            TuiAutoupdateTelemetryEventDiscriminants::CheckCompleted
            | TuiAutoupdateTelemetryEventDiscriminants::CheckFailed => EnablementState::Always,
        }
    }
}

warp_core::register_telemetry_event!(TuiAutoupdateTelemetryEvent);

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
