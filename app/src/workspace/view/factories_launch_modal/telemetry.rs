use serde_json::Value;
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

/// Telemetry for the Factories launch modal.
#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum FactoriesLaunchModalTelemetryEvent {
    Shown,
    Dismissed,
    CtaClicked,
}

impl TelemetryEvent for FactoriesLaunchModalTelemetryEvent {
    fn name(&self) -> &'static str {
        FactoriesLaunchModalTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        None
    }

    fn description(&self) -> &'static str {
        FactoriesLaunchModalTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        FactoriesLaunchModalTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        match self {
            Self::Shown | Self::Dismissed | Self::CtaClicked => false,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for FactoriesLaunchModalTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::Shown => "FactoriesLaunchModal.Shown",
            Self::Dismissed => "FactoriesLaunchModal.Dismissed",
            Self::CtaClicked => "FactoriesLaunchModal.CtaClicked",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::Shown => "The Factories launch modal was shown to the user",
            Self::Dismissed => "The user dismissed the Factories launch modal",
            Self::CtaClicked => "The user clicked the call-to-action in the Factories launch modal",
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::Shown | Self::Dismissed | Self::CtaClicked => EnablementState::Always,
        }
    }
}

warp_core::register_telemetry_event!(FactoriesLaunchModalTelemetryEvent);
