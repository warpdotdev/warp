use ::ai::index::full_source_code_embedding::{
    store_client::StoreClient, ContentHash, Fragment, RepoMetadata,
};
use itertools::Itertools;
use remote_server::proto::{
    file_context_proto, FragmentMetadata, LineRange, ReadFileContextFile, ReadFileContextRequest,
    ReadFileContextResponse,
};
use std::{collections::HashMap, path::PathBuf, str::FromStr, sync::Arc};
use string_offset::ByteOffset;
use warpui::{AppContext, ModelContext, SingletonEntity};

use crate::{
    ai::{
        agent::{AnyFileContent, FileContext, SearchCodebaseFailureReason, SearchCodebaseResult},
        blocklist::SessionContext,
    },
    features::FeatureFlag,
    remote_server::codebase_index_model::{
        RemoteCodebaseIndexModel, RemoteCodebaseSearchAvailability, RemoteCodebaseSearchContext,
    },
    server::server_api::{ServerApi, ServerApiProvider},
};

use crate::ai::get_relevant_files::controller::GetRelevantFilesController;

pub(super) enum RemoteSearchRequest {
    Pending(futures_util::stream::AbortHandle),
    Ready(SearchCodebaseResult),
}

pub(super) fn root_directory_for_search(
    session_context: &SessionContext,
    requested_codebase_path: Option<&str>,
    app: &AppContext,
) -> Option<PathBuf> {
    RemoteCodebaseIndexModel::as_ref(app)
        .active_repo_path(session_context, requested_codebase_path)
        .or_else(|| {
            requested_codebase_path
                .is_none()
                .then(|| session_context.current_working_directory().clone())
                .flatten()
        })
        .map(PathBuf::from)
}

pub(super) fn send_request(
    query: String,
    partial_paths: Option<Vec<String>>,
    session_context: SessionContext,
    requested_codebase_path: Option<String>,
    action_id: crate::ai::agent::AIAgentActionId,
    ctx: &mut ModelContext<GetRelevantFilesController>,
) -> RemoteSearchRequest {
    if !FeatureFlag::RemoteCodebaseIndexing.is_enabled() {
        return RemoteSearchRequest::Ready(SearchCodebaseResult::Failed {
            reason: SearchCodebaseFailureReason::CodebaseNotIndexed,
            message: "Remote codebase search is not enabled.".to_string(),
        });
    }

    let availability = RemoteCodebaseIndexModel::as_ref(ctx)
        .active_repo_availability(&session_context, requested_codebase_path.as_deref());
    match availability {
        RemoteCodebaseSearchAvailability::Ready(search_context) => {
            let Some(client) = remote_server::manager::RemoteServerManager::as_ref(ctx)
                .client_for_host(&search_context.remote_path.host_id)
                .cloned()
            else {
                return RemoteSearchRequest::Ready(SearchCodebaseResult::Failed {
                    reason: SearchCodebaseFailureReason::ClientError,
                    message: "Remote codebase search is unavailable because the remote server is not connected.".to_string(),
                });
            };
            let store_client = ServerApiProvider::as_ref(ctx).get();
            let abort_handle = ctx
                .spawn(
                    async move {
                        execute_remote_codebase_search(
                            query,
                            partial_paths,
                            search_context,
                            client,
                            store_client,
                        )
                        .await
                    },
                    move |me, result, ctx| {
                        me.handle_remote_search_result(result, action_id, ctx);
                    },
                )
                .abort_handle();
            RemoteSearchRequest::Pending(abort_handle)
        }
        availability @ RemoteCodebaseSearchAvailability::NotIndexed { .. } => {
            RemoteCodebaseIndexModel::handle(ctx).update(ctx, |model, ctx| {
                model.request_active_repo_index(
                    &session_context,
                    requested_codebase_path.as_deref(),
                    ctx,
                );
            });
            RemoteSearchRequest::Ready(remote_availability_failure(availability))
        }
        RemoteCodebaseSearchAvailability::NoConnectedHost
        | RemoteCodebaseSearchAvailability::NoActiveRepo
        | RemoteCodebaseSearchAvailability::Indexing { .. }
        | RemoteCodebaseSearchAvailability::Unavailable { .. } => {
            RemoteSearchRequest::Ready(remote_availability_failure(availability))
        }
    }
}

