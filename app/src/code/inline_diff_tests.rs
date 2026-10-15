use std::fs;
use std::path::Path;
use std::sync::Arc;

use ai::diff_validation::DiffDelta;
use line_ending::LineEnding;
use warp_core::ui::appearance::Appearance;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warpui::App;
use warpui::platform::WindowStyle;

use super::*;
use crate::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::code::editor::view::CodeEditorRenderOptions;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::ActiveSession;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn add_diff(app: &mut App, path: &Path, base: &str, diff: DiffType) -> ViewHandle<InlineDiffView> {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| SyncedInputState::mock());
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(FileModel::new);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            vec![],
            ctx,
        )
    });
    let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        let editor = ctx.add_typed_action_view(|ctx| {
            let editor = CodeEditorView::new(
                None,
                None,
                CodeEditorRenderOptions::new(VerticalExpansionBehavior::GrowToMaxHeight)
                    .lazy_layout(),
                ctx,
            );
            editor.reset(InitialBufferState::plain_text(base), ctx);
            // Keep the fixture's LF content independent of the platform default for empty buffers.
            let buffer = editor.model.as_ref(ctx).buffer().clone();
            buffer.update(ctx, |buffer, _| {
                buffer.set_line_ending_mode(LineEnding::LF);
            });
            editor
        });
        let mut view = InlineDiffView::new(
            editor,
            Some(diff),
            None,
            Some(StandardizedPath::try_from_local(path).unwrap()),
            ctx,
        );
        view.register_file(&DiffSessionType::Local, ctx);
        view
    });
    view
}

#[test]
fn accept_deletes_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("delete.txt");
        fs::write(&path, "original\n").unwrap();
        let view = add_diff(&mut app, &path, "original\n", DiffType::deletion(1));

        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        assert!(!path.exists());
    });
}

async fn restore(app: &mut App, view: &ViewHandle<InlineDiffView>) {
    view.update(app, |view, ctx| view.restore_and_save_diff(ctx))
        .unwrap()
        .await
        .unwrap();
}

#[test]
fn accept_move_to_same_normalized_path_writes_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.txt");
        let target = dir.path().join("./source.txt");
        let view = add_diff(
            &mut app,
            &path,
            "",
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..1,
                    insertion: "updated\n".to_owned(),
                }],
                Some(target.to_str().unwrap().to_owned()),
            ),
        );

        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "updated\n");
    });
}

#[test]
fn accept_creates_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.txt");
        let view = add_diff(
            &mut app,
            &path,
            "",
            DiffType::creation("created\n".to_owned()),
        );

        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "created\n");
    });
}

#[test]
fn restore_deleted_file_recreates_original_content() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deleted.txt");
        fs::write(&path, "original\n").unwrap();
        let view = add_diff(&mut app, &path, "original\n", DiffType::deletion(1));
        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        restore(&mut app, &view).await;

        assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");
    });
}

#[test]
fn restore_moved_file_restores_original_path_and_content() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.txt");
        let destination = dir.path().join("new.txt");
        fs::write(&path, "original\n").unwrap();
        let view = add_diff(
            &mut app,
            &path,
            "original\n",
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..2,
                    insertion: "updated\n".to_owned(),
                }],
                Some(destination.to_str().unwrap().to_owned()),
            ),
        );
        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        restore(&mut app, &view).await;

        assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");
        assert!(!destination.exists());
    });
}

#[test]
fn accept_deletes_initially_empty_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        fs::write(&path, "").unwrap();
        let view = add_diff(&mut app, &path, "", DiffType::deletion(0));

        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        assert!(!path.exists());
    });
}

#[test]
fn accept_update_to_empty_keeps_file() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        fs::write(&path, "original\n").unwrap();
        let view = add_diff(
            &mut app,
            &path,
            "original\n",
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..2,
                    insertion: String::new(),
                }],
                None,
            ),
        );

        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "");
    });
}

#[test]
fn accept_moves_file_with_updated_content() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.txt");
        let destination = dir.path().join("nested/new.txt");
        fs::write(&path, "original\n").unwrap();
        let view = add_diff(
            &mut app,
            &path,
            "original\n",
            DiffType::update(
                vec![DiffDelta {
                    replacement_line_range: 1..2,
                    insertion: "updated\n".to_owned(),
                }],
                Some(destination.to_str().unwrap().to_owned()),
            ),
        );

        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        assert!(!path.exists());
        assert_eq!(fs::read_to_string(&destination).unwrap(), "updated\n");
    });
}

#[test]
fn accept_propagates_delete_failure() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.txt");
        let view = add_diff(&mut app, &path, "", DiffType::deletion(0));

        let result = view
            .update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await;

        assert!(result.is_err());
        assert!(!path.exists());
    });
}

#[test]
fn overlapping_and_repeated_restores_do_not_repeat_a_move() {
    App::test((), |mut app| async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.txt");
        let destination = dir.path().join("new.txt");
        fs::write(&path, "original\n").unwrap();
        let view = add_diff(
            &mut app,
            &path,
            "original\n",
            DiffType::update(Vec::new(), Some(destination.to_str().unwrap().to_owned())),
        );
        view.update(&mut app, |view, ctx| view.accept_and_save_diff(ctx))
            .unwrap()
            .await
            .unwrap();

        let first = view
            .update(&mut app, |view, ctx| view.restore_and_save_diff(ctx))
            .unwrap();
        let overlapping = view
            .update(&mut app, |view, ctx| view.restore_and_save_diff(ctx))
            .unwrap();
        first.await.unwrap();
        overlapping.await.unwrap();
        assert!(!destination.exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");

        fs::write(&path, "subsequent edit\n").unwrap();
        restore(&mut app, &view).await;
        restore(&mut app, &view).await;
        assert_eq!(fs::read_to_string(&path).unwrap(), "subsequent edit\n");
        assert!(!destination.exists());
    });
}
