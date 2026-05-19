use std::{
    collections::{HashMap, HashSet},
    fs,
    future::Future,
    pin::Pin,
};

use crate::ai::skills::skill_manager::SkillWatcherEvent;
use ai::skills::{ParsedSkill, SkillProvider, SkillScope};
use repo_metadata::{
    repositories::DetectedRepositories,
    repository::{Repository, RepositorySubscriber},
    DirectoryWatcher, RepoMetadataModel, RepositoryUpdate, TargetFile,
};
use tempfile::TempDir;
use warp_util::standardized_path::StandardizedPath;
use warpui::{App, SingletonEntity};

use super::SkillWatcher;

struct NoopRepositorySubscriber;

impl RepositorySubscriber for NoopRepositorySubscriber {
    fn on_scan(
        &mut self,
        _repository: &Repository,
        _ctx: &mut warpui::ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        Box::pin(async {})
    }

    fn on_files_updated(
        &mut self,
        _repository: &Repository,
        _update: &RepositoryUpdate,
        _ctx: &mut warpui::ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        Box::pin(async {})
    }
}

/// Helper function for creating a single skill file
fn create_skill_file(dir: &TempDir, name: &str, description: &str, content: &str) -> ParsedSkill {
    let skill_content = format!(
        r#"---
name: {}
description: {}
---
{}
"#,
        name, description, content
    );
    let skills_path = dir.path().join(".agents").join("skills");
    let skill_dir_path = skills_path.join(name);
    let skill_file_path = skill_dir_path.join("SKILL.md");

    fs::create_dir_all(&skill_dir_path).unwrap();
    fs::write(&skill_file_path, skill_content.clone()).unwrap();
    let line_range_start = skill_content.clone().lines().count() - content.lines().count() + 1;
    let line_range_end = skill_content.clone().lines().count() + 1;
    ParsedSkill {
        path: skill_file_path,
        name: name.to_string(),
        description: description.to_string(),
        content: skill_content.clone(),
        line_range: Some(line_range_start..line_range_end),
        provider: SkillProvider::Agents,
        scope: SkillScope::Project,
    }
}

// ============================================================================
// Tests for handle_repository_update
// ============================================================================

#[test]
fn test_handle_repository_update_single_skill_added() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let skill = create_skill_file(&temp_dir, "test", "Test skill", "Test content");

        let update = RepositoryUpdate {
            added: HashSet::from([TargetFile::new(skill.path.clone(), false)]),
            modified: HashSet::new(),
            deleted: HashSet::new(),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        let event = rx.recv().await.unwrap();
        assert_eq!(
            event,
            SkillWatcherEvent::SkillsAdded {
                skills: vec![skill]
            }
        );
    });
}

/// Regression test for warpdotdev/warp#8897: when the provider parent (e.g. `~/.agents`)
/// is itself a symlink, file events fire under the canonical (resolved) path. Without
/// translation, the downstream filter — which compares against un-canonicalized
/// `home_skills_path()` — would silently drop them.
#[test]
fn test_handle_repository_update_translates_canonical_paths_for_symlinked_provider() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        // The actual file lives at the *original* (un-canonicalized) location.
        let skill = create_skill_file(&temp_dir, "test", "Test skill", "Test content");
        let original_provider = temp_dir.path().join(".agents");
        let canonical_provider = temp_dir.path().join("dotfiles-agents");

        // Populate the canonical→originals map as `watch_home_provider_path` would
        // have done after `dunce::canonicalize` resolved the symlink at registration.
        skill_watcher_handle.update(&mut app, |watcher, _ctx| {
            watcher
                .home_provider_canonical_to_originals
                .entry(canonical_provider.clone())
                .or_default()
                .insert(original_provider);
        });

        // Event arrives with the canonical path (what FSEvents would emit when the
        // watch was registered on the symlink target).
        let canonical_skill_path = canonical_provider
            .join("skills")
            .join("test")
            .join("SKILL.md");
        let update = RepositoryUpdate {
            added: HashSet::from([TargetFile::new(canonical_skill_path, false)]),
            modified: HashSet::new(),
            deleted: HashSet::new(),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        // The dispatched skill has the *original* path — translation worked, the
        // filter recognized it, and parse_skill read from the symlink-side location.
        let event = rx.recv().await.unwrap();
        assert_eq!(
            event,
            SkillWatcherEvent::SkillsAdded {
                skills: vec![skill]
            }
        );
        // Pin event cardinality: exactly one event, no duplicates from translation.
        assert!(rx.try_recv().is_err());
    });
}

