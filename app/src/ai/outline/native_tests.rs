use std::cell::RefCell;
use std::rc::Rc;

use repo_metadata::TargetFile;
use warp_util::standardized_path::StandardizedPath;
use warpui::App;

use super::*;

struct RepoOutlinesEventCollector;

impl Entity for RepoOutlinesEventCollector {
    type Event = ();
}

#[test]
fn fourth_repo_evicts_lru_state_and_rejects_its_stale_updates() {
    App::test((), |mut app| async move {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo_paths = ["one", "two", "three", "four"].map(|name| temp_dir.path().join(name));
        for path in &repo_paths {
            std::fs::create_dir(path).unwrap();
        }

        let watcher = app.add_singleton_model(DirectoryWatcher::new_for_testing);
        let repositories = repo_paths.clone().map(|path| {
            watcher
                .update(&mut app, |watcher, ctx| {
                    watcher.add_directory(
                        StandardizedPath::from_local_canonicalized(&path).unwrap(),
                        ctx,
                    )
                })
                .unwrap()
        });
        let evicted_outline = build_outline(&repo_paths[1], None).await.unwrap();
        let repo_outlines = app.add_model(RepoOutlines::new_for_test);
        let events = Rc::new(RefCell::new(Vec::new()));
        let collector = app.add_model(|_| RepoOutlinesEventCollector);
        collector.update(&mut app, {
            let events = events.clone();
            let repo_outlines = repo_outlines.clone();
            move |_, ctx| {
                ctx.subscribe_to_model(&repo_outlines, move |_, _, event, _| {
                    let RepoOutlinesEvent::OutlinesUpdated(path) = event;
                    events.borrow_mut().push(path.clone());
                });
            }
        });

        repo_outlines.update(&mut app, |outlines, ctx| {
            outlines.retain_outline_state(
                repo_paths[0].clone(),
                OutlineState {
                    repository: repositories[0].clone(),
                    status: OutlineStatus::Pending,
                    subscriber_id: None,
                    generation: 0,
                },
                ctx,
            );
            outlines.retain_outline_state(
                repo_paths[1].clone(),
                OutlineState {
                    repository: repositories[1].clone(),
                    status: OutlineStatus::Complete(evicted_outline),
                    subscriber_id: None,
                    generation: 1,
                },
                ctx,
            );
            outlines.start_repository_subscription(&repositories[1], repo_paths[1].clone(), 1, ctx);
        });
        repositories[1].read(&app, |repository, _| {
            assert_eq!(repository.watcher_count(), 1);
        });

        let (abort_handle, _abort_registration) = AbortHandle::new_pair();
        let observed_abort_handle = abort_handle.clone();
        repo_outlines.update(&mut app, |outlines, _| {
            outlines.active_outline_task = Some(ActiveOutlineTask {
                repo_path: repo_paths[1].clone(),
                generation: 1,
                abort_handle,
            });
        });
        for index in 2..repo_paths.len() - 1 {
            repo_outlines.update(&mut app, |outlines, ctx| {
                outlines.retain_outline_state(
                    repo_paths[index].clone(),
                    OutlineState {
                        repository: repositories[index].clone(),
                        status: OutlineStatus::Pending,
                        subscriber_id: None,
                        generation: index as u64,
                    },
                    ctx,
                );
            });
        }
        repo_outlines.update(&mut app, |outlines, ctx| {
            outlines.index_repo(repositories[0].clone(), ctx);
            outlines.retain_outline_state(
                repo_paths[3].clone(),
                OutlineState {
                    repository: repositories[3].clone(),
                    status: OutlineStatus::Pending,
                    subscriber_id: None,
                    generation: 3,
                },
                ctx,
            );
        });

        repo_outlines.read(&app, |outlines, _| {
            assert!(!outlines.outlines.contains_key(&repo_paths[1]));
            assert_eq!(outlines.outlines.len(), MAX_RETAINED_REPO_OUTLINES);
        });
        repositories[1].read(&app, |repository, _| {
            assert_eq!(repository.watcher_count(), 0);
        });
        assert!(observed_abort_handle.is_aborted());
        assert_eq!(events.borrow().as_slice(), &[repo_paths[1].clone()]);

        let replacement_outline = build_outline(&repo_paths[1], None).await.unwrap();
        repo_outlines.update(&mut app, |outlines, ctx| {
            outlines.retain_outline_state(
                repo_paths[1].clone(),
                OutlineState {
                    repository: repositories[1].clone(),
                    status: OutlineStatus::Complete(replacement_outline),
                    subscriber_id: None,
                    generation: 4,
                },
                ctx,
            );
            outlines.handle_repository_update(
                &repo_paths[1],
                1,
                RepositoryUpdate {
                    added: [TargetFile::new(repo_paths[1].join("stale.rs"), false)].into(),
                    ..Default::default()
                },
                ctx,
            );
            assert!(matches!(
                outlines.outlines[&repo_paths[1]].status,
                OutlineStatus::Complete(_)
            ));
        });
    });
}
