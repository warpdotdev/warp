use std::path::PathBuf;

use clap::{ArgGroup, Args, Subcommand};

/// Artifact-related subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum ArtifactCommand {
    /// Upload an artifact file.
    #[command(hide = true)]
    Upload(UploadArtifactArgs),
    /// Get artifact metadata.
    Get(GetArtifactArgs),
    /// Download an artifact file.
    Download(DownloadArtifactArgs),
}

#[derive(Debug, Clone, Args)]
#[command(
    group(
        ArgGroup::new("artifact_association")
            .multiple(false)
            .args(["run_id", "conversation_id"])
    )
)]
pub struct UploadArtifactArgs {
    /// Path to the artifact file to upload.
    pub path: PathBuf,

    /// Associate the uploaded artifact with a run.
    #[arg(long = "run-id")]
    pub run_id: Option<String>,

    /// Associate the uploaded artifact with a conversation.
    #[arg(long = "conversation-id")]
    pub conversation_id: Option<String>,

    /// Description for the uploaded artifact.
    #[arg(long = "description")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct DownloadArtifactArgs {
    /// UID of the artifact to download.
    pub artifact_uid: String,

    /// Write the downloaded artifact to a specific file path.
    #[arg(long = "out", short = 'o')]
    pub out: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct GetArtifactArgs {
    /// UID of the artifact to get.
    pub artifact_uid: String,
}