/// Regression test for the multi-provider shared-canonical case: when two provider
/// parents (e.g. `~/.agents` and `~/.claude`) both symlink to the same directory,
/// a single canonical event must fan out to all originals so each provider's view
/// of the skill stays in sync.
#[test]
fn test_handle_repository_update_fans_out_to_all_originals_for_shared_canonical() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        // Create the same skill file under both .agents and .claude so parse_skill
        // succeeds when called on either translated path.
        let skill_agents = create_skill_file(&temp_dir, "test", "Test skill", "Test content");
        let skill_content = fs::read_to_string(&skill_agents.path).unwrap();
        let claude_skill_dir = temp_dir.path().join(".claude").join("skills").join("test");
        fs::create_dir_all(&claude_skill_dir).unwrap();
        let claude_skill_path = claude_skill_dir.join("SKILL.md");
        fs::write(&claude_skill_path, skill_content).unwrap();

        let agents_provider = temp_dir.path().join(".agents");
        let claude_provider = temp_dir.path().join(".claude");
        let canonical_provider = temp_dir.path().join("shared-dotfiles");

        // Both originals resolve to the same canonical.
        skill_watcher_handle.update(&mut app, |watcher, _ctx| {
            let entry = watcher
                .home_provider_canonical_to_originals
                .entry(canonical_provider.clone())
                .or_default();
            entry.insert(agents_provider);
            entry.insert(claude_provider);
        });

        let canonical_skill_path = canonical_provider
            .join("skills")
            .join("test")
            .join("SKILL.md");
        let update = RepositoryUpdate {
            added: HashSet::from([TargetFile::new(canonical_skill_path, false)]),
            modified: HashSet::new(),
            deleted: HashSet::new(),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        // Two events — one per original provider. HashSet iteration is unordered,
        // so collect paths into a set for order-independent comparison.
        let mut paths_seen: HashSet<_> = HashSet::new();
        for _ in 0..2 {
            match rx.recv().await.unwrap() {
                SkillWatcherEvent::SkillsAdded { skills } => {
                    assert_eq!(skills.len(), 1);
                    paths_seen.insert(skills[0].path.clone());
                }
                other => panic!("Expected SkillsAdded, got {:?}", other),
            }
        }
        assert!(paths_seen.contains(&skill_agents.path));
        assert!(paths_seen.contains(&claude_skill_path));
        // Pin event cardinality: exactly two events from fan-out, no extras.
        assert!(rx.try_recv().is_err());
    });
}

/// When a `moved` event has mismatched canonical fan-out (e.g. `mv` from outside
/// a provider into a shared-canonical provider with two originals), the move
/// can't be paired but must not be dropped — `notify` can report a cross-boundary
/// rename without a separate add, so dropping would lose the destination skill.
/// This test pins that the destination side is salvaged as an add (fanned out
/// to all originals) and the source side as a delete.
#[test]
fn test_handle_repository_update_salvages_mismatched_moves_as_add_and_delete() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        // Two providers symlinked to the same canonical (shared-canonical setup).
        let skill_content = r#"---
