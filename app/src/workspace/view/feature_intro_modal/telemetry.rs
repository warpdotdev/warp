use serde_json::{Value, json};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

use super::FeatureIntroId;

/// Telemetry for the reusable bottom-right feature-intro popover. Every
/// registered [`super::FeatureIntro`] shares this event type, disambiguated by
/// `feature`. User identity is attached automatically by `send_telemetry_from_ctx!`.
#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum FeatureIntroModalTelemetryEvent {
    Shown { feature: FeatureIntroId },
    Dismissed { feature: FeatureIntroId },
    CtaClicked { feature: FeatureIntroId },
}

impl TelemetryEvent for FeatureIntroModalTelemetryEvent {
    fn name(&self) -> &'static str {
        FeatureIntroModalTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        let feature = match self {
            Self::Shown { feature }
            | Self::Dismissed { feature }
            | Self::CtaClicked { feature } => feature.as_key(),
        };
        Some(json!({ "feature": feature }))
    }

    fn description(&self) -> &'static str {
        FeatureIntroModalTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        FeatureIntroModalTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        match self {
            Self::Shown { .. } | Self::Dismissed { .. } | Self::CtaClicked { .. } => false,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for FeatureIntroModalTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::Shown => "FeatureIntroModal.Shown",
            Self::Dismissed => "FeatureIntroModal.Dismissed",
            Self::CtaClicked => "FeatureIntroModal.CtaClicked",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::Shown => "A feature-intro popover was shown to the user",
            Self::Dismissed => "The user dismissed a feature-intro popover",
            Self::CtaClicked => "The user clicked the call-to-action in a feature-intro popover",
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::Shown | Self::Dismissed | Self::CtaClicked => EnablementState::Always,
        }
    }
}

warp_core::register_telemetry_event!(FeatureIntroModalTelemetryEvent);
