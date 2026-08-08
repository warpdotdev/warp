use super::*;
use crate::agent::action_result::diff_application_failure::{
    DiffApplicationFailure, DiffSearchBlockFailure, MAX_DIFF_MATCH_FAILURE_BYTES,
};

// ---------------------------------------------------------------------------
// DiffApplicationFailure → proto (encode path, PRODUCT 2/7/8/10-12)
// ---------------------------------------------------------------------------

/// Helper: encode a single-failure DiffApplicationFailed result and extract
/// the proto Error from the result.  Panics if the conversion yields
/// anything other than an ApplyFileDiffs/Error.
fn encode_failures(failures: Vec<DiffApplicationFailure>) -> api::apply_file_diffs_result::Error {
    let result = api::request::input::tool_call_result::Result::try_from(
        RequestFileEditsResult::DiffApplicationFailed { failures },
    )
    .expect("DiffApplicationFailed should convert");

    let api::request::input::tool_call_result::Result::ApplyFileDiffs(apply) = result else {
        panic!("expected ApplyFileDiffs result");
    };
    let Some(api::apply_file_diffs_result::Result::Error(error)) = apply.result else {
        panic!("expected Error result");
    };
    error
}

#[test]
fn diff_application_failed_message_always_populated() {
    // PRODUCT 10-12: Error.message must always be set for back-compat even
    // when structured failures are present.
    let error = encode_failures(vec![DiffApplicationFailure::MissingFile {
        file: "foo.rs".to_string(),
    }]);
    assert!(
        !error.message.is_empty(),
        "Error.message must be populated alongside Error.failures"
    );
    assert_eq!(error.message, "foo.rs does not exist. Is the path correct?");
    assert!(
        !error.failures.is_empty(),
        "Error.failures must also be present"
    );
}

#[test]
fn diff_application_failed_missing_file_maps_to_proto_missing_file() {
    use api::apply_file_diffs_result::failure::Kind;
    let error = encode_failures(vec![DiffApplicationFailure::MissingFile {
        file: "src/lib.rs".to_string(),
    }]);
    assert_eq!(error.failures.len(), 1);
    assert!(matches!(error.failures[0].kind, Some(Kind::MissingFile(_))));
}

#[test]
fn diff_application_failed_read_failed_maps_to_proto_read_failed() {
    use api::apply_file_diffs_result::failure::Kind;
    let error = encode_failures(vec![DiffApplicationFailure::ReadFailed {
        file: "src/lib.rs".to_string(),
    }]);
    assert!(matches!(error.failures[0].kind, Some(Kind::ReadFailed(_))));
}

#[test]
fn diff_application_failed_already_exists_maps_to_proto_already_exists() {
    use api::apply_file_diffs_result::failure::Kind;
    let error = encode_failures(vec![DiffApplicationFailure::AlreadyExists {
        file: "new.rs".to_string(),
    }]);
    assert!(matches!(
        error.failures[0].kind,
        Some(Kind::AlreadyExists(_))
    ));
}

#[test]
fn diff_application_failed_multiple_file_creation_maps_correctly() {
    use api::apply_file_diffs_result::failure::Kind;
    let error = encode_failures(vec![DiffApplicationFailure::MultipleFileCreation {
        file: "dup.rs".to_string(),
    }]);
    assert!(matches!(
        error.failures[0].kind,
        Some(Kind::MultipleFileCreation(_))
    ));
}

#[test]
fn diff_application_failed_multiple_file_renames_maps_correctly() {
    use api::apply_file_diffs_result::failure::Kind;
    let error = encode_failures(vec![DiffApplicationFailure::MultipleFileRenames {
        file: "dup.rs".to_string(),
    }]);
    assert!(matches!(
        error.failures[0].kind,
        Some(Kind::MultipleFileRenames(_))
    ));
}

#[test]
fn diff_application_failed_mutated_deleted_file_maps_correctly() {
    use api::apply_file_diffs_result::failure::Kind;
    let error = encode_failures(vec![DiffApplicationFailure::MutatedDeletedFile {
        file: "gone.rs".to_string(),
    }]);
    assert!(matches!(
        error.failures[0].kind,
        Some(Kind::MutatedDeletedFile(_))
    ));
}

#[test]
fn diff_application_failed_no_diffs_applicable_maps_correctly() {
    use api::apply_file_diffs_result::failure::Kind;
    let error = encode_failures(vec![DiffApplicationFailure::NoDiffsApplicable]);
    assert!(matches!(
        error.failures[0].kind,
        Some(Kind::NoDiffsApplicable(()))
    ));
}

