use serde_json::Value;
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum SettingsTelemetryEvent {
    EnvironmentsPageOpened,
}

impl TelemetryEvent for SettingsTelemetryEvent {
    fn name(&self) -> &'static str {
        SettingsTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        None
    }

    fn description(&self) -> &'static str {
        SettingsTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        SettingsTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        match self {
            SettingsTelemetryEvent::EnvironmentsPageOpened => false,
        }
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for SettingsTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            SettingsTelemetryEventDiscriminants::EnvironmentsPageOpened => {
                "Settings.Environments.PageOpened"
            }
        }
    }

    fn description(&self) -> &'static str {
        match self {
            SettingsTelemetryEventDiscriminants::EnvironmentsPageOpened => {
                "User opened the Environments settings page"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            SettingsTelemetryEventDiscriminants::EnvironmentsPageOpened => EnablementState::Always,
        }
    }
}

warp_core::register_telemetry_event!(SettingsTelemetryEvent);
