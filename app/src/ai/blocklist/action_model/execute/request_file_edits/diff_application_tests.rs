use std::io::Write as _;
use std::sync::Arc;

use ai::diff_validation::{DiffDelta, ParsedDiff, V4AHunk};
use async_io::block_on;
use tempfile::NamedTempFile;
use vec1::vec1;
use warpui::App;

use super::*;
use crate::ai::agent::{AIIdentifiers, FileEdit};
use crate::ai::blocklist::SessionContext;
use crate::auth::auth_state::AuthState;

fn update_deltas(diff: &AIRequestedCodeDiff) -> &[DiffDelta] {
    match &diff.diff_type {
        DiffType::Update { deltas, .. } => deltas,
        other => panic!("Expected Update diff_type, got {other:?}"),
    }
}

#[test]
fn test_apply_diffs_error_when_no_diffs_applied() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "First line\nSecond line\n").unwrap();

        // Create a diff that won't match the file content.
        let invalid_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file_path.clone()),
            search: Some("1|This content doesn't exist in the file".to_string()),
            replace: Some("Replacement content".to_string()),
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(invalid_diff)],
            &session_context,
            ai_identifiers,
            app.background_executor(),
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = result.expect_err("Expected an error due to unmatched diff");
        match &errors[..] {
            [DiffApplicationError::UnmatchedDiffs { file, .. }] => {
                assert_eq!(*file, file_path);
            }
            other => panic!("Expected a single UnmatchedDiffs error, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_diffs_succeeds_with_valid_diff() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "First line\nSecond line\n").unwrap();

        // Create a valid diff
        let valid_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file_path.clone()),
            search: Some("1|First line".to_string()),
            replace: Some("Modified first line".to_string()),
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let background_executor = app.background_executor();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(valid_diff)],
            &session_context,
            ai_identifiers,
            background_executor,
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        // Should succeed with a valid diff
        assert!(result.is_ok(), "Expected Ok result but got: {result:?}");

        let diffs = result.unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_name, file_path);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].insertion, "Modified first line");
    });
}

