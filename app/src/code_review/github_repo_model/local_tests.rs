use repo_metadata::DirectoryWatcher;
use warp_util::standardized_path::StandardizedPath;
use warpui::{App, ModelHandle};

use super::*;
use crate::code_review::git_repo_model::GitRepoStatusModel;
use crate::util::git::{PrStackLayer, RepositoryInfo};

fn pr(number: u64) -> PrInfo {
    PrInfo {
        number,
        url: format!("https://github.com/warp/warp/pull/{number}"),
        state: "OPEN".to_string(),
        draft: false,
        base_branch: "main".to_string(),
    }
}

fn repository_info() -> RepositoryInfo {
    RepositoryInfo {
        name: "warp".to_string(),
        owner: Some("warpdotdev".to_string()),
        host: Some("github.com".to_string()),
    }
}

fn stack_layer(number: u64, base_ref: &str) -> PrStackLayer {
    PrStackLayer {
        pr: pr(number),
        title: format!("PR #{number}"),
        head_ref: format!("feature-{number}"),
        head_oid: "a".repeat(40),
        base_ref: base_ref.to_string(),
        base_oid: "b".repeat(40),
        merged_at: None,
    }
}

fn stack_info(number: u64, layers: Vec<PrStackLayer>) -> PrStackInfo {
    PrStackInfo {
        number,
        trunk_ref: "main".to_string(),
        layers,
    }
}

fn test_repository_handle(
    app: &mut App,
    temp_dir: &tempfile::TempDir,
) -> ModelHandle<repo_metadata::Repository> {
    let watcher_handle = app.add_singleton_model(DirectoryWatcher::new_for_testing);
    watcher_handle.update(app, |watcher, ctx| {
        watcher
            .add_directory(
                StandardizedPath::from_local_canonicalized(temp_dir.path()).unwrap(),
                ctx,
            )
            .unwrap()
    })
}

/// Builds an inert `GitHubRepoModel` over a throwaway sibling git-status
/// model. The model never subscribes or fetches; tests drive state directly.
fn new_github_repo_model_for_test(
    app: &mut App,
) -> (tempfile::TempDir, ModelHandle<LocalGitHubRepoModel>) {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let repository = test_repository_handle(app, &temp_dir);
    let git_status =
        app.add_model(move |ctx| GitRepoStatusModel::new_local_for_test(repository, None, ctx));
    let model = app.add_model(move |_| LocalGitHubRepoModel::new_for_test(git_status));
    (temp_dir, model)
}

#[test]
fn pr_info_cleared_on_branch_change() {
    App::test((), |mut app| async move {
        let (_temp_dir, model) = new_github_repo_model_for_test(&mut app);

        // On feature-a with a cached PR.
        model.update(&mut app, |model, ctx| {
            model.branch = Some("feature-a".to_string());
            model.set_pr_info_for_test(Some(pr(123)), ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.pr_info(), Some(&pr(123)));
        });

        // Switching branches clears the now-stale PR.
        model.update(&mut app, |model, ctx| {
            model.branch = Some("feature-b".to_string());
            if model.pr_info.take().is_some() {
                ctx.emit(GitHubRepoEvent::PrInfoChanged);
            }
        });
        model.read(&app, |model, _| {
            assert_eq!(model.pr_info(), None);
        });
    });
}

#[test]
fn repository_info_preserved_on_fetch_error() {
    App::test((), |mut app| async move {
        let (_temp_dir, model) = new_github_repo_model_for_test(&mut app);

        model.update(&mut app, |model, ctx| {
            model.set_repository_info_for_test(Some(repository_info()), ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.repository_info(), Some(&repository_info()));
        });

        model.update(&mut app, |model, ctx| {
            model.handle_repository_info_result(Err(anyhow::anyhow!("gh failed")), ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.repository_info(), Some(&repository_info()));
        });
    });
}

#[test]
fn repository_info_cleared_on_authoritative_empty_result() {
    App::test((), |mut app| async move {
        let (_temp_dir, model) = new_github_repo_model_for_test(&mut app);

        model.update(&mut app, |model, ctx| {
            model.set_repository_info_for_test(Some(repository_info()), ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.repository_info(), Some(&repository_info()));
        });

        model.update(&mut app, |model, ctx| {
            model.handle_repository_info_result(Ok(None), ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.repository_info(), None);
        });
    });
}

