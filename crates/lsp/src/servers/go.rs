use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use crate::CommandBuilder;
#[cfg(feature = "local_fs")]
use crate::install::fetch_latest_metadata_from_github;
use crate::language_server_candidate::{LanguageServerCandidate, LanguageServerMetadata};

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub struct GoPlsCandidate {
    client: Arc<http_client::Client>,
}

impl GoPlsCandidate {
    pub fn new(client: Arc<http_client::Client>) -> Self {
        Self { client }
    }
}

#[async_trait]
#[cfg(feature = "local_fs")]
impl LanguageServerCandidate for GoPlsCandidate {
    async fn should_suggest_for_repo(&self, path: &Path, executor: &CommandBuilder) -> bool {
        if !path.join("go.mod").exists() {
            return false;
        }

        // Check if Go is installed
        executor
            .command("go")
            .arg("version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    async fn is_installed_in_data_dir(&self, _executor: &CommandBuilder) -> bool {
        // gopls doesn't support custom installation yet
        false
    }

    async fn is_installed_on_path(&self, executor: &CommandBuilder) -> bool {
        executor
            .command("gopls")
            .arg("version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    async fn install(
        &self,
        _metadata: LanguageServerMetadata,
        executor: &CommandBuilder,
    ) -> anyhow::Result<()> {
        let output = executor
            .command("go")
            .args(["install", "golang.org/x/tools/gopls@latest"])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Failed to install gopls: {}", stderr);
        }

        Ok(())
    }

    async fn fetch_latest_server_metadata(&self) -> anyhow::Result<LanguageServerMetadata> {
        // gopls doesn't provide prebuilt binaries; it must be installed via `go install`
        fetch_latest_metadata_from_github(&self.client, "golang", "tools", None).await
    }
}

#[async_trait]
#[cfg(not(feature = "local_fs"))]
impl LanguageServerCandidate for GoPlsCandidate {
    async fn should_suggest_for_repo(&self, _path: &Path, _executor: &CommandBuilder) -> bool {
        false
    }

    async fn is_installed_in_data_dir(&self, _executor: &CommandBuilder) -> bool {
        false
    }

    async fn is_installed_on_path(&self, _executor: &CommandBuilder) -> bool {
        false
    }

    async fn install(
        &self,
        _metadata: LanguageServerMetadata,
        _executor: &CommandBuilder,
    ) -> anyhow::Result<()> {
        todo!()
    }

    async fn fetch_latest_server_metadata(&self) -> anyhow::Result<LanguageServerMetadata> {
        todo!()
    }
}
