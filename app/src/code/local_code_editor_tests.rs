use std::ops::Range;
use std::sync::Arc;

use lsp::LspManagerModel;
use repo_metadata::RepoMetadataModel;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::watcher::DirectoryWatcher;
use string_offset::CharOffset;
use vec1::vec1;
use warp_core::ui::appearance::Appearance;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_files::FileModel;
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity, ViewHandle};

use super::LocalCodeEditorView;
use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::cloud_object::model::persistence::CloudModel;
use crate::code::buffer_location::LocalOrRemotePath;
use crate::code::editor::view::{CodeEditorRenderOptions, CodeEditorView};
use crate::code::global_buffer_model::{GlobalBufferModel, GlobalBufferModelEvent};
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::search::files::model::FileSearchModel;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::terminal::keys::TerminalKeybindings;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspace::ActiveSession;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

fn init_app(app: &mut App) {
    initialize_settings_for_tests(app);

    let global_resource_handles = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
    app.add_singleton_model(DirectoryWatcher::new);
    app.add_singleton_model(|_| DetectedRepositories::default());
    app.add_singleton_model(RepoMetadataModel::new);
    app.add_singleton_model(FileSearchModel::new);
    app.add_singleton_model(FileModel::new);
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(TerminalKeybindings::new);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
    let team_client_mock = Arc::new(MockTeamClient::new());
    let workspace_client_mock = Arc::new(MockWorkspaceClient::new());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            team_client_mock.clone(),
            workspace_client_mock.clone(),
            vec![],
            ctx,
        )
    });
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
    app.add_singleton_model(|ctx| {
        PersistedWorkspace::new(vec![], std::collections::HashMap::new(), None, ctx)
    });
    app.add_singleton_model(|_| LspManagerModel::new());
    app.add_singleton_model(GlobalBufferModel::new);
}

/// Opens `path` in an editor backed by the global buffer, and waits for the read to settle.
async fn open_editor(app: &mut App, path: &std::path::Path) -> ViewHandle<LocalCodeEditorView> {
    let location = LocalOrRemotePath::Local(path.to_path_buf());
    let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        LocalCodeEditorView::new_with_global_buffer(
            location,
            |buffer_state, ctx| {
                ctx.add_typed_action_view(|ctx| {
                    CodeEditorView::new(
                        None,
                        Some(buffer_state.buffer),
                        CodeEditorRenderOptions::new(VerticalExpansionBehavior::FillMaxHeight),
                        ctx,
                    )
                })
            },
            false,
            None,
            ctx,
        )
    });

    let file_id = editor
        .read(app, |editor, _| editor.file_id())
        .expect("the editor should have a file id");
    let future_id = app.read(|ctx| {
        FileModel::as_ref(ctx)
            .get_future_handle(file_id)
            .expect("Loading future should be present")
            .future_id()
    });
    app.update(|ctx| ctx.await_spawned_future(future_id)).await;

    editor
}

/// APP-5266: a path that does not exist opens as an empty buffer that is *clean* — the unsaved
/// indicator must not appear just because the file has not been written yet.
#[test]
fn a_new_file_buffer_starts_clean_and_becomes_dirty_when_typed_into() {
    App::test((), |mut app| async move {
        init_app(&mut app);

        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("not-written-yet.md");
        let editor = open_editor(&mut app, &path).await;

        editor.read(&app, |editor, ctx| {
            assert!(editor.is_new_file(), "the file does not exist on disk yet");
            assert!(
                !editor.has_unsaved_changes(ctx),
                "an untouched new-file buffer must not look dirty"
            );
        });

        // Typing makes it dirty, which is what makes the close path warn instead of dropping it.
        editor.update(&mut app, |editor, ctx| {
            editor.editor().update(ctx, |code_editor, ctx| {
                let insertion_point: Range<CharOffset> =
                    CharOffset::from(1usize)..CharOffset::from(1usize);
                code_editor.apply_edits(vec1![("typed".to_string(), insertion_point)], ctx);
            });
        });

        editor.read(&app, |editor, ctx| {
            assert!(
                editor.has_unsaved_changes(ctx),
                "typing into a new-file buffer must register as an unsaved change"
            );
        });
    });
}

