use ai::agent::action_result::{
    AskUserQuestionResult, CallMCPToolResult, CreateDocumentsResult, EditDocumentsResult,
    FetchConversationResult, FileGlobResult, FileGlobV2Result, GrepResult,
    InsertReviewCommentsResult, ReadDocumentsResult, ReadFilesResult, ReadMCPResourceResult,
    ReadSkillResult, RequestCommandOutputResult, RequestComputerUseResult, RequestFileEditsResult,
    RunAgentsResult, SearchCodebaseFailureReason, SearchCodebaseResult, SendMessageToAgentResult,
    StartRecordingResult, StopRecordingResult, SuggestNewConversationResult, UploadArtifactResult,
    UseComputerResult,
};

use crate::ai::agent::{AIAgentActionResultType, AIAgentActionType};

/// Put in front of a hook's reason where it travels in a result's general error text, so the
/// model can tell a policy denial from a tool that failed.
pub(crate) const DENIED_BY_HOOK_PREFIX: &str = "Denied by agent hook: ";

/// The result an action finishes with when a hook denies it before it runs.
pub(crate) fn denied_result_for(
    action: &AIAgentActionType,
    reason: &str,
) -> AIAgentActionResultType {
    let message = || format!("{DENIED_BY_HOOK_PREFIX}{reason}");
    match action {
        AIAgentActionType::RequestCommandOutput { command, .. } => {
            AIAgentActionResultType::RequestCommandOutput(RequestCommandOutputResult::HookDenied {
                command: command.clone(),
                reason: reason.to_owned(),
            })
        }
        AIAgentActionType::ReadFiles(_) => {
            AIAgentActionResultType::ReadFiles(ReadFilesResult::Error(message()))
        }
        AIAgentActionType::UploadArtifact(_) => {
            AIAgentActionResultType::UploadArtifact(UploadArtifactResult::Error(message()))
        }
        AIAgentActionType::SearchCodebase(_) => {
            AIAgentActionResultType::SearchCodebase(SearchCodebaseResult::Failed {
                reason: SearchCodebaseFailureReason::ClientError,
                message: message(),
            })
        }
        AIAgentActionType::RequestFileEdits { .. } => AIAgentActionResultType::RequestFileEdits(
            RequestFileEditsResult::DiffApplicationFailed { error: message() },
        ),
        AIAgentActionType::Grep { .. } => {
            AIAgentActionResultType::Grep(GrepResult::Error(message()))
        }
        AIAgentActionType::FileGlob { .. } => {
            AIAgentActionResultType::FileGlob(FileGlobResult::Error(message()))
        }
        AIAgentActionType::FileGlobV2 { .. } => {
            AIAgentActionResultType::FileGlobV2(FileGlobV2Result::Error(message()))
        }
        AIAgentActionType::ReadMCPResource { .. } => {
            AIAgentActionResultType::ReadMCPResource(ReadMCPResourceResult::Error(message()))
        }
        AIAgentActionType::CallMCPTool { .. } => {
            AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Error(message()))
        }
        AIAgentActionType::ReadSkill(_) => {
            AIAgentActionResultType::ReadSkill(ReadSkillResult::Error(message()))
        }
        AIAgentActionType::ReadDocuments(_) => {
            AIAgentActionResultType::ReadDocuments(ReadDocumentsResult::Error(message()))
        }
        AIAgentActionType::EditDocuments(_) => {
            AIAgentActionResultType::EditDocuments(EditDocumentsResult::Error(message()))
        }
        AIAgentActionType::CreateDocuments(_) => {
            AIAgentActionResultType::CreateDocuments(CreateDocumentsResult::Error(message()))
        }
        AIAgentActionType::UseComputer(_) => {
            AIAgentActionResultType::UseComputer(UseComputerResult::Error(message()))
        }
        AIAgentActionType::RequestComputerUse(_) => {
            AIAgentActionResultType::RequestComputerUse(RequestComputerUseResult::Error(message()))
        }
        AIAgentActionType::StartRecording { .. } => {
            AIAgentActionResultType::StartRecording(StartRecordingResult::Error(message()))
        }
        AIAgentActionType::StopRecording { .. } => {
            AIAgentActionResultType::StopRecording(StopRecordingResult::Error(message()))
        }
        AIAgentActionType::FetchConversation { .. } => {
            AIAgentActionResultType::FetchConversation(FetchConversationResult::Error(message()))
        }
        AIAgentActionType::SendMessageToAgent { .. } => {
            AIAgentActionResultType::SendMessageToAgent(SendMessageToAgentResult::Error(message()))
        }
        AIAgentActionType::AskUserQuestion { .. } => {
            AIAgentActionResultType::AskUserQuestion(AskUserQuestionResult::Error(message()))
        }
        AIAgentActionType::InsertCodeReviewComments { repo_path, .. } => {
            AIAgentActionResultType::InsertReviewComments(InsertReviewCommentsResult::Error {
                repo_path: repo_path.to_string_lossy().to_string(),
                message: message(),
            })
        }
        AIAgentActionType::RunAgents(_) => {
            AIAgentActionResultType::RunAgents(RunAgentsResult::Denied { reason: message() })
        }
        // The remaining families have no wire slot for a denial reason, so the reason reaches
        // only the user notice. `SuggestNewConversation` finishes `Rejected` because a declined
        // suggestion is the wire answer for "no" whereas a cancellation is dropped.
        AIAgentActionType::SuggestNewConversation { .. } => {
            AIAgentActionResultType::SuggestNewConversation(SuggestNewConversationResult::Rejected)
        }
        AIAgentActionType::WriteToLongRunningShellCommand { .. }
        | AIAgentActionType::ReadShellCommandOutput { .. }
        | AIAgentActionType::TransferShellCommandControlToUser { .. }
        | AIAgentActionType::SuggestPrompt(_)
        | AIAgentActionType::InitProject
        | AIAgentActionType::OpenCodeReview
        | AIAgentActionType::WaitForEvents { .. } => action.cancelled_result(),
    }
}

#[cfg(test)]
#[path = "denied_result_tests.rs"]
mod tests;