name: test
description: Test skill
---
Test content
"#;
        let agents_skill_dir = temp_dir.path().join(".agents").join("skills").join("test");
        fs::create_dir_all(&agents_skill_dir).unwrap();
        let agents_skill_path = agents_skill_dir.join("SKILL.md");
        fs::write(&agents_skill_path, skill_content).unwrap();
        let claude_skill_dir = temp_dir.path().join(".claude").join("skills").join("test");
        fs::create_dir_all(&claude_skill_dir).unwrap();
        let claude_skill_path = claude_skill_dir.join("SKILL.md");
        fs::write(&claude_skill_path, skill_content).unwrap();

        let agents_provider = temp_dir.path().join(".agents");
        let claude_provider = temp_dir.path().join(".claude");
        let canonical_provider = temp_dir.path().join("shared-dotfiles");

        skill_watcher_handle.update(&mut app, |watcher, _ctx| {
            let entry = watcher
                .home_provider_canonical_to_originals
                .entry(canonical_provider.clone())
                .or_default();
            entry.insert(agents_provider.clone());
            entry.insert(claude_provider.clone());
        });

        // Cross-boundary rename: source outside any canonical, destination
        // inside the shared-canonical provider. Fan-out lengths: 1 (source
        // pass-through) vs 2 (destination expands to both originals).
        let outside_source = temp_dir.path().join("outside-source-SKILL.md");
        let canonical_destination = canonical_provider
            .join("skills")
            .join("test")
            .join("SKILL.md");
        let mut moved = HashMap::new();
        moved.insert(
            TargetFile::new(canonical_destination, false),
            TargetFile::new(outside_source.clone(), false),
        );
        let update = RepositoryUpdate {
            added: HashSet::new(),
            modified: HashSet::new(),
            deleted: HashSet::new(),
            moved,
            commit_updated: false,
            index_lock_detected: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        // Two SkillsAdded events expected (one per original at the destination).
        // The source side becomes a SkillsDeleted with the un-translated path.
        let mut added_paths: HashSet<_> = HashSet::new();
        let mut deleted_paths: HashSet<_> = HashSet::new();
        for _ in 0..3 {
            match rx.recv().await.unwrap() {
                SkillWatcherEvent::SkillsAdded { skills } => {
                    assert_eq!(skills.len(), 1);
                    added_paths.insert(skills[0].path.clone());
                }
                SkillWatcherEvent::SkillsDeleted { paths } => {
                    deleted_paths.extend(paths);
                }
            }
        }

        assert!(added_paths.contains(&agents_skill_path));
        assert!(added_paths.contains(&claude_skill_path));
        assert!(deleted_paths.contains(&outside_source));
        // Pin cardinality: no extra events beyond 2 adds + 1 delete.
        assert!(rx.try_recv().is_err());
    });
}

/// Pins the deepest-prefix matching invariant for `translate_canonical_to_original_paths`.
/// `HashMap` iteration order is unstable, so a first-match-wins implementation would
/// translate the same input two different ways across runs when canonicals nest.
/// Deepest match must always win.
#[test]
fn test_translate_canonical_picks_deepest_prefix_match() {
    let (tx, _rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let shallow_canonical = temp_dir.path().join("shared");
        let deep_canonical = shallow_canonical.join("nested");
        let shallow_original = temp_dir.path().join(".shallow-orig");
        let deep_original = temp_dir.path().join(".deep-orig");

        skill_watcher_handle.update(&mut app, |watcher, _ctx| {
            watcher
                .home_provider_canonical_to_originals
                .entry(shallow_canonical.clone())
                .or_default()
                .insert(shallow_original.clone());
            watcher
                .home_provider_canonical_to_originals
                .entry(deep_canonical.clone())
                .or_default()
                .insert(deep_original.clone());
        });

        let input = deep_canonical.join("skills").join("test").join("SKILL.md");

        skill_watcher_handle.read(&app, |watcher, _ctx| {
            let translated = watcher.translate_canonical_to_original_paths(&input);
            // Deepest match: rel = "skills/test/SKILL.md" joined under .deep-orig.
            let expected = deep_original.join("skills").join("test").join("SKILL.md");
            assert_eq!(translated, vec![expected]);
            // The shallow translation (rel = "nested/skills/...") must NOT be the result.
            let shallow_wrong = shallow_original
                .join("nested")
                .join("skills")
                .join("test")
                .join("SKILL.md");
            assert!(!translated.contains(&shallow_wrong));
        });
    });
}

