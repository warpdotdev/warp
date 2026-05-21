use std::path::Path;
#[cfg(feature = "local_fs")]
use std::path::PathBuf;

use anyhow::anyhow;
use ui_components::lightbox::{LightboxImage, LightboxImageSource};
use warp_core::report_error;
use warp_multi_agent_api as api;
#[cfg(feature = "local_fs")]
use warpui::platform::SaveFilePickerConfiguration;
use warpui::SingletonEntity;

#[cfg(feature = "local_fs")]
use crate::ai::artifact_download::default_download_filename;
use crate::ai::artifact_download::sanitized_basename;
#[cfg(feature = "local_fs")]
use crate::ai::artifact_download::{default_download_directory, download_artifact_bytes};
use crate::notebooks::NotebookId;
use crate::server::server_api::ai::ArtifactDownloadResponse;
use crate::server::server_api::ServerApiProvider;
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;
use crate::workspace::WorkspaceAction;

pub mod buttons;
pub use buttons::{ArtifactButtonsRow, ArtifactButtonsRowEvent};

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "artifact_type", content = "data")]
pub enum Artifact {
    #[serde(rename = "PLAN")]
    Plan {
        document_uid: String,
        /// None until the plan is synced to Warp Drive.
        notebook_uid: Option<NotebookId>,
        title: Option<String>,
    },
    #[serde(rename = "PULL_REQUEST")]
    PullRequest {
        url: String,
        branch: String,
        #[serde(skip_serializing)] // We derive this field from the url on deserialize
        repo: Option<String>,
        #[serde(skip_serializing)] // We derive this field from the url on deserialize
        number: Option<u32>,
    },
    #[serde(rename = "SCREENSHOT")]
    Screenshot {
        artifact_uid: String,
        mime_type: String,
        description: Option<String>,
    },
    #[serde(rename = "FILE")]
    File {
        artifact_uid: String,
        filepath: String,
        filename: String,
        mime_type: String,
        description: Option<String>,
        size_bytes: Option<i32>,
    },
}

#[derive(serde::Deserialize)]
#[serde(tag = "artifact_type", content = "data")]
enum ArtifactHelper {
    #[serde(rename = "PLAN")]
    Plan {
        document_uid: String,
        notebook_uid: Option<NotebookId>,
        title: Option<String>,
    },
    #[serde(rename = "PULL_REQUEST")]
    PullRequest { url: String, branch: String },
    #[serde(rename = "SCREENSHOT")]
    Screenshot {
        artifact_uid: String,
        mime_type: String,
        description: Option<String>,
    },
    #[serde(rename = "FILE")]
    File {
        artifact_uid: String,
        filepath: String,
        filename: String,
        mime_type: String,
        description: Option<String>,
        size_bytes: Option<i32>,
    },
}

impl<'de> serde::Deserialize<'de> for Artifact {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let helper = ArtifactHelper::deserialize(deserializer)?;
        Ok(match helper {
            ArtifactHelper::Plan {
                document_uid,
                notebook_uid,
                title,
            } => Artifact::Plan {
                document_uid,
                notebook_uid,
                title,
            },
            ArtifactHelper::PullRequest { url, branch } => {
                let (repo, number) = parse_github_pr_url(&url).unzip();
                Artifact::PullRequest {
                    url,
                    branch,
                    repo,
                    number,
                }
            }
            ArtifactHelper::Screenshot {
                artifact_uid,
                mime_type,
                description,
            } => Artifact::Screenshot {
                artifact_uid,
                mime_type,
                description,
            },
            ArtifactHelper::File {
                artifact_uid,
                filepath,
                filename,
                mime_type,
                description,
                size_bytes,
            } => Artifact::File {
                artifact_uid,
                filepath,
                filename,
                mime_type,
                description,
                size_bytes,
            },
        })
    }
}

impl From<api::message::artifact_event::PullRequestArtifact> for Artifact {
    fn from(pr: api::message::artifact_event::PullRequestArtifact) -> Self {
        let (repo, number) = parse_github_pr_url(&pr.url).unzip();
        Artifact::PullRequest {
            url: pr.url,
            branch: pr.branch,
            repo,
            number,
        }
    }
}

impl From<api::message::artifact_event::ScreenshotArtifact> for Artifact {
    fn from(screenshot: api::message::artifact_event::ScreenshotArtifact) -> Self {
        Artifact::Screenshot {
            artifact_uid: screenshot.artifact_uid,
            mime_type: screenshot.mime_type,
            description: if screenshot.description.is_empty() {
                None
            } else {
                Some(screenshot.description)
            },
        }
    }
}

impl From<api::message::artifact_event::FileArtifact> for Artifact {
    fn from(file: api::message::artifact_event::FileArtifact) -> Self {
        Artifact::File {
            artifact_uid: file.artifact_uid,
            filepath: file.filepath.clone(),
            filename: Path::new(&file.filepath)
                .file_name()
                .and_then(|file_name| file_name.to_str())
                .filter(|file_name| !file_name.trim().is_empty())
                .unwrap_or("File")
                .to_string(),
            mime_type: file.mime_type,
            description: if file.description.is_empty() {
                None
            } else {
                Some(file.description)
            },
            size_bytes: i32::try_from(file.size_bytes).ok(),
        }
    }
}

