use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Serialize;
use warp_cli::agent::OutputFormat;
use warp_cli::artifact::{
    ArtifactCommand, DownloadArtifactArgs, GetArtifactArgs, UploadArtifactArgs,
};
use warp_cli::GlobalOptions;
use warpui::platform::TerminationMode;
use warpui::{AppContext, ModelContext, SingletonEntity};

use super::artifact_upload::{
    CompletedFileArtifactUpload, FileArtifactUploadRequest, FileArtifactUploader,
};
use crate::ai::artifact_download::{download_artifact_bytes, download_destination};
#[cfg(test)]
use crate::server::server_api::ai::FileArtifactRecord;
use crate::server::server_api::ai::{AIClient, ArtifactDownloadResponse};
use crate::server::server_api::{ServerApi, ServerApiProvider};

/// Run artifact-related commands.
pub fn run(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: ArtifactCommand,
) -> Result<()> {
    let runner = ctx.add_singleton_model(|_| ArtifactCommandRunner);
    match command {
        ArtifactCommand::Upload(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.upload(args, global_options.output_format, ctx);
            });
            Ok(())
        }
        ArtifactCommand::Get(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.get(args, global_options.output_format, ctx);
            });
            Ok(())
        }
        ArtifactCommand::Download(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.download(args, global_options.output_format, ctx);
            });
            Ok(())
        }
    }
}

struct ArtifactCommandRunner;