#[test]
fn test_handle_home_files_changed_keeps_remaining_original_for_shared_canonical() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let agents_provider = temp_dir.path().join(".agents");
        let claude_provider = temp_dir.path().join(".claude");
        let canonical_provider = temp_dir.path().join("shared-dotfiles");
        fs::create_dir_all(&canonical_provider).unwrap();

        let skill_content = r#"---
name: test
description: Test skill
---
Test content
"#;
        let claude_skill_dir = claude_provider.join("skills").join("test");
        fs::create_dir_all(&claude_skill_dir).unwrap();
        let claude_skill_path = claude_skill_dir.join("SKILL.md");
        fs::write(&claude_skill_path, skill_content).unwrap();

        skill_watcher_handle.update(&mut app, |watcher, ctx| {
            let canonical_path =
                StandardizedPath::from_local_canonicalized(&canonical_provider).unwrap();
            let repo_handle = DirectoryWatcher::handle(ctx)
                .update(ctx, |directory_watcher, ctx| {
                    directory_watcher.add_directory(canonical_path, ctx)
                })
                .unwrap();

            let agents_start = repo_handle.update(ctx, |repo, ctx| {
                repo.start_watching(Box::new(NoopRepositorySubscriber), ctx)
            });
            let claude_start = repo_handle.update(ctx, |repo, ctx| {
                repo.start_watching(Box::new(NoopRepositorySubscriber), ctx)
            });

            watcher.home_provider_watchers.insert(
                agents_provider.clone(),
                (repo_handle.clone(), agents_start.subscriber_id),
            );
            watcher.home_provider_watchers.insert(
                claude_provider.clone(),
                (repo_handle, claude_start.subscriber_id),
            );

            let originals = watcher
                .home_provider_canonical_to_originals
                .entry(canonical_provider.clone())
                .or_default();
            originals.insert(agents_provider.clone());
            originals.insert(claude_provider.clone());
        });

        let delete_event = watcher::BulkFilesystemWatcherEvent {
            deleted: HashSet::from([agents_provider.clone()]),
            ..Default::default()
        };
        skill_watcher_handle.update(&mut app, |watcher, ctx| {
            watcher.handle_home_files_changed(&delete_event, ctx);
        });

        assert_eq!(
            rx.recv().await.unwrap(),
            SkillWatcherEvent::SkillsDeleted {
                paths: vec![agents_provider.clone()]
            }
        );

        skill_watcher_handle.read(&app, |watcher, ctx| {
            assert!(!watcher
                .home_provider_watchers
                .contains_key(&agents_provider));
            let (repo_handle, _) = watcher
                .home_provider_watchers
                .get(&claude_provider)
                .expect("remaining provider watcher should stay registered");
            assert_eq!(repo_handle.read(ctx, |repo, _| repo.watcher_count()), 1);

            let originals = watcher
                .home_provider_canonical_to_originals
                .get(&canonical_provider)
                .expect("canonical entry should remain for the remaining original");
            assert_eq!(originals.len(), 1);
            assert!(!originals.contains(&agents_provider));
            assert!(originals.contains(&claude_provider));
        });

        let canonical_skill_path = canonical_provider
            .join("skills")
            .join("test")
            .join("SKILL.md");
        let update = RepositoryUpdate {
            added: HashSet::from([TargetFile::new(canonical_skill_path, false)]),
            modified: HashSet::new(),
            deleted: HashSet::new(),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
        };

        skill_watcher_handle.update(&mut app, |watcher, ctx| {
            watcher.handle_repository_update(&update, ctx);
        });

        let event = rx.recv().await.unwrap();
        let SkillWatcherEvent::SkillsAdded { skills } = event else {
            panic!("Expected SkillsAdded event");
        };
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].path, claude_skill_path);
        assert!(rx.try_recv().is_err());
    });
}

