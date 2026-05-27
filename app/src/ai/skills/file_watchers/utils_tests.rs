use repo_metadata::entry::{DirectoryEntry, Entry, FileMetadata};
use repo_metadata::file_tree_store::FileTreeState;
use repo_metadata::file_tree_update::{
    DirectoryNodeMetadata, FileNodeMetadata, FileTreeEntryUpdate, RepoNodeMetadata,
};
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::{
    DirectoryWatcher, RepoMetadataModel, RepoMetadataUpdate, RepositoryIdentifier,
};
use std::path::Path;
use virtual_fs::{Stub, VirtualFS};
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::App;

use super::{
    extract_skill_parent_directory, find_skill_files_in_tree, is_home_provider_path,
    is_home_skill_directory, is_skill_file, read_skills_from_files,
};

// ============================================================================
// Tests for is_skill_file
// ============================================================================

#[test]
fn is_skill_file_valid_paths() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };

    let path = home_dir
        .join("repos")
        .join("project")
        .join(".agents")
        .join("skills")
        .join("my-skill")
        .join("SKILL.md");
    assert!(is_skill_file(&path));

    let path = home_dir
        .join(".claude")
        .join("skills")
        .join("test-skill")
        .join("SKILL.md");
    assert!(is_skill_file(&path));

    // Path with multiple levels of prefix
    let path = home_dir
        .join("very")
        .join("deep")
        .join("path")
        .join("to")
        .join("repo")
        .join(".agents")
        .join("skills")
        .join("another-skill")
        .join("SKILL.md");
    assert!(is_skill_file(&path));
}

#[test]
fn is_skill_file_invalid_provider() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };

    let path = home_dir
        .join("repos")
        .join("project")
        .join(".garbage")
        .join("skills")
        .join("my-skill")
        .join("SKILL.md");
    assert!(!is_skill_file(&path));

    let path = home_dir
        .join(".garbage")
        .join("skills")
        .join("test-skill")
        .join("SKILL.md");
    assert!(!is_skill_file(&path));
}

#[test]
fn is_skill_file_invalid_format() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };

    // Missing SKILL.md at the end
    let path = home_dir
        .join("repos")
        .join("project")
        .join(".agents")
        .join("skills")
        .join("my-skill")
        .join("README.md");
    assert!(!is_skill_file(&path));

    // No skill name directory
    let path = home_dir
        .join("repos")
        .join("project")
        .join(".agents")
        .join("skills")
        .join("SKILL.md");
    assert!(!is_skill_file(&path));

    // Extra directory level in skill name
    let path = home_dir
        .join("repos")
        .join("project")
        .join(".agents")
        .join("skills")
        .join("nested")
        .join("my-skill")
        .join("SKILL.md");
    assert!(!is_skill_file(&path));

    // Missing provider directory
    let path = home_dir
        .join("repos")
        .join("project")
        .join("skills")
        .join("my-skill")
        .join("SKILL.md");
    assert!(!is_skill_file(&path));

    // Plain file path
    let path = home_dir.join("some").join("random").join("file.txt");
    assert!(!is_skill_file(&path));
}

// ============================================================================
// Tests for extract_skill_parent_directory
// ============================================================================

#[test]
fn extract_skill_parent_directory_from_repo_root() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };
    let parent_directory = home_dir.join("repo");
    let skill_path = parent_directory
        .join(".agents")
        .join("skills")
        .join("my-skill")
        .join("SKILL.md");
    let result = extract_skill_parent_directory(&skill_path);
    assert_eq!(result.ok(), Some(parent_directory));
}

#[test]
fn extract_skill_parent_directory_from_subdirectory() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };
    let parent_directory = home_dir.join("repo").join("packages").join("frontend");
    let skill_path = parent_directory
        .join(".agents")
        .join("skills")
        .join("build")
        .join("SKILL.md");
    let result = extract_skill_parent_directory(&skill_path);
    assert_eq!(result.ok(), Some(parent_directory));
}

