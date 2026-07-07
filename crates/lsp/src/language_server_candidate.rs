use std::path::Path;

use async_trait::async_trait;

use crate::CommandBuilder;

/// Defines the detection and installation for a specific Language Server.
///
/// This trait allows us to decouple the specific logic for each server (installation,
/// detection, etc.) from the main application logic.
#[async_trait]
pub trait LanguageServerCandidate: Send + Sync {
    /// Heuristic to determine if this server is relevant for the repo at the given path.
    ///
    /// For example, a Rust server might check for `Cargo.toml` or `*.rs` files.
    /// The executor is provided for servers that need to check if a runtime is available
    /// (e.g. gopls checks if `go` is installed).
    async fn should_suggest_for_repo(&self, path: &Path, executor: &CommandBuilder) -> bool;

    /// Checks if the server binary is installed in our custom data directory.
    ///
    /// The executor is provided for servers that need to locate runtime dependencies
    /// (e.g. pyright needs to find node).
    async fn is_installed_in_data_dir(&self, executor: &CommandBuilder) -> bool;

    /// Checks if the server binary is available and working on the system PATH.
    ///
    /// Returns true only if the binary executes successfully with exit code 0.
    async fn is_installed_on_path(&self, executor: &CommandBuilder) -> bool;

    /// Checks if the server binary is currently available/executable.
    ///
    /// By default, checks the data directory first, then falls back to PATH.
    async fn is_installed(&self, executor: &CommandBuilder) -> bool {
        // First check if installed in our custom location
        if self.is_installed_in_data_dir(executor).await {
            return true;
        }

        // Fall back to checking PATH
        self.is_installed_on_path(executor).await
    }

    /// Attempts to install the server into the `.warp/` directory.
    ///
    /// The executor provides the user's PATH environment variable, which may be needed
    /// for servers that rely on external tools (e.g. gopls needs `go`).
    async fn install(
        &self,
        metadata: LanguageServerMetadata,
        executor: &CommandBuilder,
    ) -> anyhow::Result<()>;

    async fn fetch_latest_server_metadata(&self) -> anyhow::Result<LanguageServerMetadata>;
}

pub struct LanguageServerMetadata {
    pub version: String,
    /// The download URL for the server binary. None if the server cannot be
    /// downloaded directly (e.g. gopls which must be installed via `go install`).
    pub url: Option<String>,
    pub digest: Option<String>,
}
