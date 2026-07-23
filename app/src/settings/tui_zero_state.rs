use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};
use settings::macros::define_settings_group;
use settings::{SupportedPlatforms, SyncToCloud};

pub const DEFAULT_TUI_ZERO_STATE_ROTATION_PERIOD_SECONDS: f64 = 5.0;
pub const MIN_TUI_ZERO_STATE_ROTATION_PERIOD_SECONDS: f64 = 1.0;
pub const MAX_TUI_ZERO_STATE_ROTATION_PERIOD_SECONDS: f64 = 60.0;
pub const DEFAULT_TUI_ZERO_STATE_EXTRUSION_DEPTH: f64 = 0.18;
pub const MIN_TUI_ZERO_STATE_EXTRUSION_DEPTH: f64 = 0.02;
pub const MAX_TUI_ZERO_STATE_EXTRUSION_DEPTH: f64 = 0.5;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TuiZeroStateObject {
    #[default]
    BuiltIn,
    AsciiFile {
        path: PathBuf,
    },
}

impl settings_value::SettingsValue for TuiZeroStateObject {}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct TuiZeroStateRotationPeriodSeconds(f64);

impl TuiZeroStateRotationPeriodSeconds {
    pub fn get(self) -> f64 {
        self.0
    }
}

impl Default for TuiZeroStateRotationPeriodSeconds {
    fn default() -> Self {
        Self(DEFAULT_TUI_ZERO_STATE_ROTATION_PERIOD_SECONDS)
    }
}

impl<'de> Deserialize<'de> for TuiZeroStateRotationPeriodSeconds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_bounded_f64(
            deserializer,
            "rotation period",
            MIN_TUI_ZERO_STATE_ROTATION_PERIOD_SECONDS,
            MAX_TUI_ZERO_STATE_ROTATION_PERIOD_SECONDS,
        )
        .map(Self)
    }
}

impl settings_value::SettingsValue for TuiZeroStateRotationPeriodSeconds {}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct TuiZeroStateExtrusionDepth(f64);

impl TuiZeroStateExtrusionDepth {
    pub fn get(self) -> f64 {
        self.0
    }
}

impl Default for TuiZeroStateExtrusionDepth {
    fn default() -> Self {
        Self(DEFAULT_TUI_ZERO_STATE_EXTRUSION_DEPTH)
    }
}

impl<'de> Deserialize<'de> for TuiZeroStateExtrusionDepth {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_bounded_f64(
            deserializer,
            "extrusion depth",
            MIN_TUI_ZERO_STATE_EXTRUSION_DEPTH,
            MAX_TUI_ZERO_STATE_EXTRUSION_DEPTH,
        )
        .map(Self)
    }
}

impl settings_value::SettingsValue for TuiZeroStateExtrusionDepth {}

fn deserialize_bounded_f64<'de, D>(
    deserializer: D,
    name: &'static str,
    min: f64,
    max: f64,
) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = f64::deserialize(deserializer)?;
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(BoundedFloatError {
            name,
            value,
            min,
            max,
        }))
    }
}

struct BoundedFloatError {
    name: &'static str,
    value: f64,
    min: f64,
    max: f64,
}

impl fmt::Display for BoundedFloatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} must be finite and between {} and {}, got {}",
            self.name, self.min, self.max, self.value
        )
    }
}

define_settings_group!(TuiZeroStateSettings, settings: [
    object: TuiZeroStateObjectSetting {
        type: TuiZeroStateObject,
        default: TuiZeroStateObject::default(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::TUI,
        private: false,
        toml_path: "appearance.zero_state.object",
        max_table_depth: 0,
        description: "The object rotated in the Warp Agent CLI zero state. Use built_in or an ascii_file path relative to the Warp Agent CLI settings directory. Changing this setting reloads the object; editing the linked file requires a restart.",
    },
    rotation_period_seconds: TuiZeroStateRotationPeriodSecondsSetting {
        type: TuiZeroStateRotationPeriodSeconds,
        default: TuiZeroStateRotationPeriodSeconds::default(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::TUI,
        private: false,
        toml_path: "appearance.zero_state.rotation_period_seconds",
        description: "Seconds per Warp Agent CLI zero-state object rotation, from 1 through 60.",
    },
    extrusion_depth: TuiZeroStateExtrusionDepthSetting {
        type: TuiZeroStateExtrusionDepth,
        default: TuiZeroStateExtrusionDepth::default(),
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::TUI,
        private: false,
        toml_path: "appearance.zero_state.extrusion_depth",
        description: "Normalized half-depth of the extruded Warp Agent CLI zero-state object, from 0.02 through 0.5.",
    },
]);

#[cfg(test)]
#[path = "tui_zero_state_tests.rs"]
mod tests;