#[test]
fn test_apply_diffs_with_partial_failures() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "First line\nSecond line\n").unwrap();

        // Create one valid diff and one invalid diff.
        let valid_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file_path.clone()),
            search: Some("1|First line".to_string()),
            replace: Some("Modified first line".to_string()),
        };

        let invalid_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file_path.clone()),
            search: Some("1|This content doesn't exist".to_string()),
            replace: Some("Replacement content".to_string()),
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let background_executor = app.background_executor();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(valid_diff), FileEdit::Edit(invalid_diff)],
            &session_context,
            ai_identifiers,
            background_executor,
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        // With mixed valid and invalid diffs, we should get an error
        let errors = result.expect_err("Expected an error due to mixed valid/invalid diffs");
        match &errors[..] {
            [DiffApplicationError::UnmatchedDiffs { file, .. }] => {
                assert_eq!(*file, file_path);
            }
            other => panic!("Expected a single UnmatchedDiffs error, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_diffs_with_new_file() {
    // TODO(ben): Drop support for this behavior once the file-creation tool is live.
    App::test((), |app| async move {
        // Create a diff for a non-existent file with empty search (file creation)
        let non_existent_file = "non_existent_file.txt".to_string();
        let create_file_diff = ParsedDiff::StrReplaceEdit {
            file: Some(non_existent_file.clone()),
            search: Some("".to_string()),
            replace: Some("New file content".to_string()),
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let background_executor = app.background_executor();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(create_file_diff)],
            &session_context,
            ai_identifiers,
            background_executor.clone(),
            auth_state.clone(),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        // Should succeed with a file creation diff
        assert!(result.is_ok(), "Expected Ok result but got: {result:?}");

        let diffs = result.unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_name, non_existent_file);
        assert_eq!(diffs[0].failures, None);

        match &diffs[0].diff_type {
            DiffType::Create { delta } => {
                assert_eq!(delta.insertion, "New file content");
            }
            other => panic!("Expected Create diff_type, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_diffs_with_missing_file() {
    App::test((), |app| async move {
        let non_existent_file = "non_existent_file.txt".to_string();

        // Create a diff for a non-existent file with non-empty search (should fail)
        let invalid_non_existent_diff = ParsedDiff::StrReplaceEdit {
            file: Some(non_existent_file.clone()),
            search: Some("1|Some content".to_string()),
            replace: Some("New content".to_string()),
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let background_executor = app.background_executor();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = block_on(apply_edits(
            vec![FileEdit::Edit(invalid_non_existent_diff)],
            &session_context,
            ai_identifiers,
            background_executor,
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        ));

        // Should fail due to the invalid diff.
        let errors = result.expect_err("Expected an error due to missing file");
        match &errors[..] {
            [DiffApplicationError::MissingFile { file }] => {
                assert_eq!(*file, non_existent_file);
            }
            other => panic!("Expected a single MissingFile error, got {other:?}"),
        }
    });
}

#[test]
fn test_parse_diffs_with_mixed_empty_and_valid_diffs() {
    App::test((), |app| async move {
        let mut file1 = NamedTempFile::new().expect("Failed to create first temporary file");
        let file1_path = file1.path().to_string_lossy().to_string();
        writeln!(&mut file1, "File 1 content\nSecond line\n").unwrap();

        let mut file2 = NamedTempFile::new().expect("Failed to create second temporary file");
        let file2_path = file2.path().to_string_lossy().to_string();
        writeln!(&mut file2, "File 2 content\nAnother line\n").unwrap();

        let valid_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file1_path.clone()),
            search: Some("1|File 1 content".to_string()),
            replace: Some("Modified file 1 content".to_string()),
        };

        let invalid_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file2_path.clone()),
            search: Some("1|This doesn't match anything".to_string()),
            replace: Some("New content".to_string()),
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();

        let background_executor = app.background_executor();
        let auth_state = Arc::new(AuthState::new_for_test());

        // Even though we could apply a diff to file1, no diffs could be applied to file2. Overall,
        // this is an error because there's at least one file with an empty diff.
        let result = apply_edits(
            vec![FileEdit::Edit(valid_diff), FileEdit::Edit(invalid_diff)],
            &session_context,
            ai_identifiers,
            background_executor,
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        // Should fail because one of the diffs didn't match.
        let errors = result.expect_err("Expected an error due to unmatched diff on second file");
        match &errors[..] {
            [DiffApplicationError::UnmatchedDiffs { file, .. }] => {
                assert_eq!(*file, file2_path);
            }
            other => panic!("Expected a single UnmatchedDiffs error, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_diffs_noop_with_successful_change() {
    App::test((), |app| async move {
        let mut file = NamedTempFile::new().expect("Failed to create temporary file");
        writeln!(&mut file, "Line One\nLine Two\n").unwrap();
        let file_path = file.path().to_string_lossy().to_string();

        let diffs = vec![
            // This is effectively a no-op.
            ParsedDiff::StrReplaceEdit {
                file: Some(file_path.clone()),
                search: Some("1|Line one".to_string()),
                replace: Some("Line One".to_string()),
            },
            // This is a meaningful change.
            ParsedDiff::StrReplaceEdit {
                file: Some(file_path.clone()),
                search: Some("2|Line Two".to_string()),
                replace: Some("Last Line".to_string()),
            },
        ];

        let result = apply_edits(
            diffs.into_iter().map(FileEdit::Edit).collect(),
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        assert!(result.is_ok());
        let diffs = result.unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_name, file_path);
        assert_eq!(diffs[0].failures, None);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(
            deltas[0],
            DiffDelta {
                insertion: "Last Line".to_string(),
                replacement_line_range: 2..3,
            }
        );
    });
}

#[test]
fn test_apply_diffs_fails_with_only_noop() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        let content = "First line\nSecond line\n";
        writeln!(temp_file, "{content}").unwrap();

        // Create a diff that exactly matches the existing content (making it a noop)
        let noop_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file_path.clone()),
            search: Some("1|First line".to_string()),
            replace: Some("First line".to_string()),
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(noop_diff)],
            &session_context,
            ai_identifiers,
            app.background_executor(),
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = result.expect_err("Expected an error due to noop diff");
        match &errors[..] {
            [DiffApplicationError::UnmatchedDiffs {
                file,
                match_failures,
            }] => {
                assert_eq!(*file, file_path);
                assert_eq!(match_failures.noop_deltas, 1);
                assert_eq!(match_failures.fuzzy_match_failures, 0);
            }
            other => panic!("Expected a single UnmatchedDiffs error, got {other:?}"),
        }
    });
}

#[test]
fn test_multiple_file_create_edits_for_same_path() {
    App::test((), |app| async move {
        let file_path = "new_file.txt".to_string();

        // Create two FileEdit::Create edits for the same file path
        let create_edit1 = FileEdit::Create {
            file: Some(file_path.clone()),
            content: Some("First content".to_string()),
        };
        let create_edit2 = FileEdit::Create {
            file: Some(file_path.clone()),
            content: Some("Second content".to_string()),
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let background_executor = app.background_executor();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![create_edit1, create_edit2],
            &session_context,
            ai_identifiers,
            background_executor,
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        // Should fail due to multiple creation attempts for the same file
        let errors = result.expect_err("Expected an error due to multiple file creation attempts");
        match &errors[..] {
            [DiffApplicationError::MultipleFileCreation { file }] => {
                assert_eq!(*file, file_path);
            }
            other => panic!("Expected a single MultipleFileCreation error, got {other:?}"),
        }
    });
}

#[test]
fn test_mixed_create_and_edit_for_same_path() {
    App::test((), |app| async move {
        let file_path = "mixed_file.txt".to_string();

        // Create a FileEdit::Create and FileEdit::Edit for the same file path
        let create_edit = FileEdit::Create {
            file: Some(file_path.clone()),
            content: Some("New file content".to_string()),
        };
        let edit_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file_path.clone()),
            search: Some("1|Some existing content".to_string()),
            replace: Some("Modified content".to_string()),
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let background_executor = app.background_executor();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![create_edit, FileEdit::Edit(edit_diff)],
            &session_context,
            ai_identifiers,
            background_executor,
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        // Should fail due to mixed create and edit for the same file
        let errors =
            result.expect_err("Expected an error due to mixed create and edit for same file");
        match &errors[..] {
            [DiffApplicationError::MultipleFileCreation { file }] => {
                assert_eq!(*file, file_path);
            }
            other => panic!("Expected a single MultipleFileCreation error, got {other:?}"),
        }
    });
}

#[test]
fn test_delete_and_create_same_path_replaces_existing_file() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "Old line one\nOld line two").unwrap();

        let delete_edit = FileEdit::Delete {
            file: Some(file_path.clone()),
        };
        let create_edit = FileEdit::Create {
            file: Some(file_path.clone()),
            content: Some("New file content".to_string()),
        };

        let result = apply_edits(
            vec![delete_edit, create_edit],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        assert!(result.is_ok(), "Expected Ok result but got: {result:?}");
        let diffs = result.unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_name, file_path);
        assert_eq!(diffs[0].original_content, "Old line one\nOld line two\n");

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].replacement_line_range, 1..3);
        assert_eq!(deltas[0].insertion, "New file content");
    });
}