impl From<api::message::artifact_event::PlanArtifact> for Artifact {
    fn from(plan: api::message::artifact_event::PlanArtifact) -> Self {
        Artifact::Plan {
            document_uid: plan.document_id,
            notebook_uid: if plan.notebook_uid.is_empty() {
                None
            } else {
                Some(NotebookId::from(plan.notebook_uid))
            },
            title: if plan.title.is_empty() {
                None
            } else {
                Some(plan.title)
            },
        }
    }
}

impl TryFrom<warp_graphql::ai::AIConversationArtifact> for Artifact {
    type Error = ();

    fn try_from(value: warp_graphql::ai::AIConversationArtifact) -> Result<Self, Self::Error> {
        match value {
            warp_graphql::ai::AIConversationArtifact::PlanArtifact(plan) => Ok(Artifact::Plan {
                document_uid: plan.document_uid.into_inner(),
                notebook_uid: plan
                    .notebook_uid
                    .map(|id| NotebookId::from(id.into_inner())),
                title: plan.title,
            }),
            warp_graphql::ai::AIConversationArtifact::PullRequestArtifact(pr) => {
                let (repo, number) = parse_github_pr_url(&pr.url).unzip();
                Ok(Artifact::PullRequest {
                    url: pr.url,
                    branch: pr.branch,
                    repo,
                    number,
                })
            }
            warp_graphql::ai::AIConversationArtifact::ScreenshotArtifact(screenshot) => {
                Ok(Artifact::Screenshot {
                    artifact_uid: screenshot.artifact_uid.into_inner(),
                    mime_type: screenshot.mime_type,
                    description: screenshot.description,
                })
            }
            warp_graphql::ai::AIConversationArtifact::FileArtifact(file) => Ok(Artifact::File {
                artifact_uid: file.artifact_uid.into_inner(),
                filepath: file.filepath.clone(),
                filename: sanitized_basename(&file.filepath).unwrap_or(file.filepath),
                mime_type: file.mime_type,
                description: file.description,
                size_bytes: file.size_bytes,
            }),
            warp_graphql::ai::AIConversationArtifact::Unknown => Err(()),
        }
    }
}

/// Parse GitHub PR URL to extract repo and number.
/// Expected format: https://github.com/{owner}/{repo}/pull/{number}
pub fn parse_github_pr_url(url: &str) -> Option<(String, u32)> {
    if !url.contains("github.com") {
        return None;
    }
    let segments: Vec<&str> = url.split('/').collect();
    segments.windows(3).find_map(|w| {
        if w[1] != "pull" {
            return None;
        }
        Some((w[0].to_string(), w[2].parse().ok()?))
    })
}

/// Deserialize artifacts, skipping any that fail to parse.
/// This ensures task loading doesn't fail entirely if an artifact has an unknown format.
pub fn deserialize_artifacts<'de, D>(deserializer: D) -> Result<Vec<Artifact>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values: Vec<serde_json::Value> = serde::Deserialize::deserialize(deserializer)?;
    Ok(values
        .into_iter()
        .filter_map(|value| match serde_json::from_value::<Artifact>(value) {
            Ok(artifact) => Some(artifact),
            Err(e) => {
                report_error!(anyhow!("Failed to deserialize artifact, skipping: {}", e));
                None
            }
        })
        .collect())
}

pub fn file_button_label(filename: &str, filepath: &str) -> String {
    if let Some(filename) = non_empty_trimmed(filename) {
        return filename.to_string();
    }
    if let Some(filepath_basename) = sanitized_basename(filepath)
        .as_deref()
        .and_then(non_empty_trimmed)
    {
        return filepath_basename.to_string();
    }
    "File".to_string()
}

pub fn open_screenshot_lightbox<V: warpui::View>(
    artifact_uids: &[String],
    ctx: &mut warpui::ViewContext<V>,
) {
    // Open lightbox immediately with Loading placeholders.
    let loading_images: Vec<LightboxImage> = artifact_uids
        .iter()
        .map(|_| LightboxImage {
            source: LightboxImageSource::Loading,
            description: None,
        })
        .collect();
    ctx.dispatch_typed_action(&WorkspaceAction::OpenLightbox {
        images: loading_images,
        initial_index: 0,
    });

    // Fetch each signed URL independently and update the lightbox as each resolves.
    // TODO(QUALITY-318): We should cache the signed URL for each artifact UUID so
    // we avoid fetching screenshots already in the asset cache.
    let ai_client = ServerApiProvider::handle(ctx).as_ref(ctx).get_ai_client();

    for (i, uid) in artifact_uids.iter().enumerate() {
        let ai_client = ai_client.clone();
        let uid = uid.clone();
        let uid_for_callback = uid.clone();

        ctx.spawn(
            async move { ai_client.get_artifact_download(&uid).await },
            move |_me, result, ctx| {
                if let Some(image) =
                    screenshot_lightbox_image_from_download_result(result, &uid_for_callback, i)
                {
                    ctx.dispatch_typed_action(&WorkspaceAction::UpdateLightboxImage {
                        index: i,
                        image,
                    });
                }
            },
        );
    }
}