#[test]
fn extract_skill_parent_directory_from_deep_subdirectory() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };
    let parent_directory = home_dir
        .join("repo")
        .join("a")
        .join("b")
        .join("c")
        .join("d");
    let skill_path = parent_directory
        .join(".claude")
        .join("skills")
        .join("test-skill")
        .join("SKILL.md");
    let result = extract_skill_parent_directory(&skill_path);
    assert_eq!(
        result.ok(),
        Some(parent_directory),
        "Failed for path: {}",
        skill_path.display()
    );
}

#[test]
fn extract_skill_parent_directory_different_providers() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };
    let repo = home_dir.join("repo");
    let providers = [".warp", ".claude", ".codex", ".cursor", ".gemini"];
    for provider in providers {
        let path = repo
            .join(provider)
            .join("skills")
            .join("s")
            .join("SKILL.md");
        let result = extract_skill_parent_directory(&path);
        assert_eq!(
            result.ok(),
            Some(repo.clone()),
            "Failed for path: {}",
            path.display()
        );
    }
}

#[test]
fn extract_skill_parent_directory_returns_none_for_non_skill() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };

    // Not a SKILL.md file
    let path = home_dir
        .join("repo")
        .join(".agents")
        .join("skills")
        .join("my-skill")
        .join("README.md");
    assert_eq!(extract_skill_parent_directory(&path).ok(), None);

    // Wrong structure (skill directly in skills dir)
    let path = home_dir
        .join("repo")
        .join(".agents")
        .join("skills")
        .join("SKILL.md");
    assert_eq!(extract_skill_parent_directory(&path).ok(), None);

    // Too deeply nested
    let path = home_dir
        .join("repo")
        .join(".agents")
        .join("skills")
        .join("a")
        .join("b")
        .join("SKILL.md");
    assert_eq!(extract_skill_parent_directory(&path).ok(), None);

    // Not in a skills directory
    let path = home_dir.join("repo").join("src").join("SKILL.md");
    assert_eq!(extract_skill_parent_directory(&path).ok(), None);
}

// ============================================================================
// Tests for is_home_skill_directory
// ============================================================================

#[test]
fn is_home_skill_directory_true_for_home_skill_dir() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };

    // ~/.agents/skills/skill-name
    let path = home_dir.join(".agents").join("skills").join("my-skill");
    assert!(is_home_skill_directory(&path));

    // ~/.claude/skills/skill-name
    let path = home_dir.join(".claude").join("skills").join("test-skill");
    assert!(is_home_skill_directory(&path));
}

#[test]
fn is_home_skill_directory_false_for_project_skill_dir() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };

    // ~/repos/project/.agents/skills/my-skill is a project skill dir, not home
    let path = home_dir
        .join("repos")
        .join("project")
        .join(".agents")
        .join("skills")
        .join("my-skill");
    assert!(!is_home_skill_directory(&path));
}

#[test]
fn is_home_skill_directory_false_for_provider_path_itself() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };

    // ~/.agents/skills is the provider path, not a skill directory
    let path = home_dir.join(".agents").join("skills");
    assert!(!is_home_skill_directory(&path));
}

#[test]
fn is_home_skill_directory_false_for_arbitrary_path() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };

    let path = home_dir.join("some").join("random").join("dir");
    assert!(!is_home_skill_directory(&path));
}

// ============================================================================
// Tests for is_home_provider_path
// ============================================================================

#[test]
fn is_home_provider_path_true_for_known_providers() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };

    let path = home_dir.join(".agents").join("skills");
    assert!(is_home_provider_path(&path));

    if let Some(path) = warp_core::paths::warp_home_skills_dir() {
        assert!(is_home_provider_path(&path));
    }

    let path = home_dir.join(".claude").join("skills");
    assert!(is_home_provider_path(&path));

    let path = home_dir.join(".codex").join("skills");
    assert!(is_home_provider_path(&path));

    let path = home_dir.join(".cursor").join("skills");
    assert!(is_home_provider_path(&path));

    let path = home_dir.join(".gemini").join("skills");
    assert!(is_home_provider_path(&path));
}