#[test]
fn test_create_then_delete_same_path_replaces_existing_file() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "Old line one\nOld line two").unwrap();

        let create_edit = FileEdit::Create {
            file: Some(file_path.clone()),
            content: Some("New file content".to_string()),
        };
        let delete_edit = FileEdit::Delete {
            file: Some(file_path.clone()),
        };

        let result = apply_edits(
            vec![create_edit, delete_edit],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        assert!(result.is_ok(), "Expected Ok result but got: {result:?}");
        let diffs = result.unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_name, file_path);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].replacement_line_range, 1..3);
        assert_eq!(deltas[0].insertion, "New file content");
    });
}

#[test]
fn test_delete_create_and_edit_same_path_still_fails() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "Existing content").unwrap();

        let delete_edit = FileEdit::Delete {
            file: Some(file_path.clone()),
        };
        let create_edit = FileEdit::Create {
            file: Some(file_path.clone()),
            content: Some("New file content".to_string()),
        };
        let edit_diff = ParsedDiff::StrReplaceEdit {
            file: Some(file_path.clone()),
            search: Some("1|Existing content".to_string()),
            replace: Some("Modified content".to_string()),
        };

        let result = apply_edits(
            vec![delete_edit, create_edit, FileEdit::Edit(edit_diff)],
            &SessionContext::new_for_test(),
            &AIIdentifiers::default(),
            app.background_executor(),
            Arc::new(AuthState::new_for_test()),
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = result.expect_err("Expected an error due to create/edit/delete same path");
        match &errors[..] {
            [DiffApplicationError::MultipleFileCreation { file }] => {
                assert_eq!(*file, file_path);
            }
            other => panic!("Expected a single MultipleFileCreation error, got {other:?}"),
        }
    });
}