/// Auto-save deliberately does not create a file that the user has only opened and typed into;
/// the first explicit save creates it, and auto-save takes over from then on.
#[test]
fn auto_save_is_held_back_until_a_new_file_is_first_saved() {
    App::test((), |mut app| async move {
        init_app(&mut app);

        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("not-written-yet.md");
        let saved = save_events(&mut app);
        let editor = open_editor(&mut app, &path).await;

        editor.read(&app, |editor, ctx| {
            assert!(editor.is_new_file());
            assert!(
                !editor.can_auto_save(ctx),
                "auto-save must not create a file the user never saved"
            );
        });

        // The first explicit save creates the file and ends the hold-back.
        type_into(&mut app, &editor, "first");
        editor.update(&mut app, |editor, ctx| {
            editor.save_local(ctx).expect("save should dispatch");
        });
        assert!(saved.recv().await.expect("a save outcome"), "save failed");

        assert_eq!(std::fs::read_to_string(&path).expect("created"), "first");
        editor.read(&app, |editor, ctx| {
            assert!(
                !editor.is_new_file(),
                "the file exists now, so it is no longer a new file"
            );
            assert!(
                editor.can_auto_save(ctx),
                "auto-save should take over once the file exists"
            );
        });
    });
}

/// The not-yet-written marker lives on the shared buffer, so it has to be cleared globally when
/// the file is first saved. An editor that attaches to that buffer afterwards would otherwise
/// inherit a stale "new file" and treat an emptied buffer as clean — losing a delete-everything
/// edit on close.
#[test]
fn an_editor_opened_after_the_first_save_does_not_inherit_the_new_file_marker() {
    App::test((), |mut app| async move {
        init_app(&mut app);

        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("saved-then-reopened.md");
        let saved = save_events(&mut app);

        let first = open_editor(&mut app, &path).await;
        type_into(&mut app, &first, "some content");
        first.update(&mut app, |editor, ctx| {
            editor.save_local(ctx).expect("save should dispatch");
        });
        assert!(saved.recv().await.expect("a save outcome"), "save failed");

        // A second editor on the same, now-existing file.
        let second = open_editor(&mut app, &path).await;
        second.read(&app, |editor, _| {
            assert!(
                !editor.is_new_file(),
                "the file exists on disk, so this is not a new file"
            );
        });

        // Deleting everything is a real, unsaved change and must not read as clean.
        let end = second.read(&app, |editor, ctx| {
            CharOffset::from(
                editor
                    .editor()
                    .as_ref(ctx)
                    .text(ctx)
                    .as_str()
                    .chars()
                    .count()
                    + 1,
            )
        });
        second.update(&mut app, |editor, ctx| {
            editor.editor().update(ctx, |code_editor, ctx| {
                let whole_buffer: Range<CharOffset> = CharOffset::from(1usize)..end;
                code_editor.apply_edits(vec1![(String::new(), whole_buffer)], ctx);
            });
        });
        second.read(&app, |editor, ctx| {
            assert!(
                editor.has_unsaved_changes(ctx),
                "deleting every character must register as an unsaved change"
            );
        });
    });
}

/// Types `text` at the start of the editor's buffer as an undoable edit.
fn type_into(app: &mut App, editor: &ViewHandle<LocalCodeEditorView>, text: &str) {
    let text = text.to_string();
    editor.update(app, |editor, ctx| {
        editor.editor().update(ctx, |code_editor, ctx| {
            let insertion_point: Range<CharOffset> =
                CharOffset::from(1usize)..CharOffset::from(1usize);
            code_editor.apply_edits(vec1![(text, insertion_point)], ctx);
        });
    });
}

/// Streams save outcomes (`true` = saved, `false` = failed).
fn save_events(app: &mut App) -> async_channel::Receiver<bool> {
    let (sender, receiver) = async_channel::unbounded();
    let handle = GlobalBufferModel::handle(app);
    app.update(|ctx| {
        ctx.subscribe_to_model(&handle, move |_, event, _| match event {
            GlobalBufferModelEvent::FileSaved { .. } => {
                let _ = sender.try_send(true);
            }
            GlobalBufferModelEvent::FailedToSave { .. } => {
                let _ = sender.try_send(false);
            }
            _ => {}
        });
    });
    receiver
}
