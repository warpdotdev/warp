//! Functionality relating to overrides of the default per-channel version
//! info.
//!
//! For example, we may want to roll out a hotfix release for Linux, but
//! not want macOS and Windows users to need to perform another update.

use serde::{Deserialize, Serialize};

use crate::VersionInfo;

/// The set of contextual information that is relevant for applying per-version
/// overrides.
pub struct Context {
    pub target_os: Option<TargetOS>,
}

impl Context {
    pub fn from_env() -> Self {
        Context {
            target_os: TargetOS::current(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum), clap(rename_all = "lower"))]
pub enum TargetOS {
    #[serde(rename = "macos")]
    MacOS,
    #[serde(rename = "linux")]
    Linux,
    #[serde(rename = "windows")]
    Windows,
    #[serde(rename = "web")]
    Web,

    // Catch-all in case we we can't deserialize an unrecognized enum variant.
    // We need [value(skip)] here to tell `clap` to ignore this variant.
    #[serde(untagged)]
    #[cfg_attr(feature = "cli", value(skip))]
    Unknown(String),
}

impl TargetOS {
    /// Returns the current operating system, based on the build-time target_os
    /// cfg variable, or None if it is not supported.
    pub fn current() -> Option<Self> {
        if cfg!(target_family = "wasm") {
            Some(TargetOS::Web)
        } else if cfg!(target_os = "macos") {
            Some(TargetOS::MacOS)
        } else if cfg!(any(target_os = "linux", target_os = "freebsd")) {
            Some(TargetOS::Linux)
        } else if cfg!(target_os = "windows") {
            Some(TargetOS::Windows)
        } else {
            None
        }
    }

    /// Returns the name of the [`TargetOS`], or None if it is unknown.
    pub fn name(&self) -> Option<String> {
        let name = match self {
            TargetOS::MacOS => "MacOS".to_owned(),
            TargetOS::Linux => "Linux".to_owned(),
            TargetOS::Windows => "Windows".to_owned(),
            TargetOS::Web => "Web".to_owned(),
            _ => return None,
        };
        Some(name)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
enum OverridePredicate {
    #[serde(rename = "target_os")]
    TargetOS(TargetOS),
}

impl OverridePredicate {
    fn matches(&self, context: &Context) -> bool {
        match self {
            OverridePredicate::TargetOS(os) => {
                if let Some(target_os) = &context.target_os {
                    os == target_os
                } else {
                    false
                }
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct VersionOverride {
    /// The predicate which determines whether or not this override should be
    /// applied.
    predicate: OverridePredicate,
    /// The overridden version info.
    version_info: VersionInfo,
}

impl VersionInfo {
    /// Returns a copy of this [`VersionInfo`] with the first matching override
    /// applied, if any match.
    pub fn with_overrides_applied(&self, overrides: &[VersionOverride], context: &Context) -> Self {
        let mut new = self.clone();
        for version_override in overrides {
            if version_override.predicate.matches(context) {
                new.apply_override(version_override.version_info.clone());
                // We only apply the first matching override, and skip the rest.
                break;
            }
        }
        new
    }

    fn apply_override(&mut self, other: VersionInfo) {
        self.version = other.version;
        if let Some(soft_cutoff) = other.soft_cutoff {
            self.soft_cutoff = Some(soft_cutoff);
        }
        if let Some(update_by) = other.update_by {
            self.update_by = Some(update_by);
        }
        if let Some(last_prominent_update) = other.last_prominent_update {
            self.last_prominent_update = Some(last_prominent_update);
        }
        if let Some(cli_version) = other.cli_version {
            self.cli_version = Some(cli_version);
        }
    }
}

#[cfg(test)]
#[path = "overrides_tests.rs"]
mod tests;