#[test]
fn extract_skill_parent_directory_returns_home_dir_for_warp_home_skill() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };
    let Some(warp_home_skills_dir) = warp_core::paths::warp_home_skills_dir() else {
        eprintln!("Skipping test: Warp home skills directory not available");
        return;
    };

    let skill_path = warp_home_skills_dir.join("test-skill").join("SKILL.md");
    let result = extract_skill_parent_directory(&skill_path);
    assert_eq!(result.ok(), Some(home_dir));
}

#[test]
fn is_home_provider_path_false_for_unknown_provider() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };

    let path = home_dir.join(".garbage").join("skills");
    assert!(!is_home_provider_path(&path));
}

#[test]
fn is_home_provider_path_false_for_project_provider() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };

    // Project-level provider path, not home
    let path = home_dir
        .join("repos")
        .join("project")
        .join(".agents")
        .join("skills");
    assert!(!is_home_provider_path(&path));
}

#[test]
fn is_home_provider_path_false_for_partial_path() {
    let Some(home_dir) = dirs::home_dir() else {
        eprintln!("Skipping test: home directory not available");
        return;
    };

    // Just the provider directory, not the skills subdirectory
    let path = home_dir.join(".agents");
    assert!(!is_home_provider_path(&path));

    // Just home
    assert!(!is_home_provider_path(&home_dir));
}

// ============================================================================
// Tests for find_skill_files_in_tree
// ============================================================================

#[test]
fn find_skill_files_in_tree_finds_root_skills() {
    VirtualFS::test("find_root_skills", |dirs, mut vfs| {
        let repo = dirs.tests().join("repo");

        vfs.mkdir("repo/.agents/skills/root-skill-1")
            .mkdir("repo/.claude/skills/root-skill-2")
            .with_files(vec![
                Stub::FileWithContent(
                    "repo/.agents/skills/root-skill-1/SKILL.md",
                    "---\nname: root-skill-1\ndescription: test\n---\n# root-skill-1",
                ),
                Stub::FileWithContent(
                    "repo/.claude/skills/root-skill-2/SKILL.md",
                    "---\nname: root-skill-2\ndescription: test\n---\n# root-skill-2",
                ),
            ]);

        let skill1_file = Entry::File(FileMetadata::new(
            repo.join(".agents/skills/root-skill-1/SKILL.md"),
            false,
        ));
        let skill1_dir = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".agents/skills/root-skill-1"),
            )
            .unwrap(),
            children: vec![skill1_file],
            ignored: false,
            loaded: true,
        });
        let warp_skills = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".agents/skills"),
            )
            .unwrap(),
            children: vec![skill1_dir],
            ignored: false,
            loaded: true,
        });
        let warp_dir = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".agents"),
            )
            .unwrap(),
            children: vec![warp_skills],
            ignored: false,
            loaded: true,
        });

        let skill2_file = Entry::File(FileMetadata::new(
            repo.join(".claude/skills/root-skill-2/SKILL.md"),
            false,
        ));
        let skill2_dir = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".claude/skills/root-skill-2"),
            )
            .unwrap(),
            children: vec![skill2_file],
            ignored: false,
            loaded: true,
        });
        let claude_skills = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".claude/skills"),
            )
            .unwrap(),
            children: vec![skill2_dir],
            ignored: false,
            loaded: true,
        });
        let claude_dir = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".claude"),
            )
            .unwrap(),
            children: vec![claude_skills],
            ignored: false,
            loaded: true,
        });

        let root = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(&repo).unwrap(),
            children: vec![warp_dir, claude_dir],
            ignored: false,
            loaded: true,
        });

        App::test((), |mut app| async move {
            let watcher = app.add_singleton_model(DirectoryWatcher::new);
            app.add_singleton_model(|_| DetectedRepositories::default());
            let repo_handle = watcher.update(&mut app, |w, ctx| {
                w.add_directory(
                    warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo)
                        .unwrap(),
                    ctx,
                )
                .unwrap()
            });
            let state = FileTreeState::new(root, vec![], Some(repo_handle));

            let model_handle = app.add_singleton_model(RepoMetadataModel::new);
            model_handle.update(&mut app, |model, ctx| {
                let key =
                    warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo)
                        .unwrap();
                model.insert_test_state(key, state, ctx);
            });

            model_handle.read(&app, |model, ctx| {
                let repo_id = RepositoryIdentifier::try_local(&repo).unwrap();
                let skill_files = find_skill_files_in_tree(&repo_id, model, ctx);
                assert_eq!(skill_files.len(), 2);
                assert!(skill_files.contains(&LocalOrRemotePath::Local(
                    repo.join(".agents/skills/root-skill-1/SKILL.md")
                )));
                assert!(skill_files.contains(&LocalOrRemotePath::Local(
                    repo.join(".claude/skills/root-skill-2/SKILL.md")
                )));

                let local_skill_files = skill_files
                    .into_iter()
                    .filter_map(|path| path.to_local_path().map(Path::to_path_buf));
                let skills = read_skills_from_files(local_skill_files);
                assert_eq!(skills.len(), 2);
                let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
                assert!(names.contains(&"root-skill-1"));
                assert!(names.contains(&"root-skill-2"));
            });
        });
    });
}

