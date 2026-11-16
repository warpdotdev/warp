use serde::{Deserialize, Serialize};
use warpui::Entity;

#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(rename_all = "snake_case")]
pub enum BannerState {
    /// The banner is not currently visible and has not yet been seen by the user.
    #[default]
    NotDismissed,

    // The banner is open.
    Open,

    /// The banner was dismissed by the user.
    Dismissed,
}

impl Entity for BannerState {
    type Event = ();
}