#[test]
fn diff_application_failed_remote_ops_unsupported_maps_correctly() {
    use api::apply_file_diffs_result::failure::Kind;
    let error = encode_failures(vec![
        DiffApplicationFailure::RemoteFileOperationsUnsupported,
    ]);
    assert!(matches!(
        error.failures[0].kind,
        Some(Kind::RemoteFileOperationsUnsupported(()))
    ));
}

#[test]
fn diff_application_failed_opaque_maps_to_proto_opaque() {
    use api::apply_file_diffs_result::failure::Kind;
    let error = encode_failures(vec![DiffApplicationFailure::Opaque {
        message: "save error".to_string(),
    }]);
    let Some(Kind::Opaque(o)) = &error.failures[0].kind else {
        panic!("expected Opaque kind");
    };
    assert_eq!(o.message, "save error");
}

#[test]
fn diff_application_failed_changes_already_applied_maps_correctly() {
    use api::apply_file_diffs_result::failure::Kind;
    let error = encode_failures(vec![DiffApplicationFailure::ChangesAlreadyApplied {
        file: "foo.rs".to_string(),
    }]);
    assert!(matches!(
        error.failures[0].kind,
        Some(Kind::ChangesAlreadyApplied(_))
    ));
}

#[test]
fn diff_application_failed_unmatched_emits_changes_already_applied_when_noop_present() {
    // When a single UnmatchedDiffs has both fuzzy failures AND noop deltas,
    // the proto emits two separate Failure entries.
    use api::apply_file_diffs_result::failure::Kind;
    let error = encode_failures(vec![DiffApplicationFailure::UnmatchedDiffs {
        file: "foo.rs".to_string(),
        fuzzy_match_failure_count: 2,
        changes_already_applied_count: 1,
        search_block_failures: vec![],
    }]);
    assert_eq!(error.failures.len(), 2);
    assert!(matches!(
        error.failures[0].kind,
        Some(Kind::UnmatchedDiffs(_))
    ));
    assert!(matches!(
        error.failures[1].kind,
        Some(Kind::ChangesAlreadyApplied(_))
    ));
}

#[test]
fn diff_application_failed_search_block_truncated_when_over_cap() {
    // PRODUCT 7/8: truncated is set when search text exceeds MAX_DIFF_MATCH_FAILURE_BYTES.
    use api::apply_file_diffs_result::failure::Kind;
    let long_search = "x".repeat(MAX_DIFF_MATCH_FAILURE_BYTES + 100);
    let error = encode_failures(vec![DiffApplicationFailure::UnmatchedDiffs {
        file: "foo.rs".to_string(),
        fuzzy_match_failure_count: 1,
        changes_already_applied_count: 0,
        search_block_failures: vec![DiffSearchBlockFailure {
            search: long_search.clone(),
            expected_range: Some(1..10),
        }],
    }]);
    let Some(Kind::UnmatchedDiffs(u)) = &error.failures[0].kind else {
        panic!("expected UnmatchedDiffs kind");
    };
    assert_eq!(u.search_block_failures.len(), 1);
    let sb = &u.search_block_failures[0];
    assert!(
        sb.truncated,
        "truncated must be true when search exceeds cap"
    );
    assert!(
        sb.search.len() <= MAX_DIFF_MATCH_FAILURE_BYTES,
        "encoded search must not exceed byte cap"
    );
}

#[test]
fn diff_application_failed_search_block_not_truncated_when_under_cap() {
    // Short search block: truncated must be false.
    use api::apply_file_diffs_result::failure::Kind;
    let short_search = "fn foo() {".to_string();
    let error = encode_failures(vec![DiffApplicationFailure::UnmatchedDiffs {
        file: "foo.rs".to_string(),
        fuzzy_match_failure_count: 1,
        changes_already_applied_count: 0,
        search_block_failures: vec![DiffSearchBlockFailure {
            search: short_search.clone(),
            expected_range: None,
        }],
    }]);
    let Some(Kind::UnmatchedDiffs(u)) = &error.failures[0].kind else {
        panic!("expected UnmatchedDiffs kind");
    };
    let sb = &u.search_block_failures[0];
    assert!(!sb.truncated, "truncated must be false for short search");
    assert_eq!(sb.search, short_search);
}