#[test]
fn find_skill_files_in_tree_finds_subdirectory_skills() {
    VirtualFS::test("find_subdir_skills", |dirs, mut vfs| {
        let repo = dirs.tests().join("repo");

        vfs.mkdir("repo/.agents/skills/root-skill")
            .mkdir("repo/packages/frontend/.agents/skills/frontend-skill")
            .with_files(vec![
                Stub::FileWithContent(
                    "repo/.agents/skills/root-skill/SKILL.md",
                    "---\nname: root-skill\ndescription: test\n---\n# root-skill",
                ),
                Stub::FileWithContent(
                    "repo/packages/frontend/.agents/skills/frontend-skill/SKILL.md",
                    "---\nname: frontend-skill\ndescription: test\n---\n# frontend-skill",
                ),
            ]);

        let root_skill_file = Entry::File(FileMetadata::new(
            repo.join(".agents/skills/root-skill/SKILL.md"),
            false,
        ));
        let root_skill_dir = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".agents/skills/root-skill"),
            )
            .unwrap(),
            children: vec![root_skill_file],
            ignored: false,
            loaded: true,
        });
        let root_warp_skills = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".agents/skills"),
            )
            .unwrap(),
            children: vec![root_skill_dir],
            ignored: false,
            loaded: true,
        });
        let root_warp = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join(".agents"),
            )
            .unwrap(),
            children: vec![root_warp_skills],
            ignored: false,
            loaded: true,
        });

        let frontend_skill_file = Entry::File(FileMetadata::new(
            repo.join("packages/frontend/.agents/skills/frontend-skill/SKILL.md"),
            false,
        ));
        let frontend_skill_dir = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join("packages/frontend/.agents/skills/frontend-skill"),
            )
            .unwrap(),
            children: vec![frontend_skill_file],
            ignored: false,
            loaded: true,
        });
        let frontend_warp_skills = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join("packages/frontend/.agents/skills"),
            )
            .unwrap(),
            children: vec![frontend_skill_dir],
            ignored: false,
            loaded: true,
        });
        let frontend_warp = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join("packages/frontend/.agents"),
            )
            .unwrap(),
            children: vec![frontend_warp_skills],
            ignored: false,
            loaded: true,
        });
        let frontend = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join("packages/frontend"),
            )
            .unwrap(),
            children: vec![frontend_warp],
            ignored: false,
            loaded: true,
        });
        let packages = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &repo.join("packages"),
            )
            .unwrap(),
            children: vec![frontend],
            ignored: false,
            loaded: true,
        });

        let root = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(&repo).unwrap(),
            children: vec![root_warp, packages],
            ignored: false,
            loaded: true,
        });

        App::test((), |mut app| async move {
            let watcher = app.add_singleton_model(DirectoryWatcher::new);
            app.add_singleton_model(|_| DetectedRepositories::default());
            let repo_handle = watcher.update(&mut app, |w, ctx| {
                w.add_directory(
                    warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo)
                        .unwrap(),
                    ctx,
                )
                .unwrap()
            });
            let state = FileTreeState::new(root, vec![], Some(repo_handle));

            let model_handle = app.add_singleton_model(RepoMetadataModel::new);
            model_handle.update(&mut app, |model, ctx| {
                let key =
                    warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo)
                        .unwrap();
                model.insert_test_state(key, state, ctx);
            });

            model_handle.read(&app, |model, ctx| {
                let repo_id = RepositoryIdentifier::try_local(&repo).unwrap();
                let skill_files = find_skill_files_in_tree(&repo_id, model, ctx);
                assert_eq!(skill_files.len(), 2);
                assert!(skill_files.contains(&LocalOrRemotePath::Local(
                    repo.join(".agents/skills/root-skill/SKILL.md")
                )));
                assert!(skill_files.contains(&LocalOrRemotePath::Local(
                    repo.join("packages/frontend/.agents/skills/frontend-skill/SKILL.md")
                )));

                let local_skill_files = skill_files
                    .into_iter()
                    .filter_map(|path| path.to_local_path().map(Path::to_path_buf));
                let skills = read_skills_from_files(local_skill_files);
                assert_eq!(skills.len(), 2);
                let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
                assert!(names.contains(&"root-skill"));
                assert!(names.contains(&"frontend-skill"));
            });
        });
    });
}