// The controller owns request lifecycle concerns like cancellation, pending request tracking, and
// result emission. This function only contains the remote-specific pipeline: store content hashes
// -> daemon fragment metadata -> remote file reads -> fragment reranking.
async fn execute_remote_codebase_search(
    query: String,
    partial_paths: Option<Vec<String>>,
    search_context: RemoteCodebaseSearchContext,
    client: Arc<remote_server::client::RemoteServerClient>,
    store_client: Arc<ServerApi>,
) -> Result<SearchCodebaseResult, anyhow::Error> {
    let root_hash = search_context.root_hash;
    let root_hash_string = root_hash.to_string();
    let repo_path = search_context.remote_path.path.as_str().to_string();
    let candidate_hashes = store_client
        .get_relevant_fragments(
            search_context.embedding_config,
            query.clone(),
            root_hash,
            RepoMetadata {
                path: Some(repo_path.clone()),
            },
        )
        .await?;
    if candidate_hashes.is_empty() {
        return Ok(SearchCodebaseResult::Success { files: vec![] });
    }

    let candidate_hash_strings = candidate_hashes
        .iter()
        .map(ToString::to_string)
        .collect_vec();
    let metadata_response = client
        .get_fragment_metadata_from_hash(
            repo_path.clone(),
            root_hash_string,
            candidate_hash_strings,
        )
        .await?;
    if !metadata_response.missing_hashes.is_empty() {
        log::warn!(
            "Remote codebase search metadata lookup missed {} hashes for repo {}",
            metadata_response.missing_hashes.len(),
            repo_path
        );
    }
    let mut metadata = metadata_response.fragments;
    if let Some(partial_paths) = partial_paths {
        metadata.retain(|fragment| {
            partial_paths
                .iter()
                .any(|partial_path| fragment.path.contains(partial_path))
        });
    }
    if metadata.is_empty() {
        return Ok(SearchCodebaseResult::Success { files: vec![] });
    }

    let response = client
        .read_file_context(read_fragment_metadata_request(&metadata))
        .await?;
    if !response.failed_files.is_empty() && response.file_contexts.is_empty() {
        let failed = response
            .failed_files
            .iter()
            .map(|file| {
                let reason = file
                    .error
                    .as_ref()
                    .map(|error| error.message.as_str())
                    .unwrap_or("unknown error");
                format!("{}: {reason}", file.path)
            })
            .join(", ");
        return Ok(SearchCodebaseResult::Failed {
            reason: SearchCodebaseFailureReason::InvalidFilePaths,
            message: format!("Failed to read remote search result files: {failed}"),
        });
    }

    let (fragments, mut file_contexts_by_identity) =
        remote_fragments_and_file_contexts(response, &metadata)?;
    if fragments.is_empty() {
        return Ok(SearchCodebaseResult::Success { files: vec![] });
    }

    let reranked_fragments = store_client.rerank_fragments(query, fragments).await?;
    let files = reranked_fragments
        .into_iter()
        .filter_map(|fragment| {
            file_contexts_by_identity.remove(&RemoteFragmentIdentity::from_fragment(&fragment))
        })
        .collect_vec();

    Ok(SearchCodebaseResult::Success { files })
}

