use std::collections::HashMap;
use std::fs;

use ai::agent::action_result::AnyFileContent;
use ai::agent::FileLocations;
use ai::diff_validation::{DiffDelta, DiffType};
use warp_core::HostId;
use warp_files::FileModel;
use warpui::{App, SingletonEntity as _};

use super::{
    build_resolved_edits, outcome_for_action, updated_file_contexts_from_content_map,
    PersistAction, PersistDiffModel,
};
use crate::ai::agent::RequestFileEditsResult;
use crate::ai::blocklist::diff_types::{DiffSessionType, FileDiff};

/// Runs `resolve_and_persist` for the given local diffs on a fresh app and awaits the result.
async fn resolve_and_persist_local(
    app: &mut App,
    diffs: Vec<FileDiff>,
    reviewed: HashMap<String, String>,
) -> RequestFileEditsResult {
    // FileModel must exist before PersistDiffModel subscribes to it in `new`.
    app.add_singleton_model(FileModel::new);
    app.add_singleton_model(PersistDiffModel::new);
    let future = PersistDiffModel::handle(app).update(app, |model, ctx| {
        model.resolve_and_persist(diffs, reviewed, DiffSessionType::Local, ctx)
    });
    future.await
}

#[test]
fn persist_creates_a_new_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.rs").to_string_lossy().to_string();

        let result = resolve_and_persist_local(
            &mut app,
            vec![FileDiff::new(
                String::new(),
                path.clone(),
                DiffType::creation("fn main() {}\n".to_owned()),
            )],
            HashMap::new(),
        )
        .await;

        let RequestFileEditsResult::Success {
            updated_files,
            deleted_files,
            lines_added,
            ..
        } = result
        else {
            panic!("expected create to succeed");
        };
        assert_eq!(fs::read_to_string(&path).unwrap(), "fn main() {}\n");
        assert_eq!(lines_added, 1);
        assert_eq!(deleted_files, Vec::<String>::new());
        assert_eq!(updated_files.len(), 1);
        assert_eq!(updated_files[0].file_context.file_name, path);
    });
}

#[test]
fn persist_updates_an_existing_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.rs").to_string_lossy().to_string();
        fs::write(&path, "one\ntwo\nthree\n").unwrap();

        let result = resolve_and_persist_local(
            &mut app,
            vec![FileDiff::new(
                "one\ntwo\nthree\n".to_owned(),
                path.clone(),
                DiffType::update(
                    vec![DiffDelta {
                        replacement_line_range: 2..3,
                        insertion: "TWO\n".to_owned(),
                    }],
                    None,
                ),
            )],
            HashMap::new(),
        )
        .await;

        assert!(matches!(result, RequestFileEditsResult::Success { .. }));
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\nTWO\nthree\n");
    });
}

#[test]
fn persist_prefers_reviewed_content_over_deltas() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.rs").to_string_lossy().to_string();
        fs::write(&path, "one\ntwo\nthree\n").unwrap();

        let reviewed = HashMap::from([(path.clone(), "user edited\n".to_owned())]);
        let result = resolve_and_persist_local(
            &mut app,
            vec![FileDiff::new(
                "one\ntwo\nthree\n".to_owned(),
                path.clone(),
                DiffType::update(
                    vec![DiffDelta {
                        replacement_line_range: 2..3,
                        insertion: "TWO\n".to_owned(),
                    }],
                    None,
                ),
            )],
            reviewed,
        )
        .await;

        assert!(matches!(result, RequestFileEditsResult::Success { .. }));
        assert_eq!(fs::read_to_string(&path).unwrap(), "user edited\n");
    });
}

#[test]
fn persist_renames_and_reports_source_as_deleted() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let old_path = dir.path().join("old.rs").to_string_lossy().to_string();
        let new_path = dir.path().join("new.rs").to_string_lossy().to_string();
        fs::write(&old_path, "content\n").unwrap();

        let result = resolve_and_persist_local(
            &mut app,
            vec![FileDiff::new(
                "content\n".to_owned(),
                old_path.clone(),
                DiffType::update(Vec::new(), Some(new_path.clone())),
            )],
            HashMap::new(),
        )
        .await;

        let RequestFileEditsResult::Success { deleted_files, .. } = result else {
            panic!("expected rename to succeed");
        };
        assert_eq!(fs::read_to_string(&new_path).unwrap(), "content\n");
        assert!(!std::path::Path::new(&old_path).exists());
        assert_eq!(deleted_files, vec![old_path]);
    });
}

#[test]
fn persist_deletes_a_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gone.rs").to_string_lossy().to_string();
        fs::write(&path, "delete me\n").unwrap();

        let result = resolve_and_persist_local(
            &mut app,
            vec![FileDiff::new(
                "delete me\n".to_owned(),
                path.clone(),
                DiffType::deletion(1),
            )],
            HashMap::new(),
        )
        .await;

        let RequestFileEditsResult::Success { deleted_files, .. } = result else {
            panic!("expected delete to succeed");
        };
        assert!(!std::path::Path::new(&path).exists());
        assert_eq!(deleted_files, vec![path]);
    });
}

