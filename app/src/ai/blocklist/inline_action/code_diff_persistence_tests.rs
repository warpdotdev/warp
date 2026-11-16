use std::fs;

use ai::diff_validation::DiffDelta;
use warp_files::FileModel;
use warpui::platform::WindowStyle;
use warpui::{App, ReadView};

use super::*;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::test_util::terminal::initialize_app_for_terminal_view;
use crate::vim_registers::VimRegisters;

fn add_review(app: &mut App, path: &Path, diff: DiffType) -> ViewHandle<CodeDiffView> {
    initialize_app_for_terminal_view(app);
    app.add_singleton_model(FileModel::new);
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(|_| ToastStack);
    let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        let mut view = CodeDiffView::new_passive(
            &"file-edit".to_owned().into(),
            None,
            AIIdentifiers::default(),
            RequestFileEditsFormatKind::V4A,
            false,
            None,
            ctx,
        );
        view.set_candidate_diffs(
            vec![FileDiff::new(
                "original\n".to_owned(),
                path.to_str().unwrap().to_owned(),
                diff,
            )],
            ctx,
        );
        view
    });
    view
}

#[test]
#[cfg(unix)]
fn accept_symlink_delete_reports_and_removes_the_link_entry() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let referent = dir.path().join("referent.txt");
        let link = dir.path().join("link.txt");
        fs::write(&referent, "original\n").unwrap();
        std::os::unix::fs::symlink(&referent, &link).unwrap();
        let view = add_review(&mut app, &link, DiffType::deletion(1));

        let result = app
            .update(|ctx| view.downgrade().accept_and_save(ctx))
            .await;

        let RequestFileEditsResult::Success {
            deleted_files,
            updated_files,
            ..
        } = result
        else {
            panic!("expected deletion success");
        };
        assert_eq!(deleted_files, vec![link.to_str().unwrap().to_owned()]);
        assert!(updated_files.is_empty());
        assert!(fs::symlink_metadata(&link).is_err());
        assert_eq!(fs::read_to_string(&referent).unwrap(), "original\n");

        view.update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap()
            .await
            .unwrap();
        assert_eq!(fs::read_to_string(&link).unwrap(), "original\n");
        assert_eq!(fs::read_to_string(&referent).unwrap(), "original\n");
    });
}

#[test]
#[cfg(unix)]
fn accept_symlink_move_reports_and_moves_the_link_entry() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let referent = dir.path().join("referent.txt");
        let link = dir.path().join("link.txt");
        let destination = dir.path().join("moved-link.txt");
        fs::write(&referent, "original\n").unwrap();
        std::os::unix::fs::symlink(&referent, &link).unwrap();
        let view = add_review(
            &mut app,
            &link,
            DiffType::update(Vec::new(), Some(destination.to_str().unwrap().to_owned())),
        );

        let result = app
            .update(|ctx| view.downgrade().accept_and_save(ctx))
            .await;

        let RequestFileEditsResult::Success {
            deleted_files,
            updated_files,
            ..
        } = result
        else {
            panic!("expected move success");
        };
        assert_eq!(deleted_files, vec![link.to_str().unwrap().to_owned()]);
        assert_eq!(
            updated_files[0].file_context.file_name,
            destination.to_str().unwrap()
        );
        assert!(fs::symlink_metadata(&link).is_err());
        assert_eq!(fs::read_link(&destination).unwrap(), referent);
        assert_eq!(fs::read_to_string(&referent).unwrap(), "original\n");
    });
}

#[test]
fn accept_hard_link_collision_fails_before_writing_or_reporting_deletion() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("alias.txt");
        fs::write(&source, "original\n").unwrap();
        fs::hard_link(&source, &destination).unwrap();
        let view = add_review(
            &mut app,
            &source,
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..2,
                    insertion: "updated\n".to_owned(),
                }],
                Some(destination.to_str().unwrap().to_owned()),
            ),
        );

        let result = app
            .update(|ctx| view.downgrade().accept_and_save(ctx))
            .await;

        assert!(matches!(
            result,
            RequestFileEditsResult::DiffApplicationFailed { .. }
        ));
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert_eq!(fs::read_to_string(&destination).unwrap(), "original\n");
    });
}

#[test]
fn failed_reverse_move_keeps_accepted_state_and_can_be_retried() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("destination.txt");
        fs::write(&source, "original\n").unwrap();
        let view = add_review(
            &mut app,
            &source,
            DiffType::update(Vec::new(), Some(destination.to_str().unwrap().to_owned())),
        );
        assert!(matches!(
            app.update(|ctx| view.downgrade().accept_and_save(ctx))
                .await,
            RequestFileEditsResult::Success { .. }
        ));
        fs::remove_file(&destination).unwrap();

        let completion = view
            .update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap();
        assert!(app.read_view(&view, |view, _| matches!(
            view.state,
            CodeDiffState::Accepted
        )));
        assert!(completion.await.is_err());
        assert!(app.read_view(&view, |view, _| matches!(
            view.state,
            CodeDiffState::Accepted
        )));
        assert!(!source.exists());
        assert!(!destination.exists());

        fs::write(&destination, "original\n").unwrap();
        view.update(&mut app, |view, ctx| view.revert_changes(ctx))
            .unwrap()
            .await
            .unwrap();
        assert!(app.read_view(&view, |view, _| matches!(
            view.state,
            CodeDiffState::Reverted
        )));
        assert_eq!(fs::read_to_string(&source).unwrap(), "original\n");
        assert!(!destination.exists());
    });
}