#[test]
fn test_create_edit_for_existing_file() {
    App::test((), |app| async move {
        // Create a temporary file that already exists
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "Existing content").unwrap();

        // Try to create a file that already exists
        let create_edit = FileEdit::Create {
            file: Some(file_path.clone()),
            content: Some("New content".to_string()),
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let background_executor = app.background_executor();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![create_edit],
            &session_context,
            ai_identifiers,
            background_executor,
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        // Should fail because the file already exists
        let errors = result.expect_err("Expected an error because file already exists");
        match &errors[..] {
            [DiffApplicationError::AlreadyExists { file }] => {
                assert_eq!(*file, file_path);
            }
            other => panic!("Expected a single AlreadyExists error, got {other:?}"),
        }
    });
}

#[test]
fn test_format_match_error() {
    let err = DiffApplicationError::UnmatchedDiffs {
        file: "file.txt".to_string(),
        match_failures: DiffMatchFailures {
            fuzzy_match_failures: 1,
            noop_deltas: 0,
            missing_line_numbers: 0,
        },
    };

    assert_eq!(
        err.to_conversation_message(),
        "Could not apply all diffs to file.txt."
    );

    let err = DiffApplicationError::UnmatchedDiffs {
        file: "file.txt".to_string(),
        match_failures: DiffMatchFailures {
            fuzzy_match_failures: 0,
            noop_deltas: 1,
            missing_line_numbers: 0,
        },
    };

    assert_eq!(
        err.to_conversation_message(),
        "The changes to file.txt were already made."
    );

    let err = DiffApplicationError::UnmatchedDiffs {
        file: "file.txt".to_string(),
        match_failures: DiffMatchFailures {
            fuzzy_match_failures: 2,
            noop_deltas: 2,
            missing_line_numbers: 0,
        },
    };

    assert_eq!(
        err.to_conversation_message(),
        "Could not apply all diffs to file.txt. The changes to file.txt were already made."
    );
}

#[test]
fn test_format_multiple_errors() {
    let errs = vec1![
        DiffApplicationError::MissingFile {
            file: "missing.rs".to_string(),
        },
        DiffApplicationError::UnmatchedDiffs {
            file: "unmatched.rs".to_string(),
            match_failures: DiffMatchFailures {
                fuzzy_match_failures: 1,
                noop_deltas: 0,
                missing_line_numbers: 0,
            },
        },
    ];

    assert_eq!(
        DiffApplicationError::error_for_conversation(&errs),
        "* missing.rs does not exist. Is the path correct?\n* Could not apply all diffs to unmatched.rs."
    );
}

#[test]
fn test_format_single_errors() {
    let errs = vec1![DiffApplicationError::ReadFailed {
        file: "no_permissions.scala".to_string(),
        message: "permission denied".to_string(),
    },];

    assert_eq!(
        DiffApplicationError::error_for_conversation(&errs),
        "Could not read no_permissions.scala"
    );
}

// V4A Tests

