use itertools::Itertools;
use string_offset::CharOffset;
use warp_editor::render::model::BlockItem;
use warpui::integration::{AssertionCallback, AssertionOutcome, AssertionWithDataCallback};
use warpui::{async_assert, async_assert_eq, App, ViewHandle};

use crate::cloud_object::model::generic_string_model::GenericStringObjectId;
use crate::cloud_object::model::persistence::CloudModel;
use crate::integration_testing::cloud_object::assert_metadata_revision;
use crate::integration_testing::terminal::util::ExpectedOutput;
use crate::integration_testing::view_getters::{notebook_view, terminal_view};
use crate::notebooks::notebook::NotebookView;
use crate::notebooks::{CloudNotebookModel, NotebookId};
use crate::pane_group::PaneGroup;
use crate::server::ids::SyncId;
use crate::settings::{CloudPreferenceModel, Preference};

/// Asserts that the notebook in the given pane has the expected Markdown content.
pub fn assert_notebook_contents(
    tab_index: usize,
    pane_index: usize,
    expected_contents: impl ExpectedOutput + 'static,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let notebook = notebook_view(app, window_id, tab_index, pane_index);
        notebook.read(app, |notebook, ctx| {
            let contents = notebook.content(ctx);
            async_assert!(
                expected_contents.matches(&contents),
                "Expected notebook contents for window_id={window_id}, tab_index={tab_index}, pane_index={pane_index} to match:\n{expected_contents:?}\nBut got:\n{contents}")
        })
    })
}

/// Asserts that there is a json preference object in the SQLite db with the given contents.
pub fn assert_cloud_preference_exists(expected_preference: Preference) -> AssertionCallback {
    Box::new(move |app, _window_id| {
        let stored_preference =
            app.get_singleton_model_handle::<CloudModel>()
                .read(app, |cloud_model, _| {
                    let object = cloud_model
                        .get_all_objects_of_type::<GenericStringObjectId, CloudPreferenceModel>()
                        .find(|p| p.model().string_model == expected_preference)
                        .expect("Expected to find a matching preference object");
                    object.model().string_model.clone()
                });
        async_assert!(
            expected_preference == stored_preference,
            "Expected json object contents to match:\n{expected_preference:?}\nBut got:\n{stored_preference:?}"
        )
    })
}

/// Asserts metadata exists for the notebook with the given key and that the revision in that
/// metadata matches the given expected revision.
pub fn assert_notebook_metadata_revision(
    id: impl AsRef<str>,
    expected_revision: i64,
) -> AssertionCallback {
    assert_metadata_revision::<NotebookId, CloudNotebookModel>(id.as_ref(), expected_revision)
}

/// Asserts that a pane has the given notebook open.
pub fn assert_notebook_id(
    tab_index: usize,
    pane_index: usize,
    expected_id_key: impl Into<String>,
) -> AssertionWithDataCallback {
    let expected_id_key = expected_id_key.into();
    Box::new(move |app, window_id, data| {
        let expected_id = data.get(&expected_id_key).expect("No saved notebook ID");

        let notebook = notebook_view(app, window_id, tab_index, pane_index);
        notebook.read(app, |notebook, ctx| {
            let id = notebook.notebook_id(ctx);
            async_assert_eq!(
                id, Some(*expected_id),
                "Expected window_id={window_id}, tab_index={tab_index}, pane_index={pane_index} to contain {expected_id:?}, but got {id:?}")
        })
    })
}

/// Asserts that a notebook is open exactly once in the app.
pub fn assert_notebook_open(notebook_id_key: impl Into<String>) -> AssertionWithDataCallback {
    let notebook_id_key = notebook_id_key.into();
    Box::new(move |app, _, data| {
        let notebook_id = data.get(&notebook_id_key).expect("No saved notebook ID");
        let open_notebooks = notebook_views(app, *notebook_id)
            .filter(|view| !is_notebook_in_hidden_pane(app, view))
            .collect_vec();
        assert_eq!(
            open_notebooks.len(),
            1,
            "Expected exactly one open notebook for {notebook_id:?}, but found: {open_notebooks:?}"
        );

        AssertionOutcome::Success
    })
}

/// Asserts that a notebook is not open anywhere in the app.
pub fn assert_notebook_not_open(notebook_id_key: impl Into<String>) -> AssertionWithDataCallback {
    let notebook_id_key = notebook_id_key.into();
    Box::new(move |app, _, data| {
        let notebook_id = data.get(&notebook_id_key).expect("No saved notebook ID");
        if let Some(notebook_view) =
            notebook_views(app, *notebook_id).find(|view| !is_notebook_in_hidden_pane(app, view))
        {
            panic!("Expected {notebook_id:?} to be closed, but was open in {notebook_view:?}");
        }
        AssertionOutcome::Success
    })
}

/// Checks if a notebook view is in a pane that's hidden for close.
fn is_notebook_in_hidden_pane(app: &App, notebook_view: &ViewHandle<NotebookView>) -> bool {
    for window_id in app.window_ids() {
        if let Some(pane_groups) = app.views_of_type::<PaneGroup>(window_id) {
            for pane_group in pane_groups {
                let is_hidden = pane_group.read(app, |pg, ctx| {
                    for pane_id in pg.pane_ids() {
                        if let Some(notebook_pane) = pg.notebook_pane_by_pane_id(Some(pane_id)) {
                            if notebook_pane.notebook_view(ctx).id() == notebook_view.id() {
                                return pg.is_pane_hidden_for_close(pane_id);
                            }
                        }
                    }
                    false
                });

                if is_hidden {
                    return true;
                }
            }
        }
    }
    false
}

/// Finds all notebook views with the given notebook open.
fn notebook_views(app: &App, id: SyncId) -> impl Iterator<Item = ViewHandle<NotebookView>> + '_ {
    app.window_ids()
        .into_iter()
        .flat_map(|window_id| app.views_of_type::<NotebookView>(window_id))
        .flatten()
        .filter(move |view| view.read(app, |view, ctx| view.notebook_id(ctx)) == Some(id))
}

pub fn assert_open_in_warp_banner_open(tab_index: usize, pane_index: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal = terminal_view(app, window_id, tab_index, pane_index);
        terminal.read(app, |view, _ctx| {
            async_assert!(
                view.is_open_in_warp_banner_open(),
                "Expected the 'Open in Warp' banner to be open"
            )
        })
    })
}

pub fn assert_notebook_renders_mermaid_diagram(
    tab_index: usize,
    pane_index: usize,
    mermaid_block_start: usize,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let editor = notebook_view(app, window_id, tab_index, pane_index)
            .read(app, |notebook, _ctx| notebook.input_editor());
        editor.read(app, |editor, ctx| {
            let is_mermaid_diagram = editor
                .model()
                .as_ref(ctx)
                .render_state()
                .as_ref(ctx)
                .content()
                .block_at_offset(CharOffset::from(mermaid_block_start))
                .is_some_and(|block| matches!(&block.item, BlockItem::MermaidDiagram { .. }));
            async_assert!(
                is_mermaid_diagram,
                "Expected notebook editor to render Mermaid in editable mode"
            )
        })
    })
}
