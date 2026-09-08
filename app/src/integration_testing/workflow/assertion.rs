use warpui::integration::{AssertionCallback, AssertionWithDataCallback};
use warpui::{App, ViewHandle, async_assert, async_assert_eq};

use crate::integration_testing::cloud_object::assert_metadata_revision;
use crate::integration_testing::view_getters::workflow_view;
use crate::server::ids::SyncId;
use crate::workflows::workflow_view::WorkflowView;
use crate::workflows::{CloudWorkflowModel, WorkflowId};

/// Asserts metadata exists for the workflow with the given key and that the revision in that
/// metadata matches the given expected revision.
pub fn assert_workflow_metadata_revision(
    id: impl AsRef<str>,
    expected_revision: i64,
) -> AssertionCallback {
    assert_metadata_revision::<WorkflowId, CloudWorkflowModel>(id.as_ref(), expected_revision)
}

/// Asserts that a pane has the given workflow open.
pub fn assert_workflow_id(
    tab_index: usize,
    pane_index: usize,
    expected_id_key: impl Into<String>,
) -> AssertionWithDataCallback {
    let expected_id_key = expected_id_key.into();
    Box::new(move |app, window_id, data| {
        let expected_id = data.get(&expected_id_key).expect("No saved workflow ID");

        let workflow = workflow_view(app, window_id, tab_index, pane_index);
        workflow.read(app, |workflow, _ctx| {
            let id = workflow.workflow_id();
            async_assert_eq!(
                id, *expected_id,
                "Expected window_id={window_id}, tab_index={tab_index}, pane_index={pane_index} to contain {expected_id:?}, but got {id:?}")
        })
    })
}

pub fn assert_no_workflow_pane_open() -> AssertionCallback {
    Box::new(move |app, _| {
        let count = get_all_open_workflows(app).len();
        async_assert!(count == 0, "Expected no workflow panes to be open")
    })
}

pub fn assert_no_team_workflow_pane_open() -> AssertionCallback {
    Box::new(move |app, _| {
        let count = get_all_open_workflows(app)
            .iter()
            .filter(|view| view.read(app, |v, _| v.is_team_workflow()))
            .count();
        async_assert!(count == 0, "Expected no workflow panes to be open")
    })
}

pub fn assert_open_workflow_pane_count_equals(num: usize) -> AssertionCallback {
    Box::new(move |app, _| {
        let count = get_all_open_workflows(app).len();
        async_assert!(
            count == num,
            "Expected number of open workflow panes to be: {num}. Found {count} instead"
        )
    })
}

pub fn assert_open_team_workflow_pane_count_equals(num: usize) -> AssertionCallback {
    Box::new(move |app, _| {
        let count = get_all_open_workflows(app)
            .iter()
            .filter(|view| view.read(app, |v, _| v.is_team_workflow()))
            .count();
        async_assert!(
            count == num,
            "Expected number of open workflow panes to be: {num}. Found {count} instead"
        )
    })
}

/// Find number of workflows that are open by id
pub fn open_workflow_count(app: &App, id: SyncId) -> usize {
    app.window_ids()
        .into_iter()
        .flat_map(|window_id| app.views_of_type::<WorkflowView>(window_id))
        .flatten()
        .filter(move |view| view.read(app, |view, _ctx| view.workflow_id()) == id)
        .count()
}

fn get_all_open_workflows(app: &mut App) -> Vec<ViewHandle<WorkflowView>> {
    app.window_ids()
        .into_iter()
        .flat_map(|window_id| app.views_of_type::<WorkflowView>(window_id))
        .flatten()
        .collect()
}