#[test]
fn test_handle_repository_update_skill_modified() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let skill = create_skill_file(&temp_dir, "test", "Test skill", "Test content");

        let update = RepositoryUpdate {
            added: HashSet::new(),
            modified: HashSet::from([TargetFile::new(skill.path.clone(), false)]),
            deleted: HashSet::new(),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        let event = rx.recv().await.unwrap();
        assert_eq!(
            event,
            SkillWatcherEvent::SkillsAdded {
                skills: vec![skill]
            }
        );
    });
}

#[test]
fn test_handle_repository_update_skill_deleted() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let skill = create_skill_file(&temp_dir, "test", "Test skill", "Test content");

        let update = RepositoryUpdate {
            added: HashSet::new(),
            modified: HashSet::new(),
            deleted: HashSet::from([TargetFile::new(skill.path.clone(), false)]),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        let event = rx.recv().await.unwrap();
        assert_eq!(
            event,
            SkillWatcherEvent::SkillsDeleted {
                paths: vec![skill.path]
            }
        );
    });
}

#[test]
fn test_handle_repository_update_multiple_skills_deleted() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let skill_a = create_skill_file(&temp_dir, "skill-a", "Skill A", "Content A");
        let skill_b = create_skill_file(&temp_dir, "skill-b", "Skill B", "Content B");

        let update = RepositoryUpdate {
            added: HashSet::new(),
            modified: HashSet::new(),
            deleted: HashSet::from([
                TargetFile::new(skill_a.path.clone(), false),
                TargetFile::new(skill_b.path.clone(), false),
            ]),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        let event = rx.recv().await.unwrap();
        let SkillWatcherEvent::SkillsDeleted { mut paths } = event else {
            panic!("Expected SkillsDeleted event");
        };
        paths.sort();
        let mut expected = vec![skill_a.path, skill_b.path];
        expected.sort();
        assert_eq!(paths, expected);
    });
}

#[test]
fn test_handle_repository_update_skill_moved() {
    let (tx, rx) = async_channel::unbounded();

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let old_skill = create_skill_file(&temp_dir, "old-skill", "Old skill", "Old content");
        let new_skill = create_skill_file(&temp_dir, "new-skill", "New skill", "New content");

        // moved is HashMap<to_target, from_target>
        let update = RepositoryUpdate {
            added: HashSet::new(),
            modified: HashSet::new(),
            deleted: HashSet::new(),
            moved: HashMap::from([(
                TargetFile::new(new_skill.path.clone(), false),
                TargetFile::new(old_skill.path.clone(), false),
            )]),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        // Collect both events: SkillsAdded for the new location and SkillsDeleted for the old
        let event1 = rx.recv().await.unwrap();
        let event2 = rx.recv().await.unwrap();

        let added_event = SkillWatcherEvent::SkillsAdded {
            skills: vec![new_skill],
        };
        let deleted_event = SkillWatcherEvent::SkillsDeleted {
            paths: vec![old_skill.path],
        };
        assert!(
            (event1 == added_event && event2 == deleted_event)
                || (event1 == deleted_event && event2 == added_event),
            "Expected one SkillsAdded and one SkillsDeleted event; got: {event1:?} and {event2:?}"
        );
    });
}

// ============================================================================
// Tests for handle_repository_update - directory addition
// ============================================================================

