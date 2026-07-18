use std::path::PathBuf;

use pathfinder_geometry::vector::vec2f;
use warpui::platform::WindowStyle;
use warpui::{App, EntityIdSet, Presenter, WindowInvalidation};

use super::super::data::{CommitNode, FileChange, GitChangeKind, RepositorySnapshot};
use super::*;

#[test]
fn paints_snapshot_with_finite_geometry() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, SourceControlView::new);
        let repo = LocalOrRemotePath::Local(PathBuf::from("/tmp/test-repo"));
        let snapshot = RepositorySnapshot {
            staged_changes: vec![FileChange {
                path: "src/main.rs".to_string(),
                kind: GitChangeKind::Modified,
            }],
            commits: vec![
                CommitNode {
                    hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                    parents: vec!["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()],
                    author: "Test Author".to_string(),
                    timestamp: 1_700_000_000,
                    subject: "Latest commit".to_string(),
                    refs: vec![],
                },
                CommitNode {
                    hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
                    parents: vec![],
                    author: "Test Author".to_string(),
                    timestamp: 1_699_999_000,
                    subject: "Previous commit".to_string(),
                    refs: vec![],
                },
            ],
            has_more_history: true,
            has_head: true,
            ..Default::default()
        };

        view.update(&mut app, |view, ctx| {
            view.repositories = vec![repo.clone()];
            view.selected_repository = Some(repo);
            view.set_snapshot(snapshot);
            ctx.notify();
        });

        let mut updated = EntityIdSet::default();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };
        let mut presenter = Presenter::new(window_id);
        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            // Before the chevrons were explicitly sized, this paint aborted on
            // Scene::validate_rect debug assertions (infinite rect width).
            presenter.build_scene(vec2f(300., 800.), 1., None, ctx);
        });
    })
}