#[test]
fn diff_application_failed_unknown_range_encodes_as_zero() {
    // PRODUCT 8: when no expected line range is known, both fields must be 0.
    use api::apply_file_diffs_result::failure::Kind;
    let error = encode_failures(vec![DiffApplicationFailure::UnmatchedDiffs {
        file: "foo.rs".to_string(),
        fuzzy_match_failure_count: 1,
        changes_already_applied_count: 0,
        search_block_failures: vec![DiffSearchBlockFailure {
            search: "some content".to_string(),
            expected_range: None,
        }],
    }]);
    let Some(Kind::UnmatchedDiffs(u)) = &error.failures[0].kind else {
        panic!("expected UnmatchedDiffs kind");
    };
    let sb = &u.search_block_failures[0];
    assert_eq!(sb.expected_start_line, 0, "start must be 0 when unknown");
    assert_eq!(sb.expected_end_line, 0, "end must be 0 when unknown");
}

#[test]
fn diff_application_failed_known_range_encodes_correctly() {
    // Internal range 5..11 (exclusive end) → proto start=5, end=10 (inclusive).
    use api::apply_file_diffs_result::failure::Kind;
    let error = encode_failures(vec![DiffApplicationFailure::UnmatchedDiffs {
        file: "foo.rs".to_string(),
        fuzzy_match_failure_count: 1,
        changes_already_applied_count: 0,
        search_block_failures: vec![DiffSearchBlockFailure {
            search: "some content".to_string(),
            expected_range: Some(5..11),
        }],
    }]);
    let Some(Kind::UnmatchedDiffs(u)) = &error.failures[0].kind else {
        panic!("expected UnmatchedDiffs kind");
    };
    let sb = &u.search_block_failures[0];
    assert_eq!(sb.expected_start_line, 5);
    assert_eq!(sb.expected_end_line, 10); // exclusive 11 → inclusive 10
}

// ---------------------------------------------------------------------------
// Read-back from proto (PRODUCT 13)
// ---------------------------------------------------------------------------

/// Helper: build a proto Error with only a back-compat `message` field.
fn proto_error_message_only(msg: &str) -> api::apply_file_diffs_result::Error {
    api::apply_file_diffs_result::Error {
        message: msg.to_string(),
        failures: vec![],
    }
}

#[test]
fn read_back_message_only_reconstructs_as_opaque() {
    // PRODUCT 13: a proto Error with no structured failures wraps the
    // back-compat `message` in DiffApplicationFailure::Opaque.
    let error = proto_error_message_only("Could not apply diffs");
    let failures = super::failures_from_proto_error(&error);
    assert_eq!(failures.len(), 1);
    assert!(
        matches!(&failures[0], DiffApplicationFailure::Opaque { message } if message == "Could not apply diffs"),
        "Expected Opaque failure with the back-compat message"
    );
}

#[test]
fn read_back_structured_failures_reconstructs_into_category() {
    // PRODUCT 13: a proto Error carrying failures reconstructs into the
    // matching DiffApplicationFailure variant.
    use api::apply_file_diffs_result::{Failure, failure as f};
    let error = api::apply_file_diffs_result::Error {
        message: "Could not apply diffs".to_string(),
        failures: vec![
            Failure {
                kind: Some(f::Kind::MissingFile(f::MissingFile {
                    file: "missing.rs".to_string(),
                })),
            },
            Failure {
                kind: Some(f::Kind::Opaque(f::Opaque {
                    message: "legacy message".to_string(),
                })),
            },
        ],
    };
    let failures = super::failures_from_proto_error(&error);
    assert_eq!(failures.len(), 2);
    assert!(
        matches!(&failures[0], DiffApplicationFailure::MissingFile { file } if file == "missing.rs"),
        "Expected MissingFile variant"
    );
    assert!(
        matches!(&failures[1], DiffApplicationFailure::Opaque { message } if message == "legacy message"),
        "Expected Opaque variant"
    );
}

#[test]
fn read_back_unrecognised_kind_is_silently_skipped() {
    // Unknown kinds (e.g. from a newer proto) must not panic.
    use api::apply_file_diffs_result::{Failure, failure as f};
    let error = api::apply_file_diffs_result::Error {
        message: "fallback".to_string(),
        failures: vec![
            // A Failure with kind=None represents an unrecognised oneof variant.
            Failure { kind: None },
            Failure {
                kind: Some(f::Kind::MissingFile(f::MissingFile {
                    file: "foo.rs".to_string(),
                })),
            },
        ],
    };
    let failures = super::failures_from_proto_error(&error);
    // The None-kind entry is skipped; only the MissingFile survives.
    assert_eq!(failures.len(), 1);
    assert!(matches!(
        &failures[0],
        DiffApplicationFailure::MissingFile { .. }
    ));
}

