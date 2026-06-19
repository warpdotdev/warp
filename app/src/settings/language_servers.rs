//! Settings registration for user-configured LSP servers.
//!
//! Users write `[[editor.language_servers]]` entries in their `settings.toml`;
//! those entries flow through Warp's standard settings infrastructure
//! (`define_settings_group!`) into the runtime model exposed here.

use std::path::Path;

use lsp::descriptor::matcher::{match_descriptor, LspMatchedDescriptor};
use lsp::descriptor::{self, LspServerDescriptor};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use settings::macros::define_settings_group;
use settings::{Setting, SupportedPlatforms, SyncToCloud};
use settings_value::SettingsValue;

// Hand-rolls `SettingsValue::from_file_value` so deserialization routes
// through `descriptor::parse::parse_entries`, which compiles real
// `globset::GlobMatcher` values from each `pattern` string. The default
// blanket impl (`Vec<T>: SettingsValue`) goes via `serde_json::from_value`,
// which leaves the private `matcher` field at its `#[serde(skip)]`
// placeholder — every pattern would then silently match nothing at runtime.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct LspServerDescriptors(pub Vec<LspServerDescriptor>);

impl SettingsValue for LspServerDescriptors {
    fn from_file_value(value: &Value) -> Option<Self> {
        let arr = value.as_array()?;
        let result = descriptor::parse::parse_entries(arr);
        // Mirror the all-or-nothing semantics of the blanket
        // `Vec<T>: SettingsValue` impl (`crates/settings_value/src/lib.rs`):
        // any invalid entry fails the whole setting, and the macro's load_fn
        // surfaces the storage key in `SettingsFileError::InvalidSettings`.
        // Per-entry reasons go to the log so they are findable when the
        // user investigates the banner.
        if !result.errors.is_empty() {
            for err in &result.errors {
                log::warn!("editor.language_servers: {err}");
            }
            return None;
        }
        Some(Self(result.descriptors))
    }
}

define_settings_group!(LanguageServersSettings, settings: [
    language_servers: LanguageServers {
        type: LspServerDescriptors,
        default: LspServerDescriptors::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "editor.language_servers",
        description: "User-configured language servers for the editor.",
    },
]);

impl LanguageServersSettings {
    /// Returns the first user-configured descriptor whose `filetypes` matches
    /// `path`, or `None` if no custom descriptor claims the file.
    pub fn match_for_path<'a>(&'a self, path: &Path) -> Option<LspMatchedDescriptor<'a>> {
        match_descriptor(&self.language_servers.value().0, path)
    }
}

#[cfg(test)]
#[path = "language_servers_tests.rs"]
mod tests;
