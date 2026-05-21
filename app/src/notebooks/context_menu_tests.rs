use pathfinder_geometry::vector::vec2f;
use string_offset::ByteOffset;
use warp_core::ui::appearance::Appearance;
use warp_editor::model::CoreEditorModel;
use warpui::platform::WindowStyle;
use warpui::App;

use super::MenuSource;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::model::view::CloudViewModel;
use crate::editor::InteractionState;
use crate::network::NetworkStatus;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::notebooks::notebook::NotebookView;
use crate::pane_group::focus_state::{PaneFocusHandle, PaneGroupFocusState};
use crate::pane_group::{BackingView as _, PaneId};
use crate::search::files::model::FileSearchModel;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::terminal::keys::TerminalKeybindings;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspace::ActiveSession;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    let global_resources = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resources));
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| Appearance::mock());

    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(CloudViewModel::mock);
    app.add_singleton_model(|_| UserProfiles::new(vec![]));
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
    app.add_singleton_model(|_| repo_metadata::repositories::DetectedRepositories::default());
    #[cfg(feature = "local_fs")]
    app.add_singleton_model(repo_metadata::RepoMetadataModel::new);
    app.add_singleton_model(FileSearchModel::new);
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(TerminalKeybindings::new);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
}

/// Builds a list of the standard notebook context-menu items by appending the set of split-pane
/// items to the given state-specific ones.
fn standard_menu_items<'a>(items: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let mut items: Vec<_> = items.into_iter().collect();
    items.extend([
        "----",
        "Split pane right",
        "Split pane left",
        "Split pane down",
        "Split pane up",
    ]);
    items
}

#[test]
fn test_rich_text_actions() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_window_id, notebook) = app.add_window(WindowStyle::NotStealFocus, NotebookView::new);

        // With no selection, the only text action should be to paste.
        notebook.update(&mut app, |notebook, ctx| {
            let source = MenuSource::RichTextEditor {
                parent_offset: vec2f(0., 0.),
                editor: notebook.input_editor(),
            };
            notebook.context_menu().show_context_menu(source, ctx);
            assert_eq!(
                notebook.context_menu().item_names(ctx),
                standard_menu_items(["Paste"])
            );
        });

        // Once text is selected, cut/copy become available.
        notebook.update(&mut app, |notebook, ctx| {
            notebook.input_editor().update(ctx, |editor, ctx| {
                editor.reset_with_markdown("Hello, World!", ctx);
                editor
                    .model()
                    .update(ctx, |model, ctx| model.select_all(ctx));
            });

            let source = MenuSource::RichTextEditor {
                parent_offset: vec2f(0., 0.),
                editor: notebook.input_editor(),
            };
            notebook.context_menu().show_context_menu(source, ctx);
            assert_eq!(
                notebook.context_menu().item_names(ctx),
                standard_menu_items(["Cut", "Copy", "Paste"])
            );
        });

        // If the editor is read-only, cut and paste are disabled.
        notebook.update(&mut app, |notebook, ctx| {
            notebook.input_editor().update(ctx, |editor, ctx| {
                editor.set_interaction_state(InteractionState::Selectable, ctx)
            });
            let source = MenuSource::RichTextEditor {
                parent_offset: vec2f(0., 0.),
                editor: notebook.input_editor(),
            };
            notebook.context_menu().show_context_menu(source, ctx);
            assert_eq!(
                notebook.context_menu().item_names(ctx),
                standard_menu_items(["Copy"])
            );
        })
    });
}

#[test]
fn test_plain_text_actions() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_window_id, notebook) = app.add_window(WindowStyle::NotStealFocus, NotebookView::new);

        // With no selection, the only text action should be to paste.
        notebook.update(&mut app, |notebook, ctx| {
            let source = MenuSource::TextEditor {
                parent_offset: vec2f(0., 0.),
                editor: notebook.title_editor(),
            };
            notebook.context_menu().show_context_menu(source, ctx);
            assert_eq!(
                notebook.context_menu().item_names(ctx),
                standard_menu_items(["Paste"]),
            );
        });

        // Once text is selected, cut/copy become available.
        notebook.update(&mut app, |notebook, ctx| {
            notebook.title_editor().update(ctx, |editor, ctx| {
                editor.set_buffer_text("The Title", ctx);
                editor.select_ranges_by_byte_offset([ByteOffset::zero()..ByteOffset::from(4)], ctx)
            });

            let source = MenuSource::TextEditor {
                parent_offset: vec2f(0., 0.),
                editor: notebook.title_editor(),
            };
            notebook.context_menu().show_context_menu(source, ctx);
            assert_eq!(
                notebook.context_menu().item_names(ctx),
                standard_menu_items(["Cut", "Copy", "Paste"]),
            );
        });

        // If the editor is read-only, cut and paste are disabled.
        notebook.update(&mut app, |notebook, ctx| {
            notebook.title_editor().update(ctx, |editor, ctx| {
                editor.set_interaction_state(InteractionState::Selectable, ctx)
            });
            let source = MenuSource::TextEditor {
                parent_offset: vec2f(0., 0.),
                editor: notebook.title_editor(),
            };
            notebook.context_menu().show_context_menu(source, ctx);
            assert_eq!(
                notebook.context_menu().item_names(ctx),
                standard_menu_items(["Copy"])
            );
        })
    });
}

#[test]
fn test_split_pane_actions() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_window_id, notebook) = app.add_window(WindowStyle::NotStealFocus, NotebookView::new);

        // Set up focus state to simulate being in a split pane.
        let pane_id = PaneId::dummy_pane_id();
        let focus_state = app.add_model(|_| {
            PaneGroupFocusState::new(
                pane_id, None, // active_session_id
                true, // in_split_pane
            )
        });
        let focus_handle = PaneFocusHandle::new(pane_id, focus_state.clone());

        notebook.update(&mut app, |notebook, ctx| {
            notebook.set_focus_handle(focus_handle, ctx);
        });

        // In a split pane, all the management actions are available.
        notebook.update(&mut app, |notebook, ctx| {
            let source = MenuSource::TextEditor {
                parent_offset: vec2f(0., 0.),
                editor: notebook.title_editor(),
            };
            notebook.context_menu().show_context_menu(source, ctx);
            assert_eq!(
                notebook.context_menu().item_names(ctx),
                vec![
                    "Paste",
                    "----",
                    "Split pane right",
                    "Split pane left",
                    "Split pane down",
                    "Split pane up",
                    "Maximize pane",
                    "Close pane"
                ]
            );
        });

        // Modify the focus state to simulate not being in a split pane.
        focus_state.update(&mut app, |state, ctx| {
            state.set_in_split_pane_for_test(false, ctx);
        });

        // If not in a split pane, maximize and close actions are hidden.
        notebook.update(&mut app, |notebook, ctx| {
            let source = MenuSource::TextEditor {
                parent_offset: vec2f(0., 0.),
                editor: notebook.title_editor(),
            };
            notebook.context_menu().show_context_menu(source, ctx);
            assert_eq!(
                notebook.context_menu().item_names(ctx),
                vec![
                    "Paste",
                    "----",
                    "Split pane right",
                    "Split pane left",
                    "Split pane down",
                    "Split pane up",
                ]
            );
        });
    });
}