#[test]
fn find_skill_files_in_tree_returns_remote_skill_paths_for_remote_repos() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| DetectedRepositories::default());
        let model_handle = app.add_singleton_model(RepoMetadataModel::new);
        let host_id = HostId::new("test-host".to_string());
        let repo_path = StandardizedPath::try_new("/repo").unwrap();
        let skill_path =
            StandardizedPath::try_new("/repo/.agents/skills/remote-skill/SKILL.md").unwrap();
        let repo_id =
            RepositoryIdentifier::Remote(RemotePath::new(host_id.clone(), repo_path.clone()));

        let update = RepoMetadataUpdate {
            repo_path: repo_path.clone(),
            remove_entries: vec![],
            update_entries: vec![FileTreeEntryUpdate {
                parent_path_to_replace: repo_path.clone(),
                subtree_metadata: vec![
                    RepoNodeMetadata::Directory(DirectoryNodeMetadata {
                        path: StandardizedPath::try_new("/repo/.agents").unwrap(),
                        ignored: false,
                        loaded: true,
                    }),
                    RepoNodeMetadata::Directory(DirectoryNodeMetadata {
                        path: StandardizedPath::try_new("/repo/.agents/skills").unwrap(),
                        ignored: false,
                        loaded: true,
                    }),
                    RepoNodeMetadata::Directory(DirectoryNodeMetadata {
                        path: StandardizedPath::try_new("/repo/.agents/skills/remote-skill")
                            .unwrap(),
                        ignored: false,
                        loaded: true,
                    }),
                    RepoNodeMetadata::File(FileNodeMetadata {
                        path: skill_path.clone(),
                        extension: Some("md".to_string()),
                        ignored: false,
                    }),
                ],
            }],
        };

        model_handle.update(&mut app, |model, ctx| {
            model.insert_remote_snapshot(host_id.clone(), &update, ctx);
        });

        model_handle.read(&app, |model, ctx| {
            let skill_files = find_skill_files_in_tree(&repo_id, model, ctx);
            assert_eq!(
                skill_files,
                vec![LocalOrRemotePath::Remote(RemotePath::new(
                    host_id, skill_path
                ))]
            );
        });
    });
}