#[test]
fn read_back_all_unrecognised_falls_back_to_opaque() {
    // If every structured entry has an unrecognised kind (forward-compat), fall
    // back to the back-compat `message` so render() never returns an empty string.
    use api::apply_file_diffs_result::Failure;
    let error = api::apply_file_diffs_result::Error {
        message: "fallback message".to_string(),
        failures: vec![Failure { kind: None }, Failure { kind: None }],
    };
    let failures = super::failures_from_proto_error(&error);
    assert_eq!(failures.len(), 1);
    assert!(
        matches!(&failures[0], DiffApplicationFailure::Opaque { message } if message == "fallback message"),
        "all-unrecognised entries must fall back to Opaque with the back-compat message"
    );
}

#[test]
fn read_back_changes_already_applied_merged_into_unmatched_diffs() {
    // PRODUCT 9 / round-trip: diff_application_failure_to_proto emits a separate
    // ChangesAlreadyApplied when changes_already_applied_count > 0; failures_from_proto_error
    // must merge it back so render() produces the combined single-line wording.
    use api::apply_file_diffs_result::{Failure, failure as f};
    let error = api::apply_file_diffs_result::Error {
        message: "Could not apply all diffs to f.rs. The changes to f.rs were already made."
            .to_string(),
        failures: vec![
            Failure {
                kind: Some(f::Kind::UnmatchedDiffs(f::UnmatchedDiffs {
                    file: "f.rs".to_string(),
                    fuzzy_match_failure_count: 1,
                    search_block_failures: vec![],
                })),
            },
            Failure {
                kind: Some(f::Kind::ChangesAlreadyApplied(f::ChangesAlreadyApplied {
                    file: "f.rs".to_string(),
                })),
            },
        ],
    };
    let failures = super::failures_from_proto_error(&error);
    // Should merge into one combined entry, not two separate ones.
    assert_eq!(
        failures.len(),
        1,
        "ChangesAlreadyApplied must be merged back into UnmatchedDiffs"
    );
    let DiffApplicationFailure::UnmatchedDiffs {
        file,
        fuzzy_match_failure_count,
        changes_already_applied_count,
        ..
    } = &failures[0]
    else {
        panic!("expected UnmatchedDiffs after merge, got {:?}", failures[0]);
    };
    assert_eq!(file, "f.rs");
    assert_eq!(*fuzzy_match_failure_count, 1);
    assert_eq!(*changes_already_applied_count, 1);
}

#[test]
fn read_files_partial_success_converts_failed_files() {
    let result =
        api::request::input::tool_call_result::Result::try_from(ReadFilesResult::Success {
            files: vec![FileContext::new(
                "/tmp/success.txt".to_string(),
                AnyFileContent::StringContent("hello".to_string()),
                None,
                None,
            )],
            failed_files: vec![ReadFilesFailedFile {
                path: "/tmp/missing.txt".to_string(),
                message: "File not found or could not be read".to_string(),
            }],
        })
        .expect("read_files success should convert");

    let api::request::input::tool_call_result::Result::ReadFiles(result) = result else {
        panic!("expected read_files result");
    };

    let Some(api::read_files_result::Result::AnyFilesSuccess(success)) = result.result else {
        panic!("expected any files success result");
    };

    assert_eq!(success.files.len(), 1);
    assert_eq!(success.failed_reads.len(), 1);
    assert_eq!(success.failed_reads[0].path, "/tmp/missing.txt");
    assert_eq!(
        success.failed_reads[0].message,
        "File not found or could not be read"
    );
}

#[test]
fn ask_user_question_skipped_by_auto_approve_converts_to_skipped_answers() {
    let result = api::request::input::tool_call_result::Result::from(
        AskUserQuestionResult::SkippedByAutoApprove {
            question_ids: vec!["q1".to_string(), "q2".to_string()],
        },
    );

    let api::request::input::tool_call_result::Result::AskUserQuestion(result) = result else {
        panic!("expected ask_user_question result");
    };

    let Some(api::ask_user_question_result::Result::Success(success)) = result.result else {
        panic!("expected success result");
    };

    assert_eq!(success.answers.len(), 2);
    assert_eq!(success.answers[0].question_id, "q1");
    assert_eq!(success.answers[1].question_id, "q2");
    assert!(matches!(
        success.answers[0].answer,
        Some(AskUserQuestionAnswer::Skipped(()))
    ));
    assert!(matches!(
        success.answers[1].answer,
        Some(AskUserQuestionAnswer::Skipped(()))
    ));
}