#[test]
fn pr_info_cleared_when_branch_goes_away() {
    App::test((), |mut app| async move {
        let (_temp_dir, model) = new_github_repo_model_for_test(&mut app);

        model.update(&mut app, |model, ctx| {
            model.branch = Some("feature-a".to_string());
            model.set_pr_info_for_test(Some(pr(123)), ctx);
        });

        // Branch goes to `None` (e.g. metadata load failure / detached HEAD).
        model.update(&mut app, |model, ctx| {
            model.branch = None;
            if model.pr_info.take().is_some() {
                ctx.emit(GitHubRepoEvent::PrInfoChanged);
            }
        });
        model.read(&app, |model, _| {
            assert_eq!(model.pr_info(), None);
        });
    });
}

#[test]
fn repository_info_survives_branch_change() {
    App::test((), |mut app| async move {
        let (_temp_dir, model) = new_github_repo_model_for_test(&mut app);

        model.update(&mut app, |model, ctx| {
            model.branch = Some("feature-a".to_string());
            model.set_repository_info_for_test(Some(repository_info()), ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.repository_info(), Some(&repository_info()));
        });

        // Repository info is branch-independent — a branch change leaves it.
        model.update(&mut app, |model, _| {
            model.branch = Some("feature-b".to_string());
        });
        model.read(&app, |model, _| {
            assert_eq!(model.repository_info(), Some(&repository_info()));
        });
    });
}

#[test]
fn stack_info_ignores_stale_result_after_branch_change() {
    App::test((), |mut app| async move {
        let (_temp_dir, model) = new_github_repo_model_for_test(&mut app);

        model.update(&mut app, |model, ctx| {
            model.branch = Some("feature-a".to_string());
            model.set_pr_info_for_test(Some(pr(123)), ctx);
        });

        // Branch (and its PR) changes before the in-flight fetch that was
        // started for feature-a/#123 resolves.
        model.update(&mut app, |model, ctx| {
            model.branch = Some("feature-b".to_string());
            model.set_pr_info_for_test(Some(pr(456)), ctx);
        });

        // The stale result for feature-a/#123 arrives after the switch.
        let stale = stack_info(1, vec![stack_layer(123, "main"), stack_layer(200, "123")]);
        model.update(&mut app, |model, ctx| {
            model.handle_stack_info_result(
                StackDiscoveryResult::Available(stale),
                "feature-a".to_string(),
                123,
                ctx,
            );
        });

        model.read(&app, |model, _| {
            assert_eq!(model.stack_info(), None);
        });
    });
}

#[test]
fn stack_info_preserved_across_transient_failure() {
    App::test((), |mut app| async move {
        let (_temp_dir, model) = new_github_repo_model_for_test(&mut app);
        let snapshot = stack_info(1, vec![stack_layer(123, "main"), stack_layer(200, "123")]);

        model.update(&mut app, |model, ctx| {
            model.branch = Some("feature-a".to_string());
            model.set_pr_info_for_test(Some(pr(123)), ctx);
            model.set_stack_info_for_test(Some(snapshot.clone()), ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.stack_info(), Some(&snapshot));
        });

        model.update(&mut app, |model, ctx| {
            model.handle_stack_info_result(
                StackDiscoveryResult::Unavailable {
                    reason: "gh timed out".to_string(),
                },
                "feature-a".to_string(),
                123,
                ctx,
            );
        });
        model.read(&app, |model, _| {
            assert_eq!(model.stack_info(), Some(&snapshot));
        });
    });
}

#[test]
fn stack_info_cleared_on_branch_change() {
    App::test((), |mut app| async move {
        let (_temp_dir, model) = new_github_repo_model_for_test(&mut app);
        let snapshot = stack_info(1, vec![stack_layer(123, "main"), stack_layer(200, "123")]);

        model.update(&mut app, |model, ctx| {
            model.branch = Some("feature-a".to_string());
            model.set_pr_info_for_test(Some(pr(123)), ctx);
            model.set_stack_info_for_test(Some(snapshot), ctx);
        });
        model.read(&app, |model, _| {
            assert!(model.stack_info().is_some());
        });

        // Mirrors the branch-change handling in `LocalGitHubRepoModel::new`,
        // which clears `stack_info` immediately rather than waiting for a
        // fresh discovery result.
        model.update(&mut app, |model, ctx| {
            model.branch = Some("feature-b".to_string());
            if model.stack_info.take().is_some() {
                ctx.emit(GitHubRepoEvent::StackInfoChanged);
            }
        });
        model.read(&app, |model, _| {
            assert_eq!(model.stack_info(), None);
        });
    });
}