#[test]
fn persist_reports_save_failure() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        // Create a regular file, then target a path *under* it. Creating the parent
        // directory fails because a file exists where a directory is expected.
        let blocking_file = dir.path().join("not_a_dir");
        fs::write(&blocking_file, "x").unwrap();
        let path = blocking_file.join("child.rs").to_string_lossy().to_string();

        let result = resolve_and_persist_local(
            &mut app,
            vec![FileDiff::new(
                String::new(),
                path,
                DiffType::creation("data\n".to_owned()),
            )],
            HashMap::new(),
        )
        .await;

        assert!(matches!(
            result,
            RequestFileEditsResult::DiffApplicationFailed { .. }
        ));
    });
}

#[test]
fn persist_action_renames_only_on_local_sessions() {
    let rename_op = DiffType::update(Vec::new(), Some("/tmp/new.rs".to_owned()));

    assert!(matches!(
        PersistAction::resolve(&rename_op, &DiffSessionType::Local, "/tmp/old.rs"),
        PersistAction::Rename(_)
    ));
    // Remote sessions have no rename primitive: the file is written in place.
    assert!(matches!(
        PersistAction::resolve(
            &rename_op,
            &DiffSessionType::Remote(HostId::new("host".to_owned())),
            "/tmp/old.rs"
        ),
        PersistAction::Write
    ));
    // A "rename" to the same path is just a write.
    assert!(matches!(
        PersistAction::resolve(&rename_op, &DiffSessionType::Local, "/tmp/new.rs"),
        PersistAction::Write
    ));
}

#[test]
fn remote_rename_outcome_reports_update_at_original_path() {
    // The reported outcome must match the actual write: a remote rename falls
    // back to writing the original path, so nothing is deleted and the update
    // is reported at the original path — not the rename target.
    let op = DiffType::update(Vec::new(), Some("/tmp/new.rs".to_owned()));
    let action = PersistAction::resolve(
        &op,
        &DiffSessionType::Remote(HostId::new("host".to_owned())),
        "/tmp/old.rs",
    );
    let outcome = outcome_for_action(&action, "/tmp/old.rs", "content\n", "content!\n", vec![]);

    let (file_location, _) = outcome.updated.expect("expected an updated file");
    assert_eq!(file_location.name, "/tmp/old.rs");
    assert_eq!(outcome.deleted, Vec::<String>::new());
}

#[test]
fn build_resolved_edits_applies_deltas_without_reviewed_content() {
    // Headless/TUI path: no reviewed content, so final content is derived from deltas.
    let base = "one\ntwo\nthree\n";
    let diff = FileDiff::new(
        base.to_owned(),
        "/tmp/main.rs".to_owned(),
        DiffType::update(
            vec![DiffDelta {
                replacement_line_range: 2..3,
                insertion: "TWO\n".to_owned(),
            }],
            None,
        ),
    );

    let resolved = build_resolved_edits(vec![diff], &HashMap::new()).unwrap();

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].path, "/tmp/main.rs");
    assert_eq!(resolved[0].final_content, "one\nTWO\nthree\n");
}

#[test]
fn build_resolved_edits_prefers_reviewed_content() {
    // Review-surface path: reviewed content for a path overrides the delta-applied content.
    let base = "one\ntwo\nthree\n";
    let diff = FileDiff::new(
        base.to_owned(),
        "/tmp/main.rs".to_owned(),
        DiffType::update(
            vec![DiffDelta {
                replacement_line_range: 2..3,
                insertion: "TWO\n".to_owned(),
            }],
            None,
        ),
    );
    let reviewed = HashMap::from([("/tmp/main.rs".to_owned(), "user edited\n".to_owned())]);

    let resolved = build_resolved_edits(vec![diff], &reviewed).unwrap();

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].final_content, "user edited\n");
}

#[test]
fn updated_file_contexts_from_content_map_returns_changed_lines_with_context() {
    let updated_files = vec![(
        FileLocations {
            name: "src/main.rs".to_string(),
            lines: std::iter::once(12..13).collect(),
        },
        true,
    )];
    let content = (1..=30)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let content_map = HashMap::from([("src/main.rs".to_string(), content)]);

    let contexts = updated_file_contexts_from_content_map(&updated_files, &content_map);

    assert_eq!(contexts.len(), 1);
    assert!(contexts[0].was_edited_by_user);
    assert_eq!(contexts[0].file_context.file_name, "src/main.rs");
    assert_eq!(contexts[0].file_context.line_range, Some(2..23));
    assert_eq!(contexts[0].file_context.line_count, 30);
    assert_eq!(
        contexts[0].file_context.content,
        AnyFileContent::StringContent(
            (2..=22)
                .map(|line| format!("line {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    );
}

#[test]
fn updated_file_contexts_from_content_map_preserves_full_file_when_no_ranges() {
    let updated_files = vec![(
        FileLocations {
            name: "src/main.rs".to_string(),
            lines: vec![],
        },
        false,
    )];
    let content = "line 1\nline 2\n".to_string();
    let content_map = HashMap::from([("src/main.rs".to_string(), content.clone())]);

    let contexts = updated_file_contexts_from_content_map(&updated_files, &content_map);

    assert_eq!(contexts.len(), 1);
    assert!(!contexts[0].was_edited_by_user);
    assert_eq!(contexts[0].file_context.line_range, None);
    assert_eq!(contexts[0].file_context.line_count, 2);
    assert_eq!(
        contexts[0].file_context.content,
        AnyFileContent::StringContent(content)
    );
}