#[test]
fn find_skill_files_in_tree_excludes_ignored_skill_files() {
    VirtualFS::test("find_skills_ignored", |dirs, mut vfs| {
        let repo = dirs.tests().join("repo");
        vfs.mkdir("repo/.agents/skills/ignored-skill")
            .with_files(vec![Stub::FileWithContent(
                "repo/.agents/skills/ignored-skill/SKILL.md",
                "name: ignored-skill",
            )]);

        let skill_file = Entry::File(FileMetadata::new(
            repo.join(".agents/skills/ignored-skill/SKILL.md"),
            true,
        ));
        let skill_dir = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_from_local(&repo.join(".agents/skills/ignored-skill"))
                .unwrap(),
            children: vec![skill_file],
            ignored: true,
            loaded: true,
        });
        let skills_dir = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_from_local(&repo.join(".agents/skills")).unwrap(),
            children: vec![skill_dir],
            ignored: true,
            loaded: true,
        });
        let agents_dir = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_from_local(&repo.join(".agents")).unwrap(),
            children: vec![skills_dir],
            ignored: true,
            loaded: true,
        });
        let root = Entry::Directory(DirectoryEntry {
            path: StandardizedPath::try_from_local(&repo).unwrap(),
            children: vec![agents_dir],
            ignored: false,
            loaded: true,
        });

        App::test((), |mut app| async move {
            let watcher = app.add_singleton_model(DirectoryWatcher::new);
            app.add_singleton_model(|_| DetectedRepositories::default());
            let repo_handle = watcher.update(&mut app, |w, ctx| {
                w.add_directory(
                    StandardizedPath::from_local_canonicalized(&repo).unwrap(),
                    ctx,
                )
                .unwrap()
            });
            let state = FileTreeState::new(root, vec![], Some(repo_handle));

            let model_handle = app.add_singleton_model(RepoMetadataModel::new);
            model_handle.update(&mut app, |model, ctx| {
                let key = StandardizedPath::from_local_canonicalized(&repo).unwrap();
                model.insert_test_state(key, state, ctx);
            });

            model_handle.read(&app, |model, ctx| {
                let repo_id = RepositoryIdentifier::try_local(&repo).unwrap();
                assert!(find_skill_files_in_tree(&repo_id, model, ctx).is_empty());
            });
        });
    });
}
#[test]
fn find_skill_files_in_tree_empty_repo() {
    VirtualFS::test("find_skills_empty", |dirs, mut vfs| {
        let repo = dirs.tests().join("repo");
        vfs.mkdir("repo/src");

        let src = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(&repo.join("src"))
                .unwrap(),
            children: vec![],
            ignored: false,
            loaded: true,
        });
        let root = Entry::Directory(DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(&repo).unwrap(),
            children: vec![src],
            ignored: false,
            loaded: true,
        });

        App::test((), |mut app| async move {
            let watcher = app.add_singleton_model(DirectoryWatcher::new);
            app.add_singleton_model(|_| DetectedRepositories::default());
            let repo_handle = watcher.update(&mut app, |w, ctx| {
                w.add_directory(
                    warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo)
                        .unwrap(),
                    ctx,
                )
                .unwrap()
            });
            let state = FileTreeState::new(root, vec![], Some(repo_handle));

            let model_handle = app.add_singleton_model(RepoMetadataModel::new);
            model_handle.update(&mut app, |model, ctx| {
                let key =
                    warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo)
                        .unwrap();
                model.insert_test_state(key, state, ctx);
            });

            model_handle.read(&app, |model, ctx| {
                let repo_id = RepositoryIdentifier::try_local(&repo).unwrap();
                let skill_files = find_skill_files_in_tree(&repo_id, model, ctx);
                assert!(skill_files.is_empty());
            });
        });
    });
}