impl ArtifactCommandRunner {
    fn get(
        &self,
        args: GetArtifactArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();

        ctx.spawn(
            async move { get_artifact(ai_client, &args.artifact_uid).await },
            move |_, result, ctx| match result {
                Ok(artifact) => {
                    if let Err(err) = write_get_output(&artifact, output_format) {
                        super::report_fatal_error(err, ctx);
                        return;
                    }
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => super::report_fatal_error(err, ctx),
            },
        );
    }

    fn download(
        &self,
        args: DownloadArtifactArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let server_api = ServerApiProvider::as_ref(ctx).get();

        ctx.spawn(
            async move { download_artifact(ai_client, server_api, args).await },
            move |_, result, ctx| match result {
                Ok(output) => {
                    if let Err(err) = write_download_output(&output, output_format) {
                        super::report_fatal_error(err, ctx);
                        return;
                    }
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => super::report_fatal_error(err, ctx),
            },
        );
    }

    fn upload(
        &self,
        args: UploadArtifactArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_api = ServerApiProvider::as_ref(ctx).get();
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let uploader = FileArtifactUploader::new(ai_client, server_api.clone());

        ctx.spawn(
            async move {
                let request = FileArtifactUploadRequest::try_from(args)?;
                let association = uploader.resolve_upload_association(&request).await?;
                server_api.set_ambient_agent_task_id(Some(association.ambient_task_id));
                uploader.upload_with_association(request, association).await
            },
            move |_, result, ctx| match result {
                Ok(artifact) => {
                    if let Err(err) = write_upload_output(&artifact, output_format) {
                        super::report_fatal_error(err, ctx);
                        return;
                    }
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => super::report_fatal_error(err, ctx),
            },
        );
    }
}

impl warpui::Entity for ArtifactCommandRunner {
    type Event = ();
}

impl SingletonEntity for ArtifactCommandRunner {}

async fn get_artifact(
    ai_client: Arc<dyn AIClient>,
    artifact_uid: &str,
) -> Result<ArtifactDownloadResponse> {
    ai_client
        .get_artifact_download(artifact_uid)
        .await
        .with_context(|| format!("Failed to get artifact '{artifact_uid}'"))
}

async fn download_artifact(
    ai_client: Arc<dyn AIClient>,
    server_api: Arc<ServerApi>,
    args: DownloadArtifactArgs,
) -> Result<DownloadArtifactOutput> {
    let artifact = get_artifact(ai_client, &args.artifact_uid).await?;
    let path = download_destination(&artifact, args.out);
    download_artifact_bytes(server_api.http_client(), &artifact, &path).await?;
    let path = std::path::absolute(&path).unwrap_or(path);
    Ok(DownloadArtifactOutput::new(&artifact, path))
}

#[derive(Debug, Serialize)]
struct ArtifactMetadataOutput {
    artifact_uid: String,
    artifact_type: String,
    created_at: String,
    download_url: String,
    expires_at: String,
    content_type: String,
    filepath: Option<String>,
    filename: Option<String>,
    description: Option<String>,
    size_bytes: Option<i64>,
}

impl ArtifactMetadataOutput {
    fn new(artifact: &ArtifactDownloadResponse) -> Self {
        Self {
            artifact_uid: artifact.artifact_uid().to_string(),
            artifact_type: artifact.artifact_type().to_string(),
            created_at: artifact.created_at().to_rfc3339(),
            download_url: artifact.download_url().to_string(),
            expires_at: artifact.expires_at().to_rfc3339(),
            content_type: artifact.content_type().to_string(),
            filepath: artifact.filepath().map(ToString::to_string),
            filename: artifact.filename().map(ToString::to_string),
            description: artifact.description().map(ToString::to_string),
            size_bytes: artifact.size_bytes(),
        }
    }
}

#[derive(Debug, Serialize)]
struct DownloadArtifactOutput {
    artifact_uid: String,
    artifact_type: String,
    path: PathBuf,
}

impl DownloadArtifactOutput {
    fn new(artifact: &ArtifactDownloadResponse, path: PathBuf) -> Self {
        Self {
            artifact_uid: artifact.artifact_uid().to_string(),
            artifact_type: artifact.artifact_type().to_string(),
            path,
        }
    }
}

#[derive(Debug, Serialize)]
struct UploadArtifactOutput {
    artifact_uid: String,
    filepath: String,
    description: Option<String>,
    mime_type: String,
    size_bytes: Option<i64>,
}

fn write_get_output(
    artifact: &ArtifactDownloadResponse,
    output_format: OutputFormat,
) -> Result<()> {
    let mut stdout = std::io::stdout();
    write_get_output_to(&mut stdout, artifact, output_format)
}

fn write_get_output_to<W: std::io::Write>(
    output: &mut W,
    artifact: &ArtifactDownloadResponse,
    output_format: OutputFormat,
) -> Result<()> {
    let output_record = ArtifactMetadataOutput::new(artifact);

    match output_format {
        OutputFormat::Json | OutputFormat::Ndjson => {
            serde_json::to_writer(&mut *output, &output_record)
                .context("unable to write JSON output")?;
            writeln!(&mut *output)?;
        }
        OutputFormat::Pretty => {
            writeln!(&mut *output, "Artifact UID: {}", output_record.artifact_uid)?;
            writeln!(
                &mut *output,
                "Artifact type: {}",
                output_record.artifact_type
            )?;
            writeln!(&mut *output, "Created at: {}", output_record.created_at)?;
            writeln!(&mut *output, "Download URL: {}", output_record.download_url)?;
            writeln!(&mut *output, "Expires at: {}", output_record.expires_at)?;
            writeln!(&mut *output, "Content type: {}", output_record.content_type)?;
            if let Some(filepath) = output_record.filepath {
                writeln!(&mut *output, "Filepath: {filepath}")?;
            }
            if let Some(filename) = output_record.filename {
                writeln!(&mut *output, "Filename: {filename}")?;
            }
            if let Some(description) = output_record.description {
                writeln!(&mut *output, "Description: {description}")?;
            }
            if let Some(size_bytes) = output_record.size_bytes {
                writeln!(&mut *output, "Size bytes: {size_bytes}")?;
            }
        }
        OutputFormat::Text => {
            writeln!(
                &mut *output,
                "Artifact UID\tArtifact type\tCreated at\tDownload URL\tExpires at\tContent type\tFilepath\tFilename\tDescription\tSize bytes"
            )?;
            writeln!(
                &mut *output,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                output_record.artifact_uid,
                output_record.artifact_type,
                output_record.created_at,
                output_record.download_url,
                output_record.expires_at,
                output_record.content_type,
                output_record.filepath.unwrap_or_default(),
                output_record.filename.unwrap_or_default(),
                output_record.description.unwrap_or_default(),
                output_record
                    .size_bytes
                    .map(|size| size.to_string())
                    .unwrap_or_default()
            )?;
        }
    }

    Ok(())
}

fn write_download_output(
    output_record: &DownloadArtifactOutput,
    output_format: OutputFormat,
) -> Result<()> {
    let mut stdout = std::io::stdout();
    write_download_output_to(&mut stdout, output_record, output_format)
}

fn write_download_output_to<W: std::io::Write>(
    output: &mut W,
    output_record: &DownloadArtifactOutput,
    output_format: OutputFormat,
) -> Result<()> {
    match output_format {
        OutputFormat::Json | OutputFormat::Ndjson => {
            serde_json::to_writer(&mut *output, output_record)
                .context("unable to write JSON output")?;
            writeln!(&mut *output)?;
        }
        OutputFormat::Pretty => {
            writeln!(&mut *output, "Artifact downloaded")?;
            writeln!(&mut *output, "Artifact UID: {}", output_record.artifact_uid)?;
            writeln!(
                &mut *output,
                "Artifact type: {}",
                output_record.artifact_type
            )?;
            writeln!(&mut *output, "Path: {}", output_record.path.display())?;
        }
        OutputFormat::Text => {
            writeln!(&mut *output, "Artifact UID\tArtifact type\tPath")?;
            writeln!(
                &mut *output,
                "{}\t{}\t{}",
                output_record.artifact_uid,
                output_record.artifact_type,
                output_record.path.display()
            )?;
        }
    }

    Ok(())
}

fn write_upload_output(
    artifact: &CompletedFileArtifactUpload,
    output_format: OutputFormat,
) -> Result<()> {
    let mut stdout = std::io::stdout();
    write_upload_output_to(&mut stdout, artifact, output_format)
}

fn write_upload_output_to<W: std::io::Write>(
    output: &mut W,
    artifact: &CompletedFileArtifactUpload,
    output_format: OutputFormat,
) -> Result<()> {
    let output_record = UploadArtifactOutput {
        artifact_uid: artifact.artifact.artifact_uid.clone(),
        filepath: artifact.artifact.filepath.clone(),
        description: artifact.artifact.description.clone(),
        mime_type: artifact.artifact.mime_type.clone(),
        size_bytes: Some(artifact.size_bytes),
    };

    match output_format {
        OutputFormat::Json | OutputFormat::Ndjson => {
            serde_json::to_writer(&mut *output, &output_record)
                .context("unable to write JSON output")?;
            writeln!(&mut *output)?;
        }
        OutputFormat::Pretty => {
            writeln!(&mut *output, "Artifact uploaded")?;
            writeln!(&mut *output, "Artifact UID: {}", output_record.artifact_uid)?;
            writeln!(&mut *output, "Filepath: {}", output_record.filepath)?;
            writeln!(
                &mut *output,
                "Description: {}",
                output_record.description.as_deref().unwrap_or("")
            )?;
            writeln!(&mut *output, "MIME type: {}", output_record.mime_type)?;
            writeln!(
                &mut *output,
                "Size bytes: {}",
                output_record
                    .size_bytes
                    .map(|size| size.to_string())
                    .unwrap_or_default()
            )?;
        }
        OutputFormat::Text => {
            writeln!(
                &mut *output,
                "Artifact UID\tFilepath\tDescription\tMIME type\tSize bytes"
            )?;
            writeln!(
                &mut *output,
                "{}\t{}\t{}\t{}\t{}",
                output_record.artifact_uid,
                output_record.filepath,
                output_record.description.unwrap_or_default(),
                output_record.mime_type,
                output_record
                    .size_bytes
                    .map(|size| size.to_string())
                    .unwrap_or_default()
            )?;
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "artifact_tests.rs"]
mod tests;
