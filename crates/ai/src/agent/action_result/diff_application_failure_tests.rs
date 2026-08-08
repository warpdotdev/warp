use super::*;

// ---------------------------------------------------------------------------
// render() — wording preservation (no-regression lock-in)
// ---------------------------------------------------------------------------

#[test]
fn render_single_unmatched() {
    let failures = vec![DiffApplicationFailure::UnmatchedDiffs {
        file: "file.txt".to_string(),
        fuzzy_match_failure_count: 1,
        changes_already_applied_count: 0,
        search_block_failures: vec![],
    }];
    assert_eq!(render(&failures), "Could not apply all diffs to file.txt.");
}

#[test]
fn render_single_noop() {
    let failures = vec![DiffApplicationFailure::UnmatchedDiffs {
        file: "file.txt".to_string(),
        fuzzy_match_failure_count: 0,
        changes_already_applied_count: 1,
        search_block_failures: vec![],
    }];
    assert_eq!(
        render(&failures),
        "The changes to file.txt were already made."
    );
}

#[test]
fn render_unmatched_and_noop_in_one_entry() {
    // Single entry → no bullet prefix; wording matches old to_conversation_message().
    let failures = vec![DiffApplicationFailure::UnmatchedDiffs {
        file: "file.txt".to_string(),
        fuzzy_match_failure_count: 2,
        changes_already_applied_count: 2,
        search_block_failures: vec![],
    }];
    assert_eq!(
        render(&failures),
        "Could not apply all diffs to file.txt. The changes to file.txt were already made."
    );
}

#[test]
fn render_multiple_failures_uses_bullets() {
    let failures = vec![
        DiffApplicationFailure::MissingFile {
            file: "missing.rs".to_string(),
        },
        DiffApplicationFailure::UnmatchedDiffs {
            file: "unmatched.rs".to_string(),
            fuzzy_match_failure_count: 1,
            changes_already_applied_count: 0,
            search_block_failures: vec![],
        },
    ];
    assert_eq!(
        render(&failures),
        "* missing.rs does not exist. Is the path correct?\n* Could not apply all diffs to unmatched.rs."
    );
}

#[test]
fn render_single_read_failed() {
    let failures = vec![DiffApplicationFailure::ReadFailed {
        file: "no_permissions.scala".to_string(),
    }];
    assert_eq!(render(&failures), "Could not read no_permissions.scala");
}

#[test]
fn render_opaque() {
    let failures = vec![DiffApplicationFailure::Opaque {
        message: "Something went wrong".to_string(),
    }];
    assert_eq!(render(&failures), "Something went wrong");
}

#[test]
fn render_changes_already_applied_standalone() {
    let failures = vec![DiffApplicationFailure::ChangesAlreadyApplied {
        file: "main.rs".to_string(),
    }];
    assert_eq!(
        render(&failures),
        "The changes to main.rs were already made."
    );
}

#[test]
fn render_already_exists() {
    let failures = vec![DiffApplicationFailure::AlreadyExists {
        file: "new.rs".to_string(),
    }];
    assert_eq!(
        render(&failures),
        "Could not create new.rs because it already exists."
    );
}

#[test]
fn render_no_diffs_applicable() {
    let failures = vec![DiffApplicationFailure::NoDiffsApplicable];
    assert_eq!(render(&failures), "No diffs could be applied.");
}

#[test]
fn render_remote_file_operations_unsupported() {
    let failures = vec![DiffApplicationFailure::RemoteFileOperationsUnsupported];
    assert_eq!(
        render(&failures),
        "The file read/edit tool is not available on this remote session. Try using a different tool."
    );
}

// ---------------------------------------------------------------------------
// Redacted Debug — sensitive content must not leak into logs or crash reports
// ---------------------------------------------------------------------------

#[test]
fn debug_redacts_file_path_in_all_variants() {
    let sensitive_path = "super/secret/path.rs";
    let variants: Vec<DiffApplicationFailure> = vec![
        DiffApplicationFailure::UnmatchedDiffs {
            file: sensitive_path.to_string(),
            fuzzy_match_failure_count: 1,
            changes_already_applied_count: 0,
            search_block_failures: vec![],
        },
        DiffApplicationFailure::ChangesAlreadyApplied {
            file: sensitive_path.to_string(),
        },
        DiffApplicationFailure::MissingFile {
            file: sensitive_path.to_string(),
        },
        DiffApplicationFailure::ReadFailed {
            file: sensitive_path.to_string(),
        },
        DiffApplicationFailure::AlreadyExists {
            file: sensitive_path.to_string(),
        },
        DiffApplicationFailure::MultipleFileCreation {
            file: sensitive_path.to_string(),
        },
        DiffApplicationFailure::MultipleFileRenames {
            file: sensitive_path.to_string(),
        },
        DiffApplicationFailure::MutatedDeletedFile {
            file: sensitive_path.to_string(),
        },
        DiffApplicationFailure::Opaque {
            message: format!("Error touching {sensitive_path}"),
        },
    ];
    for failure in &variants {
        let debug_str = format!("{failure:?}");
        assert!(
            !debug_str.contains(sensitive_path),
            "Sensitive path must not appear in Debug output for {:?}",
            std::mem::discriminant(failure)
        );
        assert!(
            debug_str.contains("<redacted>"),
            "Debug output should contain <redacted> marker"
        );
    }
}

#[test]
fn debug_redacts_search_text_in_search_block_failure() {
    let sensitive_search = "my secret function_name(args)";
    let block = DiffSearchBlockFailure {
        search: sensitive_search.to_string(),
        expected_range: Some(10..20),
    };
    let debug_str = format!("{block:?}");
    assert!(
        !debug_str.contains(sensitive_search),
        "search text must not appear in DiffSearchBlockFailure Debug output"
    );
    assert!(debug_str.contains("<redacted>"));
    // The expected_range (non-sensitive) should still be present.
    assert!(debug_str.contains("expected_range"));
}

#[test]
fn debug_redacts_search_text_in_unmatched_diffs() {
    let sensitive_search = "let very_secret_variable = 42;";
    let failure = DiffApplicationFailure::UnmatchedDiffs {
        file: "any_file.rs".to_string(),
        fuzzy_match_failure_count: 1,
        changes_already_applied_count: 0,
        search_block_failures: vec![DiffSearchBlockFailure {
            search: sensitive_search.to_string(),
            expected_range: Some(1..5),
        }],
    };
    let debug_str = format!("{failure:?}");
    assert!(
        !debug_str.contains(sensitive_search),
        "search text must not appear in UnmatchedDiffs Debug output"
    );
    assert!(
        !debug_str.contains("any_file.rs"),
        "file path must not appear in Debug output"
    );
}
