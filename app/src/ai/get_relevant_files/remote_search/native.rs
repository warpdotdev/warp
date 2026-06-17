use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use ::ai::index::full_source_code_embedding::search_shaping::{
    build_fragments_from_file_contents, fragments_to_context_locations,
};
use ::ai::index::full_source_code_embedding::store_client::StoreClient;
use ::ai::index::full_source_code_embedding::{
    ContentHash, FragmentMetadata as AiFragmentMetadata, FragmentMetadataLocation, RepoMetadata,
};
use ::ai::index::locations::CodeContextLocation;
use itertools::Itertools;
use remote_server::proto::{
    file_context_proto, FragmentMetadata as ProtoFragmentMetadata, LineRange, ReadFileContextFile,
    ReadFileContextRequest, ReadFileContextResponse,
};
use string_offset::ByteOffset;
use warpui::{AppContext, ModelContext, SingletonEntity};

use crate::ai::agent::{
    AnyFileContent, FileContext, SearchCodebaseFailureReason, SearchCodebaseResult,
};
use crate::ai::blocklist::SessionContext;
use crate::ai::codebase_auto_indexing::{
    should_use_codebase_indexing, CodebaseAutoIndexingSurface,
};
use crate::ai::get_relevant_files::controller::GetRelevantFilesController;
use crate::remote_server::codebase_index_model::{
    RemoteCodebaseIndexModel, RemoteCodebaseSearchAvailability, RemoteCodebaseSearchContext,
};
use crate::server::server_api::{ServerApi, ServerApiProvider};

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
    if !should_use_codebase_indexing(CodebaseAutoIndexingSurface::Remote, ctx) {
        return RemoteSearchRequest::Ready(SearchCodebaseResult::Failed {
            reason: SearchCodebaseFailureReason::CodebaseNotIndexed,
            message: "Remote codebase search is not enabled.".to_string(),
        });
    }

    let availability = RemoteCodebaseIndexModel::as_ref(ctx)
        .active_repo_availability(&session_context, requested_codebase_path.as_deref());
    match availability {
        RemoteCodebaseSearchAvailability::Ready(search_context) => {
            let handle = remote_server::manager::RemoteServerManager::as_ref(ctx)
                .host_request_handle(&search_context.remote_path.host_id);
            if search_context.is_stale {
                let remote_path = search_context.remote_path.clone();
                let sync_requested = remote_server::manager::RemoteServerManager::handle(ctx)
                    .update(ctx, |manager, ctx| {
                        manager.trigger_codebase_incremental_sync(remote_path, ctx)
                    });
                if !sync_requested {
                    log::warn!(
                        "Remote codebase search is using a stale index because incremental sync could not be requested"
                    );
                }
            }
            let store_client = ServerApiProvider::as_ref(ctx).get();
            let abort_handle = ctx
                .spawn(
                    async move {
                        execute_remote_codebase_search(
                            query,
                            partial_paths,
                            search_context,
                            handle,
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
    handle: remote_server::manager::HostRequestHandle,
    store_client: Arc<ServerApi>,
) -> Result<SearchCodebaseResult, anyhow::Error> {
    let root_hash = search_context.root_hash;
    let root_hash_string = root_hash.to_string();
    let repo_path = search_context.remote_path.path.as_str().to_string();
    let embedding_config = store_client
        .codebase_context_config()
        .await?
        .embedding_config;
    log::debug!(
        "[Remote codebase indexing] Remote codebase search using embedding config: repo_path={repo_path} embedding_config={embedding_config:?}"
    );
    let candidate_hashes = store_client
        .get_relevant_fragments(
            embedding_config,
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
    let metadata_response = handle
        .get_fragment_metadata_from_hash(
            repo_path.clone(),
            root_hash_string,
            candidate_hash_strings,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Fragment metadata lookup failed: {e}"))?;
    if !metadata_response.missing_hashes.is_empty() {
        log::warn!(
            "Remote codebase search metadata lookup missed {} hashes for repo {}",
            metadata_response.missing_hashes.len(),
            repo_path
        );
    }
    let mut remote_fragments = metadata_response.fragments;
    if let Some(partial_paths) = partial_paths {
        remote_fragments.retain(|fragment| {
            partial_paths
                .iter()
                .any(|partial_path| fragment.path.contains(partial_path))
        });
    }
    if remote_fragments.is_empty() {
        return Ok(SearchCodebaseResult::Success { files: vec![] });
    }

    // Convert daemon protobuf metadata into the shared search-shaping metadata format so
    // remote search reuses the same fragment reconstruction and context expansion path as
    // local search.
    let parsed_fragment_metadata = remote_fragments
        .into_iter()
        .filter_map(|fragment| match remote_fragment_metadata(fragment) {
            Ok(metadata) => Some(metadata),
            Err(err) => {
                log::warn!("Failed to parse remote codebase fragment metadata: {err:?}");
                None
            }
        })
        .collect_vec();
    if parsed_fragment_metadata.is_empty() {
        return Ok(SearchCodebaseResult::Success { files: vec![] });
    }

    let response = handle
        .read_file_context(read_full_fragment_files_request(&parsed_fragment_metadata))
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
    let file_contents = file_contents_from_response(response);
    let read_fragment_result = build_fragments_from_file_contents(
        parsed_fragment_metadata.iter().cloned(),
        &file_contents,
    );
    if !read_fragment_result.fail_to_read_path.is_empty() {
        log::warn!(
            "Remote codebase search failed to read {} fragment file(s)",
            read_fragment_result.fail_to_read_path.len()
        );
    }
    let fragments = read_fragment_result.successfully_read;
    if fragments.is_empty() {
        return Ok(SearchCodebaseResult::Success { files: vec![] });
    }

    let reranked_fragments = store_client.rerank_fragments(query, fragments).await?;
    let metadata_by_hash = fragment_metadata_by_hash(&parsed_fragment_metadata);
    let locations = fragments_to_context_locations(
        reranked_fragments,
        |hash| metadata_by_hash.get(hash).map(Vec::as_slice),
        RETRIEVE_FRAGMENT_CONTEXT_LENGTH,
    );
    if locations.is_empty() {
        return Ok(SearchCodebaseResult::Success { files: vec![] });
    }

    let response = handle
        .read_file_context(read_context_locations_request(&locations))
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
    let files = response
        .file_contexts
        .into_iter()
        .filter_map(proto_file_context_to_file_context)
        .collect_vec();

    Ok(SearchCodebaseResult::Success { files })
}

const RETRIEVE_FRAGMENT_CONTEXT_LENGTH: usize = 0;

fn remote_fragment_metadata(
    fragment: ProtoFragmentMetadata,
) -> anyhow::Result<(ContentHash, AiFragmentMetadata)> {
    let content_hash = ContentHash::from_str(&fragment.content_hash)?;
    Ok((
        content_hash,
        AiFragmentMetadata {
            absolute_path: PathBuf::from(fragment.path),
            location: FragmentMetadataLocation {
                start_line: fragment.start_line as usize,
                end_line: fragment.end_line as usize,
                byte_range: ByteOffset::from(fragment.byte_start as usize)
                    ..ByteOffset::from(fragment.byte_end as usize),
            },
        },
    ))
}

fn read_full_fragment_files_request(
    metadata: &[(ContentHash, AiFragmentMetadata)],
) -> ReadFileContextRequest {
    let mut seen_paths = HashSet::new();
    ReadFileContextRequest {
        files: metadata
            .iter()
            .filter_map(|(_, fragment)| {
                let path = fragment.absolute_path.to_string_lossy().to_string();
                seen_paths
                    .insert(path.clone())
                    .then_some(ReadFileContextFile {
                        path,
                        line_ranges: vec![],
                    })
            })
            .collect(),
        max_file_bytes: None,
        max_batch_bytes: None,
    }
}

fn file_contents_from_response(response: ReadFileContextResponse) -> HashMap<PathBuf, String> {
    let mut file_contents = HashMap::new();
    for file_context in response.file_contexts {
        if file_context.line_range_start.is_some() || file_context.line_range_end.is_some() {
            continue;
        }
        if let Some(file_context_proto::Content::TextContent(content)) = file_context.content {
            file_contents.insert(PathBuf::from(file_context.file_name), content);
        }
    }
    file_contents
}

fn fragment_metadata_by_hash(
    metadata: &[(ContentHash, AiFragmentMetadata)],
) -> HashMap<ContentHash, Vec<AiFragmentMetadata>> {
    let mut metadata_by_hash: HashMap<ContentHash, Vec<AiFragmentMetadata>> = HashMap::new();
    for (content_hash, metadata) in metadata {
        metadata_by_hash
            .entry(content_hash.clone())
            .or_default()
            .push(metadata.clone());
    }
    metadata_by_hash
}

fn read_context_locations_request(
    locations: &HashSet<CodeContextLocation>,
) -> ReadFileContextRequest {
    ReadFileContextRequest {
        files: locations
            .iter()
            .map(|location| match location {
                CodeContextLocation::WholeFile(path) => ReadFileContextFile {
                    path: path.to_string_lossy().to_string(),
                    line_ranges: vec![],
                },
                CodeContextLocation::Fragment(fragment) => ReadFileContextFile {
                    path: fragment.path.to_string_lossy().to_string(),
                    line_ranges: fragment
                        .line_ranges
                        .iter()
                        .map(|range| LineRange {
                            start: range.start as u32,
                            end: range.end as u32,
                        })
                        .collect(),
                },
            })
            .collect(),
        max_file_bytes: None,
        max_batch_bytes: None,
    }
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
                    "The remote codebase at {} is not indexed yet.",
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

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;
