use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use ::ai::project_context::model::ProjectContextModel;
use futures::channel::oneshot;
use repo_metadata::local_model::IndexedRepoState;
use repo_metadata::{RepoMetadataEvent, RepoMetadataModel, RepositoryIdentifier};
use warp_util::standardized_path::StandardizedPath;
use warpui::r#async::FutureExt as _;
use warpui::{App, SingletonEntity};

use super::PersistedWorkspace;
use crate::test_util::terminal::initialize_app_for_terminal_view;
use crate::test_util::{Stub, VirtualFS};

#[test]
fn user_added_workspace_fully_indexes_non_git_directory() {
    VirtualFS::test("user_added_workspace_full_index", |dirs, mut vfs| {
        vfs.mkdir("workspace/src/deep")
            .with_files(vec![Stub::FileWithContent(
                "workspace/src/deep/main.rs",
                "fn main() {}\n",
            )]);

        let workspace = dirs.tests().join("workspace");
        let deep_file = workspace.join("src/deep/main.rs");

        App::test((), |mut app| async move {
            initialize_app_for_terminal_view(&mut app);
            app.add_singleton_model(|ctx| ProjectContextModel::new_from_persisted(vec![], ctx));

            let workspace_path = StandardizedPath::from_local_canonicalized(&workspace).unwrap();
            let deep_file_path = StandardizedPath::try_from_local(&deep_file).unwrap();
            let repo_id = RepositoryIdentifier::local(workspace_path.clone());

            let (tx, rx) = oneshot::channel();
            let completed = Rc::new(RefCell::new(Some(tx)));
            let completed_for_event = completed.clone();
            let workspace_path_for_event = workspace_path.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(
                    &RepoMetadataModel::handle(ctx),
                    move |_, event: &RepoMetadataEvent, _ctx| {
                        if matches!(
                            event,
                            RepoMetadataEvent::RepositoryUpdated {
                                id: RepositoryIdentifier::Local(path)
                            } if path == &workspace_path_for_event
                        ) {
                            if let Some(tx) = completed_for_event.borrow_mut().take() {
                                let _ = tx.send(());
                            }
                        }
                    },
                );
            });

            PersistedWorkspace::handle(&app).update(&mut app, |workspaces, ctx| {
                workspaces.user_added_workspace(workspace.clone(), ctx);
                assert!(matches!(
                    RepoMetadataModel::as_ref(ctx).repository_state(&repo_id, ctx),
                    Some(IndexedRepoState::Pending(_))
                ));
            });
            rx.with_timeout(Duration::from_secs(5))
                .await
                .expect("timed out waiting for user-added workspace index")
                .expect("user-added workspace index completion sender dropped");

            app.read(|ctx| {
                let metadata = RepoMetadataModel::as_ref(ctx);
                assert!(!metadata.is_lazy_loaded_path(&workspace_path, ctx));
                let Some(IndexedRepoState::Indexed(state)) =
                    metadata.repository_state(&repo_id, ctx)
                else {
                    panic!("expected fully indexed user-added workspace");
                };
                assert!(state.entry.contains(&deep_file_path));
            });
        });
    });
}