#[test]
fn test_apply_v4a_edits_simple_match() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(
            &mut temp_file,
            "function foo() {{\n    console.log('hello');\n    return 42;\n}}"
        )
        .unwrap();

        // Create a V4A edit with context
        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(file_path.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "function foo() {".to_string(),
                old: "    console.log('hello');".to_string(),
                new: "    console.log('world');".to_string(),
                post_context: "    return 42;".to_string(),
            }],
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &session_context,
            ai_identifiers,
            app.background_executor(),
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        assert!(result.is_ok(), "Expected Ok result but got: {result:?}");
        let diffs = result.unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_name, file_path);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].insertion, "    console.log('world');");
        assert_eq!(deltas[0].replacement_line_range, 2..3);
    });
}

#[test]
fn test_apply_v4a_edits_with_jump_context() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(
            &mut temp_file,
            "class Foo {{\n    def bar():\n        pass\n    def baz():\n        return 1\n}}"
        )
        .unwrap();

        // Create a V4A edit with change context
        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(file_path.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec!["class Foo".to_string()],
                pre_context: "    def bar():".to_string(),
                old: "        pass".to_string(),
                new: "        return None".to_string(),
                post_context: "    def baz():".to_string(),
            }],
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &session_context,
            ai_identifiers,
            app.background_executor(),
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        assert!(result.is_ok(), "Expected Ok result but got: {result:?}");
        let diffs = result.unwrap();
        assert_eq!(diffs.len(), 1);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].insertion, "        return None");
    });
}

#[test]
fn test_apply_v4a_edits_no_match() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "First line\nSecond line\n").unwrap();

        // Create a V4A edit that won't match
        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(file_path.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "Non-existent pre context".to_string(),
                old: "Non-existent old content".to_string(),
                new: "New content".to_string(),
                post_context: "Non-existent post context".to_string(),
            }],
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &session_context,
            ai_identifiers,
            app.background_executor(),
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = result.expect_err("Expected an error due to unmatched V4A edit");
        match &errors[..] {
            [DiffApplicationError::UnmatchedDiffs { file, .. }] => {
                assert_eq!(*file, file_path);
            }
            other => panic!("Expected a single UnmatchedDiffs error, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_v4a_edits_noop() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "Line One\nLine Two\nLine Three").unwrap();

        // Create a V4A edit where old and new are identical (noop)
        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(file_path.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "Line One".to_string(),
                old: "Line Two".to_string(),
                new: "Line Two".to_string(),
                post_context: "Line Three".to_string(),
            }],
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &session_context,
            ai_identifiers,
            app.background_executor(),
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = result.expect_err("Expected an error due to noop V4A edit");
        match &errors[..] {
            [DiffApplicationError::UnmatchedDiffs {
                file,
                match_failures,
            }] => {
                assert_eq!(*file, file_path);
                assert_eq!(match_failures.noop_deltas, 1);
            }
            other => panic!("Expected a single UnmatchedDiffs error, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_v4a_edits_multiline_change() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(
            &mut temp_file,
            "def calculate():\n    x = 1\n    y = 2\n    return x + y\n"
        )
        .unwrap();

        // Create a V4A edit with multiline old and new content
        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(file_path.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "def calculate():".to_string(),
                old: "    x = 1\n    y = 2".to_string(),
                new: "    x = 10\n    y = 20".to_string(),
                post_context: "    return x + y".to_string(),
            }],
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &session_context,
            ai_identifiers,
            app.background_executor(),
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        assert!(result.is_ok(), "Expected Ok result but got: {result:?}");
        let diffs = result.unwrap();
        assert_eq!(diffs.len(), 1);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].insertion, "    x = 10\n    y = 20");
        assert_eq!(deltas[0].replacement_line_range, 2..4);
    });
}

