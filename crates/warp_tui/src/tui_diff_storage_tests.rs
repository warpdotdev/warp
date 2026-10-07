use std::fs;

use warp_core::HostId;
use warpui::App;

use super::*;

/// Builds a [`TuiDiffStorage`] over `diffs`, registering the `FileModel` its
/// writes go through.
fn add_tui_storage(
    app: &mut App,
    diffs: Vec<FileDiff>,
    session_type: DiffSessionType,
) -> ModelHandle<TuiDiffStorage> {
    app.add_singleton_model(FileModel::new);
    app.add_model(|_| TuiDiffStorage::new(diffs, session_type))
}

/// Runs the shared accept flow for local diffs on a fresh app and awaits the result.
async fn accept_local(app: &mut App, diffs: Vec<FileDiff>) -> RequestFileEditsResult {
    let model = add_tui_storage(app, diffs, DiffSessionType::Local);
    let future = model.update(app, |model, ctx| model.accept_and_save(ctx));
    future.await
}

#[test]
fn accept_creates_a_new_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.rs").to_string_lossy().to_string();

        let result = accept_local(
            &mut app,
            vec![FileDiff::new(
                String::new(),
                path.clone(),
                DiffType::creation("fn main() {}\n".to_owned()),
            )],
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
fn accept_applies_deltas_to_update_a_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.rs").to_string_lossy().to_string();
        fs::write(&path, "one\ntwo\nthree\n").unwrap();

        let result = accept_local(
            &mut app,
            vec![FileDiff::new(
                "one\ntwo\nthree\n".to_owned(),
                path.clone(),
                DiffType::update(
                    vec![DiffDelta {
                        replacement_line_range: 2..3,
                        // Production insertions omit the trailing newline.
                        insertion: "TWO".to_owned(),
                    }],
                    None,
                ),
            )],
        )
        .await;

        assert!(matches!(result, RequestFileEditsResult::Success { .. }));
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\nTWO\nthree\n");
    });
}

#[test]
fn accept_renames_and_reports_source_as_deleted() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let old_path = dir.path().join("old.rs").to_string_lossy().to_string();
        let new_path = dir.path().join("new.rs").to_string_lossy().to_string();
        fs::write(&old_path, "content\n").unwrap();

        let result = accept_local(
            &mut app,
            vec![FileDiff::new(
                "content\n".to_owned(),
                old_path.clone(),
                DiffType::update(Vec::new(), Some(new_path.clone())),
            )],
        )
        .await;

        let RequestFileEditsResult::Success { deleted_files, .. } = result else {
            panic!("expected rename to succeed");
        };
        assert_eq!(fs::read_to_string(&new_path).unwrap(), "content\n");
        assert!(!Path::new(&old_path).exists());
        assert_eq!(deleted_files, vec![old_path]);
    });
}

#[test]
fn accept_deletes_a_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gone.rs").to_string_lossy().to_string();
        fs::write(&path, "delete me\n").unwrap();

        let result = accept_local(
            &mut app,
            vec![FileDiff::new(
                "delete me\n".to_owned(),
                path.clone(),
                DiffType::deletion(1),
            )],
        )
        .await;

        let RequestFileEditsResult::Success { deleted_files, .. } = result else {
            panic!("expected delete to succeed");
        };
        assert!(!Path::new(&path).exists());
        assert_eq!(deleted_files, vec![path]);
    });
}

#[test]
fn accept_reports_write_dispatch_failure() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        // Create a regular file, then target a path *under* it. Creating the parent
        // directory fails because a file exists where a directory is expected.
        let blocking_file = dir.path().join("not_a_dir");
        fs::write(&blocking_file, "x").unwrap();
        let path = blocking_file.join("child.rs").to_string_lossy().to_string();

        let result = accept_local(
            &mut app,
            vec![FileDiff::new(
                String::new(),
                path,
                DiffType::creation("data\n".to_owned()),
            )],
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
    let diff = FileDiff::new("content\n".to_owned(), "/tmp/old.rs".to_owned(), op.clone());
    let action = PersistAction::resolve(
        &op,
        &DiffSessionType::Remote(HostId::new("host".to_owned())),
        "/tmp/old.rs",
    );

    let state = persist_outcome(&action, &diff, "/tmp/old.rs", "content!\n");

    let updated = state.updated.expect("expected an updated file");
    assert_eq!(updated.path, "/tmp/old.rs");
    assert_eq!(state.deleted_paths, Vec::<String>::new());
}

#[test]
fn final_content_from_op_applies_deltas() {
    // No surface-supplied content (no editor buffers): final content is
    // derived from the diff's deltas.
    let op = DiffType::update(
        vec![DiffDelta {
            replacement_line_range: 2..3,
            insertion: "TWO\n".to_owned(),
        }],
        None,
    );

    let final_content = final_content_from_op("one\ntwo\nthree\n", &op).unwrap();

    assert_eq!(final_content, "one\nTWO\nthree\n");
}

#[test]
fn apply_deltas_normalizes_newline_less_insertions() {
    // Insertions commonly omit the trailing newline (e.g. search/replace
    // blocks are joined with "\n"); a raw splice would run the replacement
    // into the next preserved line ("one\nTWOthree\n").
    let deltas = vec![DiffDelta {
        replacement_line_range: 2..3,
        insertion: "TWO\nTWO-AND-A-HALF".to_owned(),
    }];

    let final_content = apply_deltas_to_content("one\ntwo\nthree\n", &deltas).unwrap();

    assert_eq!(final_content, "one\nTWO\nTWO-AND-A-HALF\nthree\n");
}
