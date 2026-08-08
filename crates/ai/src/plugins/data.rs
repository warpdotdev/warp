//! Persistent `PLUGIN_DATA` directories for plugin instances.
//!
//! Agent Plugins §9.1 requires `PLUGIN_DATA` to be outside the package, writable, dedicated to
//! one installed plugin instance, and preserved when the package contents change. The directory
//! is therefore keyed by identity that survives an update — frontend, source, scope, and manifest
//! name — and deliberately excludes the manifest version and any digest of package contents.
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::identity::{PluginInstanceId, filesystem_safe_segment};

/// Absolute durable root a Factory worker provides for plugin data, **with the Factory UID
/// already included** by the server.
///
/// The client appends only `<scope>/<plugin-key>` below it. Composing the UID is deliberately not
/// the client's job: the path shape previously lived as prose in two repositories and the two
/// implementations disagreed, so there is now exactly one place it can be wrong, and it is the
/// side that owns the storage.
///
/// A worker that cannot provide a writable persistent root omits this variable entirely, and the
/// client then refuses to start that plugin's stdio servers rather than falling back to ephemeral
/// storage, which would break the persistence guarantee in §9.1.
pub const PLUGIN_DATA_ROOT_ENV: &str = "WARP_PLUGIN_DATA_ROOT";

/// The Factory UID for the current run.
///
/// Identity and diagnostics only. It is deliberately **never** used for path composition: it is
/// already baked into [`PLUGIN_DATA_ROOT_ENV`], and appending it again would produce a second,
/// divergent layout.
pub const FACTORY_UID_ENV: &str = "WARP_FACTORY_UID";

/// Which front-end owns a plugin runtime instance.
///
/// The GUI and the TUI discover the same packages but do not share running MCP processes or
/// writable plugin state, matching the existing frontend-specific MCP state boundary. Two
/// concurrently running client versions must not mutate one plugin's data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluginFrontend {
    Gui,
    Tui,
}

impl PluginFrontend {
    fn key_token(self) -> &'static str {
        match self {
            PluginFrontend::Gui => "gui",
            PluginFrontend::Tui => "tui",
        }
    }
}

/// Resolves the persistent data directory for a plugin instance.
pub trait PluginDataLocator {
    /// Returns the instance's data directory without creating it.
    fn data_dir(&self, instance: &PluginInstanceId) -> PathBuf;

    /// Creates the instance's data directory and returns it.
    ///
    /// Called immediately before the first stdio start for the instance, never during discovery
    /// or validation.
    fn ensure_data_dir(&self, instance: &PluginInstanceId) -> io::Result<PathBuf> {
        let dir = self.data_dir(instance);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

/// The filesystem-safe key that identifies one plugin instance's data directory.
///
/// Stable across package updates and distinct across frontends, sources, scopes, and names.
pub fn plugin_data_instance_key(frontend: PluginFrontend, instance: &PluginInstanceId) -> String {
    let mut hasher = Sha256::new();
    // Length-prefix each field so that two different splits of the same concatenated bytes
    // cannot collide (e.g. scope "agent/a" + name "b" versus scope "agent" + name "a/b").
    for field in [
        frontend.key_token(),
        &format!("{:?}", instance.source.kind),
        &instance.source.stable_identity,
        &instance.scope.key_token(),
        &instance.manifest_name,
    ] {
        hasher.update(field.len().to_le_bytes());
        hasher.update(field.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut key, byte| {
            let _ = write!(key, "{byte:02x}");
            key
        })
}

/// Locates plugin data under a base directory owned by the active frontend.
#[derive(Debug, Clone)]
pub struct LocalPluginDataLocator {
    base: PathBuf,
    frontend: PluginFrontend,
}

impl LocalPluginDataLocator {
    /// Creates a locator rooted at `<base>/plugins/data`, for **local plugins only**.
    ///
    /// Interactive clients pass `warp_core::paths::data_dir()`. A Factory runtime must not use
    /// this layout: see [`FactoryPluginDataLocator`], which composes the path the worker's
    /// durable root expects instead of nesting this one underneath it.
    pub fn new(base: impl AsRef<Path>, frontend: PluginFrontend) -> Self {
        Self {
            base: base.as_ref().to_path_buf(),
            frontend,
        }
    }

    /// The directory that holds every instance's data for this locator.
    pub fn root(&self) -> PathBuf {
        self.base.join("plugins").join("data")
    }
}

impl PluginDataLocator for LocalPluginDataLocator {
    fn data_dir(&self, instance: &PluginInstanceId) -> PathBuf {
        self.root()
            .join(plugin_data_instance_key(self.frontend, instance))
    }
}

/// Locates plugin data for a Factory run, under the worker's durable root.
///
/// The composed path is exactly `<WARP_PLUGIN_DATA_ROOT>/<scope>/<plugin-key>`. The root already
/// carries the Factory UID, so runs under different Factories cannot collide even though nothing
/// below the root mentions a UID.
///
/// This deliberately does not reuse the local `plugins/data/<hash>` layout. The two are different
/// contracts — one is private to this client, the other is shared with the worker — and nesting
/// the private one under the shared root is the defect this type exists to prevent.
#[derive(Debug, Clone)]
pub struct FactoryPluginDataLocator {
    root: PathBuf,
    factory_uid: Option<String>,
}

impl FactoryPluginDataLocator {
    /// Creates a locator over the worker's durable root.
    ///
    /// `factory_uid` is recorded for diagnostics and never enters the path.
    pub fn new(root: impl AsRef<Path>, factory_uid: Option<String>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            factory_uid,
        }
    }

    /// The Factory UID for the current run, when the worker supplied one.
    pub fn factory_uid(&self) -> Option<&str> {
        self.factory_uid.as_deref()
    }

    /// The durable root, exactly as the worker exported it.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The `<plugin-key>` segment for an instance.
    ///
    /// A conformant manifest name passes through unchanged, so real Factory paths stay legible.
    pub fn plugin_key(instance: &PluginInstanceId) -> String {
        filesystem_safe_segment(&instance.manifest_name)
    }
}

impl PluginDataLocator for FactoryPluginDataLocator {
    fn data_dir(&self, instance: &PluginInstanceId) -> PathBuf {
        // Exactly two segments below the root, each already reduced to a safe name, so no
        // author-supplied value can introduce a separator or a parent reference.
        self.root
            .join(instance.scope.path_segment())
            .join(Self::plugin_key(instance))
    }
}

#[cfg(test)]
#[path = "data_tests.rs"]
mod tests;