#[test]
fn test_apply_v4a_edits_nested_jump_context() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(
            &mut temp_file,
            "class Outer {{\n    class Inner {{\n        def method():\n            pass\n    }}\n}}"
        )
        .unwrap();

        // Create a V4A edit with nested change context
        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(file_path.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec!["class Outer".to_string(), "class Inner".to_string()],
                pre_context: "        def method():".to_string(),
                old: "            pass".to_string(),
                new: "            return True".to_string(),
                post_context: "    }".to_string(),
            }],
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &session_context,
            ai_identifiers,
            app.background_executor(),
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        assert!(result.is_ok(), "Expected Ok result but got: {result:?}");
        let diffs = result.unwrap();
        assert_eq!(diffs.len(), 1);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].insertion, "            return True");
    });
}

#[test]
fn test_apply_v4a_edits_missing_file() {
    App::test((), |app| async move {
        let non_existent_file = "non_existent_file.txt".to_string();

        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(non_existent_file.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "pre".to_string(),
                old: "old content".to_string(),
                new: "new content".to_string(),
                post_context: "post".to_string(),
            }],
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &session_context,
            ai_identifiers,
            app.background_executor(),
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        let errors = result.expect_err("Expected an error due to missing file");
        match &errors[..] {
            [DiffApplicationError::MissingFile { file }] => {
                assert_eq!(*file, non_existent_file);
            }
            other => panic!("Expected a single MissingFile error, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_v4a_edits_empty_context() {
    App::test((), |app| async move {
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        let file_path = temp_file.path().to_string_lossy().to_string();
        writeln!(&mut temp_file, "first\nsecond\nthird").unwrap();

        // Create a V4A edit with empty pre and post context
        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(file_path.clone()),
            move_to: None,
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "".to_string(),
                old: "second".to_string(),
                new: "SECOND".to_string(),
                post_context: "".to_string(),
            }],
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &session_context,
            ai_identifiers,
            app.background_executor(),
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        assert!(result.is_ok(), "Expected Ok result but got: {result:?}");
        let diffs = result.unwrap();
        assert_eq!(diffs.len(), 1);

        let deltas = update_deltas(&diffs[0]);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].insertion, "SECOND");
    });
}

// V4A Rename Tests

