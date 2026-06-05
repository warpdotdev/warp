use super::*;
use crate::proto::{
    delete_file_response, discard_files_response, save_buffer_response, server_message,
    write_file_response, DeleteFileResponse, DeleteFileSuccess, DiscardFilesError,
    DiscardFilesResponse, DiscardFilesSuccess, FileOperationError, SaveBufferResponse,
    SaveBufferSuccess, ServerMessage, WriteFileResponse, WriteFileSuccess,
};

fn msg(inner: server_message::Message) -> ServerMessage {
    ServerMessage {
        request_id: "req-1".to_string(),
        message: Some(inner),
    }
}

#[test]
fn write_file_success_is_ok_empty_is_err() {
    let success = msg(server_message::Message::WriteFileResponse(
        WriteFileResponse {
            result: Some(write_file_response::Result::Success(WriteFileSuccess {})),
        },
    ));
    assert!(write_file_result(&success).is_ok());

    let empty = msg(server_message::Message::WriteFileResponse(
        WriteFileResponse { result: None },
    ));
    assert!(write_file_result(&empty).is_err());
}

#[test]
fn write_file_error_propagates_message() {
    let err = msg(server_message::Message::WriteFileResponse(
        WriteFileResponse {
            result: Some(write_file_response::Result::Error(FileOperationError {
                message: "disk full".to_string(),
            })),
        },
    ));
    assert_eq!(write_file_result(&err).unwrap_err(), "disk full");
}

#[test]
fn write_file_wrong_variant_is_err() {
    let wrong = msg(server_message::Message::SaveBufferResponse(
        SaveBufferResponse {
            result: Some(save_buffer_response::Result::Success(SaveBufferSuccess {})),
        },
    ));
    assert!(write_file_result(&wrong).is_err());
}

#[test]
fn save_buffer_success_is_ok_empty_is_err() {
    let success = msg(server_message::Message::SaveBufferResponse(
        SaveBufferResponse {
            result: Some(save_buffer_response::Result::Success(SaveBufferSuccess {})),
        },
    ));
    assert!(save_buffer_result(&success).is_ok());

    let empty = msg(server_message::Message::SaveBufferResponse(
        SaveBufferResponse { result: None },
    ));
    assert!(save_buffer_result(&empty).is_err());
}

#[test]
fn save_buffer_error_propagates_message() {
    let err = msg(server_message::Message::SaveBufferResponse(
        SaveBufferResponse {
            result: Some(save_buffer_response::Result::Error(FileOperationError {
                message: "permission denied".to_string(),
            })),
        },
    ));
    assert_eq!(save_buffer_result(&err).unwrap_err(), "permission denied");
}

#[test]
fn delete_file_success_is_ok_empty_is_err() {
    let success = msg(server_message::Message::DeleteFileResponse(
        DeleteFileResponse {
            result: Some(delete_file_response::Result::Success(DeleteFileSuccess {})),
        },
    ));
    assert!(delete_file_result(&success).is_ok());

    let empty = msg(server_message::Message::DeleteFileResponse(
        DeleteFileResponse { result: None },
    ));
    assert!(delete_file_result(&empty).is_err());
}

#[test]
fn delete_file_error_propagates_message() {
    let err = msg(server_message::Message::DeleteFileResponse(
        DeleteFileResponse {
            result: Some(delete_file_response::Result::Error(FileOperationError {
                message: "no such file".to_string(),
            })),
        },
    ));
    assert_eq!(delete_file_result(&err).unwrap_err(), "no such file");
}

#[test]
fn discard_files_success_is_ok() {
    let success = msg(server_message::Message::DiscardFilesResponse(
        DiscardFilesResponse {
            result: Some(discard_files_response::Result::Success(
                DiscardFilesSuccess {},
            )),
        },
    ));
    assert!(discard_files_result(&success).is_ok());
}

#[test]
fn discard_files_error_propagates_message() {
    let err = msg(server_message::Message::DiscardFilesResponse(
        DiscardFilesResponse {
            result: Some(discard_files_response::Result::Error(DiscardFilesError {
                message: "merge conflict".to_string(),
            })),
        },
    ));
    assert_eq!(discard_files_result(&err).unwrap_err(), "merge conflict");
}

#[test]
fn discard_files_empty_result_is_err() {
    let empty = msg(server_message::Message::DiscardFilesResponse(
        DiscardFilesResponse { result: None },
    ));
    assert!(discard_files_result(&empty).is_err());
}

/// Guard: every host-scoped request variant must have an explicit, intentional
/// response disposition. This match is exhaustive, so adding a new
/// `host_scoped_request::Message` variant fails to compile until it is
/// classified here — a prompt to add a `host_response` parser (or document why
/// the response is parsed at the manager call site).
#[test]
fn every_host_scoped_request_has_a_response_disposition() {
    use crate::proto::host_scoped_request::Message as M;

    fn disposition(m: &M) -> &'static str {
        match m {
            // Parsed via the helpers in this module.
            M::WriteFile(_) => "host_response::write_file_result",
            M::SaveBuffer(_) => "host_response::save_buffer_result",
            M::DeleteFile(_) => "host_response::delete_file_result",
            M::DiscardFiles(_) => "host_response::discard_files_result",
            // Richer responses parsed at the manager call site.
            M::ReadFileContext(_) => "manager::read_file_context",
            M::GetFragmentMetadataFromHash(_) => "manager::get_fragment_metadata_from_hash",
            M::UploadHandoffSnapshot(_) => "manager::upload_handoff_snapshot",
            M::GetBranches(_) => "manager::get_branches",
            M::IndexCodebase(_) => "manager::index_codebase",
            M::DropCodebaseIndex(_) => "manager::drop_codebase_index",
            M::ResyncCodebase(_) => "manager::resync_codebase",
            M::ResolveConflict(_) => "manager::resolve_conflict",
            M::GitCommitChain(_) => "manager::commit_chain",
            M::GitPush(_) => "manager::push",
            M::GitCreatePr(_) => "manager::create_pr",
            M::GitGetPrInfo(_) => "manager::get_pr_info",
            M::GitGenerateCommitMessage(_) => "manager::generate_commit_message",
            M::GitGetCommittedBranchFiles(_) => "manager::get_committed_branch_files",
        }
    }

    // Referenced so the exhaustive match is compiled and checked.
    let _ = disposition;
}
