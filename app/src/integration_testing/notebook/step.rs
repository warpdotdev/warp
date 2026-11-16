use std::sync::Arc;

use string_offset::CharOffset;
use warp_editor::model::CoreEditorModel;
use warpui::integration::TestStep;
use warpui::windowing::WindowManager;
use warpui::{App, SingletonEntity, ViewHandle, WindowId, async_assert};

use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObjectEventEntrypoint, Space};
use crate::drive::OpenWarpDriveObjectSettings;
use crate::integration_testing::view_getters::{notebook_view, workspace_view};
use crate::notebooks::manager::NotebookSource;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ClientId, SyncId};
use crate::workspaces::user_workspaces::UserWorkspaces;

fn notebook_editor(
    app: &App,
    window_id: WindowId,
    tab_index: usize,
    pane_index: usize,
) -> ViewHandle<crate::notebooks::editor::view::RichTextEditorView> {
    notebook_view(app, window_id, tab_index, pane_index)
        .read(app, |notebook, _ctx| notebook.input_editor())
}

/// Create a personal notebook and save its sync ID into the step data.
pub fn create_a_personal_notebook(key: impl Into<String>, title: impl Into<String>) -> TestStep {
    let key = key.into();
    let title = Arc::new(title.into());
    TestStep::new("Create a personal notebook")
        .with_action(move |app, _, data| {
            let client_id = ClientId::new();
            let sync_id = SyncId::ClientId(client_id);
            UpdateManager::handle(app).update(app, |update_manager, ctx| {
                update_manager.create_notebook(
                    client_id,
                    UserWorkspaces::as_ref(ctx)
                        .personal_drive(ctx)
                        .expect("User UID must be set in tests"),
                    None,
                    Default::default(),
                    CloudObjectEventEntrypoint::ManagementUI,
                    true,
                    ctx,
                );

                // Set a title so that the notebook is not considered empty.
                update_manager.update_notebook_title(title.clone(), sync_id, ctx);
            });

            data.insert(key.clone(), sync_id);
        })
        .add_assertion(move |app, _| {
            CloudModel::handle(app).read(app, |cloud_model, ctx| {
                async_assert!(
                    cloud_model
                        .active_cloud_objects_in_space(Space::Personal, ctx)
                        .count()
                        > 0,
                    "Notebook exists"
                )
            })
        })
}

/// Open the notebook saved at `notebook_key` in the active tab of the window saved at `window_key`
pub fn open_notebook(window_key: impl Into<String>, notebook_key: impl Into<String>) -> TestStep {
    let window_key = window_key.into();
    let notebook_key = notebook_key.into();
    TestStep::new("Open notebook").with_action(move |app, _, data| {
        let notebook_id: &SyncId = data.get(&notebook_key).expect("No saved notebook ID");
        let window_id: &WindowId = data.get(&window_key).expect("No saved window ID");
        workspace_view(app, *window_id).update(app, |workspace, ctx| {
            // If the notebook isn't open yet, opening it won't focus the window (we only change
            // focus if switching to an already-open window). Since the user wouldn't be able to
            // open a notebook in an unfocused window, switch focus explicitly here.
            WindowManager::as_ref(ctx).show_window_and_focus_app(*window_id);
            workspace.open_notebook(
                &NotebookSource::Existing(*notebook_id),
                &OpenWarpDriveObjectSettings::default(),
                ctx,
                true,
            );
        })
    })
}

pub fn enter_notebook_edit_mode_and_set_markdown(
    tab_index: usize,
    pane_index: usize,
    markdown: impl Into<String>,
) -> TestStep {
    let markdown = markdown.into();
    TestStep::new("Enter notebook edit mode and set Markdown").with_action(
        move |app, window_id, _| {
            let notebook = notebook_view(app, window_id, tab_index, pane_index);
            notebook.update(app, |notebook, ctx| notebook.toggle_mode(ctx));
            let editor = notebook_editor(app, window_id, tab_index, pane_index);
            editor.update(app, |editor, ctx| {
                editor.reset_with_markdown(&markdown, ctx);
            });
        },
    )
}

pub fn move_notebook_cursor_to_offset(
    tab_index: usize,
    pane_index: usize,
    offset: usize,
) -> TestStep {
    TestStep::new("Move notebook cursor to offset").with_action(move |app, window_id, _| {
        let editor = notebook_editor(app, window_id, tab_index, pane_index);
        editor.update(app, |editor, ctx| {
            editor.model().update(ctx, |model, ctx| {
                model.cursor_at(CharOffset::from(offset), ctx)
            });
        });
    })
}
