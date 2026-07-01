use std::fs;

use ai::diff_validation::{DiffDelta, DiffType};
use warp_files::FileModel;
use warpui::App;

use super::{PersistDiffModel, ResolvedFileEdit};
use crate::ai::agent::RequestFileEditsResult;
use crate::ai::blocklist::diff_types::DiffSessionType;

/// Runs `persist` for the given local edits on a fresh app and awaits the result.
async fn persist_local(app: &mut App, files: Vec<ResolvedFileEdit>) -> RequestFileEditsResult {
    // FileModel must exist before PersistDiffModel subscribes to it in `new`.
    app.add_singleton_model(FileModel::new);
    let persist = app.add_model(PersistDiffModel::new);
    let future = persist.update(app, |model, ctx| {
        model.persist(files, DiffSessionType::Local, ctx)
    });
    future.await
}

#[test]
fn persist_creates_a_new_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.rs").to_string_lossy().to_string();

        let result = persist_local(
            &mut app,
            vec![ResolvedFileEdit {
                path: path.clone(),
                base_content: String::new(),
                op: DiffType::creation("fn main() {}\n".to_owned()),
                final_content: "fn main() {}\n".to_owned(),
            }],
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

        let result = persist_local(
            &mut app,
            vec![ResolvedFileEdit {
                path: path.clone(),
                base_content: "one\ntwo\nthree\n".to_owned(),
                op: DiffType::update(
                    vec![DiffDelta {
                        replacement_line_range: 2..3,
                        insertion: "TWO\n".to_owned(),
                    }],
                    None,
                ),
                final_content: "one\nTWO\nthree\n".to_owned(),
            }],
        )
        .await;

        assert!(matches!(result, RequestFileEditsResult::Success { .. }));
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\nTWO\nthree\n");
    });
}

#[test]
fn persist_renames_and_reports_source_as_deleted() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let old_path = dir.path().join("old.rs").to_string_lossy().to_string();
        let new_path = dir.path().join("new.rs").to_string_lossy().to_string();
        fs::write(&old_path, "content\n").unwrap();

        let result = persist_local(
            &mut app,
            vec![ResolvedFileEdit {
                path: old_path.clone(),
                base_content: "content\n".to_owned(),
                op: DiffType::update(Vec::new(), Some(new_path.clone())),
                final_content: "content\n".to_owned(),
            }],
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

        let result = persist_local(
            &mut app,
            vec![ResolvedFileEdit {
                path: path.clone(),
                base_content: "delete me\n".to_owned(),
                op: DiffType::deletion(1),
                final_content: String::new(),
            }],
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

        let result = persist_local(
            &mut app,
            vec![ResolvedFileEdit {
                path,
                base_content: String::new(),
                op: DiffType::creation("data\n".to_owned()),
                final_content: "data\n".to_owned(),
            }],
        )
        .await;

        assert!(matches!(
            result,
            RequestFileEditsResult::DiffApplicationFailed { .. }
        ));
    });
}