#[test]
fn test_apply_v4a_rename_to_nonexistent_file() {
    App::test((), |app| async move {
        let mut source_file = NamedTempFile::new().expect("Failed to create source file");
        let source_path = source_file.path().to_string_lossy().to_string();
        writeln!(&mut source_file, "line one\nline two\nline three").unwrap();

        // Target file does not exist
        let target_path = format!("{}_renamed.txt", source_path);

        // Create a V4A edit with rename to non-existent file
        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(source_path.clone()),
            move_to: Some(target_path.clone()),
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "line one".to_string(),
                old: "line two".to_string(),
                new: "LINE TWO MODIFIED".to_string(),
                post_context: "line three".to_string(),
            }],
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &session_context,
            ai_identifiers,
            app.background_executor(),
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        assert!(result.is_ok(), "Expected Ok result but got: {result:?}");
        let diffs = result.unwrap();

        // Should produce a single Update diff with rename
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_name, source_path);

        match &diffs[0].diff_type {
            DiffType::Update { deltas, rename } => {
                assert_eq!(*rename, Some(target_path.into()));
                assert_eq!(deltas.len(), 1);
                assert_eq!(deltas[0].insertion, "LINE TWO MODIFIED");
            }
            other => panic!("Expected Update diff_type with rename, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_v4a_rename_to_existing_file() {
    App::test((), |app| async move {
        // Create source file A
        let mut source_file = NamedTempFile::new().expect("Failed to create source file");
        let source_path = source_file.path().to_string_lossy().to_string();
        writeln!(
            &mut source_file,
            "source line one\nsource line two\nsource line three"
        )
        .unwrap();

        // Create target file B (already exists)
        let mut target_file = NamedTempFile::new().expect("Failed to create target file");
        let target_path = target_file.path().to_string_lossy().to_string();
        writeln!(&mut target_file, "target old content\nshould be replaced").unwrap();

        // Create a V4A edit to rename A to B (where B exists) with a modification
        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(source_path.clone()),
            move_to: Some(target_path.clone()),
            hunks: vec![V4AHunk {
                change_context: vec![],
                pre_context: "source line one".to_string(),
                old: "source line two".to_string(),
                new: "MODIFIED LINE TWO".to_string(),
                post_context: "source line three".to_string(),
            }],
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &session_context,
            ai_identifiers,
            app.background_executor(),
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        assert!(result.is_ok(), "Expected Ok result but got: {result:?}");
        let diffs = result.unwrap();

        // Should produce TWO diffs: deletion for source, update for target
        assert_eq!(diffs.len(), 2);

        // First diff: deletion of source file A
        assert_eq!(diffs[0].file_name, source_path);
        match &diffs[0].diff_type {
            DiffType::Delete { .. } => {}
            other => panic!("Expected Delete diff_type for source, got {other:?}"),
        }

        // Second diff: update of target file B with source content (after applying deltas)
        assert_eq!(diffs[1].file_name, target_path);
        match &diffs[1].diff_type {
            DiffType::Update { deltas, rename } => {
                assert!(rename.is_none(), "Target update should not have rename");
                // Two deltas: one replaces target with source content, one applies the modification
                assert_eq!(deltas.len(), 2);
                // First delta: replaces target content with source content
                assert!(
                    deltas[0].insertion.contains("source line one"),
                    "Should contain source content"
                );
                assert!(
                    deltas[0].insertion.contains("source line two"),
                    "First delta should contain original source"
                );
                assert!(
                    deltas[0].insertion.contains("source line three"),
                    "Should contain source content"
                );
                // Second delta: applies the modification
                assert!(
                    deltas[1].insertion.contains("MODIFIED LINE TWO"),
                    "Should contain modified line"
                );
            }
            other => panic!("Expected Update diff_type for target, got {other:?}"),
        }
    });
}

#[test]
fn test_apply_v4a_rename_to_existing_file_no_deltas() {
    App::test((), |app| async move {
        // Create source file A
        let mut source_file = NamedTempFile::new().expect("Failed to create source file");
        let source_path = source_file.path().to_string_lossy().to_string();
        writeln!(&mut source_file, "source content only").unwrap();

        // Create target file B (already exists)
        let mut target_file = NamedTempFile::new().expect("Failed to create target file");
        let target_path = target_file.path().to_string_lossy().to_string();
        writeln!(&mut target_file, "target old content").unwrap();

        // Create a V4A edit to rename A to B with no actual content changes
        // (empty hunks list means just a rename)
        let v4a_edit = ParsedDiff::V4AEdit {
            file: Some(source_path.clone()),
            move_to: Some(target_path.clone()),
            hunks: vec![],
        };

        let ai_identifiers = &AIIdentifiers::default();
        let session_context = SessionContext::new_for_test();
        let auth_state = Arc::new(AuthState::new_for_test());

        let result = apply_edits(
            vec![FileEdit::Edit(v4a_edit)],
            &session_context,
            ai_identifiers,
            app.background_executor(),
            auth_state,
            false,
            |path| async move { FileReadResult::from(std::fs::read_to_string(path)) },
        )
        .await;

        assert!(result.is_ok(), "Expected Ok result but got: {result:?}");
        let diffs = result.unwrap();

        // Should produce TWO diffs: deletion for source, update for target
        assert_eq!(diffs.len(), 2);

        // First diff: deletion of source file A
        assert_eq!(diffs[0].file_name, source_path);
        match &diffs[0].diff_type {
            DiffType::Delete { .. } => {}
            other => panic!("Expected Delete diff_type for source, got {other:?}"),
        }

        // Second diff: update of target file B with source content (no modifications)
        assert_eq!(diffs[1].file_name, target_path);
        match &diffs[1].diff_type {
            DiffType::Update { deltas, rename } => {
                assert!(rename.is_none());
                assert_eq!(deltas.len(), 1);
                // The insertion should be exactly the source content (including trailing newline from writeln!)
                assert_eq!(deltas[0].insertion, "source content only\n");
            }
            other => panic!("Expected Update diff_type for target, got {other:?}"),
        }
    });
}
