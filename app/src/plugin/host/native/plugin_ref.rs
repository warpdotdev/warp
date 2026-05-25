use std::path::PathBuf;
use std::{fs, io};

pub(super) const PLUGIN_ENTRYPOINT_JS_FILE_NAME: &str = "main.js";

#[derive(thiserror::Error, Debug)]
pub(super) enum PluginLoadError {
    #[error("Failed to load plugin: {0:?}")]
    File(#[from] io::Error),

    #[error("Missing source for builtin plugin: {0:?}")]
    MissingBuiltin(BuiltInPluginType),
}

/// Represents "Built-in" plugins. Each variant corresponds to a plugin bundled with Warp by
/// default (e.g. Completions/Command Signatures)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum BuiltInPluginType {
    Completions,
}

impl BuiltInPluginType {
    #[cfg(feature = "completions_v2")]
    pub(super) fn plugin_bytes(&self) -> Option<Vec<u8>> {
        match self {
            BuiltInPluginType::Completions => {
                command_signatures_v2::CommandSignaturesJs::get("main.js")
                    .map(|bytes| bytes.data.into())
            }
        }
    }

    #[cfg(not(feature = "completions_v2"))]
    pub(super) fn plugin_bytes(&self) -> Option<Vec<u8>> {
        None
    }
}

/// References a single plugin.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum PluginRef {
    /// Refers to plugin source on disk.
    Path(PathBuf),

    /// Refers to a "built-in" plugin bundled with the Warp binary.
    BuiltIn(BuiltInPluginType),
}

impl PluginRef {
    pub(super) fn plugin_bytes(&self) -> Result<Vec<u8>, PluginLoadError> {
        match self {
            PluginRef::Path(path) => {
                let entrypoint_file_path = path.join(PLUGIN_ENTRYPOINT_JS_FILE_NAME);
                fs::read(entrypoint_file_path).map_err(PluginLoadError::from)
            }
            PluginRef::BuiltIn(builtin_plugin_type) => match builtin_plugin_type.plugin_bytes() {
                Some(bytes) => Ok(bytes),
                None => Err(PluginLoadError::MissingBuiltin(*builtin_plugin_type)),
            },
        }
    }
}