/// When a non-skill directory is added within a known repo, `handle_repository_update` should
/// queue the repo root in `queued_project_directory_creations` for a later skill scan.
#[test]
fn test_handle_repository_update_non_skill_directory_added_queues_project_directory() {
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        let detected_repos_handle = app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let (tx, _rx) = async_channel::unbounded();
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let canonical_repo = StandardizedPath::from_local_canonicalized(temp_dir.path()).unwrap();

        // Register the temp dir as a known repo root so get_root_for_path resolves it.
        detected_repos_handle.update(&mut app, |repos, _| {
            repos.insert_test_repo_root(canonical_repo.clone());
        });

        // Seed watched_repos so get_watched_repo_path can resolve the temp dir to this root.
        // Use the canonicalized path to match what CanonicalizedPath::try_from resolves on macOS
        // (where /var is a symlink to /private/var).
        skill_watcher_handle.update(&mut app, |watcher, _| {
            watcher
                .watched_repos
                .insert(canonical_repo.to_local_path().unwrap());
        });

        // The added path must exist on disk for CanonicalizedPath resolution.
        let new_dir = canonical_repo.to_local_path().unwrap().join("new-feature");
        fs::create_dir_all(&new_dir).unwrap();

        let update = RepositoryUpdate {
            added: HashSet::from([TargetFile::new(new_dir, false)]),
            modified: HashSet::new(),
            deleted: HashSet::new(),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        // The repo root should be queued for a skill scan.
        skill_watcher_handle.read(&app, |watcher, _| {
            assert_eq!(watcher.queued_project_directory_creations.len(), 1);
            assert_eq!(
                watcher.queued_project_directory_creations[0].path,
                canonical_repo.to_local_path().unwrap()
            );
        });
    });
}

/// A modified non-skill file in a known repo should NOT queue anything in
/// `queued_project_directory_creations`; only directory additions can introduce new skill files.
#[test]
fn test_handle_repository_update_non_skill_file_modified_in_repo_does_not_queue_project_directory()
{
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        let detected_repos_handle = app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let (tx, _rx) = async_channel::unbounded();
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let canonical_repo = StandardizedPath::from_local_canonicalized(temp_dir.path()).unwrap();

        detected_repos_handle.update(&mut app, |repos, _| {
            repos.insert_test_repo_root(canonical_repo.clone());
        });

        // Create the file on disk so CanonicalizedPath resolution succeeds.
        let readme = temp_dir.path().join("README.md");
        fs::write(&readme, "# Project").unwrap();

        let update = RepositoryUpdate {
            added: HashSet::new(),
            modified: HashSet::from([TargetFile::new(readme, false)]),
            deleted: HashSet::new(),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        // Modifying a plain file must NOT queue a project directory scan.
        skill_watcher_handle.read(&app, |watcher, _| {
            assert_eq!(watcher.queued_project_directory_creations.len(), 0);
        });
    });
}

/// When a regular (non-skill) file is added within a known repo, `handle_repository_update`
/// should NOT queue anything in `queued_project_directory_creations` because only directory
/// additions may introduce new skill files.
#[test]
fn test_handle_repository_update_non_skill_file_added_does_not_queue_project_directory() {
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new_for_testing);
        let detected_repos_handle = app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        let (tx, _rx) = async_channel::unbounded();
        let skill_watcher_handle = app.add_model(|ctx| SkillWatcher::new_for_testing(ctx, tx));

        let temp_dir = TempDir::new().unwrap();
        let canonical_repo = StandardizedPath::from_local_canonicalized(temp_dir.path()).unwrap();

        detected_repos_handle.update(&mut app, |repos, _| {
            repos.insert_test_repo_root(canonical_repo.clone());
        });

        // Create a regular file (not a directory, not a skill file) on disk.
        let readme = temp_dir.path().join("README.md");
        fs::write(&readme, "# Project").unwrap();

        let update = RepositoryUpdate {
            added: HashSet::from([TargetFile::new(readme, false)]),
            modified: HashSet::new(),
            deleted: HashSet::new(),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        };

        skill_watcher_handle.update(&mut app, |skill_watcher, ctx| {
            skill_watcher.handle_repository_update(&update, ctx);
        });

        // A plain file being added must NOT queue a project directory scan.
        skill_watcher_handle.read(&app, |watcher, _| {
            assert_eq!(watcher.queued_project_directory_creations.len(), 0);
        });
    });
}

// ============================================================================
// Tests for handle_queued_project_directory_creations
// ============================================================================
