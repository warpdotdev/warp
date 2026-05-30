use std::env;
use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use blocking::unblock;
use warp_cli::artifact::UploadArtifactArgs;

use super::common::parse_ambient_task_id;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::ServerAIConversationMetadata;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::{
    AIClient, CreateFileArtifactUploadRequest, CreateFileArtifactUploadResponse,
    FileArtifactRecord, FileArtifactUploadTargetInfo,
};
use crate::server::server_api::harness_support::FileUploadBody;
use crate::server::server_api::presigned_upload::upload_file_to_target;
use crate::server::server_api::ServerApi;
use crate::util::image::{infer_mime_type, MIME_SNIFF_BYTES};

const OZ_RUN_ID_ENV_VAR: &str = "OZ_RUN_ID";

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct FileArtifactUploadRequest {
    pub(crate) path: PathBuf,
    pub(crate) run_id: Option<AmbientAgentTaskId>,
    pub(crate) conversation_id: Option<ServerConversationToken>,
    pub(crate) description: Option<String>,
}

impl TryFrom<UploadArtifactArgs> for FileArtifactUploadRequest {
    type Error = anyhow::Error;

    fn try_from(value: UploadArtifactArgs) -> Result<Self> {
        let run_id = match value.run_id {
            Some(run_id) => Some(parse_run_id(&run_id, "Invalid run ID")?),
            None => None,
        };

        Ok(Self {
            path: value.path,
            run_id,
            conversation_id: value.conversation_id.map(ServerConversationToken::new),
            description: value.description,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CompletedFileArtifactUpload {
    pub(crate) artifact: FileArtifactRecord,
    pub(crate) size_bytes: i64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct ResolvedUploadAssociation {
    conversation_id: Option<ServerConversationToken>,
    run_id: Option<AmbientAgentTaskId>,
    pub(crate) ambient_task_id: AmbientAgentTaskId,
}

#[derive(Debug, Clone)]
struct PreparedUploadArtifact {
    path: PathBuf,
    filepath: String,
    mime_type: String,
    file_size: u64,
}

impl PreparedUploadArtifact {
    fn from_path(path: PathBuf) -> Result<Self> {
        // `infer` only needs leading signature bytes, so avoid buffering the whole artifact
        // before we stream the file body to the upload target.
        let (file_size, mime_sniff_bytes) = file_size_and_prefix_for_path(&path, MIME_SNIFF_BYTES)?;

        Ok(Self {
            filepath: normalize_artifact_filepath(&path),
            mime_type: infer_mime_type(&path, &mime_sniff_bytes),
            file_size,
            path,
        })
    }

    fn graphql_size_bytes(&self) -> Option<i32> {
        checked_graphql_size_bytes_for_upload(&self.path, self.file_size)
    }
}

pub(crate) struct FileArtifactUploader {
    ai_client: Arc<dyn AIClient>,
    server_api: Arc<ServerApi>,
}

impl FileArtifactUploader {
    pub(crate) fn new(ai_client: Arc<dyn AIClient>, server_api: Arc<ServerApi>) -> Self {
        Self {
            ai_client,
            server_api,
        }
    }

    pub(crate) async fn upload_with_association(
        &self,
        request: FileArtifactUploadRequest,
        association: ResolvedUploadAssociation,
    ) -> Result<CompletedFileArtifactUpload> {
        let FileArtifactUploadRequest {
            path, description, ..
        } = request;

        let artifact = self.prepare_upload_artifact(path).await?;
        let create_response = self
            .create_upload_target(association, description, &artifact)
            .await?;

        let checksum = self
            .upload_artifact_bytes(&create_response.upload_target, &artifact)
            .await?;
        let uploaded_artifact = self
            .confirm_upload(create_response.artifact.artifact_uid, checksum)
            .await?;
        let size_bytes = i64::try_from(artifact.file_size)
            .context("Artifact file size exceeds supported range")?;

        Ok(CompletedFileArtifactUpload {
            artifact: uploaded_artifact,
            size_bytes,
        })
    }

    async fn prepare_upload_artifact(&self, path: PathBuf) -> Result<PreparedUploadArtifact> {
        unblock(move || PreparedUploadArtifact::from_path(path)).await
    }

    async fn create_upload_target(
        &self,
        association: ResolvedUploadAssociation,
        description: Option<String>,
        artifact: &PreparedUploadArtifact,
    ) -> Result<CreateFileArtifactUploadResponse> {
        self.ai_client
            .create_file_artifact_upload_target(CreateFileArtifactUploadRequest {
                conversation_id: association
                    .conversation_id
                    .as_ref()
                    .map(|token| token.as_str().to_string()),
                run_id: association.run_id.as_ref().map(ToString::to_string),
                filepath: artifact.filepath.clone(),
                description,
                mime_type: Some(artifact.mime_type.clone()),
                size_bytes: artifact.graphql_size_bytes(),
            })
            .await
            .context("Failed to create file artifact upload target")
    }

    async fn upload_artifact_bytes(
        &self,
        target: &FileArtifactUploadTargetInfo,
        artifact: &PreparedUploadArtifact,
    ) -> Result<String> {
        upload_file_to_target(
            self.server_api.http_client(),
            target,
            FileUploadBody::new(artifact.path.clone()),
        )
        .await
    }

    async fn confirm_upload(
        &self,
        artifact_uid: String,
        checksum: String,
    ) -> Result<FileArtifactRecord> {
        self.ai_client
            .confirm_file_artifact_upload(artifact_uid, checksum)
            .await
            .context("Failed to confirm file artifact upload")
    }

    pub(crate) async fn resolve_upload_association(
        &self,
        request: &FileArtifactUploadRequest,
    ) -> Result<ResolvedUploadAssociation> {
        let conversation_task_id = match (request.run_id.as_ref(), request.conversation_id.as_ref())
        {
            // we were given a conversation id, so we need to resolve the task id from the conversation via the api
            (None, Some(conversation_id)) => {
                Some(self.resolve_conversation_task_id(conversation_id).await)
            }
            _ => None,
        };

        resolve_upload_association_from_sources(
            request.run_id,
            request.conversation_id.clone(),
            conversation_task_id,
            load_env_run_id()?,
        )
    }

    async fn resolve_conversation_task_id(
        &self,
        conversation_id: &ServerConversationToken,
    ) -> Result<AmbientAgentTaskId> {
        let metadata = self
            .ai_client
            .list_ai_conversation_metadata(Some(vec![conversation_id.as_str().to_string()]))
            .await
            .with_context(|| {
                format!(
                    "Failed to load conversation '{}' to resolve artifact upload headers",
                    conversation_id.as_str()
                )
            })?;

        let metadata = single_conversation_metadata(conversation_id.as_str(), metadata)
            .with_context(|| {
                format!(
                    "Failed to load conversation '{}' to resolve artifact upload headers",
                    conversation_id.as_str()
                )
            })?;

        ambient_task_id_from_conversation_metadata(conversation_id.as_str(), metadata)
    }
}

fn normalize_artifact_filepath(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn file_size_and_prefix_for_path(path: &Path, max_bytes: usize) -> Result<(u64, Vec<u8>)> {
    let mut file = File::open(path)
        .with_context(|| format!("Failed to open artifact file '{}'", path.display()))?;
    let file_size = file
        .metadata()
        .with_context(|| format!("Failed to stat artifact file '{}'", path.display()))?
        .len();
    let mut bytes = vec![0; max_bytes];
    let bytes_read = file
        .read(&mut bytes)
        .with_context(|| format!("Failed to read artifact file '{}'", path.display()))?;
    bytes.truncate(bytes_read);
    Ok((file_size, bytes))
}

fn checked_graphql_size_bytes_for_upload(path: &Path, size_bytes: u64) -> Option<i32> {
    let graphql_size_bytes = i32::try_from(size_bytes).ok();
    if graphql_size_bytes.is_none() {
        // The backing upload can handle large files, but the GraphQL field is still `Int`.
        // Dropping `size_bytes` preserves the upload request instead of failing on conversion.
        log::warn!(
            "Artifact file '{}' is {} bytes, which exceeds the GraphQL size_bytes limit of {} bytes; omitting size_bytes from the upload target request",
            path.display(),
            size_bytes,
            i32::MAX,
        );
    }

    graphql_size_bytes
}

fn single_conversation_metadata(
    conversation_id: &str,
    mut metadata: Vec<ServerAIConversationMetadata>,
) -> Result<ServerAIConversationMetadata> {
    match metadata.len() {
        0 => bail!("Conversation not found"),
        1 => Ok(metadata.pop().expect("metadata length checked")),
        _ => bail!("Multiple conversations found for '{conversation_id}'"),
    }
}

fn ambient_task_id_from_conversation_metadata(
    conversation_id: &str,
    metadata: ServerAIConversationMetadata,
) -> Result<AmbientAgentTaskId> {
    metadata.ambient_agent_task_id.ok_or_else(|| {
        anyhow!("Conversation '{conversation_id}' is not backed by a cloud agent task")
    })
}

fn parse_run_id(run_id: &str, error_prefix: &str) -> Result<AmbientAgentTaskId> {
    parse_ambient_task_id(run_id, error_prefix)
}

fn load_env_run_id() -> Result<Option<String>> {
    match env::var(OZ_RUN_ID_ENV_VAR) {
        Ok(run_id) => Ok(Some(run_id)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(anyhow!(
            "{OZ_RUN_ID_ENV_VAR} is set but is not valid Unicode"
        )),
    }
}

fn resolve_env_run_id(env_run_id: Option<String>) -> Result<AmbientAgentTaskId> {
    let Some(run_id) = env_run_id else {
        bail!("{OZ_RUN_ID_ENV_VAR} is not set");
    };

    parse_run_id(&run_id, "Invalid OZ_RUN_ID")
}

fn resolve_upload_association_from_sources(
    explicit_run_id: Option<AmbientAgentTaskId>,
    explicit_conversation_id: Option<ServerConversationToken>,
    conversation_task_id: Option<Result<AmbientAgentTaskId>>,
    env_run_id: Option<String>,
) -> Result<ResolvedUploadAssociation> {
    // Precedence is deliberate:
    // 1. An explicit run ID is authoritative and must not silently fall back.
    // 2. A conversation ID stays attached to the artifact even if we have to borrow the ambient
    //    task ID from `OZ_RUN_ID` because the conversation lacks cloud-task metadata.
    // 3. `OZ_RUN_ID` becomes the sole source of truth only when the caller supplied nothing else.
    if let Some(run_id) = explicit_run_id {
        let ambient_task_id = run_id;
        return Ok(ResolvedUploadAssociation {
            conversation_id: None,
            run_id: Some(run_id),
            ambient_task_id,
        });
    }

    if let Some(conversation_id) = explicit_conversation_id {
        match conversation_task_id
            .ok_or_else(|| anyhow!("conversation resolution should be provided"))?
        {
            Ok(ambient_task_id) => {
                return Ok(ResolvedUploadAssociation {
                    conversation_id: Some(conversation_id),
                    run_id: None,
                    ambient_task_id,
                });
            }
            Err(conversation_err) => {
                let env_err = match resolve_env_run_id(env_run_id) {
                    Ok(ambient_task_id) => {
                        log::warn!(
                            "Conversation '{}' task resolution failed ({conversation_err}); falling back to {OZ_RUN_ID_ENV_VAR} for ambient task context",
                            conversation_id.as_str()
                        );
                        return Ok(ResolvedUploadAssociation {
                            conversation_id: Some(conversation_id),
                            run_id: None,
                            ambient_task_id,
                        });
                    }
                    Err(env_err) => env_err,
                };

                return Err(anyhow!(
                    "Failed to resolve artifact upload association for conversation '{}': {conversation_err}; also failed to use {OZ_RUN_ID_ENV_VAR}: {env_err}",
                    conversation_id.as_str()
                ));
            }
        }
    }

    let ambient_task_id = resolve_env_run_id(env_run_id).map_err(|env_err| {
        anyhow!(
            "Failed to resolve artifact upload association: no usable --run-id or --conversation-id was provided, and {OZ_RUN_ID_ENV_VAR}: {env_err}"
        )
    })?;

    Ok(ResolvedUploadAssociation {
        conversation_id: None,
        run_id: Some(ambient_task_id),
        ambient_task_id,
    })
}

#[cfg(test)]
#[path = "artifact_upload_tests.rs"]
mod tests;