fn read_fragment_metadata_request(metadata: &[FragmentMetadata]) -> ReadFileContextRequest {
    ReadFileContextRequest {
        files: metadata
            .iter()
            .map(|fragment| {
                let line_ranges =
                    if fragment.start_line > 0 && fragment.end_line >= fragment.start_line {
                        vec![LineRange {
                            start: fragment.start_line,
                            end: fragment.end_line.saturating_add(1),
                        }]
                    } else {
                        vec![]
                    };
                ReadFileContextFile {
                    path: fragment.path.clone(),
                    line_ranges,
                }
            })
            .collect(),
        max_file_bytes: None,
        max_batch_bytes: None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct RemoteFragmentIdentity {
    content_hash: String,
    path: String,
    byte_start: u64,
    byte_end: u64,
}

impl RemoteFragmentIdentity {
    fn from_metadata(metadata: &FragmentMetadata) -> Self {
        Self {
            content_hash: metadata.content_hash.clone(),
            path: metadata.path.clone(),
            byte_start: metadata.byte_start,
            byte_end: metadata.byte_end,
        }
    }

    fn from_fragment(fragment: &Fragment) -> Self {
        let byte_range = fragment.byte_range();
        Self {
            content_hash: fragment.content_hash().to_string(),
            path: fragment.absolute_path().to_string_lossy().to_string(),
            byte_start: byte_range.start.as_usize() as u64,
            byte_end: byte_range.end.as_usize() as u64,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct RemoteReadContextKey {
    path: String,
    line_range_start: Option<u32>,
    line_range_end: Option<u32>,
}

impl RemoteReadContextKey {
    fn from_metadata(metadata: &FragmentMetadata) -> Self {
        let line_range = (metadata.start_line > 0 && metadata.end_line >= metadata.start_line)
            .then_some((metadata.start_line, metadata.end_line.saturating_add(1)));
        Self {
            path: metadata.path.clone(),
            line_range_start: line_range.map(|(start, _)| start),
            line_range_end: line_range.map(|(_, end)| end),
        }
    }

    fn from_file_context(file_context: &remote_server::proto::FileContextProto) -> Self {
        Self {
            path: file_context.file_name.clone(),
            line_range_start: file_context.line_range_start,
            line_range_end: file_context.line_range_end,
        }
    }
}

fn remote_fragments_and_file_contexts(
    response: ReadFileContextResponse,
    metadata: &[FragmentMetadata],
) -> anyhow::Result<(Vec<Fragment>, HashMap<RemoteFragmentIdentity, FileContext>)> {
    let mut fragments = Vec::new();
    let mut file_contexts_by_read_key: HashMap<RemoteReadContextKey, Vec<FileContext>> =
        HashMap::new();
    for file_context in response.file_contexts {
        let read_key = RemoteReadContextKey::from_file_context(&file_context);
        let Some(file_context) = proto_file_context_to_file_context(file_context) else {
            continue;
        };
        file_contexts_by_read_key
            .entry(read_key)
            .or_default()
            .push(file_context);
    }

    let mut file_contexts_by_identity = HashMap::new();

    for fragment_metadata in metadata {
        let read_key = RemoteReadContextKey::from_metadata(fragment_metadata);
        let Some(file_context) = file_contexts_by_read_key
            .get(&read_key)
            .and_then(|file_contexts| file_contexts.last())
            .cloned()
        else {
            continue;
        };
        let AnyFileContent::StringContent(content) = &file_context.content else {
            continue;
        };
        let content_hash = ContentHash::from_str(&fragment_metadata.content_hash)?;
        fragments.push(Fragment::from_byte_range(
            content.clone(),
            content_hash,
            PathBuf::from(fragment_metadata.path.clone()),
            ByteOffset::from(fragment_metadata.byte_start as usize)
                ..ByteOffset::from(fragment_metadata.byte_end as usize),
        ));
        file_contexts_by_identity.insert(
            RemoteFragmentIdentity::from_metadata(fragment_metadata),
            file_context,
        );
    }

    Ok((fragments, file_contexts_by_identity))
}

// Keep this conversion at the AI boundary: `FileContext` lives in the `ai` crate, so the
// `remote_server` protocol/client crate should not depend on it just to return typed agent
// file contexts.
fn proto_file_context_to_file_context(
    file_context: remote_server::proto::FileContextProto,
) -> Option<FileContext> {
    let content = match file_context.content? {
        file_context_proto::Content::TextContent(text) => AnyFileContent::StringContent(text),
        file_context_proto::Content::BinaryContent(bytes) => AnyFileContent::BinaryContent(bytes),
    };
    let line_range = match (file_context.line_range_start, file_context.line_range_end) {
        (Some(start), Some(end)) => Some(start as usize..end as usize),
        (Some(_), None) | (None, Some(_)) | (None, None) => None,
    };
    let last_modified = file_context
        .last_modified_epoch_millis
        .map(|ms| std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms));
    Some(FileContext {
        file_name: file_context.file_name,
        content,
        line_range,
        last_modified,
        line_count: file_context.line_count as usize,
    })
}

fn remote_availability_failure(
    availability: RemoteCodebaseSearchAvailability,
) -> SearchCodebaseResult {
    match availability {
        RemoteCodebaseSearchAvailability::NoConnectedHost => SearchCodebaseResult::Failed {
            reason: SearchCodebaseFailureReason::ClientError,
            message:
                "Remote codebase search is unavailable because the remote host is not connected."
                    .to_string(),
        },
        RemoteCodebaseSearchAvailability::NoActiveRepo => SearchCodebaseResult::Failed {
            reason: SearchCodebaseFailureReason::CodebaseNotIndexed,
            message: "The current remote directory is not in a known codebase.".to_string(),
        },
        RemoteCodebaseSearchAvailability::NotIndexed { remote_path } => {
            SearchCodebaseResult::Failed {
                reason: SearchCodebaseFailureReason::CodebaseNotIndexed,
                message: format!(
                    "The remote codebase at {} is not indexed yet. Indexing has been requested; try again after it finishes.",
                    remote_path.path.as_str()
                ),
            }
        }
        RemoteCodebaseSearchAvailability::Indexing { remote_path } => {
            SearchCodebaseResult::Failed {
                reason: SearchCodebaseFailureReason::CodebaseNotIndexed,
                message: format!(
                    "The remote codebase at {} is still being indexed. Try again later.",
                    remote_path.path.as_str()
                ),
            }
        }
        RemoteCodebaseSearchAvailability::Unavailable {
            remote_path,
            message,
        } => SearchCodebaseResult::Failed {
            reason: SearchCodebaseFailureReason::CodebaseNotIndexed,
            message: format!(
                "Remote codebase search is unavailable for {}: {message}",
                remote_path.path.as_str()
            ),
        },
        RemoteCodebaseSearchAvailability::Ready(_) => SearchCodebaseResult::Failed {
            reason: SearchCodebaseFailureReason::ClientError,
            message: "Remote codebase search was unexpectedly unavailable.".to_string(),
        },
    }
}
