use std::path::PathBuf;

use warpui::{AppContext, ModelContext};

use crate::ai::{
    agent::{AIAgentActionId, SearchCodebaseFailureReason, SearchCodebaseResult},
    blocklist::SessionContext,
};

use crate::ai::get_relevant_files::controller::GetRelevantFilesController;

pub(super) enum RemoteSearchRequest {
    Ready(SearchCodebaseResult),
}

pub(super) fn root_directory_for_search(
    _session_context: &SessionContext,
    _requested_codebase_path: Option<&str>,
    _app: &AppContext,
) -> Option<PathBuf> {
    None
}

pub(super) fn send_request(
    _query: String,
    _partial_paths: Option<Vec<String>>,
    _session_context: SessionContext,
    _requested_codebase_path: Option<String>,
    _action_id: AIAgentActionId,
    _ctx: &mut ModelContext<GetRelevantFilesController>,
) -> RemoteSearchRequest {
    RemoteSearchRequest::Ready(SearchCodebaseResult::Failed {
        reason: SearchCodebaseFailureReason::CodebaseNotIndexed,
        message: "Remote codebase search is not available in this environment.".to_string(),
    })
}