fn screenshot_lightbox_image_from_download_result(
    result: anyhow::Result<ArtifactDownloadResponse>,
    uid_for_callback: &str,
    index: usize,
) -> Option<LightboxImage> {
    match result {
        Ok(ArtifactDownloadResponse::Screenshot { data, .. }) => Some(LightboxImage {
            source: LightboxImageSource::Resolved {
                asset_source: asset_cache::url_source(data.download_url),
            },
            description: data
                .description
                .filter(|description| !description.is_empty()),
        }),
        Ok(ArtifactDownloadResponse::File { .. }) => {
            log::warn!("Artifact {uid_for_callback} was not a screenshot");
            None
        }
        Err(e) => {
            log::warn!("Failed to load screenshot artifact {index}: {e}");
            Some(LightboxImage {
                source: LightboxImageSource::Loading,
                description: Some("Failed to load".to_string()),
            })
        }
    }
}

pub fn download_file_artifact<V: warpui::View>(
    artifact_uid: &str,
    ctx: &mut warpui::ViewContext<V>,
) {
    let ai_client = ServerApiProvider::handle(ctx).as_ref(ctx).get_ai_client();
    let artifact_uid = artifact_uid.to_string();
    let artifact_uid_for_request = artifact_uid.clone();

    ctx.spawn(
        async move {
            ai_client
                .get_artifact_download(&artifact_uid_for_request)
                .await
        },
        move |_me, result, ctx| match result {
            Ok(artifact) => open_file_download_result(&artifact_uid, artifact, ctx),
            Err(error) => {
                log::warn!("Failed to load file artifact {artifact_uid}: {error}");
                show_file_download_toast(
                    &artifact_uid,
                    DismissibleToast::error(
                        crate::i18n::tr_static(ctx, "Failed to prepare file download.").to_string(),
                    ),
                    ctx,
                );
            }
        },
    );
}

fn open_file_download_result<V: warpui::View>(
    artifact_uid: &str,
    artifact: ArtifactDownloadResponse,
    ctx: &mut warpui::ViewContext<V>,
) {
    match artifact {
        ArtifactDownloadResponse::File { .. } => {
            #[cfg(feature = "local_fs")]
            {
                open_file_download_picker(artifact, ctx);
            }

            #[cfg(not(feature = "local_fs"))]
            {
                ctx.open_url(artifact.download_url());
            }
        }
        ArtifactDownloadResponse::Screenshot { .. } => {
            log::warn!("Artifact {artifact_uid} was not a file");
        }
    }
}

#[cfg(feature = "local_fs")]
fn open_file_download_picker<V: warpui::View>(
    artifact: ArtifactDownloadResponse,
    ctx: &mut warpui::ViewContext<V>,
) {
    let mut config = SaveFilePickerConfiguration::new()
        .with_default_filename(default_download_filename(&artifact));
    if let Some(default_directory) = default_download_directory() {
        config = config.with_default_directory(default_directory);
    }

    ctx.open_save_file_picker(
        move |path_opt: Option<String>, _me: &mut V, ctx: &mut warpui::ViewContext<V>| {
            let Some(path) = path_opt else {
                return;
            };
            let server_api = ServerApiProvider::handle(ctx).as_ref(ctx).get();
            let artifact = artifact.clone();
            let artifact_uid = artifact.artifact_uid().to_string();
            let path = PathBuf::from(path);
            let toast_filename = download_toast_filename(&path);
            let artifact_for_download = artifact.clone();
            ctx.spawn(
                async move {
                    download_artifact_bytes(server_api.http_client(), &artifact_for_download, &path)
                        .await
                },
                move |_me, result, ctx| match result {
                    Ok(()) => show_file_download_toast(
                        &artifact_uid,
                        DismissibleToast::success(format!("Downloaded {toast_filename}.")),
                        ctx,
                    ),
                    Err(error) => {
                        log::warn!("Failed to download file artifact {artifact_uid}: {error}");
                        show_file_download_toast(
                            &artifact_uid,
                            DismissibleToast::error(format!(
                                "Failed to download {toast_filename}."
                            )),
                            ctx,
                        );
                    }
                },
            );
        },
        config,
    );
}

fn show_file_download_toast<V: warpui::View>(
    artifact_uid: &str,
    toast: DismissibleToast<WorkspaceAction>,
    ctx: &mut warpui::ViewContext<V>,
) {
    let toast_id = format!("artifact_download:{artifact_uid}");
    let window_id = ctx.window_id();
    ToastStack::handle(ctx).update(ctx, move |toast_stack, ctx| {
        toast_stack.add_ephemeral_toast(toast.with_object_id(toast_id), window_id, ctx);
    });
}

#[cfg(feature = "local_fs")]
fn download_toast_filename(path: &Path) -> String {
    path.file_name()
        .and_then(|file_name| file_name.to_str())
        .filter(|file_name| !file_name.is_empty())
        .unwrap_or("file")
        .to_string()
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
