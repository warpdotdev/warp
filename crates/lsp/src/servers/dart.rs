use std::path::Path;

use async_trait::async_trait;

use crate::CommandBuilder;
use crate::language_server_candidate::{LanguageServerCandidate, LanguageServerMetadata};

pub struct DartAnalysisServerCandidate;

#[async_trait]
#[cfg(feature = "local_fs")]
impl LanguageServerCandidate for DartAnalysisServerCandidate {
    async fn should_suggest_for_repo(&self, path: &Path, executor: &CommandBuilder) -> bool {
        path.join("pubspec.yaml").exists() && self.is_installed_on_path(executor).await
    }

    async fn is_installed_in_data_dir(&self, _executor: &CommandBuilder) -> bool {
        false
    }

    async fn is_installed_on_path(&self, executor: &CommandBuilder) -> bool {
        executor
            .command("dart")
            .args(["language-server", "--help"])
            .output()
            .await
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    async fn install(
        &self,
        _metadata: LanguageServerMetadata,
        _executor: &CommandBuilder,
    ) -> anyhow::Result<()> {
        anyhow::bail!(
            "The Dart Analysis Server is bundled with the Dart SDK. Install Dart or Flutter and ensure `dart` is available on your PATH."
        )
    }

    async fn fetch_latest_server_metadata(&self) -> anyhow::Result<LanguageServerMetadata> {
        Ok(LanguageServerMetadata {
            version: "bundled".to_string(),
            url: None,
            digest: None,
        })
    }
}

#[async_trait]
#[cfg(not(feature = "local_fs"))]
impl LanguageServerCandidate for DartAnalysisServerCandidate {
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
        anyhow::bail!("Dart language support requires local filesystem access")
    }

    async fn fetch_latest_server_metadata(&self) -> anyhow::Result<LanguageServerMetadata> {
        anyhow::bail!("Dart language support requires local filesystem access")
    }
}
