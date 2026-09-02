mod telemetry;
mod view;

pub use telemetry::FactoriesLaunchModalTelemetryEvent;
pub use view::{
    FACTORIES_LAUNCH_SEEN_KEY, FactoriesLaunchModal, FactoriesLaunchModalEvent, init,
    with_email_id_prefill,
};
